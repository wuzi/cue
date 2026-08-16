use chrono::{Duration, TimeZone, Utc};
use cue::{
    app::{RuntimeWarningAction, RuntimeWarningState},
    model::{NewReminder, Reminder},
    notifications::{CUE_DESKTOP_ID, NotificationSpec},
    scheduler::{NotificationError, ReminderNotifier},
};

struct NoopNotifier;

impl ReminderNotifier for NoopNotifier {
    fn send(&self, _id: &str, _reminder: &Reminder) -> Result<(), NotificationError> {
        Ok(())
    }

    fn withdraw(&self, _id: &str) {}
}

#[test]
fn notification_spec_keeps_all_actions_available_outside_the_window() {
    let now = Utc.with_ymd_and_hms(2027, 1, 3, 10, 0, 0).unwrap();
    let reminder =
        Reminder::create(NewReminder::new("Call Ada", now + Duration::hours(1)), now).unwrap();

    let spec = NotificationSpec::for_reminder(&reminder);

    assert_eq!(spec.title, "Reminder");
    assert_eq!(spec.body, "Call Ada");
    assert_eq!(spec.priority, gio::NotificationPriority::High);
    assert_eq!(spec.default_action, "app.show-reminder");
    assert_eq!(spec.target, reminder.id.to_string());
    assert_eq!(
        spec.buttons
            .iter()
            .map(|button| (button.label.as_str(), button.action.as_str()))
            .collect::<Vec<_>>(),
        vec![("Done", "app.done"), ("Snooze 10 min", "app.snooze")]
    );
}

#[test]
fn notifier_availability_defaults_to_available_for_existing_fakes() {
    assert_eq!(NoopNotifier.availability(), Ok(()));
}

#[test]
fn missing_desktop_entry_error_carries_the_cue_desktop_id() {
    assert_eq!(
        NotificationError::missing_desktop_entry(),
        NotificationError::MissingDesktopEntry {
            desktop_id: CUE_DESKTOP_ID.to_owned(),
        }
    );
}

#[test]
fn notification_diagnostics_are_deduplicated_but_runtime_errors_are_not() {
    let mut warnings = RuntimeWarningState::default();
    let message =
        "Notifications are unavailable in this development run. Install Cue to receive reminders.";

    assert_eq!(
        warnings.report_notification_warning(message, false),
        RuntimeWarningAction::LogOnly
    );
    assert_eq!(
        warnings.report_notification_warning(message, false),
        RuntimeWarningAction::Ignore
    );
    assert_eq!(warnings.take_pending(), vec![message]);
    let unavailable = NotificationError::Unavailable("test notification service".into());
    assert_eq!(
        warnings.report_notification_error(&unavailable, false),
        (
            RuntimeWarningAction::LogOnly,
            "notification service unavailable: test notification service".into(),
        )
    );
    assert_eq!(
        warnings.report_notification_error(&unavailable, false),
        (
            RuntimeWarningAction::LogOnly,
            "notification service unavailable: test notification service".into(),
        )
    );
    assert!(warnings.take_pending().is_empty());
    assert_eq!(
        warnings.report_runtime_error("database write failed", false),
        RuntimeWarningAction::LogOnly
    );
    assert_eq!(
        warnings.report_runtime_error("database write failed", false),
        RuntimeWarningAction::LogOnly
    );
    assert!(warnings.take_pending().is_empty());
}
