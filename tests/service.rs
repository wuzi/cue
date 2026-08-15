use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use chrono::{DateTime, Duration, TimeZone, Utc};
use remind_me::{
    model::{NewReminder, Reminder},
    repository::{ReminderRepository, SqliteReminderRepository},
    scheduler::{Clock, NotificationError, ReminderNotifier, stable_notification_id},
    service::{ActionOutcome, ReminderService},
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
    sent: RefCell<Vec<String>>,
    withdrawn: RefCell<Vec<String>>,
}

impl ReminderNotifier for RecordingNotifier {
    fn send(&self, id: &str, _reminder: &Reminder) -> Result<(), NotificationError> {
        self.sent.borrow_mut().push(id.to_owned());
        Ok(())
    }

    fn withdraw(&self, id: &str) {
        self.withdrawn.borrow_mut().push(id.to_owned());
    }
}

fn create_service(
    now: DateTime<Utc>,
) -> (
    Rc<SqliteReminderRepository>,
    Rc<FakeClock>,
    Rc<RecordingNotifier>,
    ReminderService,
) {
    let repository = Rc::new(SqliteReminderRepository::in_memory().unwrap());
    let clock = Rc::new(FakeClock(Cell::new(now)));
    let notifier = Rc::new(RecordingNotifier::default());
    let service = ReminderService::new(repository.clone(), clock.clone(), notifier.clone());
    (repository, clock, notifier, service)
}

#[test]
fn create_persists_and_changes_background_hold_policy() {
    let now = at(1_800_000_000);
    let (repository, _, _, service) = create_service(now);
    assert!(!service.should_hold_background().unwrap());

    let created = service
        .create(NewReminder::new("  Call Ada  ", now + Duration::hours(1)))
        .unwrap();

    assert_eq!(repository.get(created.id).unwrap(), created);
    assert!(service.should_hold_background().unwrap());
}

#[test]
fn completing_a_delivered_reminder_withdraws_notification_and_releases_hold() {
    let now = at(1_800_000_000);
    let (_, clock, notifier, service) = create_service(now);
    let created = service
        .create(NewReminder::new("Call Ada", now + Duration::minutes(1)))
        .unwrap();
    clock.set(now + Duration::minutes(2));
    service.refresh().unwrap();

    let completed = service.complete(created.id).unwrap();

    assert!(completed.completed_at.is_some());
    assert_eq!(
        notifier.withdrawn.borrow().as_slice(),
        &[stable_notification_id(created.id)]
    );
    assert!(!service.should_hold_background().unwrap());
}

#[test]
fn snooze_withdraws_current_notification_and_schedules_ten_minutes() {
    let now = at(1_800_000_000);
    let (_, clock, notifier, service) = create_service(now);
    let created = service
        .create(NewReminder::new("Call Ada", now + Duration::minutes(1)))
        .unwrap();
    let fired_at = now + Duration::minutes(2);
    clock.set(fired_at);
    service.refresh().unwrap();

    let snoozed = service.snooze(created.id).unwrap();

    assert_eq!(snoozed.due_at, fired_at + Duration::minutes(10));
    assert_eq!(snoozed.notified_at, None);
    assert_eq!(notifier.withdrawn.borrow().len(), 1);
}

#[test]
fn notification_targets_ignore_invalid_or_stale_ids() {
    let now = at(1_800_000_000);
    let (_, _, _, service) = create_service(now);

    assert_eq!(
        service.complete_target("not-a-uuid").unwrap(),
        ActionOutcome::Ignored
    );
    assert_eq!(
        service
            .snooze_target("7de10c1e-cc52-4da1-8040-a8f509ba0589")
            .unwrap(),
        ActionOutcome::Ignored
    );
}

#[test]
fn stale_notification_targets_do_not_reactivate_completed_reminders() {
    let now = at(1_800_000_000);
    let (_, _, notifier, service) = create_service(now);
    let created = service
        .create(NewReminder::new("Call Ada", now + Duration::hours(1)))
        .unwrap();
    service.complete(created.id).unwrap();
    let withdrawals_before_stale_action = notifier.withdrawn.borrow().len();

    assert_eq!(
        service.snooze_target(&created.id.to_string()).unwrap(),
        ActionOutcome::Ignored
    );
    assert_eq!(
        service.complete_target(&created.id.to_string()).unwrap(),
        ActionOutcome::Ignored
    );
    assert!(service.list_active().unwrap().is_empty());
    assert_eq!(service.list_history().unwrap().len(), 1);
    assert_eq!(
        notifier.withdrawn.borrow().len(),
        withdrawals_before_stale_action
    );
}

#[test]
fn notification_open_targets_resolve_only_active_reminders() {
    let now = at(1_800_000_000);
    let (_, clock, _, service) = create_service(now);
    let active = service
        .create(NewReminder::new("Call Ada", now + Duration::minutes(1)))
        .unwrap();
    clock.set(now + Duration::minutes(2));
    service.refresh().unwrap();

    assert_eq!(
        service
            .resolve_active_target(&active.id.to_string())
            .unwrap(),
        Some(active.id)
    );
    assert_eq!(service.resolve_active_target("not-a-uuid").unwrap(), None);

    service.complete(active.id).unwrap();
    assert_eq!(
        service
            .resolve_active_target(&active.id.to_string())
            .unwrap(),
        None
    );
}

#[test]
fn notification_targets_from_before_an_edit_are_ignored() {
    let now = at(1_800_000_000);
    let (_, clock, _, service) = create_service(now);
    let created = service
        .create(NewReminder::new("Call Ada", now + Duration::minutes(1)))
        .unwrap();
    clock.set(now + Duration::minutes(2));
    service.refresh().unwrap();
    let edited_due_at = now + Duration::days(1);
    service
        .edit(created.id, "Call Ada later", edited_due_at)
        .unwrap();

    assert_eq!(
        service.snooze_target(&created.id.to_string()).unwrap(),
        ActionOutcome::Ignored
    );
    let edited = service.get(created.id).unwrap();
    assert_eq!(edited.message, "Call Ada later");
    assert_eq!(edited.due_at, edited_due_at);
    assert_eq!(edited.notified_at, None);
}

#[test]
fn delete_can_be_undone_and_completed_history_can_be_cleared() {
    let now = at(1_800_000_000);
    let (_, _, _, service) = create_service(now);
    let created = service
        .create(NewReminder::new("Call Ada", now + Duration::hours(1)))
        .unwrap();

    let removed = service.delete(created.id).unwrap();
    assert!(service.list_active().unwrap().is_empty());
    service.restore(&removed).unwrap();
    service.complete(created.id).unwrap();

    assert_eq!(service.list_history().unwrap().len(), 1);
    assert_eq!(service.clear_history().unwrap(), 1);
    assert!(service.list_history().unwrap().is_empty());
}

#[test]
fn undoing_a_delivered_deletion_rearms_the_notification() {
    let now = at(1_800_000_000);
    let (_, clock, notifier, service) = create_service(now);
    let created = service
        .create(NewReminder::new("Call Ada", now + Duration::minutes(1)))
        .unwrap();
    clock.set(now + Duration::minutes(2));
    service.refresh().unwrap();
    assert_eq!(notifier.sent.borrow().len(), 1);

    let removed = service.delete(created.id).unwrap();
    service.restore(&removed).unwrap();

    assert_eq!(notifier.sent.borrow().len(), 2);
    assert!(service.get(created.id).unwrap().notified_at.is_some());
}
