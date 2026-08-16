use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use glib::variant::ToVariant;
use uuid::Uuid;

use crate::model::Reminder;

pub struct FlatGroup {
    pub widget: gtk::Box,
    pub rows: gtk::ListBox,
}

impl FlatGroup {
    pub fn new(title: &str) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let heading = gtk::Label::new(Some(title));
        heading.set_halign(gtk::Align::Start);
        heading.set_margin_start(8);
        heading.add_css_class("caption-heading");
        heading.add_css_class("dim-label");
        let rows = gtk::ListBox::new();
        rows.set_selection_mode(gtk::SelectionMode::None);
        rows.add_css_class("flat-reminder-list");
        widget.append(&heading);
        widget.append(&rows);
        Self { widget, rows }
    }
}

pub fn active_reminder_row(reminder: &Reminder, overdue: bool, due: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_focusable(true);
    row.set_activatable(true);
    row.set_title(&reminder.message);
    row.set_subtitle(due);
    row.add_css_class("reminder-row");

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    controls.set_valign(gtk::Align::Center);
    controls.set_can_target(false);
    controls.add_css_class("row-controls");

    let done = gtk::Button::from_icon_name("object-select-symbolic");
    done.add_css_class("flat");
    done.set_tooltip_text(Some(&gettextrs::gettext("Mark done")));
    done.set_action_name(Some("win.complete"));
    done.set_action_target_value(Some(&reminder.id.to_string().to_variant()));
    controls.append(&done);

    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("view-more-symbolic");
    menu_button.add_css_class("flat");
    menu_button.set_tooltip_text(Some(&gettextrs::gettext("Reminder options")));
    menu_button.set_menu_model(Some(&reminder_menu(reminder.id, overdue)));
    controls.append(&menu_button);
    row.add_suffix(&controls);

    let hovered = Rc::new(Cell::new(false));
    let focused = Rc::new(Cell::new(false));
    let pointer = gtk::EventControllerMotion::new();
    let controls_on_enter = controls.clone();
    let hovered_on_enter = hovered.clone();
    pointer.connect_enter(move |_, _, _| {
        hovered_on_enter.set(true);
        controls_on_enter.set_can_target(true);
    });
    let controls_on_leave = controls.clone();
    let hovered_on_leave = hovered.clone();
    let focused_on_leave = focused.clone();
    pointer.connect_leave(move |_| {
        hovered_on_leave.set(false);
        controls_on_leave.set_can_target(focused_on_leave.get());
    });
    row.add_controller(pointer);

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
    row.add_controller(focus);

    let menu_to_open = menu_button.clone();
    row.connect_activated(move |_| menu_to_open.popup());
    row
}

fn reminder_menu(id: Uuid, overdue: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    let target = id.to_string().to_variant();
    let done = gio::MenuItem::new(Some(&gettextrs::gettext("Done")), None);
    done.set_action_and_target_value(Some("win.complete"), Some(&target));
    menu.append_item(&done);
    if overdue {
        let snooze = gio::MenuItem::new(Some(&gettextrs::gettext("Snooze 10 minutes")), None);
        snooze.set_action_and_target_value(Some("win.snooze"), Some(&target));
        menu.append_item(&snooze);
    }
    let edit = gio::MenuItem::new(Some(&gettextrs::gettext("Edit")), None);
    edit.set_action_and_target_value(Some("win.edit"), Some(&target));
    menu.append_item(&edit);
    let delete = gio::MenuItem::new(Some(&gettextrs::gettext("Delete")), None);
    delete.set_action_and_target_value(Some("win.delete"), Some(&target));
    menu.append_item(&delete);
    menu
}
