use chrono::{DateTime, Duration, TimeZone, Utc};
use cue::{
    model::{DeletedCanvasItem, NewReminder, Reminder},
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
fn new_database_uses_schema_two_and_persists_canvas_draft() {
    let repository = SqliteReminderRepository::in_memory().unwrap();

    assert_eq!(repository.schema_version().unwrap(), 2);
    assert_eq!(repository.load_canvas_draft().unwrap(), "");

    repository.save_canvas_draft("unfinished @tom").unwrap();
    assert_eq!(repository.load_canvas_draft().unwrap(), "unfinished @tom");
}

#[test]
fn schema_one_migration_backfills_only_active_reminders_in_writing_order() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("version-one.db");
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
            PRAGMA user_version = 1;",
        )
        .unwrap();

    let now = at(1_800_000_000);
    let first = reminder("Written first", now + Duration::hours(3), now);
    let second = reminder(
        "Written second",
        now + Duration::hours(1),
        now + Duration::seconds(1),
    );
    let mut completed = reminder(
        "Already done",
        now + Duration::hours(2),
        now + Duration::seconds(2),
    );
    completed.complete(now + Duration::hours(4));
    for item in [&second, &completed, &first] {
        connection
            .execute(
                "INSERT INTO reminders (
                    id, message, due_at, created_at, updated_at, notified_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    item.id.to_string(),
                    item.message,
                    item.due_at.timestamp(),
                    item.created_at.timestamp(),
                    item.updated_at.timestamp(),
                    item.notified_at.map(|value| value.timestamp()),
                    item.completed_at.map(|value| value.timestamp()),
                ],
            )
            .unwrap();
    }
    drop(connection);

    let repository = SqliteReminderRepository::open(path).unwrap();
    let entries = repository.list_canvas_entries().unwrap();

    assert_eq!(repository.schema_version().unwrap(), 2);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].message, "Written first");
    assert_eq!(entries[0].reminder_id, Some(first.id));
    assert_eq!(entries[0].position, 1);
    assert_eq!(entries[1].message, "Written second");
    assert_eq!(entries[1].reminder_id, Some(second.id));
    assert_eq!(entries[1].position, 2);
    assert_eq!(repository.list_history().unwrap(), vec![completed]);
}

#[test]
fn canvas_entries_keep_writing_order_and_restore_working_text() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let reminder = reminder("Call Ada", now + Duration::hours(1), now);

    let note = repository
        .append_canvas_entry("First note", None, now)
        .unwrap();
    let scheduled = repository
        .append_canvas_entry("Call Ada", Some(&reminder), now + Duration::seconds(1))
        .unwrap();
    repository
        .save_canvas_working_text(
            scheduled.id,
            Some("Call Ada @tom"),
            now + Duration::seconds(2),
        )
        .unwrap();

    let entries = repository.list_canvas_entries().unwrap();
    assert_eq!(entries[0], note);
    assert_eq!(entries[1].id, scheduled.id);
    assert_eq!(entries[1].position, 2);
    assert_eq!(entries[1].reminder_id, Some(reminder.id));
    assert_eq!(entries[1].working_text.as_deref(), Some("Call Ada @tom"));
    assert_eq!(repository.get(reminder.id).unwrap(), reminder);
}

#[test]
fn note_and_reminder_conversion_is_atomic_and_preserves_position() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let note = repository
        .append_canvas_entry("Call Ada", None, now)
        .unwrap();
    let scheduled = reminder("Call Ada", now + Duration::hours(1), now);

    let upgraded = repository
        .attach_canvas_reminder(note.id, &scheduled, now + Duration::seconds(1))
        .unwrap();
    assert_eq!(upgraded.position, note.position);
    assert_eq!(upgraded.reminder_id, Some(scheduled.id));
    assert_eq!(repository.get(scheduled.id).unwrap(), scheduled);

    let (downgraded, removed) = repository
        .detach_canvas_reminder(note.id, "Call Ada later", now + Duration::seconds(2))
        .unwrap();
    assert_eq!(removed, scheduled);
    assert_eq!(downgraded.position, note.position);
    assert_eq!(downgraded.message, "Call Ada later");
    assert_eq!(downgraded.reminder_id, None);
    assert!(matches!(
        repository.get(scheduled.id),
        Err(RepositoryError::NotFound(id)) if id == scheduled.id
    ));
}

#[test]
fn completing_canvas_reminder_removes_entry_but_retains_history() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let scheduled = reminder("Call Ada", now + Duration::hours(1), now);
    let entry = repository
        .append_canvas_entry("Call Ada", Some(&scheduled), now)
        .unwrap();

    let completed = repository
        .complete_canvas_reminder(scheduled.id, now + Duration::minutes(2))
        .unwrap();

    assert!(completed.completed_at.is_some());
    assert!(matches!(
        repository.get_canvas_entry(entry.id),
        Err(RepositoryError::CanvasEntryNotFound(id)) if id == entry.id
    ));
    assert_eq!(repository.list_history().unwrap(), vec![completed]);
}

#[test]
fn deleting_and_restoring_canvas_item_restores_note_reminder_and_position() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let note = repository
        .append_canvas_entry("Keep this", None, now)
        .unwrap();
    let scheduled = reminder("Call Ada", now + Duration::hours(1), now);
    let reminder_entry = repository
        .append_canvas_entry("Call Ada", Some(&scheduled), now)
        .unwrap();

    let deleted_note = repository.delete_canvas_entry(note.id).unwrap();
    let deleted_reminder = repository.delete_canvas_entry(reminder_entry.id).unwrap();
    assert_eq!(
        deleted_note,
        DeletedCanvasItem {
            entry: note.clone(),
            reminder: None,
        }
    );
    assert_eq!(deleted_reminder.reminder, Some(scheduled.clone()));
    assert!(repository.list_canvas_entries().unwrap().is_empty());

    repository.restore_canvas_item(&deleted_reminder).unwrap();
    repository.restore_canvas_item(&deleted_note).unwrap();
    let restored = repository.list_canvas_entries().unwrap();
    assert_eq!(restored, vec![note, reminder_entry]);
    assert_eq!(repository.get(scheduled.id).unwrap(), scheduled);
}

#[test]
fn failed_upgrade_rolls_back_inserted_reminder_and_keeps_note() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let note = repository
        .append_canvas_entry("Call Ada", None, now)
        .unwrap();
    let scheduled = reminder("Call Ada", now + Duration::hours(1), now);

    let missing_entry = uuid::Uuid::new_v4();
    assert!(matches!(
        repository.attach_canvas_reminder(missing_entry, &scheduled, now),
        Err(RepositoryError::CanvasEntryNotFound(id)) if id == missing_entry
    ));
    assert!(matches!(
        repository.get(scheduled.id),
        Err(RepositoryError::NotFound(id)) if id == scheduled.id
    ));
    assert_eq!(repository.get_canvas_entry(note.id).unwrap(), note);
}

#[test]
fn attaching_to_an_already_scheduled_entry_preserves_the_original_reminder() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let original = reminder("Original", now + Duration::hours(1), now);
    let entry = repository
        .append_canvas_entry("Original", Some(&original), now)
        .unwrap();
    let replacement = reminder("Replacement", now + Duration::hours(2), now);

    assert!(matches!(
        repository.attach_canvas_reminder(entry.id, &replacement, now),
        Err(RepositoryError::CanvasEntryHasReminder(id)) if id == entry.id
    ));
    assert_eq!(repository.get(original.id).unwrap(), original);
    assert!(matches!(
        repository.get(replacement.id),
        Err(RepositoryError::NotFound(id)) if id == replacement.id
    ));
}

#[test]
fn failed_snooze_working_text_update_rolls_back_the_reminder() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let scheduled = reminder("Call Ada", now + Duration::hours(1), now);
    repository.insert(&scheduled).unwrap();
    let unrelated = repository
        .append_canvas_entry("Unrelated note", None, now)
        .unwrap();

    assert!(matches!(
        repository.snooze_canvas_reminder(
            scheduled.id,
            now + Duration::minutes(2),
            Some((unrelated.id, "changed")),
        ),
        Err(RepositoryError::CanvasEntryNotFound(id)) if id == unrelated.id
    ));
    assert_eq!(repository.get(scheduled.id).unwrap(), scheduled);
    assert_eq!(
        repository
            .get_canvas_entry(unrelated.id)
            .unwrap()
            .working_text,
        None
    );
}

#[test]
fn exact_reminder_edit_updates_the_linked_canvas_entry() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    let scheduled = reminder("Call Ada", now + Duration::hours(1), now);
    let entry = repository
        .append_canvas_entry("Call Ada", Some(&scheduled), now)
        .unwrap();
    repository
        .save_canvas_working_text(entry.id, Some("unfinished"), now)
        .unwrap();

    repository
        .edit(
            scheduled.id,
            "Call Ada later",
            now + Duration::hours(3),
            now + Duration::minutes(1),
        )
        .unwrap();

    let updated = repository.get_canvas_entry(entry.id).unwrap();
    assert_eq!(updated.message, "Call Ada later");
    assert_eq!(updated.working_text, None);
}

#[test]
fn restoring_after_new_writes_reclaims_the_original_canvas_position() {
    let now = at(1_800_000_000);
    let repository = SqliteReminderRepository::in_memory().unwrap();
    repository.append_canvas_entry("First", None, now).unwrap();
    let second = repository.append_canvas_entry("Second", None, now).unwrap();
    let deleted = repository.delete_canvas_entry(second.id).unwrap();
    repository
        .append_canvas_entry("New third", None, now)
        .unwrap();

    repository.restore_canvas_item(&deleted).unwrap();

    let entries = repository.list_canvas_entries().unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.position, entry.message.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "First"), (2, "Second"), (3, "New third")]
    );
}
