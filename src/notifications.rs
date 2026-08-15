use gio::prelude::ApplicationExt;
use glib::variant::ToVariant;

use crate::{
    model::Reminder,
    scheduler::{NotificationError, ReminderNotifier},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationButton {
    pub label: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSpec {
    pub title: String,
    pub body: String,
    pub default_action: String,
    pub target: String,
    pub buttons: Vec<NotificationButton>,
}

impl NotificationSpec {
    pub fn for_reminder(reminder: &Reminder) -> Self {
        Self {
            title: gettextrs::gettext("Reminder"),
            body: reminder.message.clone(),
            default_action: "app.show-reminder".into(),
            target: reminder.id.to_string(),
            buttons: vec![
                NotificationButton {
                    label: gettextrs::gettext("Done"),
                    action: "app.done".into(),
                },
                NotificationButton {
                    label: gettextrs::gettext("Snooze 10 min"),
                    action: "app.snooze".into(),
                },
            ],
        }
    }
}

pub struct GioReminderNotifier {
    application: gio::Application,
}

impl GioReminderNotifier {
    pub fn new(application: gio::Application) -> Self {
        Self { application }
    }
}

impl ReminderNotifier for GioReminderNotifier {
    fn send(&self, id: &str, reminder: &Reminder) -> Result<(), NotificationError> {
        let spec = NotificationSpec::for_reminder(reminder);
        let target = spec.target.to_variant();
        let notification = gio::Notification::new(&spec.title);
        notification.set_body(Some(&spec.body));
        notification.set_priority(gio::NotificationPriority::Normal);
        notification.set_default_action_and_target_value(&spec.default_action, Some(&target));
        for button in spec.buttons {
            notification.add_button_with_target_value(&button.label, &button.action, Some(&target));
        }
        self.application.send_notification(Some(id), &notification);
        Ok(())
    }

    fn withdraw(&self, id: &str) {
        self.application.withdraw_notification(id);
    }
}
