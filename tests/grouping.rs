use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::America::Sao_Paulo;
use cue::{
    grouping::{ReminderGroup, group_active_reminders},
    model::{NewReminder, Reminder},
};

fn reminder(message: &str, due_at: DateTime<Utc>, created_at: DateTime<Utc>) -> Reminder {
    Reminder::create(NewReminder::new(message, due_at), created_at).unwrap()
}

#[test]
fn active_reminders_are_grouped_by_local_day_and_overdue_state() {
    let now = Sao_Paulo
        .with_ymd_and_hms(2027, 1, 3, 23, 30, 0)
        .unwrap()
        .with_timezone(&Utc);
    let created_at = now - Duration::days(2);
    let items = vec![
        reminder("Overdue", now - Duration::minutes(1), created_at),
        reminder("Today", now + Duration::minutes(15), created_at),
        reminder("Tomorrow", now + Duration::hours(1), created_at),
        reminder("Later", now + Duration::days(3), created_at),
    ];

    let grouped = group_active_reminders(items, now, &Sao_Paulo);

    assert_eq!(grouped[&ReminderGroup::Overdue][0].message, "Overdue");
    assert_eq!(grouped[&ReminderGroup::Today][0].message, "Today");
    assert_eq!(grouped[&ReminderGroup::Tomorrow][0].message, "Tomorrow");
    assert_eq!(grouped[&ReminderGroup::Later][0].message, "Later");
}

#[test]
fn completed_items_are_excluded_even_if_the_caller_supplies_them() {
    let now = Utc.with_ymd_and_hms(2027, 1, 3, 12, 0, 0).unwrap();
    let mut completed = reminder("Done", now + Duration::hours(1), now - Duration::days(1));
    completed.complete(now);

    let grouped = group_active_reminders(vec![completed], now, &Utc);

    assert!(grouped.values().all(Vec::is_empty));
}
