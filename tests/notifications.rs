use chrono::{Duration, TimeZone, Utc};
use remind_me::{
    model::{NewReminder, Reminder},
    notifications::NotificationSpec,
};

#[test]
fn notification_spec_keeps_all_actions_available_outside_the_window() {
    let now = Utc.with_ymd_and_hms(2027, 1, 3, 10, 0, 0).unwrap();
    let reminder =
        Reminder::create(NewReminder::new("Call Ada", now + Duration::hours(1)), now).unwrap();

    let spec = NotificationSpec::for_reminder(&reminder);

    assert_eq!(spec.title, "Reminder");
    assert_eq!(spec.body, "Call Ada");
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
