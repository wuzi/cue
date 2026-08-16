use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration as StdDuration,
};

use adw::prelude::*;
use chrono::{Datelike, Local, NaiveDate, Timelike, Utc};
use gio::prelude::ActionMapExt;
use glib::variant::{FromVariant, StaticVariantType};
use uuid::Uuid;

use crate::{
    canvas::{
        escape_message, format_schedule_suffix, normalize_registered_suffix,
        normalize_stored_working_suffix,
    },
    grouping::{ReminderGroup, group_active_reminders},
    model::{CanvasItem, CanvasSchedule, DeletedCanvasItem, Reminder, ReminderError},
    repository::RepositoryError,
    schedule::{
        DaySpec, ScheduleError, ScheduleExpression, ScheduleParseError, ScheduleParseStatus,
        parse_english,
    },
    service::{ReminderService, ServiceError},
    time_utils::{ClockFormat, default_due_time, format_clock_time, resolve_local_datetime},
};

use super::{
    UiBuildError, WindowWidgets, build_window,
    canvas::CanvasEditor,
    rows,
    schedule_picker::{PickerSelection, SchedulePicker},
};

struct CanvasSlot {
    editor: Rc<CanvasEditor>,
    item: Option<CanvasItem>,
    committed_text: String,
}

struct SuggestionMenu {
    popover: gtk::Popover,
    list: gtk::ListBox,
    target: Option<Uuid>,
    anchor_tick: gtk::TickCallbackId,
}

const SUGGESTIONS: [Option<&str>; 6] = [
    Some("in 15 minutes"),
    Some("in 30 minutes"),
    Some("in 1 hour"),
    Some("tomorrow 9am"),
    Some("next Monday 9am"),
    None,
];

pub struct MainWindow {
    widgets: WindowWidgets,
    service: Rc<ReminderService>,
    slots: RefCell<Vec<CanvasSlot>>,
    autosaves: RefCell<HashMap<String, glib::SourceId>>,
    reminder_to_focus: Cell<Option<Uuid>>,
    suggestions: RefCell<Option<SuggestionMenu>>,
    custom_popover: RefCell<Option<Rc<SchedulePicker>>>,
    style_signal_ids: RefCell<Vec<glib::SignalHandlerId>>,
    on_mutation: Box<dyn Fn()>,
    on_closed: Box<dyn Fn()>,
}

impl MainWindow {
    pub fn new(
        application: &adw::Application,
        service: Rc<ReminderService>,
        on_mutation: impl Fn() + 'static,
        on_closed: impl Fn() + 'static,
    ) -> Result<Rc<Self>, UiBuildError> {
        let window = Rc::new(Self {
            widgets: build_window(application)?,
            service,
            slots: RefCell::new(Vec::new()),
            autosaves: RefCell::new(HashMap::new()),
            reminder_to_focus: Cell::new(None),
            suggestions: RefCell::new(None),
            custom_popover: RefCell::new(None),
            style_signal_ids: RefCell::new(Vec::new()),
            on_mutation: Box::new(on_mutation),
            on_closed: Box::new(on_closed),
        });
        window.install_menu();
        window.install_window_actions();
        window.connect_signals();
        window.connect_appearance();
        window.refresh();
        Ok(window)
    }

    pub fn present(&self) {
        self.widgets.window.present();
        if self.widgets.navigation_view.visible_page_tag().as_deref() == Some("canvas") {
            self.focus_after_rebuild(self.focused_canvas_position(), true);
        }
    }

    pub fn widget(&self) -> &adw::ApplicationWindow {
        &self.widgets.window
    }

    pub fn show_error_message(&self, message: &str) {
        self.show_error(message);
    }

    pub fn show_reminder(self: &Rc<Self>, id: Uuid) {
        if !self.flush_canvas() {
            return;
        }
        self.reminder_to_focus.set(Some(id));
        while self.widgets.navigation_view.visible_page_tag().as_deref() != Some("canvas")
            && self.widgets.navigation_view.pop()
        {}
        self.refresh_views(None);
        self.widgets.window.present();
    }

    pub fn refresh(self: &Rc<Self>) {
        let focus = self.focused_canvas_position();
        if !self.flush_canvas() {
            return;
        }
        self.refresh_views(focus);
    }

    fn refresh_views(self: &Rc<Self>, focus: Option<(Option<Uuid>, i32)>) {
        self.refresh_canvas(focus);
        match self.service.list_active() {
            Ok(reminders) => self.rebuild_active(reminders),
            Err(error) => self.show_error(&error.to_string()),
        }
        match self.service.list_history() {
            Ok(history) => self.rebuild_history(history),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    pub fn confirm_quit(&self, application: &adw::Application) {
        if !self.flush_canvas() {
            return;
        }
        match self.service.should_hold_background() {
            Ok(false) => {
                application.quit();
                return;
            }
            Ok(true) => {}
            Err(error) => {
                self.show_error(&error.to_string());
                return;
            }
        }
        let dialog = adw::AlertDialog::new(
            Some(&gettextrs::gettext("Quit with reminders pending?")),
            Some(&gettextrs::gettext(
                "Reminders cannot notify you after the application quits.",
            )),
        );
        dialog.add_response("cancel", &gettextrs::gettext("Cancel"));
        dialog.add_response("quit", &gettextrs::gettext("Quit"));
        dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let application = application.clone();
        dialog.connect_response(Some("quit"), move |_, _| application.quit());
        dialog.present(Some(&self.widgets.window));
    }

    fn install_menu(&self) {
        let menu = gio::Menu::new();
        menu.append(
            Some(&gettextrs::gettext("Active Reminders")),
            Some("win.show-active-list"),
        );
        menu.append(
            Some(&gettextrs::gettext("History")),
            Some("win.show-history"),
        );
        menu.append(
            Some(&gettextrs::gettext("About Remind Me")),
            Some("app.about"),
        );
        menu.append(Some(&gettextrs::gettext("Quit")), Some("app.quit"));
        self.widgets.menu_button.set_menu_model(Some(&menu));
    }

    fn install_window_actions(self: &Rc<Self>) {
        self.add_uuid_action("complete", |window, id| window.complete(id));
        self.add_uuid_action("snooze", |window, id| window.snooze(id));
        self.add_uuid_action("edit", |window, id| window.show_edit_dialog(id));
        self.add_uuid_action("delete", |window, id| window.delete_reminder_with_undo(id));
        self.add_uuid_action("delete-canvas", |window, id| {
            window.delete_canvas_with_undo(id);
        });
        self.add_uuid_action("custom-time", |window, id| {
            if let Some(editor) = window.editor_for_entry(Some(id)) {
                window.show_custom_when_popover(editor);
            }
        });

        for (name, tag) in [
            ("show-active-list", "active-list"),
            ("show-history", "history"),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(window) = weak.upgrade()
                    && window.widgets.navigation_view.visible_page_tag().as_deref() != Some(tag)
                {
                    window.widgets.navigation_view.push_by_tag(tag);
                }
            });
            self.widgets.window.add_action(&action);
        }

        let commit = gio::SimpleAction::new("commit-canvas", None);
        let weak = Rc::downgrade(self);
        commit.connect_activate(move |_, _| {
            if let Some(window) = weak.upgrade() {
                window.commit_focused();
            }
        });
        self.widgets.window.add_action(&commit);
    }

    fn add_uuid_action(self: &Rc<Self>, name: &str, callback: impl Fn(&Rc<Self>, Uuid) + 'static) {
        let action = gio::SimpleAction::new(name, Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else { return };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            if let Ok(id) = Uuid::parse_str(&target) {
                callback(&window, id);
            }
        });
        self.widgets.window.add_action(&action);
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.widgets.active_list_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.widgets.navigation_view.push_by_tag("active-list");
            }
        });
        let weak = Rc::downgrade(self);
        self.widgets.clear_history_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.confirm_clear_history();
            }
        });
        let weak = Rc::downgrade(self);
        self.widgets.window.connect_close_request(move |_| {
            if let Some(window) = weak.upgrade() {
                if !window.flush_canvas() {
                    return glib::Propagation::Stop;
                }
                (window.on_closed)();
            }
            glib::Propagation::Proceed
        });
    }

    fn connect_appearance(self: &Rc<Self>) {
        let manager = adw::StyleManager::default();
        let weak = Rc::downgrade(self);
        let accent = manager.connect_accent_color_notify(move |manager| {
            if let Some(window) = weak.upgrade() {
                window.update_canvas_accents(manager);
            }
        });
        let weak = Rc::downgrade(self);
        let dark = manager.connect_dark_notify(move |manager| {
            if let Some(window) = weak.upgrade() {
                window.update_canvas_accents(manager);
            }
        });
        self.style_signal_ids.borrow_mut().extend([accent, dark]);
    }

    fn update_canvas_accents(&self, manager: &adw::StyleManager) {
        for slot in self.slots.borrow().iter() {
            slot.editor.update_accent(manager);
        }
    }

    fn refresh_canvas(self: &Rc<Self>, focus: Option<(Option<Uuid>, i32)>) {
        let items = match self.service.list_canvas() {
            Ok(items) => items,
            Err(error) => {
                self.show_error(&error.to_string());
                return;
            }
        };
        if self.sync_canvas(&items) {
            self.focus_after_rebuild(focus, false);
        } else {
            self.rebuild_canvas(items, focus);
        }
    }

    fn sync_canvas(self: &Rc<Self>, items: &[CanvasItem]) -> bool {
        let same_structure = {
            let slots = self.slots.borrow();
            slots.len() == items.len() + 1
                && slots.iter().zip(items).all(|(slot, item)| {
                    slot.editor.entry_id == Some(item.entry.id)
                        && slot.editor.reminder_id == item.entry.reminder_id
                })
        };
        if !same_structure {
            return false;
        }

        let now = self.service.now();
        let clock_format = system_clock_format();
        let mut updates = Vec::with_capacity(items.len());
        {
            let mut slots = self.slots.borrow_mut();
            for (slot, item) in slots.iter_mut().zip(items.iter().cloned()) {
                let old_text = slot.editor.text();
                let old_committed = slot.committed_text.clone();
                let old_suffix = slot.editor.committed_suffix();
                let suffix = item.reminder.as_ref().map(|reminder| {
                    format_schedule_suffix(reminder.due_at, now, &Local, clock_format)
                });
                let committed_text = committed_canvas_text(&item, suffix.as_deref());
                let visible_text = item.entry.working_text.as_deref().map_or_else(
                    || committed_text.clone(),
                    |working| {
                        if old_text != old_committed {
                            match (old_suffix.as_deref(), suffix.as_deref()) {
                                (Some(previous), Some(current)) => {
                                    normalize_registered_suffix(&old_text, previous, current)
                                }
                                _ => old_text.clone(),
                            }
                        } else {
                            item.reminder.as_ref().map_or_else(
                                || working.to_owned(),
                                |reminder| {
                                    normalize_stored_working_suffix(
                                        working,
                                        reminder.due_at,
                                        item.entry.updated_at,
                                        now,
                                        &Local,
                                        clock_format,
                                    )
                                },
                            )
                        }
                    },
                );
                let needs_persist = item
                    .entry
                    .working_text
                    .as_deref()
                    .is_some_and(|working| working != visible_text);
                slot.item = Some(item);
                slot.committed_text = committed_text;
                slot.editor.set_committed_suffix(suffix);
                updates.push((slot.editor.clone(), visible_text, needs_persist));
            }
        }

        for (editor, text, needs_persist) in updates {
            if editor.text() != text {
                let cursor = editor.input.buffer().cursor_position();
                editor.input.buffer().set_text(&text);
                let buffer = editor.input.buffer();
                let cursor = cursor.clamp(0, buffer.char_count());
                buffer.place_cursor(&buffer.iter_at_offset(cursor));
            }
            self.update_editor(&editor);
            if needs_persist {
                let _ = self.flush_editor(&editor);
            }
        }
        true
    }

    fn rebuild_canvas(self: &Rc<Self>, items: Vec<CanvasItem>, focus: Option<(Option<Uuid>, i32)>) {
        self.cancel_autosaves();
        self.dismiss_suggestions();
        self.dismiss_custom_popover();
        clear_box(&self.widgets.canvas_entries);
        self.slots.borrow_mut().clear();
        for item in items {
            let suffix = item.reminder.as_ref().map(|reminder| {
                format_schedule_suffix(
                    reminder.due_at,
                    self.service.now(),
                    &Local,
                    system_clock_format(),
                )
            });
            let committed_text = committed_canvas_text(&item, suffix.as_deref());
            let text = item.entry.working_text.as_deref().map_or_else(
                || committed_text.clone(),
                |working| {
                    item.reminder.as_ref().map_or_else(
                        || working.to_owned(),
                        |reminder| {
                            normalize_stored_working_suffix(
                                working,
                                reminder.due_at,
                                item.entry.updated_at,
                                self.service.now(),
                                &Local,
                                system_clock_format(),
                            )
                        },
                    )
                },
            );
            let editor = Rc::new(CanvasEditor::saved(&item, &text, suffix));
            self.widgets.canvas_entries.append(&editor.root);
            self.slots.borrow_mut().push(CanvasSlot {
                editor: editor.clone(),
                item: Some(item),
                committed_text,
            });
            self.connect_editor(editor);
        }
        let draft_text = self.service.load_canvas_draft().unwrap_or_else(|error| {
            self.show_error(&error.to_string());
            String::new()
        });
        let draft = Rc::new(CanvasEditor::draft(&draft_text));
        self.widgets.canvas_entries.append(&draft.root);
        self.slots.borrow_mut().push(CanvasSlot {
            editor: draft.clone(),
            item: None,
            committed_text: String::new(),
        });
        self.connect_editor(draft);
        self.focus_after_rebuild(focus, true);
    }

    fn connect_editor(self: &Rc<Self>, editor: Rc<CanvasEditor>) {
        self.update_editor(&editor);
        let weak = Rc::downgrade(self);
        let changed_editor = Rc::downgrade(&editor);
        editor.input.buffer().connect_changed(move |_| {
            if let (Some(window), Some(changed_editor)) = (weak.upgrade(), changed_editor.upgrade())
            {
                window.update_editor(&changed_editor);
                window.schedule_autosave(changed_editor.clone());
            }
        });

        let weak = Rc::downgrade(self);
        let cursor_editor = Rc::downgrade(&editor);
        editor
            .input
            .buffer()
            .connect_cursor_position_notify(move |_| {
                if let (Some(window), Some(cursor_editor)) =
                    (weak.upgrade(), cursor_editor.upgrade())
                {
                    window.update_suggestion_anchor(&cursor_editor);
                }
            });

        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        let keyed_editor = Rc::downgrade(&editor);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let (Some(window), Some(keyed_editor)) = (weak.upgrade(), keyed_editor.upgrade())
            else {
                return glib::Propagation::Proceed;
            };
            match key {
                gtk::gdk::Key::Up if window.suggestions_for(&keyed_editor) => {
                    window.move_suggestion(-1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Down if window.suggestions_for(&keyed_editor) => {
                    window.move_suggestion(1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Escape if window.suggestions_for(&keyed_editor) => {
                    window.dismiss_suggestions();
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Escape if keyed_editor.entry_id.is_some() => {
                    window.revert_editor(&keyed_editor);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Menu => {
                    if keyed_editor.open_actions() {
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gtk::gdk::Key::F10 if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                    if keyed_editor.open_actions() {
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter
                    if !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) =>
                {
                    if window.suggestions_for(&keyed_editor) {
                        window.accept_selected_suggestion();
                    } else {
                        window.commit_editor(&keyed_editor);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        editor.input.add_controller(keys);

        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        let focused_editor = Rc::downgrade(&editor);
        focus.connect_leave(move |_| {
            if let (Some(window), Some(focused_editor)) = (weak.upgrade(), focused_editor.upgrade())
            {
                let _ = window.flush_editor(&focused_editor);
                glib::idle_add_local_once(move || {
                    if !window.editor_is_focused(&focused_editor)
                        && window.suggestions_for(&focused_editor)
                    {
                        window.dismiss_suggestions();
                    }
                });
            }
        });
        editor.input.add_controller(focus);

        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        let clicked_editor = Rc::downgrade(&editor);
        click.connect_released(move |gesture, _, x, y| {
            if gesture.current_button() != 1 {
                return;
            }
            let (Some(window), Some(clicked_editor)) = (weak.upgrade(), clicked_editor.upgrade())
            else {
                return;
            };
            let Some(iter) = clicked_editor.iter_at_widget_location(x as i32, y as i32) else {
                return;
            };
            let text = clicked_editor.text();
            let parsed = parse_english(&text);
            let Some(span) = parsed.schedule_span else {
                return;
            };
            let start = text[..span.start].chars().count() as i32;
            let end = text[..span.end].chars().count() as i32;
            if (start..=end).contains(&iter.offset()) {
                window.show_custom_when_popover_at(clicked_editor.clone(), Some(iter.offset()));
            }
        });
        editor.input.add_controller(click);
    }

    fn update_editor(self: &Rc<Self>, editor: &Rc<CanvasEditor>) {
        let text = editor.text();
        let parsed = parse_english(&text);
        let committed = self
            .slot(editor.entry_id)
            .is_some_and(|slot| slot.committed_text == text);
        let keeps_existing = keeps_existing_suffix(editor, &text, parsed.schedule_span.as_ref());
        editor.apply_schedule_span(
            &text,
            parsed.schedule_span.clone(),
            committed && editor.reminder_id.is_some(),
        );
        let error = if committed {
            None
        } else if parsed.message.chars().count() > 280 {
            Some(gettextrs::gettext(
                "Notes and reminder messages can contain at most 280 characters",
            ))
        } else {
            match &parsed.status {
                ScheduleParseStatus::Default => None,
                ScheduleParseStatus::Valid(_) if keeps_existing => None,
                ScheduleParseStatus::Valid(schedule) => self
                    .service
                    .preview_schedule(schedule, &Local)
                    .err()
                    .map(|error| localized_service_error(&error)),
                ScheduleParseStatus::Partial => {
                    Some(gettextrs::gettext("Finish the schedule after @"))
                }
                ScheduleParseStatus::Invalid(error) => Some(localized_schedule_parse_error(*error)),
            }
        };
        editor.set_error(error.as_deref());
        let committed_suffix = editor.committed_suffix();
        editor.set_dirty_registered(!committed, committed_suffix.as_deref().unwrap_or_default());
        if matches!(parsed.status, ScheduleParseStatus::Partial) && self.editor_is_focused(editor) {
            self.show_suggestions(editor.clone());
        } else if self.suggestions_for(editor) {
            self.dismiss_suggestions();
        }
    }

    fn commit_focused(self: &Rc<Self>) {
        let focused = gtk::prelude::GtkWindowExt::focus(&self.widgets.window);
        let editor = self
            .slots
            .borrow()
            .iter()
            .find(|slot| {
                focused
                    .as_ref()
                    .is_some_and(|focused| slot.editor.input.upcast_ref::<gtk::Widget>() == focused)
            })
            .map(|slot| slot.editor.clone())
            .or_else(|| self.editor_for_entry(None));
        if let Some(editor) = editor {
            self.commit_editor(&editor);
        }
    }

    fn commit_editor(self: &Rc<Self>, editor: &Rc<CanvasEditor>) {
        self.dismiss_suggestions();
        let text = editor.text();
        let parsed = parse_english(&text);
        if parsed.message.chars().count() > 280 {
            editor.set_error(Some(&gettextrs::gettext(
                "Notes and reminder messages can contain at most 280 characters",
            )));
            return;
        }
        let keeps_existing = keeps_existing_suffix(editor, &text, parsed.schedule_span.as_ref());
        let schedule = match parsed.status {
            ScheduleParseStatus::Default => CanvasSchedule::None,
            ScheduleParseStatus::Valid(_) if keeps_existing => CanvasSchedule::KeepExisting,
            ScheduleParseStatus::Valid(expression) => CanvasSchedule::Replace(expression),
            ScheduleParseStatus::Partial => {
                editor.set_error(Some(&gettextrs::gettext("Finish the schedule after @")));
                return;
            }
            ScheduleParseStatus::Invalid(error) => {
                editor.set_error(Some(&localized_schedule_parse_error(error)));
                return;
            }
        };
        if parsed.message.trim().is_empty() {
            if matches!(schedule, CanvasSchedule::None) {
                if let Some(entry_id) = editor.entry_id {
                    self.delete_canvas_with_undo(entry_id);
                }
            } else {
                editor.set_error(Some(&gettextrs::gettext(
                    "Enter a note or reminder message",
                )));
            }
            return;
        }
        if !self.flush_canvas() {
            return;
        }
        let result = if let Some(entry_id) = editor.entry_id {
            self.service
                .commit_canvas_edit(entry_id, parsed.message, schedule, &Local)
        } else {
            self.service
                .commit_canvas_draft(parsed.message, schedule, &Local)
        };
        match result {
            Ok(_) => {
                self.after_mutation();
                self.focus_draft_later();
            }
            Err(error) => editor.set_error(Some(&localized_service_error(&error))),
        }
    }

    fn revert_editor(self: &Rc<Self>, editor: &Rc<CanvasEditor>) {
        let Some(entry_id) = editor.entry_id else {
            return;
        };
        if !self.flush_canvas_except_entry(entry_id) {
            return;
        }
        if let Some(source) = self
            .autosaves
            .borrow_mut()
            .remove(&editor_key(Some(entry_id)))
        {
            source.remove();
        }
        if let Err(error) = self.service.discard_canvas_working_text(entry_id) {
            self.show_error(&error.to_string());
            return;
        }
        self.refresh_views(Some((Some(entry_id), 0)));
    }

    fn schedule_autosave(self: &Rc<Self>, editor: Rc<CanvasEditor>) {
        let key = editor_key(editor.entry_id);
        if let Some(source) = self.autosaves.borrow_mut().remove(&key) {
            source.remove();
        }
        let closure_key = key.clone();
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(StdDuration::from_millis(350), move || {
            if let Some(window) = weak.upgrade() {
                window.autosaves.borrow_mut().remove(&closure_key);
                window.persist_editor(&editor);
            }
        });
        self.autosaves.borrow_mut().insert(key, source);
    }

    fn flush_editor(&self, editor: &Rc<CanvasEditor>) -> bool {
        let key = editor_key(editor.entry_id);
        if let Some(source) = self.autosaves.borrow_mut().remove(&key) {
            source.remove();
        }
        self.persist_editor(editor)
    }

    fn persist_editor(&self, editor: &Rc<CanvasEditor>) -> bool {
        let text = editor.text();
        let result = if let Some(entry_id) = editor.entry_id {
            let committed = self
                .slot(Some(entry_id))
                .map(|slot| slot.committed_text.clone())
                .unwrap_or_default();
            let dirty = text != committed;
            let persisted = if dirty {
                self.normalized_working_text(editor, &text)
            } else {
                text
            };
            self.service
                .save_canvas_working_text(entry_id, dirty.then_some(persisted.as_str()))
        } else {
            self.service.save_canvas_draft(&text)
        };
        match result {
            Ok(()) => true,
            Err(error) => {
                self.show_error(&format!(
                    "{}: {error}",
                    gettextrs::gettext("Could not save canvas changes")
                ));
                false
            }
        }
    }

    fn flush_canvas(&self) -> bool {
        let editors = self
            .slots
            .borrow()
            .iter()
            .map(|slot| slot.editor.clone())
            .collect::<Vec<_>>();
        let mut all_saved = true;
        for editor in editors {
            if !self.flush_editor(&editor) {
                all_saved = false;
            }
        }
        all_saved
    }

    fn flush_canvas_except_entry(&self, excluded: Uuid) -> bool {
        let editors = self
            .slots
            .borrow()
            .iter()
            .filter(|slot| slot.editor.entry_id != Some(excluded))
            .map(|slot| slot.editor.clone())
            .collect::<Vec<_>>();
        let mut all_saved = true;
        for editor in editors {
            if !self.flush_editor(&editor) {
                all_saved = false;
            }
        }
        all_saved
    }

    fn normalized_working_text(&self, editor: &CanvasEditor, text: &str) -> String {
        let previous = editor.committed_suffix();
        let Some(previous) = previous.as_deref() else {
            return text.to_owned();
        };
        let Some(reminder) = self
            .slot(editor.entry_id)
            .and_then(|slot| slot.item.as_ref()?.reminder.clone())
        else {
            return text.to_owned();
        };
        let current = format_schedule_suffix(
            reminder.due_at,
            self.service.now(),
            &Local,
            system_clock_format(),
        );
        normalize_registered_suffix(text, previous, &current)
    }

    fn cancel_autosaves(&self) {
        for (_, source) in self.autosaves.borrow_mut().drain() {
            source.remove();
        }
    }

    fn show_suggestions(self: &Rc<Self>, editor: Rc<CanvasEditor>) {
        if self.suggestions_for(&editor) {
            self.update_suggestion_anchor(&editor);
            return;
        }
        self.dismiss_suggestions();
        let popover = gtk::Popover::new();
        popover.set_parent(&editor.input);
        popover.set_autohide(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_pointing_to(Some(&editor.cursor_anchor_rect()));
        popover.add_css_class("schedule-suggestions");
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("suggestions");
        for choice in SUGGESTIONS {
            let row = gtk::ListBoxRow::new();
            row.set_focusable(false);
            let text = choice
                .map(str::to_owned)
                .unwrap_or_else(|| gettextrs::gettext("Custom…"));
            let label = gtk::Label::new(Some(&text));
            label.set_halign(gtk::Align::Start);
            label.set_margin_top(7);
            label.set_margin_bottom(7);
            label.set_margin_start(12);
            label.set_margin_end(12);
            row.set_child(Some(&label));
            list.append(&row);
        }
        list.select_row(list.row_at_index(0).as_ref());
        popover.set_child(Some(&list));
        let weak = Rc::downgrade(self);
        list.connect_row_activated(move |_, row| {
            if let Some(window) = weak.upgrade() {
                window.accept_suggestion(row.index() as usize);
            }
        });
        popover.popup();
        let weak = Rc::downgrade(self);
        let anchor_editor = Rc::downgrade(&editor);
        let anchor_tick = editor.input.add_tick_callback(move |_, _| {
            let (Some(window), Some(editor)) = (weak.upgrade(), anchor_editor.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            if !window.suggestions_for(&editor) {
                return glib::ControlFlow::Break;
            }
            window.update_suggestion_anchor(&editor);
            glib::ControlFlow::Continue
        });
        *self.suggestions.borrow_mut() = Some(SuggestionMenu {
            popover,
            list,
            target: editor.entry_id,
            anchor_tick,
        });
        editor.input.grab_focus();
    }

    fn update_suggestion_anchor(&self, editor: &CanvasEditor) {
        if let Some(menu) = self.suggestions.borrow().as_ref()
            && menu.target == editor.entry_id
        {
            let anchor = editor.cursor_anchor_rect();
            menu.popover.set_pointing_to(Some(&anchor));
        }
    }

    fn dismiss_suggestions(&self) {
        if let Some(menu) = self.suggestions.borrow_mut().take() {
            menu.anchor_tick.remove();
            menu.popover.popdown();
            menu.popover.unparent();
        }
    }

    fn suggestions_for(&self, editor: &CanvasEditor) -> bool {
        self.suggestions
            .borrow()
            .as_ref()
            .is_some_and(|menu| menu.target == editor.entry_id)
    }

    fn move_suggestion(&self, offset: i32) {
        let suggestions = self.suggestions.borrow();
        let Some(menu) = suggestions.as_ref() else {
            return;
        };
        let current = menu.list.selected_row().map_or(0, |row| row.index());
        let next = (current + offset).clamp(0, SUGGESTIONS.len() as i32 - 1);
        menu.list.select_row(menu.list.row_at_index(next).as_ref());
    }

    fn accept_selected_suggestion(self: &Rc<Self>) {
        let selected = self
            .suggestions
            .borrow()
            .as_ref()
            .and_then(|menu| menu.list.selected_row())
            .map_or(0, |row| row.index() as usize);
        self.accept_suggestion(selected);
    }

    fn accept_suggestion(self: &Rc<Self>, index: usize) {
        let target = self
            .suggestions
            .borrow()
            .as_ref()
            .map(|menu| menu.target)
            .unwrap_or(None);
        self.dismiss_suggestions();
        let Some(editor) = self.editor_for_entry(target) else {
            return;
        };
        match SUGGESTIONS.get(index).copied().flatten() {
            Some(phrase) => self.replace_schedule_text(&editor, phrase),
            None => self.show_custom_when_popover(editor),
        }
    }

    fn replace_schedule_text(&self, editor: &CanvasEditor, phrase: &str) {
        let input = editor.text();
        let parsed = parse_english(&input);
        let message_end = parsed
            .schedule_span
            .as_ref()
            .map_or(input.len(), |span| span.start);
        let message = input[..message_end].trim_end();
        editor
            .input
            .buffer()
            .set_text(&format!("{message} @{phrase}"));
        editor.input.grab_focus();
    }

    fn show_custom_when_popover(self: &Rc<Self>, editor: Rc<CanvasEditor>) {
        self.show_custom_when_popover_at(editor, None);
    }

    fn show_custom_when_popover_at(
        self: &Rc<Self>,
        editor: Rc<CanvasEditor>,
        anchor_offset: Option<i32>,
    ) {
        self.dismiss_suggestions();
        self.dismiss_custom_popover();
        let parsed = parse_english(&editor.text());
        let current = match parsed.status {
            ScheduleParseStatus::Valid(schedule) => Some(schedule),
            _ => None,
        };
        let local_due = current
            .as_ref()
            .and_then(|schedule| self.service.preview_schedule(schedule, &Local).ok())
            .or_else(|| {
                editor
                    .reminder_id
                    .and_then(|id| self.service.get(id).ok().map(|reminder| reminder.due_at))
            })
            .unwrap_or_else(|| default_due_time(self.service.now()))
            .with_timezone(&Local);
        let anchor = anchor_offset.map_or_else(
            || editor.cursor_anchor_rect(),
            |offset| editor.anchor_rect_at_offset(offset),
        );
        let picker = SchedulePicker::new(local_due, self.service.now(), system_clock_format());
        let weak = Rc::downgrade(self);
        let apply_editor = Rc::downgrade(&editor);
        picker.connect_apply(move |PickerSelection { date, time }| {
            let (Some(window), Some(apply_editor)) = (weak.upgrade(), apply_editor.upgrade())
            else {
                return Ok(());
            };
            let schedule = ScheduleExpression::Date {
                day: DaySpec::Exact(date),
                time: Some(time),
            };
            if let Err(problem) = window.service.preview_schedule(&schedule, &Local) {
                return Err(localized_service_error(&problem));
            }
            if let Some(entry_id) = apply_editor.entry_id
                && apply_editor.reminder_id.is_some()
            {
                let parsed = parse_english(&apply_editor.text());
                if !window.flush_canvas() {
                    return Err(gettextrs::gettext("Could not save canvas changes"));
                }
                match window.service.commit_canvas_edit(
                    entry_id,
                    parsed.message,
                    CanvasSchedule::Replace(schedule),
                    &Local,
                ) {
                    Ok(_) => window.after_mutation(),
                    Err(problem) => return Err(localized_service_error(&problem)),
                }
            } else {
                window.replace_schedule_text(&apply_editor, &canonical_custom_phrase(date, time));
            }
            Ok(())
        });
        let weak = Rc::downgrade(self);
        let closed_picker = Rc::downgrade(&picker);
        picker.connect_closed(move || {
            let (Some(window), Some(picker)) = (weak.upgrade(), closed_picker.upgrade()) else {
                return;
            };
            let mut current = window.custom_popover.borrow_mut();
            if current
                .as_ref()
                .is_some_and(|current| Rc::ptr_eq(current, &picker))
            {
                current.take();
            }
            drop(current);
            picker.unparent();
        });
        let anchor_editor = Rc::downgrade(&editor);
        picker.popup_at(&editor.input, &anchor, move || {
            anchor_editor.upgrade().map(|editor| {
                anchor_offset.map_or_else(
                    || editor.cursor_anchor_rect(),
                    |offset| editor.anchor_rect_at_offset(offset),
                )
            })
        });
        *self.custom_popover.borrow_mut() = Some(picker);
    }

    fn dismiss_custom_popover(&self) {
        let picker = self.custom_popover.borrow_mut().take();
        if let Some(picker) = picker {
            picker.popdown_and_unparent();
        }
    }

    fn rebuild_active(&self, reminders: Vec<Reminder>) {
        clear_box(&self.widgets.active_groups);
        let is_empty = reminders.is_empty();
        self.widgets
            .active_content
            .set_visible_child_name(if is_empty { "empty" } else { "list" });
        let grouped = group_active_reminders(reminders, self.service.now(), &Local);
        for group_name in ReminderGroup::ALL {
            let reminders = &grouped[&group_name];
            if reminders.is_empty() {
                continue;
            }
            let group = rows::FlatGroup::new(&localized_group_title(group_name));
            for reminder in reminders {
                let overdue = group_name == ReminderGroup::Overdue;
                group.rows.append(&rows::active_reminder_row(
                    reminder,
                    overdue,
                    &format_due(reminder.due_at, overdue),
                ));
            }
            self.widgets.active_groups.append(&group.widget);
        }
    }

    fn rebuild_history(&self, reminders: Vec<Reminder>) {
        clear_box(&self.widgets.history_list);
        let empty = reminders.is_empty();
        self.widgets.history_empty.set_visible(empty);
        self.widgets.history_list.set_visible(!empty);
        self.widgets.clear_history_button.set_sensitive(!empty);
        if empty {
            return;
        }
        let group = rows::FlatGroup::new(&gettextrs::gettext("Completed"));
        for reminder in reminders {
            let completed = reminder
                .completed_at
                .map_or_else(String::new, |completed_at| {
                    format!(
                        "{} {}",
                        gettextrs::gettext("Completed"),
                        format_local_datetime(completed_at)
                    )
                });
            group.rows.append(&rows::history_row(&reminder, &completed));
        }
        self.widgets.history_list.append(&group.widget);
    }

    fn complete(self: &Rc<Self>, id: Uuid) {
        if !self.flush_canvas() {
            return;
        }
        match self.service.complete(id) {
            Ok(_) => self.after_mutation(),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn snooze(self: &Rc<Self>, id: Uuid) {
        if !self.flush_canvas() {
            return;
        }
        match self.service.snooze(id) {
            Ok(_) => self.after_mutation(),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn delete_canvas_with_undo(self: &Rc<Self>, id: Uuid) {
        if !self.flush_canvas() {
            return;
        }
        match self.service.delete_canvas_entry(id) {
            Ok(deleted) => self.show_deleted_canvas_undo(deleted),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn show_deleted_canvas_undo(self: &Rc<Self>, deleted: DeletedCanvasItem) {
        self.after_mutation();
        let toast = adw::Toast::new(&gettextrs::gettext("Canvas entry deleted"));
        toast.set_button_label(Some(&gettextrs::gettext("Undo")));
        let weak = Rc::downgrade(self);
        toast.connect_button_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                if !window.flush_canvas() {
                    return;
                }
                match window.service.restore_canvas_item(&deleted) {
                    Ok(()) => window.after_mutation(),
                    Err(error) => window.show_error(&error.to_string()),
                }
            }
        });
        self.widgets.toast_overlay.add_toast(toast);
    }

    fn delete_reminder_with_undo(self: &Rc<Self>, id: Uuid) {
        if !self.flush_canvas() {
            return;
        }
        let canvas_id = match self.service.list_canvas() {
            Ok(items) => items
                .into_iter()
                .find(|item| item.entry.reminder_id == Some(id))
                .map(|item| item.entry.id),
            Err(error) => {
                self.show_error(&error.to_string());
                return;
            }
        };
        if let Some(canvas_id) = canvas_id {
            self.delete_canvas_with_undo(canvas_id);
            return;
        }
        match self.service.delete(id) {
            Ok(reminder) => {
                self.after_mutation();
                let toast = adw::Toast::new(&gettextrs::gettext("Reminder deleted"));
                toast.set_button_label(Some(&gettextrs::gettext("Undo")));
                let weak = Rc::downgrade(self);
                toast.connect_button_clicked(move |_| {
                    if let Some(window) = weak.upgrade() {
                        if !window.flush_canvas() {
                            return;
                        }
                        match window.service.restore(&reminder) {
                            Ok(()) => window.after_mutation(),
                            Err(error) => window.show_error(&error.to_string()),
                        }
                    }
                });
                self.widgets.toast_overlay.add_toast(toast);
            }
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn show_edit_dialog(self: &Rc<Self>, id: Uuid) {
        let Ok(reminder) = self.service.get(id) else {
            return;
        };
        let local_due = reminder.due_at.with_timezone(&Local);
        let dialog = adw::Dialog::builder()
            .title(gettextrs::gettext("Edit Reminder"))
            .content_width(460)
            .content_height(520)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = gtk::Label::new(Some(&gettextrs::gettext("Edit Reminder")));
        title.add_css_class("title");
        header.set_title_widget(Some(&title));
        let cancel = gtk::Button::with_label(&gettextrs::gettext("Cancel"));
        let save = gtk::Button::with_label(&gettextrs::gettext("Save"));
        save.add_css_class("suggested-action");
        header.pack_start(&cancel);
        header.pack_end(&save);
        toolbar.add_top_bar(&header);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let entry = adw::EntryRow::new();
        entry.set_title(&gettextrs::gettext("Reminder message"));
        entry.set_max_length(280);
        entry.set_text(&reminder.message);
        let group = adw::PreferencesGroup::new();
        group.add(&entry);
        let calendar = gtk::Calendar::new();
        if let Some(date) = glib_local_noon(local_due.date_naive()) {
            calendar.set_date(&date);
        }
        let time = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        time.set_halign(gtk::Align::Center);
        let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        hour.set_value(local_due.hour() as f64);
        hour.set_tooltip_text(Some(&gettextrs::gettext("Hour")));
        let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        minute.set_value(local_due.minute() as f64);
        minute.set_tooltip_text(Some(&gettextrs::gettext("Minute")));
        time.append(&hour);
        time.append(&gtk::Label::new(Some(":")));
        time.append(&minute);
        let error = gtk::Label::new(None);
        error.set_visible(false);
        error.set_halign(gtk::Align::Start);
        error.set_wrap(true);
        error.add_css_class("error");
        content.append(&group);
        content.append(&calendar);
        content.append(&time);
        content.append(&error);
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));
        dialog.set_default_widget(Some(&save));
        dialog.set_focus(Some(&entry));
        let cancel_dialog = dialog.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(dialog) = cancel_dialog.upgrade() {
                dialog.close();
            }
        });
        let weak = Rc::downgrade(self);
        let save_dialog = dialog.downgrade();
        save.connect_clicked(move |_| {
            let Some(window) = weak.upgrade() else { return };
            let selected = calendar.date();
            let Some(date) = NaiveDate::from_ymd_opt(
                selected.year(),
                selected.month() as u32,
                selected.day_of_month() as u32,
            ) else {
                show_inline_error(&error, &gettextrs::gettext("Choose a valid date"));
                return;
            };
            let Some(local) =
                date.and_hms_opt(hour.value_as_int() as u32, minute.value_as_int() as u32, 0)
            else {
                show_inline_error(&error, &gettextrs::gettext("Choose a valid time"));
                return;
            };
            let Ok(due_at) = resolve_local_datetime(&Local, local) else {
                show_inline_error(
                    &error,
                    &gettextrs::gettext("Choose a valid local date and time"),
                );
                return;
            };
            if !window.flush_canvas() {
                return;
            }
            match window.service.edit(id, &entry.text(), due_at) {
                Ok(_) => {
                    if let Some(dialog) = save_dialog.upgrade() {
                        dialog.close();
                    }
                    window.after_mutation();
                }
                Err(problem) => {
                    show_inline_error(&error, &localized_service_error(&problem));
                }
            }
        });
        dialog.present(Some(&self.widgets.window));
    }

    fn confirm_clear_history(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::new(
            Some(&gettextrs::gettext("Clear reminder history?")),
            Some(&gettextrs::gettext("This cannot be undone.")),
        );
        dialog.add_response("cancel", &gettextrs::gettext("Cancel"));
        dialog.add_response("clear", &gettextrs::gettext("Clear"));
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(Some("clear"), move |_, _| {
            if let Some(window) = weak.upgrade() {
                if !window.flush_canvas() {
                    return;
                }
                match window.service.clear_history() {
                    Ok(_) => window.after_mutation(),
                    Err(error) => window.show_error(&error.to_string()),
                }
            }
        });
        dialog.present(Some(&self.widgets.window));
    }

    fn slot(&self, id: Option<Uuid>) -> Option<std::cell::Ref<'_, CanvasSlot>> {
        std::cell::Ref::filter_map(self.slots.borrow(), |slots| {
            slots.iter().find(|slot| slot.editor.entry_id == id)
        })
        .ok()
    }

    fn editor_for_entry(&self, id: Option<Uuid>) -> Option<Rc<CanvasEditor>> {
        self.slots
            .borrow()
            .iter()
            .find(|slot| slot.editor.entry_id == id)
            .map(|slot| slot.editor.clone())
    }

    fn editor_is_focused(&self, editor: &CanvasEditor) -> bool {
        gtk::prelude::GtkWindowExt::focus(&self.widgets.window)
            .as_ref()
            .is_some_and(|focused| focused == editor.input.upcast_ref::<gtk::Widget>())
    }

    fn focus_after_rebuild(&self, previous: Option<(Option<Uuid>, i32)>, fallback_to_draft: bool) {
        let requested = self.reminder_to_focus.take();
        let slots = self.slots.borrow();
        let target = slots
            .iter()
            .find(|slot| {
                requested.is_some() && slot.editor.reminder_id == requested
                    || requested.is_none()
                        && previous.is_some_and(|(id, _)| slot.editor.entry_id == id)
            })
            .or_else(|| {
                fallback_to_draft
                    .then(|| slots.iter().find(|slot| slot.editor.entry_id.is_none()))
                    .flatten()
            })
            .map(|slot| {
                let offset = previous
                    .filter(|(id, _)| *id == slot.editor.entry_id)
                    .map_or(0, |(_, offset)| offset);
                (slot.editor.input.clone(), offset)
            });
        if let Some((input, offset)) = target {
            let restore = move |input: &gtk::TextView| {
                input.grab_focus();
                let buffer = input.buffer();
                let offset = offset.clamp(0, buffer.char_count());
                buffer.place_cursor(&buffer.iter_at_offset(offset));
            };
            restore(&input);
            glib::idle_add_local_once(move || {
                restore(&input);
            });
        }
    }

    fn focused_canvas_position(&self) -> Option<(Option<Uuid>, i32)> {
        let focused = gtk::prelude::GtkWindowExt::focus(&self.widgets.window)?;
        self.slots
            .borrow()
            .iter()
            .find(|slot| slot.editor.input.upcast_ref::<gtk::Widget>() == &focused)
            .map(|slot| {
                (
                    slot.editor.entry_id,
                    slot.editor.input.buffer().cursor_position(),
                )
            })
    }

    fn focus_draft_later(&self) {
        if let Some(editor) = self.editor_for_entry(None) {
            glib::idle_add_local_once(move || {
                editor.input.grab_focus();
            });
        }
    }

    fn after_mutation(self: &Rc<Self>) {
        self.refresh_views(None);
        (self.on_mutation)();
    }

    fn show_error(&self, message: &str) {
        self.widgets
            .toast_overlay
            .add_toast(adw::Toast::new(message));
    }
}

impl Drop for MainWindow {
    fn drop(&mut self) {
        let manager = adw::StyleManager::default();
        for id in self.style_signal_ids.get_mut().drain(..) {
            manager.disconnect(id);
        }
    }
}

fn committed_canvas_text(item: &CanvasItem, suffix: Option<&str>) -> String {
    let message = escape_message(&item.entry.message);
    suffix.map_or(message.clone(), |suffix| format!("{message} {suffix}"))
}

fn keeps_existing_suffix(
    editor: &CanvasEditor,
    text: &str,
    span: Option<&std::ops::Range<usize>>,
) -> bool {
    editor
        .committed_suffix()
        .as_ref()
        .is_some_and(|suffix| span.is_some_and(|span| text[span.clone()].trim() == suffix))
}

fn editor_key(id: Option<Uuid>) -> String {
    id.map_or_else(|| "draft".to_owned(), |id| id.to_string())
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn show_inline_error(label: &gtk::Label, message: &str) {
    label.set_label(message);
    label.set_visible(true);
}

fn format_due(due_at: chrono::DateTime<Utc>, overdue: bool) -> String {
    let formatted = format_local_datetime(due_at);
    if overdue {
        format!("{} {formatted}", gettextrs::gettext("Due"))
    } else {
        formatted
    }
}

fn format_local_datetime(date_time: chrono::DateTime<Utc>) -> String {
    glib::DateTime::from_unix_local(date_time.timestamp())
        .and_then(|value| {
            let date = value.format("%x")?;
            let time = format_clock_time(
                value.hour() as u32,
                value.minute() as u32,
                system_clock_format(),
            );
            Ok(format!("{date} {time}"))
        })
        .unwrap_or_else(|_| date_time.with_timezone(&Local).to_rfc2822())
}

fn localized_group_title(group: ReminderGroup) -> String {
    match group {
        ReminderGroup::Overdue => gettextrs::gettext("Overdue"),
        ReminderGroup::Today => gettextrs::gettext("Today"),
        ReminderGroup::Tomorrow => gettextrs::gettext("Tomorrow"),
        ReminderGroup::Later => gettextrs::gettext("Later"),
    }
}

fn localized_service_error(error: &ServiceError) -> String {
    match error {
        ServiceError::InvalidReminder(error)
        | ServiceError::Repository(RepositoryError::InvalidReminder(error)) => {
            localized_reminder_error(*error)
        }
        ServiceError::InvalidSchedule(error) => localized_schedule_error(*error),
        _ => error.to_string(),
    }
}

fn localized_schedule_parse_error(error: ScheduleParseError) -> String {
    match error {
        ScheduleParseError::Unsupported => gettextrs::gettext("I don't understand that schedule"),
        ScheduleParseError::InvalidAmount => gettextrs::gettext("Use a positive amount of time"),
        ScheduleParseError::InvalidDate => gettextrs::gettext("Choose a valid date"),
        ScheduleParseError::OutOfRange => gettextrs::gettext("That schedule is too far away"),
    }
}

fn localized_schedule_error(error: ScheduleError) -> String {
    match error {
        ScheduleError::DueTimeNotFuture => gettextrs::gettext("Choose a time in the future"),
        ScheduleError::NonexistentLocalTime => {
            gettextrs::gettext("That local time does not exist because the clock changes then")
        }
        ScheduleError::InvalidDate => gettextrs::gettext("Choose a valid date"),
        ScheduleError::OutOfRange => gettextrs::gettext("That schedule is out of range"),
    }
}

fn canonical_custom_phrase(date: NaiveDate, time: chrono::NaiveTime) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (hour, suffix) = match time.hour() {
        0 => (12, "AM"),
        1..=11 => (time.hour(), "AM"),
        12 => (12, "PM"),
        hour => (hour - 12, "PM"),
    };
    format!(
        "{} {} {} {hour}:{:02} {suffix}",
        MONTHS[date.month0() as usize],
        date.day(),
        date.year(),
        time.minute()
    )
}

fn localized_reminder_error(error: ReminderError) -> String {
    match error {
        ReminderError::EmptyMessage => gettextrs::gettext("Enter a note or reminder message"),
        ReminderError::MessageTooLong => {
            gettextrs::gettext("Notes and reminder messages can contain at most 280 characters")
        }
        ReminderError::DueTimeNotFuture => gettextrs::gettext("Choose a time in the future"),
    }
}

fn glib_local_noon(date: NaiveDate) -> Option<glib::DateTime> {
    glib::DateTime::new(
        &glib::TimeZone::local(),
        date.year(),
        date.month() as i32,
        date.day() as i32,
        12,
        0,
        0.0,
    )
    .ok()
}

fn system_clock_format() -> ClockFormat {
    let Some(source) = gio::SettingsSchemaSource::default() else {
        return ClockFormat::TwentyFourHour;
    };
    if source.lookup("org.gnome.desktop.interface", true).is_none() {
        return ClockFormat::TwentyFourHour;
    }
    let settings = gio::Settings::new("org.gnome.desktop.interface");
    if settings.string("clock-format") == "12h" {
        ClockFormat::TwelveHour
    } else {
        ClockFormat::TwentyFourHour
    }
}
