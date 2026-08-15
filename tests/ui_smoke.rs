use std::rc::Rc;

use adw::prelude::*;
use chrono::{Duration, Utc};
use gio::prelude::ListModelExt;
use remind_me::{
    model::NewReminder, notifications::GioReminderNotifier, repository::SqliteReminderRepository,
    resources, scheduler::SystemClock, service::ReminderService, ui,
};

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
    assert_eq!(widgets.message_entry.title(), "Reminder message");
    assert_eq!(widgets.message_entry.max_length(), 280);
    assert_eq!(widgets.add_button.label().as_deref(), Some("Add Reminder"));
    assert!(!widgets.composer_error.is_visible());
    assert_eq!(widgets.window.default_width(), 720);
    assert_eq!(widgets.window.default_height(), 700);
    assert!(!widgets.bottom_switcher.reveals());

    widgets.window.set_default_size(480, 650);
    widgets.window.present();
    drain_main_context();
    assert!(!widgets.header_switcher.is_visible());
    assert!(widgets.bottom_switcher.reveals());
    widgets.window.close();
    drain_main_context();

    let repository = Rc::new(SqliteReminderRepository::in_memory().unwrap());
    let notifier = Rc::new(GioReminderNotifier::new(
        application.clone().upcast::<gio::Application>(),
    ));
    let service = Rc::new(ReminderService::new(
        repository,
        Rc::new(SystemClock),
        notifier,
    ));
    let reminder = service
        .create(NewReminder::new("Call Ada", Utc::now() + Duration::days(1)))
        .unwrap();
    let window = ui::MainWindow::new(&application, service, || {}, || {}).unwrap();
    window.present();
    drain_main_context();

    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.edit",
        Some(&reminder.id.to_string().to_variant()),
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
