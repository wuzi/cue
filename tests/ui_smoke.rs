use std::rc::Rc;

use adw::prelude::*;
use chrono::{DateTime, Duration, TimeZone, Utc};
use gio::prelude::ListModelExt;
use remind_me::{
    model::Reminder,
    repository::{ReminderRepository, SqliteReminderRepository},
    resources,
    schedule::{ScheduleParseStatus, parse_english},
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
    assert_eq!(
        widgets.navigation_view.visible_page_tag().as_deref(),
        Some("reminders")
    );
    assert_eq!(widgets.composer_input.wrap_mode(), gtk::WrapMode::WordChar);
    assert!(!widgets.composer_input.accepts_tab());
    assert_eq!(
        widgets.composer_placeholder.label(),
        "What should I remind you about?"
    );
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
    assert_eq!(widgets.schedule_preview.label(), "In 1 hour");
    assert!(!widgets.composer_error.is_visible());
    assert_eq!(
        widgets.composer_error.accessible_role(),
        gtk::AccessibleRole::Alert
    );
    assert_eq!(widgets.window.default_width(), 560);
    assert_eq!(widgets.window.default_height(), 540);
    assert_eq!(
        widgets.reminders_content.visible_child_name().as_deref(),
        Some("empty")
    );
    assert!(!is_descendant_of(
        widgets.composer_input.upcast_ref(),
        widgets.reminders_scroller.upcast_ref()
    ));
    assert!(
        find_label(
            widgets.reminders_empty.upcast_ref(),
            "Call Ada @tomorrow 9am"
        )
        .is_some()
    );
    assert!(
        find_label(
            widgets.reminders_empty.upcast_ref(),
            "Take a break @in 30 minutes"
        )
        .is_some()
    );

    widgets.window.set_default_size(360, 500);
    widgets.window.present();
    drain_main_context();
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

    let message = find_descendant::<gtk::TextView>(window.widget().upcast_ref()).unwrap();
    let preview = find_label(window.widget().upcast_ref(), "In 1 hour").unwrap();
    message
        .buffer()
        .set_text("First line\nSecond line @in 15 minutes");
    drain_main_context();
    assert_eq!(
        buffer_text(&message.buffer()),
        "First line Second line @in 15 minutes"
    );
    message.grab_focus();
    message.buffer().set_text("Walk @");
    drain_main_context();
    let suggestions = find_css_class(window.widget().upcast_ref(), "suggestions")
        .unwrap()
        .downcast::<gtk::ListBox>()
        .unwrap();
    assert_eq!(suggestions.selected_row().unwrap().index(), 0);
    assert!(message.has_focus());
    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.use-suggestion",
        Some(&"in 15 minutes".to_variant()),
    )
    .unwrap();
    drain_main_context();
    assert_eq!(buffer_text(&message.buffer()), "Walk @in 15 minutes");

    message.buffer().set_text("Call Ada @in 30 minutes");
    drain_main_context();
    assert!(!preview.label().is_empty());
    let schedule = message.buffer().iter_at_offset(10);
    assert!(
        schedule
            .tags()
            .iter()
            .any(|tag| tag.name().as_deref() == Some("schedule"))
    );
    find_icon_button(window.widget().upcast_ref(), "list-add-symbolic")
        .unwrap()
        .emit_clicked();
    drain_main_context();

    let reminders = repository.list_active().unwrap();
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].message, "Call Ada");
    assert_eq!(reminders[0].due_at, now + Duration::minutes(30));
    assert_eq!(buffer_text(&message.buffer()), "");
    assert_eq!(preview.label(), "In 1 hour");
    let content = find_descendant::<gtk::Stack>(window.widget().upcast_ref()).unwrap();
    assert_eq!(content.visible_child_name().as_deref(), Some("list"));
    assert!(find_descendant::<adw::PreferencesGroup>(content.upcast_ref()).is_none());
    let saved_row = find_action_row(window.widget().upcast_ref(), "Call Ada").unwrap();
    let touch_menu = find_descendant::<gtk::MenuButton>(saved_row.upcast_ref()).unwrap();
    assert!(
        touch_menu
            .menu_model()
            .is_some_and(|menu| menu.n_items() >= 3)
    );
    let row_controls = find_css_class(saved_row.upcast_ref(), "row-controls").unwrap();
    assert!(!row_controls.can_target());
    saved_row.grab_focus();
    drain_main_context();
    assert!(row_controls.can_target());
    message.grab_focus();
    drain_main_context();
    assert!(!row_controls.can_target());

    message.buffer().set_text("Call Ada @someday");
    drain_main_context();
    let add = find_icon_button(window.widget().upcast_ref(), "list-add-symbolic").unwrap();
    assert!(!add.is_sensitive());
    assert_eq!(buffer_text(&message.buffer()), "Call Ada @someday");

    message.buffer().set_text("Lunch @tomorrow noon");
    drain_main_context();
    let preview_button = find_css_class(window.widget().upcast_ref(), "schedule-preview")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    preview_button.emit_clicked();
    drain_main_context();
    let custom = find_calendar_popover(window.widget().upcast_ref()).unwrap();
    let before_custom = buffer_text(&message.buffer());
    find_descendant::<gtk::Calendar>(custom.upcast_ref())
        .unwrap()
        .set_date(&glib::DateTime::from_unix_local(1).unwrap());
    find_button(custom.upcast_ref(), "Apply")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    assert!(custom.parent().is_some());
    assert!(
        find_label(custom.upcast_ref(), "Choose a time in the future")
            .is_some_and(|label| label.is_visible())
    );
    assert_eq!(buffer_text(&message.buffer()), before_custom);
    find_button(custom.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    drain_main_context();

    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.show-history", None).unwrap();
    drain_main_context();
    let navigation = find_descendant::<adw::NavigationView>(window.widget().upcast_ref()).unwrap();
    assert_eq!(navigation.visible_page_tag().as_deref(), Some("history"));
    assert!(navigation.pop());
    drain_main_context();
    assert_eq!(navigation.visible_page_tag().as_deref(), Some("reminders"));

    let custom_window = ui::MainWindow::new(&application, service.clone(), || {}, || {}).unwrap();
    custom_window.present();
    drain_main_context();
    let custom_input =
        find_descendant::<gtk::TextView>(custom_window.widget().upcast_ref()).unwrap();
    custom_input.buffer().set_text("Plan @tomorrow noon");
    drain_main_context();
    find_css_class(custom_window.widget().upcast_ref(), "schedule-preview")
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    drain_main_context();
    let custom = find_calendar_popover(custom_window.widget().upcast_ref()).unwrap();
    find_button(custom.upcast_ref(), "Apply")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    let applied = buffer_text(&custom_input.buffer());
    assert!(applied.starts_with("Plan @"));
    assert!(!applied.contains("tomorrow"));
    assert!(matches!(
        parse_english(&applied).status,
        ScheduleParseStatus::Valid(_)
    ));
    custom_window.widget().close();
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

fn find_label(root: &gtk::Widget, label: &str) -> Option<gtk::Label> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(found) = widget.clone().downcast::<gtk::Label>()
            && found.label() == label
        {
            return Some(found);
        }
        if let Some(found) = find_label(&widget, label) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_icon_button(root: &gtk::Widget, icon_name: &str) -> Option<gtk::Button> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.icon_name().as_deref() == Some(icon_name)
        {
            return Some(button);
        }
        if let Some(found) = find_icon_button(&widget, icon_name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
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

fn find_css_class(root: &gtk::Widget, css_class: &str) -> Option<gtk::Widget> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(css_class) {
            return Some(widget);
        }
        if let Some(found) = find_css_class(&widget, css_class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn buffer_text(buffer: &gtk::TextBuffer) -> glib::GString {
    buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
}

fn find_calendar_popover(root: &gtk::Widget) -> Option<gtk::Popover> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(popover) = widget.clone().downcast::<gtk::Popover>()
            && find_descendant::<gtk::Calendar>(popover.upcast_ref()).is_some()
        {
            return Some(popover);
        }
        if let Some(found) = find_calendar_popover(&widget) {
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
