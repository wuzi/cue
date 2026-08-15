use std::rc::Rc;

use adw::prelude::*;
use chrono::{DateTime, Duration, TimeZone, Utc};
use gio::prelude::ListModelExt;
use remind_me::{
    model::Reminder,
    repository::{ReminderRepository, SqliteReminderRepository},
    resources,
    scheduler::{Clock, NotificationError, ReminderNotifier},
    service::ReminderService,
    ui,
};

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct NoopNotifier;

impl ReminderNotifier for NoopNotifier {
    fn send(&self, _id: &str, _reminder: &Reminder) -> Result<(), NotificationError> {
        Ok(())
    }

    fn withdraw(&self, _id: &str) {}
}

#[test]
fn gtk_smoke_covers_layout_adaptation_and_input_preservation() {
    gtk::init().unwrap();
    adw::init().unwrap();
    resources::register().unwrap();
    let application = adw::Application::builder()
        .application_id("io.github.wuzi.RemindMe.Test")
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();

    let widgets = ui::build_window(&application).unwrap();

    assert_eq!(widgets.window.title().as_deref(), Some("Remind Me"));
    assert_eq!(widgets.view_stack.pages().n_items(), 2);
    assert_eq!(widgets.message_entry.title(), "Message");
    assert_eq!(widgets.message_entry.max_length(), 280);
    assert_eq!(widgets.add_button.label(), None);
    assert_eq!(
        widgets.add_button.icon_name().as_deref(),
        Some("list-add-symbolic")
    );
    assert_eq!(
        widgets.add_button.tooltip_text().as_deref(),
        Some("Add reminder")
    );
    assert!(widgets.add_button.has_css_class("circular"));
    assert_eq!(widgets.when_row.title(), "When");
    assert_eq!(widgets.when_row.subtitle().as_deref(), Some("In 1 hour"));
    assert!(!widgets.composer_error.is_visible());
    assert_eq!(widgets.window.default_width(), 640);
    assert_eq!(widgets.window.default_height(), 620);
    assert_eq!(
        widgets.reminders_content.visible_child_name().as_deref(),
        Some("empty")
    );
    assert!(!is_descendant_of(
        widgets.message_entry.upcast_ref(),
        widgets.reminders_scroller.upcast_ref()
    ));
    assert!(!widgets.bottom_switcher.reveals());

    widgets.window.set_default_size(480, 650);
    widgets.window.present();
    drain_main_context();
    assert!(!widgets.header_switcher.is_visible());
    assert!(widgets.bottom_switcher.reveals());
    assert!(!widgets.reminders_scroller.is_mapped());
    widgets.window.close();
    drain_main_context();

    let now = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let repository = Rc::new(SqliteReminderRepository::in_memory().unwrap());
    let service = Rc::new(ReminderService::new(
        repository.clone(),
        Rc::new(FixedClock(now)),
        Rc::new(NoopNotifier),
    ));
    let window = ui::MainWindow::new(&application, service.clone(), || {}, || {}).unwrap();
    window.present();
    drain_main_context();

    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.set-when",
        Some(&"30m".to_variant()),
    )
    .unwrap();
    let message = find_descendant::<adw::EntryRow>(window.widget().upcast_ref()).unwrap();
    let when = find_action_row(window.widget().upcast_ref(), "When").unwrap();
    assert_eq!(when.subtitle().as_deref(), Some("In 30 minutes"));
    message.set_text("Call Ada");
    message.emit_by_name::<()>("entry-activated", &[]);
    drain_main_context();

    let reminders = repository.list_active().unwrap();
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].due_at, now + Duration::minutes(30));
    assert_eq!(message.text(), "");
    assert_eq!(when.subtitle().as_deref(), Some("In 1 hour"));
    let content = find_descendant::<gtk::Stack>(window.widget().upcast_ref()).unwrap();
    assert_eq!(content.visible_child_name().as_deref(), Some("list"));

    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.set-when",
        Some(&"30m".to_variant()),
    )
    .unwrap();
    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.custom-when", None).unwrap();
    drain_main_context();
    let custom_dialog = window
        .widget()
        .dialogs()
        .item(0)
        .unwrap()
        .downcast::<adw::Dialog>()
        .unwrap();
    assert_eq!(custom_dialog.title(), "Custom time");
    find_button(custom_dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    assert_eq!(when.subtitle().as_deref(), Some("In 30 minutes"));

    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.custom-when", None).unwrap();
    drain_main_context();
    let custom_dialog = window
        .widget()
        .dialogs()
        .item(0)
        .unwrap()
        .downcast::<adw::Dialog>()
        .unwrap();
    let calendar = find_descendant::<gtk::Calendar>(custom_dialog.upcast_ref()).unwrap();
    calendar.set_date(&glib::DateTime::from_unix_local(1).unwrap());
    find_button(custom_dialog.upcast_ref(), "Select")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    assert_eq!(window.widget().dialogs().n_items(), 1);
    assert_eq!(when.subtitle().as_deref(), Some("In 30 minutes"));
    find_button(custom_dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    drain_main_context();

    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.edit",
        Some(&reminders[0].id.to_string().to_variant()),
    )
    .unwrap();
    drain_main_context();
    let dialog = window
        .widget()
        .dialogs()
        .item(0)
        .unwrap()
        .downcast::<adw::Dialog>()
        .unwrap();
    let entry = find_descendant::<adw::EntryRow>(dialog.upcast_ref()).unwrap();
    let save = find_button(dialog.upcast_ref(), "Save").unwrap();
    entry.set_text("");

    save.emit_clicked();
    drain_main_context();

    assert_eq!(window.widget().dialogs().n_items(), 1);
    assert_eq!(entry.text(), "");
}

fn is_descendant_of(widget: &gtk::Widget, ancestor: &gtk::Widget) -> bool {
    let mut parent = widget.parent();
    while let Some(candidate) = parent {
        if candidate == *ancestor {
            return true;
        }
        parent = candidate.parent();
    }
    false
}

fn find_action_row(root: &gtk::Widget, title: &str) -> Option<adw::ActionRow> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(row) = widget.clone().downcast::<adw::ActionRow>()
            && row.title() == title
        {
            return Some(row);
        }
        if let Some(found) = find_action_row(&widget, title) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn drain_main_context() {
    while glib::MainContext::default().iteration(false) {}
}

fn find_descendant<T>(root: &gtk::Widget) -> Option<T>
where
    T: glib::object::IsA<gtk::Widget> + glib::types::StaticType,
{
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(found) = widget.clone().downcast::<T>() {
            return Some(found);
        }
        if let Some(found) = find_descendant::<T>(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button);
        }
        if let Some(found) = find_button(&widget, label) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}
