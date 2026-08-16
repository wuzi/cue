use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use chrono::{DateTime, Duration, TimeZone, Utc};
use cue::{
    model::{CanvasEntry, DeletedCanvasItem, NewReminder, Reminder, ScheduleState},
    repository::{ReminderRepository, RepositoryError, SqliteReminderRepository},
    scheduler::{
        Clock, NotificationError, ReminderNotifier, Scheduler, SchedulerError,
        refresh_wakeup_delay, stable_notification_id, wakeup_delay,
    },
};
use uuid::Uuid;

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
    sounds: Cell<usize>,
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

    fn play_delivery_sound(&self) {
        self.sounds.set(self.sounds.get() + 1);
    }
}

struct FailingFinalStateRepository {
    inner: SqliteReminderRepository,
}

impl FailingFinalStateRepository {
    fn in_memory() -> Self {
        Self {
            inner: SqliteReminderRepository::in_memory().unwrap(),
        }
    }
}

macro_rules! delegate_repository_method {
    ($name:ident($($argument:ident: $type:ty),* $(,)?) -> $output:ty) => {
        fn $name(&self, $($argument: $type),*) -> Result<$output, RepositoryError> {
            self.inner.$name($($argument),*)
        }
    };
}

impl ReminderRepository for FailingFinalStateRepository {
    delegate_repository_method!(insert(reminder: &Reminder) -> ());
    delegate_repository_method!(restore(reminder: &Reminder) -> ());
    delegate_repository_method!(get(id: Uuid) -> Reminder);

    fn list_active(&self) -> Result<Vec<Reminder>, RepositoryError> {
        panic!("scheduler refresh must use targeted due/state queries")
    }

    fn list_due(&self, now: DateTime<Utc>) -> Result<Vec<Reminder>, RepositoryError> {
        self.inner.list_due(now)
    }

    fn schedule_state(&self) -> Result<ScheduleState, RepositoryError> {
        Err(RepositoryError::Database(
            rusqlite::Error::ExecuteReturnedResults,
        ))
    }

    delegate_repository_method!(list_history() -> Vec<Reminder>);
    delegate_repository_method!(edit(
        id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Reminder);
    delegate_repository_method!(mark_notified(id: Uuid, now: DateTime<Utc>) -> Reminder);
    delegate_repository_method!(snooze(id: Uuid, now: DateTime<Utc>) -> Reminder);
    delegate_repository_method!(snooze_canvas_reminder(
        id: Uuid,
        now: DateTime<Utc>,
        working_text: Option<(Uuid, &str)>,
    ) -> Reminder);
    delegate_repository_method!(complete(id: Uuid, now: DateTime<Utc>) -> Reminder);
    delegate_repository_method!(delete(id: Uuid) -> Reminder);
    delegate_repository_method!(clear_history() -> usize);
    delegate_repository_method!(list_canvas_entries() -> Vec<CanvasEntry>);
    delegate_repository_method!(append_canvas_entry(
        message: &str,
        reminder: Option<&Reminder>,
        now: DateTime<Utc>,
    ) -> CanvasEntry);
    delegate_repository_method!(save_canvas_working_text(
        id: Uuid,
        text: Option<&str>,
        now: DateTime<Utc>,
    ) -> ());
    delegate_repository_method!(load_canvas_draft() -> String);
    delegate_repository_method!(save_canvas_draft(text: &str) -> ());
    delegate_repository_method!(get_canvas_entry(id: Uuid) -> CanvasEntry);
    delegate_repository_method!(attach_canvas_reminder(
        entry_id: Uuid,
        reminder: &Reminder,
        now: DateTime<Utc>,
    ) -> CanvasEntry);
    delegate_repository_method!(detach_canvas_reminder(
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> (CanvasEntry, Reminder));
    delegate_repository_method!(complete_canvas_reminder(
        reminder_id: Uuid,
        now: DateTime<Utc>,
    ) -> Reminder);
    delegate_repository_method!(delete_canvas_entry(id: Uuid) -> DeletedCanvasItem);
    delegate_repository_method!(restore_canvas_item(item: &DeletedCanvasItem) -> ());
    delegate_repository_method!(update_canvas_note(
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> CanvasEntry);
    delegate_repository_method!(rename_canvas_reminder(
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> (CanvasEntry, Reminder));
    delegate_repository_method!(reschedule_canvas_reminder(
        entry_id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> (CanvasEntry, Reminder));
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
    assert_eq!(
        first.state,
        ScheduleState {
            has_active: true,
            next_due: None,
        }
    );
    assert!(second.delivered.is_empty());
    assert_eq!(notifier.sent.borrow().len(), 1);
    assert!(repository.get(item.id).unwrap().notified_at.is_some());
}

#[test]
fn multiple_due_reminders_share_one_delivery_sound() {
    let now = at(1_800_000_000);
    let (repository, clock, notifier, scheduler) = create_scheduler(now);
    let first = Reminder::create(
        NewReminder::new("Call Ada", now + Duration::minutes(1)),
        now,
    )
    .unwrap();
    let second = Reminder::create(
        NewReminder::new("Send the report", now + Duration::minutes(2)),
        now,
    )
    .unwrap();
    repository.insert(&first).unwrap();
    repository.insert(&second).unwrap();
    clock.set(now + Duration::minutes(3));

    let result = scheduler.refresh().unwrap();

    assert_eq!(result.delivered, vec![first.id, second.id]);
    assert_eq!(notifier.sent.borrow().len(), 2);
    assert_eq!(notifier.sounds.get(), 1);
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
    assert_eq!(notifier.sounds.get(), 0);

    notifier.fail.set(false);
    assert_eq!(scheduler.refresh().unwrap().delivered, vec![item.id]);
    assert_eq!(notifier.sounds.get(), 1);
}

#[test]
fn failed_final_schedule_read_does_not_play_delivery_sound() {
    let now = at(1_800_000_000);
    let repository = Rc::new(FailingFinalStateRepository::in_memory());
    let clock = Rc::new(FakeClock(Cell::new(now)));
    let notifier = Rc::new(RecordingNotifier::default());
    let scheduler = Scheduler::new(repository.clone(), clock.clone(), notifier.clone());
    let item = Reminder::create(
        NewReminder::new("Call Ada", now + Duration::minutes(1)),
        now,
    )
    .unwrap();
    repository.insert(&item).unwrap();
    clock.set(now + Duration::minutes(2));

    let result = scheduler.refresh();

    assert!(matches!(result, Err(SchedulerError::Repository(_))));
    assert_eq!(notifier.sent.borrow().len(), 1);
    assert_eq!(notifier.sounds.get(), 0);
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
    assert_eq!(notifier.sounds.get(), 0);
    assert_eq!(scheduler.next_due().unwrap(), None);
}

#[test]
fn wakeup_delay_uses_exact_near_due_time_and_caps_the_safety_check() {
    let now = at(1_800_000_000);

    assert_eq!(
        wakeup_delay(now, Some(now + Duration::seconds(5))),
        Some(std::time::Duration::from_secs(5))
    );
    assert_eq!(
        wakeup_delay(now, Some(now + Duration::hours(3))),
        Some(std::time::Duration::from_secs(30))
    );
    assert_eq!(
        wakeup_delay(now, Some(now - Duration::minutes(1))),
        Some(std::time::Duration::ZERO)
    );
    assert_eq!(wakeup_delay(now, None), None);
    assert_eq!(refresh_wakeup_delay(true, now, None), None);
}

#[test]
fn failed_refresh_retries_on_the_thirty_second_safety_interval() {
    let now = at(1_800_000_000);

    assert_eq!(
        refresh_wakeup_delay(false, now, Some(now - Duration::minutes(1))),
        Some(std::time::Duration::from_secs(30))
    );
}
