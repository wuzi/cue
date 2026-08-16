use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

use adw::prelude::*;
use glib::variant::ToVariant;
use uuid::Uuid;

use crate::model::CanvasItem;

pub struct CanvasEditor {
    pub root: gtk::Box,
    pub input: gtk::TextView,
    pub error: gtk::Label,
    pub status: gtk::Image,
    pub entry_id: Option<Uuid>,
    pub reminder_id: Option<Uuid>,
    dirty: Cell<bool>,
    programmatic_update: Cell<bool>,
    committed_suffix: RefCell<Option<String>>,
    options_button: Option<gtk::MenuButton>,
    draft_tag: gtk::TextTag,
    registered_tag: gtk::TextTag,
}

impl CanvasEditor {
    pub fn draft(text: &str) -> Self {
        Self::new(None, None, text, None)
    }

    pub fn saved(item: &CanvasItem, text: &str, committed_suffix: Option<String>) -> Self {
        Self::new(
            Some(item.entry.id),
            item.entry.reminder_id,
            text,
            committed_suffix,
        )
    }

    fn new(
        entry_id: Option<Uuid>,
        reminder_id: Option<Uuid>,
        text: &str,
        committed_suffix: Option<String>,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.add_css_class("canvas-entry-container");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row.add_css_class("canvas-entry-row");
        if entry_id.is_some() {
            row.add_css_class("canvas-saved-row");
        }
        let input = gtk::TextView::new();
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        input.set_accepts_tab(false);
        input.set_hexpand(true);
        input.set_top_margin(5);
        input.set_bottom_margin(5);
        input.set_left_margin(4);
        input.set_right_margin(4);
        input.add_css_class(if entry_id.is_some() {
            "canvas-entry"
        } else {
            "canvas-draft"
        });
        if entry_id.is_none() {
            root.set_vexpand(true);
            input.set_vexpand(true);
        }
        input.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
            "Canvas entry",
        ))]);

        let draft_tag = gtk::TextTag::builder()
            .name("draft-schedule")
            .underline(gtk::pango::Underline::Single)
            .build();
        let registered_tag = gtk::TextTag::builder()
            .name("registered-schedule")
            .weight(700)
            .build();
        input.buffer().tag_table().add(&draft_tag);
        input.buffer().tag_table().add(&registered_tag);
        update_accent(&registered_tag, &adw::StyleManager::default());

        let status = gtk::Image::from_icon_name("alarm-symbolic");
        status.set_tooltip_text(Some(&gettextrs::gettext("Reminder is still scheduled")));
        status.set_visible(false);
        status.add_css_class("accent");
        status.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
            "Reminder is still scheduled",
        ))]);
        row.append(&status);
        row.append(&input);

        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        controls.set_valign(gtk::Align::Center);
        controls.set_can_target(false);
        controls.add_css_class("canvas-entry-controls");
        let menu_button = entry_id.map(|id| {
            if let Some(reminder_id) = reminder_id {
                let done = gtk::Button::from_icon_name("object-select-symbolic");
                done.add_css_class("flat");
                done.set_tooltip_text(Some(&gettextrs::gettext("Mark done")));
                done.set_action_name(Some("win.complete"));
                done.set_action_target_value(Some(&reminder_id.to_string().to_variant()));
                controls.append(&done);
            }
            let menu = gtk::MenuButton::new();
            menu.set_icon_name("view-more-symbolic");
            menu.add_css_class("flat");
            menu.set_tooltip_text(Some(&gettextrs::gettext("Entry options")));
            menu.set_menu_model(Some(&entry_menu(id, reminder_id)));
            controls.append(&menu);
            row.append(&controls);
            menu
        });
        root.append(&row);

        let error = gtk::Label::new(None);
        error.set_visible(false);
        error.set_halign(gtk::Align::Start);
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_margin_start(4);
        error.set_accessible_role(gtk::AccessibleRole::Alert);
        error.add_css_class("error");
        error.add_css_class("caption");
        root.append(&error);

        if let Some(ref menu) = menu_button {
            install_reveal_behavior(&row, &input, &controls);
            let menu_for_click = menu.clone();
            let click = gtk::GestureClick::new();
            click.set_button(3);
            click.connect_released(move |_, _, _, _| menu_for_click.popup());
            row.add_controller(click);
            let menu_for_press = menu.clone();
            let long_press = gtk::GestureLongPress::new();
            long_press.connect_pressed(move |_, _, _| menu_for_press.popup());
            row.add_controller(long_press);
        }

        input.buffer().set_text(text);
        Self {
            root,
            input,
            error,
            status,
            entry_id,
            reminder_id,
            dirty: Cell::new(false),
            programmatic_update: Cell::new(false),
            committed_suffix: RefCell::new(committed_suffix),
            options_button: menu_button,
            draft_tag,
            registered_tag,
        }
    }

    pub fn text(&self) -> String {
        let buffer = self.input.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    pub fn mark_clean(&self) {
        self.dirty.set(false);
    }

    pub fn is_programmatic_update(&self) -> bool {
        self.programmatic_update.get()
    }

    pub fn set_text_from_model(&self, text: &str) {
        self.programmatic_update.set(true);
        self.input.buffer().set_text(text);
        self.programmatic_update.set(false);
    }

    pub fn cursor_anchor_rect(&self) -> gtk::gdk::Rectangle {
        self.anchor_rect(None)
    }

    pub fn anchor_rect_at_offset(&self, offset: i32) -> gtk::gdk::Rectangle {
        let buffer = self.input.buffer();
        let offset = offset.clamp(0, buffer.char_count());
        self.anchor_rect(Some(&buffer.iter_at_offset(offset)))
    }

    pub fn iter_at_widget_location(&self, x: i32, y: i32) -> Option<gtk::TextIter> {
        let (buffer_x, buffer_y) =
            self.input
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x, y);
        self.input.iter_at_location(buffer_x, buffer_y)
    }

    fn anchor_rect(&self, iter: Option<&gtk::TextIter>) -> gtk::gdk::Rectangle {
        let buffer = self.input.buffer();
        let validation_iter = iter
            .cloned()
            .unwrap_or_else(|| buffer.iter_at_offset(buffer.cursor_position()));
        // Force the text layout to validate after wrapping or line changes before
        // asking for the strong cursor rectangle.
        let _ = self.input.iter_location(&validation_iter);
        let (strong, _) = self.input.cursor_locations(iter);
        let (x, y) =
            self.input
                .buffer_to_window_coords(gtk::TextWindowType::Widget, strong.x(), strong.y());
        gtk::gdk::Rectangle::new(x, y, strong.width().max(1), strong.height().max(1))
    }

    pub fn apply_schedule_span(&self, input: &str, span: Option<Range<usize>>, saved: bool) {
        let buffer = self.input.buffer();
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag(&self.draft_tag, &start, &end);
        buffer.remove_tag(&self.registered_tag, &start, &end);
        if let Some(span) = span {
            let start = input[..span.start].chars().count() as i32;
            let end = input[..span.end].chars().count() as i32;
            buffer.apply_tag(
                if saved {
                    &self.registered_tag
                } else {
                    &self.draft_tag
                },
                &buffer.iter_at_offset(start),
                &buffer.iter_at_offset(end),
            );
        }
    }

    pub fn set_error(&self, message: Option<&str>) {
        if let Some(message) = message {
            self.error.set_label(message);
            self.error.set_visible(true);
            self.input
                .update_property(&[gtk::accessible::Property::Description(message)]);
        } else {
            self.error.set_visible(false);
            self.input
                .update_property(
                    &[gtk::accessible::Property::Description(&gettextrs::gettext(
                        "Write a note or add an English schedule beginning with at sign",
                    ))],
                );
        }
    }

    pub fn set_dirty_registered(&self, dirty: bool, scheduled_description: &str) {
        let visible = dirty && self.reminder_id.is_some();
        self.status.set_visible(visible);
        if visible {
            let description = format!(
                "{}: {scheduled_description}",
                gettextrs::gettext("Still scheduled")
            );
            self.status.set_tooltip_text(Some(&description));
            self.input
                .update_property(&[gtk::accessible::Property::Description(&description)]);
        }
    }

    pub fn committed_suffix(&self) -> Option<String> {
        self.committed_suffix.borrow().clone()
    }

    pub fn set_committed_suffix(&self, suffix: Option<String>) {
        *self.committed_suffix.borrow_mut() = suffix;
    }

    pub fn update_accent(&self, manager: &adw::StyleManager) {
        update_accent(&self.registered_tag, manager);
    }

    pub fn open_actions(&self) -> bool {
        self.options_button.as_ref().is_some_and(|button| {
            button.popup();
            true
        })
    }
}

fn entry_menu(entry_id: Uuid, reminder_id: Option<Uuid>) -> gio::Menu {
    let menu = gio::Menu::new();
    if let Some(reminder_id) = reminder_id {
        let target = reminder_id.to_string().to_variant();
        let done = gio::MenuItem::new(Some(&gettextrs::gettext("Done")), None);
        done.set_action_and_target_value(Some("win.complete"), Some(&target));
        menu.append_item(&done);
        let snooze = gio::MenuItem::new(Some(&gettextrs::gettext("Snooze 10 minutes")), None);
        snooze.set_action_and_target_value(Some("win.snooze"), Some(&target));
        menu.append_item(&snooze);
        let edit = gio::MenuItem::new(Some(&gettextrs::gettext("Edit")), None);
        edit.set_action_and_target_value(Some("win.edit"), Some(&target));
        menu.append_item(&edit);
    }
    let delete = gio::MenuItem::new(Some(&gettextrs::gettext("Delete")), None);
    delete.set_action_and_target_value(
        Some("win.delete-canvas"),
        Some(&entry_id.to_string().to_variant()),
    );
    menu.append_item(&delete);
    menu
}

fn install_reveal_behavior(row: &gtk::Box, input: &gtk::TextView, controls: &gtk::Box) {
    let hovered = Rc::new(Cell::new(false));
    let focused = Rc::new(Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    let controls_on_enter = controls.clone();
    let hovered_on_enter = hovered.clone();
    motion.connect_enter(move |_, _, _| {
        hovered_on_enter.set(true);
        controls_on_enter.set_can_target(true);
    });
    let controls_on_leave = controls.clone();
    let hovered_on_leave = hovered.clone();
    let focused_on_leave = focused.clone();
    motion.connect_leave(move |_| {
        hovered_on_leave.set(false);
        controls_on_leave.set_can_target(focused_on_leave.get());
    });
    row.add_controller(motion);

    let focus = gtk::EventControllerFocus::new();
    let controls_on_focus = controls.clone();
    let focused_on_enter = focused.clone();
    focus.connect_enter(move |_| {
        focused_on_enter.set(true);
        controls_on_focus.set_can_target(true);
    });
    let controls_after_focus = controls.clone();
    focus.connect_leave(move |_| {
        focused.set(false);
        controls_after_focus.set_can_target(hovered.get());
    });
    input.add_controller(focus);
}

fn update_accent(tag: &gtk::TextTag, manager: &adw::StyleManager) {
    let color = manager.accent_color().to_standalone_rgba(manager.is_dark());
    tag.set_foreground_rgba(Some(&color));
}
