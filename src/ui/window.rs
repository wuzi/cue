use std::{cell::Cell, cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Datelike, Duration, Local, NaiveDate, Timelike, Utc};
use gio::prelude::ActionMapExt;
use glib::variant::{FromVariant, StaticVariantType};
use uuid::Uuid;

use crate::{
    grouping::{ReminderGroup, group_active_reminders},
    model::{Reminder, ReminderError},
    repository::RepositoryError,
    schedule::{
        ScheduleError, ScheduleExpression, ScheduleParseError, ScheduleParseStatus, parse_english,
    },
    service::{ReminderService, ServiceError},
    time_utils::{ClockFormat, default_due_time, format_clock_time, resolve_local_datetime},
};

use super::{UiBuildError, WindowWidgets, build_window, composer::SmartComposer, rows};

pub struct MainWindow {
    widgets: WindowWidgets,
    composer: SmartComposer,
    service: Rc<ReminderService>,
    reminder_to_focus: Cell<Option<Uuid>>,
    suggestions: RefCell<Option<SuggestionMenu>>,
    on_mutation: Box<dyn Fn()>,
    on_closed: Box<dyn Fn()>,
}

struct SuggestionMenu {
    popover: gtk::Popover,
    list: gtk::ListBox,
}

const SUGGESTIONS: [Option<&str>; 6] = [
    Some("in 15 minutes"),
    Some("in 30 minutes"),
    Some("in 1 hour"),
    Some("tomorrow 9am"),
    Some("next Monday 9am"),
    None,
];

impl MainWindow {
    pub fn new(
        application: &adw::Application,
        service: Rc<ReminderService>,
        on_mutation: impl Fn() + 'static,
        on_closed: impl Fn() + 'static,
    ) -> Result<Rc<Self>, UiBuildError> {
        let widgets = build_window(application)?;
        let composer = SmartComposer::new(&widgets);
        let window = Rc::new(Self {
            widgets,
            composer,
            service,
            reminder_to_focus: Cell::new(None),
            suggestions: RefCell::new(None),
            on_mutation: Box::new(on_mutation),
            on_closed: Box::new(on_closed),
        });
        window.install_menu();
        window.install_window_actions();
        window.connect_signals();
        window.update_composer();
        window.refresh();
        Ok(window)
    }

    pub fn present(&self) {
        self.widgets.window.present();
        if self.widgets.navigation_view.visible_page_tag().as_deref() == Some("reminders") {
            self.widgets.composer_input.grab_focus();
        }
    }

    pub fn widget(&self) -> &adw::ApplicationWindow {
        &self.widgets.window
    }

    pub fn show_error_message(&self, message: &str) {
        self.show_error(message);
    }

    pub fn show_reminder(&self, id: Uuid) {
        self.reminder_to_focus.set(Some(id));
        while self.widgets.navigation_view.visible_page_tag().as_deref() != Some("reminders")
            && self.widgets.navigation_view.pop()
        {}
        self.refresh();
        self.present();
    }

    pub fn refresh(&self) {
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
        let complete = gio::SimpleAction::new("complete", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        complete.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            if let Ok(id) = Uuid::parse_str(&target) {
                window.complete(id);
            }
        });
        self.widgets.window.add_action(&complete);

        let snooze = gio::SimpleAction::new("snooze", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        snooze.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            if let Ok(id) = Uuid::parse_str(&target) {
                window.snooze(id);
            }
        });
        self.widgets.window.add_action(&snooze);

        let edit = gio::SimpleAction::new("edit", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        edit.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            if let Ok(id) = Uuid::parse_str(&target) {
                window.show_edit_dialog(id);
            }
        });
        self.widgets.window.add_action(&edit);

        let delete = gio::SimpleAction::new("delete", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        delete.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            if let Ok(id) = Uuid::parse_str(&target) {
                window.delete_with_undo(id);
            }
        });
        self.widgets.window.add_action(&delete);

        let show_history = gio::SimpleAction::new("show-history", None);
        let weak = Rc::downgrade(self);
        show_history.connect_activate(move |_, _| {
            if let Some(window) = weak.upgrade()
                && window.widgets.navigation_view.visible_page_tag().as_deref() != Some("history")
            {
                window.widgets.navigation_view.push_by_tag("history");
            }
        });
        self.widgets.window.add_action(&show_history);

        let submit = gio::SimpleAction::new("submit", None);
        let weak = Rc::downgrade(self);
        submit.connect_activate(move |_, _| {
            if let Some(window) = weak.upgrade() {
                window.submit_composer();
            }
        });
        self.widgets.window.add_action(&submit);

        let suggestion =
            gio::SimpleAction::new("use-suggestion", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        suggestion.connect_activate(move |_, target| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if let Some(phrase) = target.and_then(String::from_variant) {
                window.apply_suggestion(&phrase);
            }
        });
        self.widgets.window.add_action(&suggestion);

        let custom = gio::SimpleAction::new("custom-time", None);
        let weak = Rc::downgrade(self);
        custom.connect_activate(move |_, _| {
            if let Some(window) = weak.upgrade() {
                window.show_custom_when_popover();
            }
        });
        self.widgets.window.add_action(&custom);
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.widgets.add_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.submit_composer();
            }
        });

        let weak = Rc::downgrade(self);
        self.widgets
            .composer_input
            .buffer()
            .connect_changed(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.update_composer();
                }
            });

        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(window) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gtk::gdk::Key::Up if window.has_suggestions() => {
                    window.move_suggestion(-1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Down if window.has_suggestions() => {
                    window.move_suggestion(1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Escape if window.has_suggestions() => {
                    window.dismiss_suggestions();
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                    if window.has_suggestions() {
                        window.accept_selected_suggestion();
                    } else {
                        window.submit_composer();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.widgets.composer_input.add_controller(keys);

        let schedule_click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        schedule_click.connect_released(move |_, _, x, y| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(iter) = window
                .widgets
                .composer_input
                .iter_at_location(x as i32, y as i32)
            else {
                return;
            };
            let input = window.composer.text();
            let parsed = parse_english(&input);
            let Some(span) = parsed.schedule_span else {
                return;
            };
            let start = input[..span.start].chars().count() as i32;
            let end = input[..span.end].chars().count() as i32;
            if (start..=end).contains(&iter.offset()) {
                window.show_custom_when_popover();
            }
        });
        self.widgets.composer_input.add_controller(schedule_click);

        let weak = Rc::downgrade(self);
        self.widgets.preview_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                glib::idle_add_local_once(move || window.show_custom_when_popover());
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
                (window.on_closed)();
            }
            glib::Propagation::Proceed
        });
    }

    fn submit_composer(&self) {
        let input = self.composer.text();
        let parsed = parse_english(&input);
        let schedule = match &parsed.status {
            ScheduleParseStatus::Default => default_schedule(),
            ScheduleParseStatus::Valid(schedule) => schedule.clone(),
            ScheduleParseStatus::Partial => {
                self.composer
                    .set_error(Some(&gettextrs::gettext("Finish the schedule after @")));
                return;
            }
            ScheduleParseStatus::Invalid(error) => {
                self.composer
                    .set_error(Some(&localized_schedule_parse_error(*error)));
                return;
            }
        };
        let result = self
            .service
            .create_scheduled(parsed.message, &schedule, &Local);
        match result {
            Ok(_) => {
                self.composer.clear();
                self.refresh();
                (self.on_mutation)();
            }
            Err(ServiceError::InvalidReminder(error)) => {
                self.composer
                    .set_error(Some(&localized_reminder_error(error)));
            }
            Err(error) => self
                .composer
                .set_error(Some(&localized_service_error(&error))),
        }
    }

    fn update_composer(self: &Rc<Self>) {
        let input = self.composer.text();
        let parsed = parse_english(&input);
        self.composer.update_placeholder();
        self.composer
            .update_span(&input, parsed.schedule_span.clone());
        self.dismiss_suggestions();

        if parsed.message.chars().count() > 280 {
            self.composer.set_can_submit(false);
            self.composer.set_error(Some(&gettextrs::gettext(
                "Reminder messages can contain at most 280 characters",
            )));
            return;
        }

        match &parsed.status {
            ScheduleParseStatus::Default => {
                self.composer.set_preview(&gettextrs::gettext("In 1 hour"));
                self.composer.set_error(None);
                self.composer
                    .set_can_submit(!parsed.message.trim().is_empty());
            }
            ScheduleParseStatus::Partial => {
                self.composer
                    .set_preview(&gettextrs::gettext("Incomplete schedule"));
                self.composer
                    .set_error(Some(&gettextrs::gettext("Finish the schedule after @")));
                self.composer.set_can_submit(false);
                self.show_suggestions();
            }
            ScheduleParseStatus::Invalid(error) => {
                self.composer
                    .set_preview(&gettextrs::gettext("Invalid schedule"));
                self.composer
                    .set_error(Some(&localized_schedule_parse_error(*error)));
                self.composer.set_can_submit(false);
            }
            ScheduleParseStatus::Valid(schedule) => {
                match self.service.preview_schedule(schedule, &Local) {
                    Ok(due_at) => {
                        self.composer.set_preview(&format_local_datetime(due_at));
                        self.composer.set_error(None);
                        self.composer
                            .set_can_submit(!parsed.message.trim().is_empty());
                    }
                    Err(error) => {
                        self.composer
                            .set_preview(&gettextrs::gettext("Invalid schedule"));
                        self.composer
                            .set_error(Some(&localized_service_error(&error)));
                        self.composer.set_can_submit(false);
                    }
                }
            }
        }
    }

    fn show_suggestions(self: &Rc<Self>) {
        if self.suggestions.borrow().is_some() {
            return;
        }
        let popover = gtk::Popover::new();
        popover.set_parent(&self.widgets.composer_card);
        popover.set_autohide(false);
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
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |popover| {
            popover.unparent();
            if let Some(window) = weak.upgrade() {
                window.suggestions.borrow_mut().take();
            }
        });
        popover.popup();
        *self.suggestions.borrow_mut() = Some(SuggestionMenu { popover, list });
        self.widgets.composer_input.grab_focus();
    }

    fn dismiss_suggestions(&self) {
        let menu = self.suggestions.borrow_mut().take();
        if let Some(menu) = menu {
            menu.popover.popdown();
        }
    }

    fn has_suggestions(&self) -> bool {
        self.suggestions.borrow().is_some()
    }

    fn move_suggestion(&self, offset: i32) {
        let suggestions = self.suggestions.borrow();
        let Some(menu) = suggestions.as_ref() else {
            return;
        };
        let current = menu.list.selected_row().map_or(0, |row| row.index());
        let last = SUGGESTIONS.len() as i32 - 1;
        let next = (current + offset).clamp(0, last);
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
        self.dismiss_suggestions();
        match SUGGESTIONS.get(index).copied().flatten() {
            Some(phrase) => self.apply_suggestion(phrase),
            None => self.show_custom_when_popover(),
        }
    }

    fn apply_suggestion(self: &Rc<Self>, phrase: &str) {
        let input = self.composer.text();
        let parsed = parse_english(&input);
        let message_end = parsed
            .schedule_span
            .as_ref()
            .map_or(input.len(), |span| span.start);
        let message = input[..message_end].trim_end();
        self.widgets
            .composer_input
            .buffer()
            .set_text(&format!("{message} @{phrase}"));
        self.widgets.composer_input.grab_focus();
    }

    fn show_custom_when_popover(self: &Rc<Self>) {
        self.dismiss_suggestions();
        let local_due = self
            .service
            .preview_schedule(&current_schedule(&self.composer.text()), &Local)
            .or_else(|_| self.service.preview_schedule(&default_schedule(), &Local))
            .unwrap_or_else(|_| default_due_time(Utc::now()))
            .with_timezone(&Local);

        let popover = gtk::Popover::new();
        popover.set_parent(&self.widgets.preview_button);
        popover.add_css_class("menu");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_size_request(300, -1);
        let title = gtk::Label::new(Some(&gettextrs::gettext("Custom time")));
        title.add_css_class("heading");
        title.set_halign(gtk::Align::Start);
        content.append(&title);
        let cancel = gtk::Button::with_label(&gettextrs::gettext("Cancel"));
        let select = gtk::Button::with_label(&gettextrs::gettext("Apply"));
        select.add_css_class("suggested-action");
        let calendar = gtk::Calendar::new();
        if let Some(date) = glib_local_noon(local_due.date_naive()) {
            calendar.set_date(&date);
        }
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        controls.set_halign(gtk::Align::Center);
        let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        hour.set_value(local_due.hour() as f64);
        hour.set_wrap(true);
        hour.set_tooltip_text(Some(&gettextrs::gettext("Hour")));
        let separator = gtk::Label::new(Some(":"));
        let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        minute.set_value(local_due.minute() as f64);
        minute.set_wrap(true);
        minute.set_tooltip_text(Some(&gettextrs::gettext("Minute")));
        controls.append(&hour);
        controls.append(&separator);
        controls.append(&minute);
        let error_label = gtk::Label::new(None);
        error_label.set_visible(false);
        error_label.set_halign(gtk::Align::Start);
        error_label.set_wrap(true);
        error_label.add_css_class("error");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        actions.append(&cancel);
        actions.append(&select);
        content.append(&calendar);
        content.append(&controls);
        content.append(&error_label);
        content.append(&actions);
        popover.set_child(Some(&content));

        let error_to_clear = error_label.clone();
        calendar.connect_day_selected(move |_| error_to_clear.set_visible(false));
        let error_to_clear = error_label.clone();
        hour.connect_value_changed(move |_| error_to_clear.set_visible(false));
        let error_to_clear = error_label.clone();
        minute.connect_value_changed(move |_| error_to_clear.set_visible(false));

        let popover_to_cancel = popover.clone();
        cancel.connect_clicked(move |_| {
            popover_to_cancel.popdown();
        });

        let weak = Rc::downgrade(self);
        let popover_to_select = popover.clone();
        select.connect_clicked(move |_| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let selected = calendar.date();
            let Some(date) = NaiveDate::from_ymd_opt(
                selected.year(),
                selected.month() as u32,
                selected.day_of_month() as u32,
            ) else {
                show_inline_error(&error_label, &gettextrs::gettext("Choose a valid date"));
                return;
            };
            let Some(time) = chrono::NaiveTime::from_hms_opt(
                hour.value_as_int() as u32,
                minute.value_as_int() as u32,
                0,
            ) else {
                show_inline_error(&error_label, &gettextrs::gettext("Choose a valid time"));
                return;
            };
            let schedule = ScheduleExpression::Date {
                day: crate::schedule::DaySpec::Exact(date),
                time: Some(time),
            };
            if let Err(error) = window.service.preview_schedule(&schedule, &Local) {
                show_inline_error(&error_label, &localized_service_error(&error));
                return;
            }
            window.apply_suggestion(&canonical_custom_phrase(date, time));
            popover_to_select.popdown();
        });
        let popover_to_unparent = popover.clone();
        popover.connect_closed(move |_| popover_to_unparent.unparent());
        popover.popup();
    }

    fn rebuild_active(&self, reminders: Vec<Reminder>) {
        clear_box(&self.widgets.active_groups);
        let is_empty = reminders.is_empty();
        self.widgets
            .reminders_content
            .set_visible_child_name(if is_empty { "empty" } else { "list" });
        let grouped = group_active_reminders(reminders, Utc::now(), &Local);

        for group_name in ReminderGroup::ALL {
            let reminders = &grouped[&group_name];
            if reminders.is_empty() {
                continue;
            }
            let group = rows::FlatGroup::new(&localized_group_title(group_name));
            for reminder in reminders {
                let overdue = group_name == ReminderGroup::Overdue;
                let row = rows::active_reminder_row(
                    reminder,
                    overdue,
                    &format_due(reminder.due_at, overdue),
                );
                if self.reminder_to_focus.get() == Some(reminder.id) {
                    self.reminder_to_focus.set(None);
                    let row_to_focus = row.clone();
                    glib::idle_add_local_once(move || {
                        row_to_focus.grab_focus();
                    });
                }
                group.rows.append(&row);
            }
            self.widgets.active_groups.append(&group.widget);
        }
    }

    fn rebuild_history(&self, reminders: Vec<Reminder>) {
        clear_box(&self.widgets.history_list);
        let is_empty = reminders.is_empty();
        self.widgets.history_empty.set_visible(is_empty);
        self.widgets.history_list.set_visible(!is_empty);
        self.widgets.clear_history_button.set_sensitive(!is_empty);
        if is_empty {
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

    fn complete(&self, id: Uuid) {
        match self.service.complete(id) {
            Ok(_) => self.after_mutation(),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn snooze(&self, id: Uuid) {
        match self.service.snooze(id) {
            Ok(_) => self.after_mutation(),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn delete_with_undo(self: &Rc<Self>, id: Uuid) {
        match self.service.delete(id) {
            Ok(reminder) => {
                self.after_mutation();
                let toast = adw::Toast::new(&gettextrs::gettext("Reminder deleted"));
                toast.set_button_label(Some(&gettextrs::gettext("Undo")));
                let weak = Rc::downgrade(self);
                toast.connect_button_clicked(move |_| {
                    if let Some(window) = weak.upgrade() {
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
        let toolbar_view = adw::ToolbarView::new();
        let header_bar = adw::HeaderBar::new();
        let title = gtk::Label::new(Some(&gettextrs::gettext("Edit Reminder")));
        title.add_css_class("title");
        header_bar.set_title_widget(Some(&title));
        let cancel = gtk::Button::with_label(&gettextrs::gettext("Cancel"));
        let save = gtk::Button::with_label(&gettextrs::gettext("Save"));
        save.add_css_class("suggested-action");
        header_bar.pack_start(&cancel);
        header_bar.pack_end(&save);
        toolbar_view.add_top_bar(&header_bar);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        let clamp = adw::Clamp::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        let entry = adw::EntryRow::new();
        entry.set_title(&gettextrs::gettext("Reminder message"));
        entry.set_max_length(280);
        entry.set_text(&reminder.message);
        let message_group = adw::PreferencesGroup::new();
        message_group.add(&entry);
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
        let error_label = gtk::Label::new(None);
        error_label.set_visible(false);
        error_label.set_halign(gtk::Align::Start);
        error_label.set_wrap(true);
        error_label.add_css_class("error");
        content.append(&message_group);
        content.append(&calendar);
        content.append(&time);
        content.append(&error_label);
        clamp.set_child(Some(&content));
        scrolled.set_child(Some(&clamp));
        toolbar_view.set_content(Some(&scrolled));
        dialog.set_child(Some(&toolbar_view));
        dialog.set_default_widget(Some(&save));
        dialog.set_focus(Some(&entry));

        let dialog_to_cancel = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog_to_cancel.close();
        });

        let error_to_clear = error_label.clone();
        entry.connect_changed(move |_| {
            error_to_clear.set_visible(false);
        });

        let weak = Rc::downgrade(self);
        let dialog_to_save = dialog.clone();
        save.connect_clicked(move |_| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let selected = calendar.date();
            let Some(date) = NaiveDate::from_ymd_opt(
                selected.year(),
                selected.month() as u32,
                selected.day_of_month() as u32,
            ) else {
                error_label.set_label(&gettextrs::gettext("Choose a valid date"));
                error_label.set_visible(true);
                return;
            };
            let Some(local) =
                date.and_hms_opt(hour.value_as_int() as u32, minute.value_as_int() as u32, 0)
            else {
                error_label.set_label(&gettextrs::gettext("Choose a valid time"));
                error_label.set_visible(true);
                return;
            };
            let Ok(due_at) = resolve_local_datetime(&Local, local) else {
                error_label.set_label(&gettextrs::gettext("Choose a valid local date and time"));
                error_label.set_visible(true);
                return;
            };
            match window.service.edit(id, &entry.text(), due_at) {
                Ok(_) => {
                    dialog_to_save.close();
                    window.after_mutation();
                }
                Err(error) => {
                    entry.add_css_class("error");
                    error_label.set_label(&localized_service_error(&error));
                    error_label.set_visible(true);
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
                match window.service.clear_history() {
                    Ok(_) => window.after_mutation(),
                    Err(error) => window.show_error(&error.to_string()),
                }
            }
        });
        dialog.present(Some(&self.widgets.window));
    }

    fn after_mutation(&self) {
        self.refresh();
        (self.on_mutation)();
    }

    fn show_error(&self, message: &str) {
        self.widgets
            .toast_overlay
            .add_toast(adw::Toast::new(message));
    }
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

fn default_schedule() -> ScheduleExpression {
    ScheduleExpression::Relative(Duration::hours(1))
}

fn current_schedule(input: &str) -> ScheduleExpression {
    match parse_english(input).status {
        ScheduleParseStatus::Valid(schedule) => schedule,
        _ => default_schedule(),
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
    let (twelve_hour, suffix) = match time.hour() {
        0 => (12, "AM"),
        1..=11 => (time.hour(), "AM"),
        12 => (12, "PM"),
        hour => (hour - 12, "PM"),
    };
    format!(
        "{} {} {} {twelve_hour}:{:02} {suffix}",
        MONTHS[date.month0() as usize],
        date.day(),
        date.year(),
        time.minute()
    )
}

fn localized_reminder_error(error: ReminderError) -> String {
    match error {
        ReminderError::EmptyMessage => gettextrs::gettext("Enter a reminder message"),
        ReminderError::MessageTooLong => {
            gettextrs::gettext("Reminder messages can contain at most 280 characters")
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
