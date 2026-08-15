use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use chrono::{DateTime, Duration, TimeZone, Utc};
use remind_me::{
    model::{NewReminder, Reminder},
    repository::{ReminderRepository, SqliteReminderRepository},
    scheduler::{
        Clock, NotificationError, ReminderNotifier, Scheduler, stable_notification_id, wakeup_delay,
    },
};

fn at(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0).single().unwrap()
}

struct FakeClock(Cell<DateTime<Utc>>);

impl FakeClock {
    fn set(&self, now: DateTime<Utc>) {
        self.0.set(now);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.0.get()
    }
}

#[derive(Default)]
struct RecordingNotifier {
    sent: RefCell<Vec<(String, Reminder)>>,
    fail: Cell<bool>,
}

impl ReminderNotifier for RecordingNotifier {
    fn send(&self, id: &str, reminder: &Reminder) -> Result<(), NotificationError> {
        if self.fail.get() {
            return Err(NotificationError::Unavailable(
                "test notification service".into(),
            ));
        }
        self.sent
            .borrow_mut()
            .push((id.to_owned(), reminder.clone()));
        Ok(())
    }

    fn withdraw(&self, _id: &str) {}
}

fn create_scheduler(
    now: DateTime<Utc>,
) -> (
    Rc<SqliteReminderRepository>,
    Rc<FakeClock>,
    Rc<RecordingNotifier>,
    Scheduler,
) {
    let repository = Rc::new(SqliteReminderRepository::in_memory().unwrap());
    let clock = Rc::new(FakeClock(Cell::new(now)));
    let notifier = Rc::new(RecordingNotifier::default());
    let scheduler = Scheduler::new(repository.clone(), clock.clone(), notifier.clone());
    (repository, clock, notifier, scheduler)
}

#[test]
fn overdue_reminder_is_dispatched_once_and_marked_notified() {
    let now = at(1_800_000_000);
    let (repository, clock, notifier, scheduler) = create_scheduler(now);
    let item = Reminder::create(
        NewReminder::new("Call Ada", now + Duration::minutes(1)),
        now,
    )
    .unwrap();
    repository.insert(&item).unwrap();
    clock.set(now + Duration::minutes(2));

    let first = scheduler.refresh().unwrap();
    let second = scheduler.refresh().unwrap();

    assert_eq!(first.delivered, vec![item.id]);
    assert!(second.delivered.is_empty());
    assert_eq!(notifier.sent.borrow().len(), 1);
    assert!(repository.get(item.id).unwrap().notified_at.is_some());
}

#[test]
fn notification_id_is_stable_and_namespaced_by_uuid() {
    let id = uuid::Uuid::parse_str("7de10c1e-cc52-4da1-8040-a8f509ba0589").unwrap();

    assert_eq!(
        stable_notification_id(id),
        "reminder-7de10c1e-cc52-4da1-8040-a8f509ba0589"
    );
}

#[test]
fn next_due_tracks_future_clock_advances_and_suspend_like_jumps() {
    let now = at(1_800_000_000);
    let (repository, clock, notifier, scheduler) = create_scheduler(now);
    let item =
        Reminder::create(NewReminder::new("Call Ada", now + Duration::hours(3)), now).unwrap();
    repository.insert(&item).unwrap();

    assert_eq!(scheduler.next_due().unwrap(), Some(item.due_at));
    clock.set(now + Duration::hours(4));
    scheduler.refresh().unwrap();

    assert_eq!(notifier.sent.borrow().len(), 1);
    assert_eq!(scheduler.next_due().unwrap(), None);
}

#[test]
fn failed_notification_is_left_unnotified_for_a_later_retry() {
    let now = at(1_800_000_000);
    let (repository, clock, notifier, scheduler) = create_scheduler(now);
    let item = Reminder::create(
        NewReminder::new("Call Ada", now + Duration::minutes(1)),
        now,
    )
    .unwrap();
    repository.insert(&item).unwrap();
    clock.set(now + Duration::minutes(2));
    notifier.fail.set(true);

    assert!(scheduler.refresh().is_err());
    assert_eq!(repository.get(item.id).unwrap().notified_at, None);

    notifier.fail.set(false);
    assert_eq!(scheduler.refresh().unwrap().delivered, vec![item.id]);
}

#[test]
fn completed_reminders_are_not_scheduled() {
    let now = at(1_800_000_000);
    let (repository, clock, notifier, scheduler) = create_scheduler(now);
    let item = Reminder::create(
        NewReminder::new("Call Ada", now + Duration::minutes(1)),
        now,
    )
    .unwrap();
    repository.insert(&item).unwrap();
    repository
        .complete(item.id, now + Duration::seconds(30))
        .unwrap();
    clock.set(now + Duration::minutes(2));

    scheduler.refresh().unwrap();

    assert!(notifier.sent.borrow().is_empty());
    assert_eq!(scheduler.next_due().unwrap(), None);
}

#[test]
fn wakeup_delay_uses_exact_near_due_time_and_caps_the_safety_check() {
    let now = at(1_800_000_000);

    assert_eq!(
        wakeup_delay(now, Some(now + Duration::seconds(5))),
        std::time::Duration::from_secs(5)
    );
    assert_eq!(
        wakeup_delay(now, Some(now + Duration::hours(3))),
        std::time::Duration::from_secs(30)
    );
    assert_eq!(
        wakeup_delay(now, Some(now - Duration::minutes(1))),
        std::time::Duration::ZERO
    );
    assert_eq!(wakeup_delay(now, None), std::time::Duration::from_secs(30));
}
