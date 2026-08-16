use gio::prelude::{ApplicationExt, SettingsExt};
use glib::variant::ToVariant;
use gtk::gdk::prelude::DisplayExt;

use crate::{
    model::Reminder,
    scheduler::{NotificationError, ReminderNotifier},
};

pub const CUE_DESKTOP_ID: &str = "io.github.wuzi.Cue.desktop";

const GNOME_NOTIFICATIONS_SCHEMA: &str = "org.gnome.desktop.notifications";
const GNOME_APP_NOTIFICATIONS_SCHEMA: &str = "org.gnome.desktop.notifications.application";
const CUE_NOTIFICATION_SETTINGS_PATH: &str =
    "/org/gnome/desktop/notifications/application/io-github-wuzi-cue/";

fn system_bell_allowed(
    global_banners: bool,
    app_enabled: bool,
    app_banners: bool,
    sound_alerts: bool,
) -> bool {
    global_banners && app_enabled && app_banners && sound_alerts
}

fn play_system_bell_if_allowed(
    global_banners: bool,
    app_enabled: bool,
    app_banners: bool,
    sound_alerts: bool,
    play: impl FnOnce(),
) {
    if system_bell_allowed(global_banners, app_enabled, app_banners, sound_alerts) {
        play();
    }
}

fn desktop_entry_availability(
    desktop_entry: Option<gio_unix::DesktopAppInfo>,
) -> Result<(), NotificationError> {
    desktop_entry
        .map(|_| ())
        .ok_or_else(NotificationError::missing_desktop_entry)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationButton {
    pub label: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationSpec {
    pub title: String,
    pub body: String,
    pub priority: gio::NotificationPriority,
    pub default_action: String,
    pub target: String,
    pub buttons: Vec<NotificationButton>,
}

impl NotificationSpec {
    pub fn for_reminder(reminder: &Reminder) -> Self {
        Self {
            title: gettextrs::gettext("Reminder"),
            body: reminder.message.clone(),
            priority: gio::NotificationPriority::High,
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
    notification_settings: gio::Settings,
    app_notification_settings: gio::Settings,
}

impl GioReminderNotifier {
    pub fn new(application: gio::Application) -> Self {
        Self {
            application,
            notification_settings: gio::Settings::new(GNOME_NOTIFICATIONS_SCHEMA),
            app_notification_settings: gio::Settings::with_path(
                GNOME_APP_NOTIFICATIONS_SCHEMA,
                CUE_NOTIFICATION_SETTINGS_PATH,
            ),
        }
    }
}

impl ReminderNotifier for GioReminderNotifier {
    fn availability(&self) -> Result<(), NotificationError> {
        desktop_entry_availability(gio_unix::DesktopAppInfo::new(CUE_DESKTOP_ID))
    }

    fn send(&self, id: &str, reminder: &Reminder) -> Result<(), NotificationError> {
        self.availability()?;
        let spec = NotificationSpec::for_reminder(reminder);
        let target = spec.target.to_variant();
        let notification = gio::Notification::new(&spec.title);
        notification.set_body(Some(&spec.body));
        notification.set_priority(spec.priority);
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

    fn play_delivery_sound(&self) {
        play_system_bell_if_allowed(
            self.notification_settings.boolean("show-banners"),
            self.app_notification_settings.boolean("enable"),
            self.app_notification_settings.boolean("show-banners"),
            self.app_notification_settings
                .boolean("enable-sound-alerts"),
            || {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.beep();
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{CUE_DESKTOP_ID, desktop_entry_availability};
    use crate::scheduler::NotificationError;

    #[test]
    fn unavailable_desktop_entry_returns_the_typed_missing_entry_error() {
        assert_eq!(
            desktop_entry_availability(None),
            Err(NotificationError::MissingDesktopEntry {
                desktop_id: CUE_DESKTOP_ID.to_owned(),
            })
        );
    }

    #[test]
    fn system_bell_requires_every_gnome_notification_gate() {
        assert!(super::system_bell_allowed(true, true, true, true));
        assert!(!super::system_bell_allowed(false, true, true, true));
        assert!(!super::system_bell_allowed(true, false, true, true));
        assert!(!super::system_bell_allowed(true, true, false, true));
        assert!(!super::system_bell_allowed(true, true, true, false));
    }

    #[test]
    fn permitted_policy_requests_one_system_bell() {
        let bell_count = Cell::new(0);

        super::play_system_bell_if_allowed(true, true, true, true, || {
            bell_count.set(bell_count.get() + 1);
        });

        assert_eq!(bell_count.get(), 1);
    }
}
