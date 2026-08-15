use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::America::New_York;
use remind_me::{
    model::{NewReminder, Reminder},
    repository::{ReminderRepository, SqliteReminderRepository},
    schedule::{DaySpec, ScheduleError, ScheduleExpression},
    scheduler::{Clock, NotificationError, ReminderNotifier, stable_notification_id},
    service::{ActionOutcome, ReminderService},
};

fn at(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0).single().unwrap()
}

struct FakeClock {
    now: Cell<DateTime<Utc>>,
    reads: Cell<usize>,
}

impl FakeClock {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            now: Cell::new(now),
            reads: Cell::new(0),
        }
    }

    fn set(&self, now: DateTime<Utc>) {
        self.now.set(now);
    }

    fn reads(&self) -> usize {
        self.reads.get()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.reads.set(self.reads.get() + 1);
        self.now.get()
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
    let clock = Rc::new(FakeClock::new(now));
    let notifier = Rc::new(RecordingNotifier::default());
    let service = ReminderService::new(repository.clone(), clock.clone(), notifier.clone());
    (repository, clock, notifier, service)
}

#[test]
fn relative_creation_uses_exact_preset_delays_and_refreshes_scheduling() {
    let now = at(1_800_000_000);
    for delay in [
        Duration::minutes(15),
        Duration::minutes(30),
        Duration::hours(1),
        Duration::hours(3),
        Duration::hours(24),
    ] {
        let (repository, clock, _, service) = create_service(now);

        let created = service.create_relative("Call Ada", delay).unwrap();

        assert_eq!(created.due_at, now + delay);
        assert_eq!(created.created_at, now);
        assert_eq!(created.updated_at, now);
        assert_eq!(repository.get(created.id).unwrap(), created);
        assert_eq!(service.next_due().unwrap(), Some(now + delay));
        assert_eq!(clock.reads(), 2);
    }
}

#[test]
fn relative_creation_rejects_non_positive_delays_without_persisting() {
    let now = at(1_800_000_000);
    for delay in [Duration::zero(), Duration::minutes(-1)] {
        let (repository, clock, _, service) = create_service(now);

        let error = service.create_relative("Call Ada", delay).unwrap_err();

        assert!(matches!(
            error,
            remind_me::service::ServiceError::InvalidReminder(
                remind_me::model::ReminderError::DueTimeNotFuture
            )
        ));
        assert!(repository.list_active().unwrap().is_empty());
        assert_eq!(clock.reads(), 1);
    }
}

#[test]
fn schedule_preview_uses_the_service_clock_without_persisting() {
    let now = New_York
        .with_ymd_and_hms(2026, 8, 15, 16, 42, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let (repository, clock, _, service) = create_service(now);
    let schedule = ScheduleExpression::Date {
        day: DaySpec::Tomorrow,
        time: None,
    };

    let preview = service.preview_schedule(&schedule, &New_York).unwrap();

    assert_eq!(
        preview,
        New_York
            .with_ymd_and_hms(2026, 8, 16, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );
    assert_eq!(clock.reads(), 1);
    assert!(repository.list_active().unwrap().is_empty());
}

#[test]
fn scheduled_creation_captures_now_once_then_refreshes_the_scheduler() {
    let now = at(1_800_000_000);
    let (repository, clock, _, service) = create_service(now);
    let schedule = ScheduleExpression::Relative(Duration::hours(24));

    let created = service
        .create_scheduled("Call Ada", &schedule, &Utc)
        .unwrap();

    assert_eq!(created.due_at, now + Duration::hours(24));
    assert_eq!(created.created_at, now);
    assert_eq!(repository.get(created.id).unwrap(), created);
    assert_eq!(service.next_due().unwrap(), Some(now + Duration::hours(24)));
    assert_eq!(clock.reads(), 2);
}

#[test]
fn scheduled_creation_rejects_resolution_errors_without_persisting() {
    let now = at(1_800_000_000);
    let (repository, clock, _, service) = create_service(now);
    let schedule = ScheduleExpression::Relative(Duration::zero());

    let error = service
        .create_scheduled("Call Ada", &schedule, &Utc)
        .unwrap_err();

    assert!(matches!(
        error,
        remind_me::service::ServiceError::InvalidSchedule(ScheduleError::DueTimeNotFuture)
    ));
    assert!(repository.list_active().unwrap().is_empty());
    assert_eq!(clock.reads(), 1);
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
