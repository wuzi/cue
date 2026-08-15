use chrono::{DateTime, Duration, TimeZone, Utc};
use remind_me::model::{NewReminder, Reminder, ReminderError};

fn at(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0).single().unwrap()
}

#[test]
fn new_reminder_trims_message_and_sets_timestamps() {
    let now = at(1_800_000_000);
    let due_at = now + Duration::hours(1);

    let reminder = Reminder::create(NewReminder::new("  Call Ada  ", due_at), now).unwrap();

    assert_eq!(reminder.message, "Call Ada");
    assert_eq!(reminder.due_at, due_at);
    assert_eq!(reminder.created_at, now);
    assert_eq!(reminder.updated_at, now);
    assert_eq!(reminder.notified_at, None);
    assert_eq!(reminder.completed_at, None);
}

#[test]
fn new_reminder_rejects_blank_and_overlong_messages() {
    let now = at(1_800_000_000);
    let due_at = now + Duration::hours(1);

    assert_eq!(
        Reminder::create(NewReminder::new("   ", due_at), now).unwrap_err(),
        ReminderError::EmptyMessage
    );
    assert_eq!(
        Reminder::create(NewReminder::new("x".repeat(281), due_at), now).unwrap_err(),
        ReminderError::MessageTooLong
    );
}

#[test]
fn new_reminder_requires_a_future_due_time() {
    let now = at(1_800_000_000);

    assert_eq!(
        Reminder::create(NewReminder::new("Call Ada", now), now).unwrap_err(),
        ReminderError::DueTimeNotFuture
    );
}

#[test]
fn snooze_moves_due_time_ten_minutes_and_rearms_delivery() {
    let now = at(1_800_000_000);
    let mut reminder =
        Reminder::create(NewReminder::new("Call Ada", now + Duration::hours(1)), now).unwrap();
    reminder.notified_at = Some(now + Duration::hours(1));
    let snoozed_at = now + Duration::hours(2);

    reminder.snooze(snoozed_at);

    assert_eq!(reminder.due_at, snoozed_at + Duration::minutes(10));
    assert_eq!(reminder.updated_at, snoozed_at);
    assert_eq!(reminder.notified_at, None);
    assert_eq!(reminder.completed_at, None);
}

#[test]
fn edit_revalidates_and_rearms_a_reminder() {
    let now = at(1_800_000_000);
    let mut reminder =
        Reminder::create(NewReminder::new("Call Ada", now + Duration::hours(1)), now).unwrap();
    reminder.notified_at = Some(now + Duration::hours(1));
    reminder.completed_at = Some(now + Duration::hours(1));
    let edited_at = now + Duration::hours(2);
    let new_due_at = edited_at + Duration::days(1);

    reminder
        .edit("  Send Ada the notes  ", new_due_at, edited_at)
        .unwrap();

    assert_eq!(reminder.message, "Send Ada the notes");
    assert_eq!(reminder.due_at, new_due_at);
    assert_eq!(reminder.updated_at, edited_at);
    assert_eq!(reminder.notified_at, None);
    assert_eq!(reminder.completed_at, None);
}

#[test]
fn delivery_and_completion_change_observable_state() {
    let now = at(1_800_000_000);
    let mut reminder =
        Reminder::create(NewReminder::new("Call Ada", now + Duration::hours(1)), now).unwrap();
    let due_at = reminder.due_at;

    assert!(reminder.is_active());
    assert!(!reminder.is_due(now));
    assert!(reminder.is_due(due_at));

    reminder.mark_notified(due_at);
    assert_eq!(reminder.notified_at, Some(due_at));

    reminder.complete(due_at + Duration::minutes(1));
    assert!(!reminder.is_active());
    assert_eq!(reminder.completed_at, Some(due_at + Duration::minutes(1)));
}
