use chrono::{DateTime, Duration, TimeZone, Utc};
use cue::{
    model::{NewReminder, Reminder},
    repository::{ReminderRepository, RepositoryError, SqliteReminderRepository},
};
use tempfile::tempdir;

fn at(timestamp: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(timestamp, 0).single().unwrap()
}

fn reminder(message: &str, due_at: DateTime<Utc>, now: DateTime<Utc>) -> Reminder {
    Reminder::create(NewReminder::new(message, due_at), now).unwrap()
}

#[test]
fn opening_repository_applies_schema_version_two() {
    let repository = SqliteReminderRepository::in_memory().unwrap();

    assert_eq!(repository.schema_version().unwrap(), 2);
}

#[test]
fn opening_a_newer_schema_returns_a_typed_error() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("newer.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 3;")
        .unwrap();
    drop(connection);

    assert!(matches!(
        SqliteReminderRepository::open(path),
        Err(RepositoryError::UnsupportedSchema(3))
    ));
}

#[test]
fn active_and_history_lists_are_filtered_and_sorted() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let later = reminder("Later", now + Duration::hours(3), now);
    let sooner = reminder("Sooner", now + Duration::hours(1), now);
    let mut completed = reminder("Done", now + Duration::hours(2), now);
    completed.complete(now + Duration::hours(4));

    repository.insert(&later).unwrap();
    repository.insert(&completed).unwrap();
    repository.insert(&sooner).unwrap();

    let active = repository.list_active().unwrap();
    let history = repository.list_history().unwrap();
    assert_eq!(
        active
            .iter()
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>(),
        vec!["Sooner", "Later"]
    );
    assert_eq!(
        history
            .iter()
            .map(|item| item.message.as_str())
            .collect::<Vec<_>>(),
        vec!["Done"]
    );
}

#[test]
fn repository_lifecycle_operations_persist_the_model_transitions() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let item = reminder("Call Ada", now + Duration::hours(1), now);
    let id = item.id;
    repository.insert(&item).unwrap();

    let notified_at = now + Duration::hours(1);
    let notified = repository.mark_notified(id, notified_at).unwrap();
    assert_eq!(notified.notified_at, Some(notified_at));

    let snoozed_at = notified_at + Duration::minutes(2);
    let snoozed = repository.snooze(id, snoozed_at).unwrap();
    assert_eq!(snoozed.due_at, snoozed_at + Duration::minutes(10));
    assert_eq!(snoozed.notified_at, None);

    let edited_at = snoozed_at + Duration::minutes(1);
    let edited_due_at = edited_at + Duration::days(1);
    let edited = repository
        .edit(id, "  Send Ada notes  ", edited_due_at, edited_at)
        .unwrap();
    assert_eq!(edited.message, "Send Ada notes");
    assert_eq!(edited.due_at, edited_due_at);

    let completed_at = edited_at + Duration::hours(1);
    let completed = repository.complete(id, completed_at).unwrap();
    assert_eq!(completed.completed_at, Some(completed_at));
    assert!(repository.list_active().unwrap().is_empty());
    assert_eq!(repository.list_history().unwrap(), vec![completed]);
}

#[test]
fn rejected_edit_leaves_the_stored_reminder_unchanged() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let item = reminder("Call Ada", now + Duration::hours(1), now);
    repository.insert(&item).unwrap();

    assert!(matches!(
        repository.edit(item.id, "Changed", now, now),
        Err(RepositoryError::InvalidReminder(_))
    ));
    assert_eq!(repository.get(item.id).unwrap(), item);
}

#[test]
fn delete_restore_and_clear_history_are_reversible_until_cleared() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let item = reminder("Call Ada", now + Duration::hours(1), now);
    repository.insert(&item).unwrap();

    let removed = repository.delete(item.id).unwrap();
    assert_eq!(removed, item);
    assert!(matches!(
        repository.get(item.id),
        Err(RepositoryError::NotFound(id)) if id == item.id
    ));

    repository.restore(&removed).unwrap();
    repository
        .complete(item.id, now + Duration::hours(2))
        .unwrap();
    assert_eq!(repository.clear_history().unwrap(), 1);
    assert!(repository.list_history().unwrap().is_empty());
}

#[test]
fn file_repository_survives_reopening() {
    let now = at(1_800_000_000);
    let directory = tempdir().unwrap();
    let path = directory.path().join("reminders.db");
    let item = reminder("Call Ada", now + Duration::hours(1), now);

    SqliteReminderRepository::open(&path)
        .unwrap()
        .insert(&item)
        .unwrap();

    let reopened = SqliteReminderRepository::open(&path).unwrap();
    assert_eq!(reopened.get(item.id).unwrap(), item);
}
