use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use chrono::{Datelike, Local, NaiveDate, Timelike, Utc};
use gio::prelude::ActionMapExt;
use glib::variant::{FromVariant, StaticVariantType, ToVariant};
use uuid::Uuid;

use crate::{
    grouping::{ReminderGroup, group_active_reminders},
    model::{NewReminder, Reminder, ReminderError},
    repository::RepositoryError,
    service::{ReminderService, ServiceError},
    time_utils::{ClockFormat, default_due_time, format_clock_time, resolve_local_datetime},
};

use super::{UiBuildError, WindowWidgets, build_window};

pub struct MainWindow {
    widgets: WindowWidgets,
    service: Rc<ReminderService>,
    selected_date: Cell<NaiveDate>,
    selected_hour: Cell<u32>,
    selected_minute: Cell<u32>,
    reminder_to_focus: Cell<Option<Uuid>>,
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
        let widgets = build_window(application)?;
        let local_due = default_due_time(Utc::now()).with_timezone(&Local);
        let window = Rc::new(Self {
            widgets,
            service,
            selected_date: Cell::new(local_due.date_naive()),
            selected_hour: Cell::new(local_due.hour()),
            selected_minute: Cell::new(local_due.minute()),
            reminder_to_focus: Cell::new(None),
            on_mutation: Box::new(on_mutation),
            on_closed: Box::new(on_closed),
        });
        window.install_menu();
        window.install_window_actions();
        window.connect_signals();
        window.update_composer_labels();
        window.refresh();
        Ok(window)
    }

    pub fn present(&self) {
        self.widgets.window.present();
    }

    pub fn widget(&self) -> &adw::ApplicationWindow {
        &self.widgets.window
    }

    pub fn show_error_message(&self, message: &str) {
        self.show_error(message);
    }

    pub fn show_reminder(&self, id: Uuid) {
        self.reminder_to_focus.set(Some(id));
        self.widgets.view_stack.set_visible_child_name("reminders");
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
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.widgets.add_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.submit_composer();
            }
        });

        let weak = Rc::downgrade(self);
        self.widgets.message_entry.connect_changed(move |_| {
            if let Some(window) = weak.upgrade() {
                window.clear_composer_error();
            }
        });

        let weak = Rc::downgrade(self);
        self.widgets
            .message_entry
            .connect_entry_activated(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.submit_composer();
                }
            });

        let weak = Rc::downgrade(self);
        self.widgets.date_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.show_date_popover();
            }
        });

        let weak = Rc::downgrade(self);
        self.widgets.time_button.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.show_time_popover();
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
        self.clear_composer_error();
        self.widgets.message_entry.remove_css_class("error");
        let Some(due_at) = self.selected_due_time() else {
            self.show_composer_error(&gettextrs::gettext("Choose a valid local date and time"));
            return;
        };
        match self
            .service
            .create(NewReminder::new(self.widgets.message_entry.text(), due_at))
        {
            Ok(_) => {
                self.widgets.message_entry.set_text("");
                self.reset_composer_time();
                self.refresh();
                (self.on_mutation)();
            }
            Err(ServiceError::InvalidReminder(error)) => {
                self.widgets.message_entry.add_css_class("error");
                self.show_composer_error(&localized_reminder_error(error));
            }
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn selected_due_time(&self) -> Option<chrono::DateTime<Utc>> {
        let local = self.selected_date.get().and_hms_opt(
            self.selected_hour.get(),
            self.selected_minute.get(),
            0,
        )?;
        resolve_local_datetime(&Local, local).ok()
    }

    fn reset_composer_time(&self) {
        let local_due = default_due_time(Utc::now()).with_timezone(&Local);
        self.selected_date.set(local_due.date_naive());
        self.selected_hour.set(local_due.hour());
        self.selected_minute.set(local_due.minute());
        self.update_composer_labels();
    }

    fn update_composer_labels(&self) {
        let today = Local::now().date_naive();
        let date = self.selected_date.get();
        let date_label = if date == today {
            gettextrs::gettext("Today")
        } else if date == today.succ_opt().expect("tomorrow is representable") {
            gettextrs::gettext("Tomorrow")
        } else {
            glib_local_noon(date)
                .and_then(|value| value.format("%x").ok())
                .map(|value| value.to_string())
                .unwrap_or_else(|| date.to_string())
        };
        self.widgets.date_button.set_label(&date_label);
        self.widgets.time_button.set_label(&format_clock_time(
            self.selected_hour.get(),
            self.selected_minute.get(),
            system_clock_format(),
        ));
    }

    fn show_date_popover(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let calendar = gtk::Calendar::new();
        let date = self.selected_date.get();
        if let Some(glib_date) = glib_local_noon(date) {
            calendar.set_date(&glib_date);
        }
        let done = gtk::Button::with_label(&gettextrs::gettext("Done"));
        done.add_css_class("suggested-action");
        content.append(&calendar);
        content.append(&done);
        popover.set_child(Some(&content));
        popover.set_parent(&self.widgets.date_button);

        let weak = Rc::downgrade(self);
        let popover_clone = popover.clone();
        done.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                let selected = calendar.date();
                if let Some(date) = NaiveDate::from_ymd_opt(
                    selected.year(),
                    selected.month() as u32,
                    selected.day_of_month() as u32,
                ) {
                    window.selected_date.set(date);
                    window.update_composer_labels();
                }
            }
            popover_clone.popdown();
        });
        popover.popup();
    }

    fn show_time_popover(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
        hour.set_value(self.selected_hour.get() as f64);
        hour.set_wrap(true);
        hour.set_tooltip_text(Some(&gettextrs::gettext("Hour")));
        let separator = gtk::Label::new(Some(":"));
        let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        minute.set_value(self.selected_minute.get() as f64);
        minute.set_wrap(true);
        minute.set_tooltip_text(Some(&gettextrs::gettext("Minute")));
        controls.append(&hour);
        controls.append(&separator);
        controls.append(&minute);
        let done = gtk::Button::with_label(&gettextrs::gettext("Done"));
        done.add_css_class("suggested-action");
        content.append(&controls);
        content.append(&done);
        popover.set_child(Some(&content));
        popover.set_parent(&self.widgets.time_button);

        let weak = Rc::downgrade(self);
        let popover_clone = popover.clone();
        done.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.selected_hour.set(hour.value_as_int() as u32);
                window.selected_minute.set(minute.value_as_int() as u32);
                window.update_composer_labels();
            }
            popover_clone.popdown();
        });
        popover.popup();
    }

    fn rebuild_active(&self, reminders: Vec<Reminder>) {
        clear_box(&self.widgets.active_groups);
        let is_empty = reminders.is_empty();
        self.widgets.reminders_empty.set_visible(is_empty);
        self.widgets.active_groups.set_visible(!is_empty);
        let grouped = group_active_reminders(reminders, Utc::now(), &Local);

        for group_name in ReminderGroup::ALL {
            let reminders = &grouped[&group_name];
            if reminders.is_empty() {
                continue;
            }
            let group = adw::PreferencesGroup::new();
            group.set_title(&localized_group_title(group_name));
            for reminder in reminders {
                group.add(&self.active_row(reminder, group_name == ReminderGroup::Overdue));
            }
            self.widgets.active_groups.append(&group);
        }
    }

    fn active_row(&self, reminder: &Reminder, overdue: bool) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_focusable(true);
        row.set_title(&reminder.message);
        row.set_subtitle(&format_due(reminder.due_at, overdue));

        if self.reminder_to_focus.get() == Some(reminder.id) {
            self.reminder_to_focus.set(None);
            let row_to_focus = row.clone();
            glib::idle_add_local_once(move || {
                row_to_focus.grab_focus();
            });
        }

        if overdue {
            let snooze = gtk::Button::from_icon_name("alarm-symbolic");
            snooze.add_css_class("flat");
            snooze.set_valign(gtk::Align::Center);
            snooze.set_tooltip_text(Some(&gettextrs::gettext("Snooze 10 minutes")));
            snooze.set_action_name(Some("win.snooze"));
            snooze.set_action_target_value(Some(&reminder.id.to_string().to_variant()));
            row.add_suffix(&snooze);
        }

        let done = gtk::Button::from_icon_name("object-select-symbolic");
        done.add_css_class("flat");
        done.set_valign(gtk::Align::Center);
        done.set_tooltip_text(Some(&gettextrs::gettext("Mark done")));
        done.set_action_name(Some("win.complete"));
        done.set_action_target_value(Some(&reminder.id.to_string().to_variant()));
        row.add_suffix(&done);

        let menu_button = gtk::MenuButton::new();
        menu_button.set_icon_name("view-more-symbolic");
        menu_button.add_css_class("flat");
        menu_button.set_valign(gtk::Align::Center);
        menu_button.set_tooltip_text(Some(&gettextrs::gettext("Reminder options")));
        menu_button.set_menu_model(Some(&reminder_menu(reminder.id)));
        row.add_suffix(&menu_button);
        row
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

        let group = adw::PreferencesGroup::new();
        group.set_title(&gettextrs::gettext("Completed"));
        for reminder in reminders {
            let row = adw::ActionRow::new();
            row.set_title(&reminder.message);
            if let Some(completed_at) = reminder.completed_at {
                row.set_subtitle(&format!(
                    "{} {}",
                    gettextrs::gettext("Completed"),
                    format_local_datetime(completed_at)
                ));
            }
            group.add(&row);
        }
        self.widgets.history_list.append(&group);
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

    fn show_composer_error(&self, message: &str) {
        self.widgets.composer_error.set_label(message);
        self.widgets.composer_error.set_visible(true);
    }

    fn clear_composer_error(&self) {
        self.widgets.composer_error.set_visible(false);
    }
}

fn reminder_menu(id: Uuid) -> gio::Menu {
    let menu = gio::Menu::new();
    let target = id.to_string().to_variant();
    let edit = gio::MenuItem::new(Some(&gettextrs::gettext("Edit")), None);
    edit.set_action_and_target_value(Some("win.edit"), Some(&target));
    menu.append_item(&edit);
    let delete = gio::MenuItem::new(Some(&gettextrs::gettext("Delete")), None);
    delete.set_action_and_target_value(Some("win.delete"), Some(&target));
    menu.append_item(&delete);
    menu
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
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
        _ => error.to_string(),
    }
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
