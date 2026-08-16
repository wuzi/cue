use chrono::{DateTime, Duration, TimeZone, Utc};
use cue::{
    model::{NewReminder, Reminder, ScheduleState},
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
fn opening_repository_applies_schema_version_three() {
    let repository = SqliteReminderRepository::in_memory().unwrap();

    assert_eq!(repository.schema_version().unwrap(), 3);
}

#[test]
fn opening_a_newer_schema_returns_a_typed_error() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("newer.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 4;")
        .unwrap();
    drop(connection);

    assert!(matches!(
        SqliteReminderRepository::open(path),
        Err(RepositoryError::UnsupportedSchema(4))
    ));
}

#[test]
fn due_list_and_schedule_state_exclude_notified_and_completed_items() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let due = reminder("Due", now + Duration::minutes(1), now);
    let future = reminder("Future", now + Duration::hours(2), now);
    let mut notified = reminder("Already notified", now + Duration::minutes(2), now);
    notified.mark_notified(now + Duration::minutes(2));
    let mut completed = reminder("Completed", now + Duration::minutes(3), now);
    completed.complete(now + Duration::minutes(4));
    for item in [&future, &notified, &completed, &due] {
        repository.insert(item).unwrap();
    }

    assert_eq!(
        repository.list_due(now + Duration::minutes(10)).unwrap(),
        vec![due.clone()]
    );
    repository
        .mark_notified(due.id, now + Duration::minutes(10))
        .unwrap();
    assert_eq!(
        repository.schedule_state().unwrap(),
        ScheduleState {
            has_active: true,
            next_due: Some(future.due_at),
        }
    );

    repository
        .mark_notified(future.id, now + Duration::hours(2))
        .unwrap();
    assert_eq!(
        repository.schedule_state().unwrap(),
        ScheduleState {
            has_active: true,
            next_due: None,
        }
    );
}

#[test]
fn schema_two_migration_adds_the_pending_due_partial_index() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("version-two.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE reminders (
                id TEXT PRIMARY KEY NOT NULL,
                message TEXT NOT NULL CHECK(length(message) BETWEEN 1 AND 280),
                due_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                notified_at INTEGER,
                completed_at INTEGER
            );
            CREATE INDEX reminders_active_due_idx
                ON reminders(due_at) WHERE completed_at IS NULL;
            CREATE INDEX reminders_history_idx
                ON reminders(completed_at DESC) WHERE completed_at IS NOT NULL;
            CREATE TABLE canvas_entries (
                id TEXT PRIMARY KEY NOT NULL,
                position INTEGER NOT NULL,
                message TEXT NOT NULL CHECK(length(message) BETWEEN 1 AND 280),
                reminder_id TEXT UNIQUE REFERENCES reminders(id) ON DELETE CASCADE,
                working_text TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX canvas_entries_position_idx
                ON canvas_entries(position, created_at);
            CREATE TABLE canvas_state (
                id INTEGER PRIMARY KEY NOT NULL CHECK(id = 1),
                draft_text TEXT NOT NULL
            );
            INSERT INTO canvas_state (id, draft_text) VALUES (1, '');
            PRAGMA user_version = 2;",
        )
        .unwrap();
    drop(connection);

    let repository = SqliteReminderRepository::open(&path).unwrap();
    assert_eq!(repository.schema_version().unwrap(), 3);
    drop(repository);

    let connection = rusqlite::Connection::open(path).unwrap();
    let index_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            ["reminders_pending_due_idx"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(index_sql.contains("due_at, created_at"));
    assert!(index_sql.contains("completed_at IS NULL AND notified_at IS NULL"));
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
