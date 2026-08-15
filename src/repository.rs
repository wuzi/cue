use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{CanvasEntry, DeletedCanvasItem, Reminder, ReminderError};

const SCHEMA_VERSION: i64 = 2;

pub trait ReminderRepository {
    fn insert(&self, reminder: &Reminder) -> Result<(), RepositoryError>;
    fn restore(&self, reminder: &Reminder) -> Result<(), RepositoryError>;
    fn get(&self, id: Uuid) -> Result<Reminder, RepositoryError>;
    fn list_active(&self) -> Result<Vec<Reminder>, RepositoryError>;
    fn list_history(&self) -> Result<Vec<Reminder>, RepositoryError>;
    fn edit(
        &self,
        id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Reminder, RepositoryError>;
    fn mark_notified(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError>;
    fn snooze(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError>;
    fn snooze_canvas_reminder(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
        working_text: Option<(Uuid, &str)>,
    ) -> Result<Reminder, RepositoryError>;
    fn complete(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError>;
    fn delete(&self, id: Uuid) -> Result<Reminder, RepositoryError>;
    fn clear_history(&self) -> Result<usize, RepositoryError>;
    fn list_canvas_entries(&self) -> Result<Vec<CanvasEntry>, RepositoryError>;
    fn append_canvas_entry(
        &self,
        message: &str,
        reminder: Option<&Reminder>,
        now: DateTime<Utc>,
    ) -> Result<CanvasEntry, RepositoryError>;
    fn save_canvas_working_text(
        &self,
        id: Uuid,
        text: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), RepositoryError>;
    fn load_canvas_draft(&self) -> Result<String, RepositoryError>;
    fn save_canvas_draft(&self, text: &str) -> Result<(), RepositoryError>;
    fn get_canvas_entry(&self, id: Uuid) -> Result<CanvasEntry, RepositoryError>;
    fn attach_canvas_reminder(
        &self,
        entry_id: Uuid,
        reminder: &Reminder,
        now: DateTime<Utc>,
    ) -> Result<CanvasEntry, RepositoryError>;
    fn detach_canvas_reminder(
        &self,
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError>;
    fn complete_canvas_reminder(
        &self,
        reminder_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Reminder, RepositoryError>;
    fn delete_canvas_entry(&self, id: Uuid) -> Result<DeletedCanvasItem, RepositoryError>;
    fn restore_canvas_item(&self, item: &DeletedCanvasItem) -> Result<(), RepositoryError>;
    fn update_canvas_note(
        &self,
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> Result<CanvasEntry, RepositoryError>;
    fn rename_canvas_reminder(
        &self,
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError>;
    fn reschedule_canvas_reminder(
        &self,
        entry_id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError>;
}

pub struct SqliteReminderRepository {
    connection: Connection,
}

impl SqliteReminderRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RepositoryError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self, RepositoryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, RepositoryError> {
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
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
            )?;
            transaction.commit()?;
        } else if version == 1 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE canvas_entries (
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
                INSERT INTO canvas_entries (
                    id, position, message, reminder_id, working_text, created_at, updated_at
                )
                SELECT
                    id,
                    ROW_NUMBER() OVER (ORDER BY created_at ASC, id ASC),
                    message,
                    id,
                    NULL,
                    created_at,
                    updated_at
                FROM reminders
                WHERE completed_at IS NULL;
                PRAGMA user_version = 2;",
            )?;
            transaction.commit()?;
        } else if version != SCHEMA_VERSION {
            return Err(RepositoryError::UnsupportedSchema(version));
        }

        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> Result<i64, RepositoryError> {
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    fn save(connection: &Connection, reminder: &Reminder) -> Result<(), RepositoryError> {
        let changed = connection.execute(
            "UPDATE reminders SET
                message = ?2,
                due_at = ?3,
                created_at = ?4,
                updated_at = ?5,
                notified_at = ?6,
                completed_at = ?7
             WHERE id = ?1",
            params![
                reminder.id.to_string(),
                reminder.message,
                reminder.due_at.timestamp(),
                reminder.created_at.timestamp(),
                reminder.updated_at.timestamp(),
                reminder.notified_at.map(|value| value.timestamp()),
                reminder.completed_at.map(|value| value.timestamp()),
            ],
        )?;
        if changed == 0 {
            return Err(RepositoryError::NotFound(reminder.id));
        }
        Ok(())
    }

    fn get_from(connection: &Connection, id: Uuid) -> Result<Reminder, RepositoryError> {
        connection
            .query_row(
                "SELECT id, message, due_at, created_at, updated_at, notified_at, completed_at
                 FROM reminders WHERE id = ?1",
                [id.to_string()],
                reminder_from_row,
            )
            .optional()?
            .ok_or(RepositoryError::NotFound(id))
    }

    fn list(&self, sql: &str) -> Result<Vec<Reminder>, RepositoryError> {
        let mut statement = self.connection.prepare(sql)?;
        let reminders = statement
            .query_map([], reminder_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(reminders)
    }

    fn insert_reminder(
        connection: &Connection,
        reminder: &Reminder,
    ) -> Result<(), RepositoryError> {
        connection.execute(
            "INSERT INTO reminders (
                id, message, due_at, created_at, updated_at, notified_at, completed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                reminder.id.to_string(),
                reminder.message,
                reminder.due_at.timestamp(),
                reminder.created_at.timestamp(),
                reminder.updated_at.timestamp(),
                reminder.notified_at.map(|value| value.timestamp()),
                reminder.completed_at.map(|value| value.timestamp()),
            ],
        )?;
        Ok(())
    }

    fn get_canvas_from(connection: &Connection, id: Uuid) -> Result<CanvasEntry, RepositoryError> {
        connection
            .query_row(
                "SELECT id, position, message, reminder_id, working_text, created_at, updated_at
                 FROM canvas_entries WHERE id = ?1",
                [id.to_string()],
                canvas_entry_from_row,
            )
            .optional()?
            .ok_or(RepositoryError::CanvasEntryNotFound(id))
    }

    fn insert_canvas(connection: &Connection, entry: &CanvasEntry) -> Result<(), RepositoryError> {
        connection.execute(
            "INSERT INTO canvas_entries (
                id, position, message, reminder_id, working_text, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id.to_string(),
                entry.position,
                entry.message,
                entry.reminder_id.map(|value| value.to_string()),
                entry.working_text,
                entry.created_at.timestamp(),
                entry.updated_at.timestamp(),
            ],
        )?;
        Ok(())
    }
}

impl ReminderRepository for SqliteReminderRepository {
    fn insert(&self, reminder: &Reminder) -> Result<(), RepositoryError> {
        Self::insert_reminder(&self.connection, reminder)
    }

    fn restore(&self, reminder: &Reminder) -> Result<(), RepositoryError> {
        self.insert(reminder)
    }

    fn get(&self, id: Uuid) -> Result<Reminder, RepositoryError> {
        Self::get_from(&self.connection, id)
    }

    fn list_active(&self) -> Result<Vec<Reminder>, RepositoryError> {
        self.list(
            "SELECT id, message, due_at, created_at, updated_at, notified_at, completed_at
             FROM reminders WHERE completed_at IS NULL ORDER BY due_at ASC, created_at ASC",
        )
    }

    fn list_history(&self) -> Result<Vec<Reminder>, RepositoryError> {
        self.list(
            "SELECT id, message, due_at, created_at, updated_at, notified_at, completed_at
             FROM reminders WHERE completed_at IS NOT NULL
             ORDER BY completed_at DESC, created_at DESC",
        )
    }

    fn edit(
        &self,
        id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut reminder = Self::get_from(&transaction, id)?;
        reminder.edit(message, due_at, now)?;
        Self::save(&transaction, &reminder)?;
        transaction.execute(
            "UPDATE canvas_entries
             SET message = ?2, working_text = NULL, updated_at = ?3
             WHERE reminder_id = ?1",
            params![id.to_string(), reminder.message, now.timestamp()],
        )?;
        transaction.commit()?;
        Ok(reminder)
    }

    fn mark_notified(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut reminder = Self::get_from(&transaction, id)?;
        reminder.mark_notified(now);
        Self::save(&transaction, &reminder)?;
        transaction.commit()?;
        Ok(reminder)
    }

    fn snooze(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut reminder = Self::get_from(&transaction, id)?;
        reminder.snooze(now);
        Self::save(&transaction, &reminder)?;
        transaction.commit()?;
        Ok(reminder)
    }

    fn snooze_canvas_reminder(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
        working_text: Option<(Uuid, &str)>,
    ) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut reminder = Self::get_from(&transaction, id)?;
        reminder.snooze(now);
        Self::save(&transaction, &reminder)?;
        if let Some((entry_id, text)) = working_text {
            let changed = transaction.execute(
                "UPDATE canvas_entries
                 SET working_text = ?3, updated_at = ?4
                 WHERE id = ?1 AND reminder_id = ?2",
                params![entry_id.to_string(), id.to_string(), text, now.timestamp()],
            )?;
            if changed == 0 {
                return Err(RepositoryError::CanvasEntryNotFound(entry_id));
            }
        }
        transaction.commit()?;
        Ok(reminder)
    }

    fn complete(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut reminder = Self::get_from(&transaction, id)?;
        reminder.complete(now);
        Self::save(&transaction, &reminder)?;
        transaction.commit()?;
        Ok(reminder)
    }

    fn delete(&self, id: Uuid) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let reminder = Self::get_from(&transaction, id)?;
        transaction.execute("DELETE FROM reminders WHERE id = ?1", [id.to_string()])?;
        transaction.commit()?;
        Ok(reminder)
    }

    fn clear_history(&self) -> Result<usize, RepositoryError> {
        Ok(self
            .connection
            .execute("DELETE FROM reminders WHERE completed_at IS NOT NULL", [])?)
    }

    fn list_canvas_entries(&self) -> Result<Vec<CanvasEntry>, RepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, position, message, reminder_id, working_text, created_at, updated_at
             FROM canvas_entries ORDER BY position ASC, created_at ASC, id ASC",
        )?;
        Ok(statement
            .query_map([], canvas_entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn append_canvas_entry(
        &self,
        message: &str,
        reminder: Option<&Reminder>,
        now: DateTime<Utc>,
    ) -> Result<CanvasEntry, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let position = transaction.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM canvas_entries",
            [],
            |row| row.get(0),
        )?;
        let entry = CanvasEntry::create(message, reminder.map(|item| item.id), position, now)?;
        if let Some(reminder) = reminder {
            Self::insert_reminder(&transaction, reminder)?;
        }
        Self::insert_canvas(&transaction, &entry)?;
        transaction.execute("UPDATE canvas_state SET draft_text = '' WHERE id = 1", [])?;
        transaction.commit()?;
        Ok(entry)
    }

    fn save_canvas_working_text(
        &self,
        id: Uuid,
        text: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        let changed = self.connection.execute(
            "UPDATE canvas_entries SET working_text = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), text, now.timestamp()],
        )?;
        if changed == 0 {
            return Err(RepositoryError::CanvasEntryNotFound(id));
        }
        Ok(())
    }

    fn load_canvas_draft(&self) -> Result<String, RepositoryError> {
        Ok(self.connection.query_row(
            "SELECT draft_text FROM canvas_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn save_canvas_draft(&self, text: &str) -> Result<(), RepositoryError> {
        self.connection.execute(
            "UPDATE canvas_state SET draft_text = ?1 WHERE id = 1",
            [text],
        )?;
        Ok(())
    }

    fn get_canvas_entry(&self, id: Uuid) -> Result<CanvasEntry, RepositoryError> {
        Self::get_canvas_from(&self.connection, id)
    }

    fn attach_canvas_reminder(
        &self,
        entry_id: Uuid,
        reminder: &Reminder,
        now: DateTime<Utc>,
    ) -> Result<CanvasEntry, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = Self::get_canvas_from(&transaction, entry_id)?;
        if entry.reminder_id.is_some() {
            return Err(RepositoryError::CanvasEntryHasReminder(entry_id));
        }
        Self::insert_reminder(&transaction, reminder)?;
        entry.message = reminder.message.clone();
        entry.reminder_id = Some(reminder.id);
        entry.working_text = None;
        entry.updated_at = now;
        transaction.execute(
            "UPDATE canvas_entries SET
                message = ?2, reminder_id = ?3, working_text = NULL, updated_at = ?4
             WHERE id = ?1",
            params![
                entry.id.to_string(),
                entry.message,
                reminder.id.to_string(),
                entry.updated_at.timestamp(),
            ],
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    fn detach_canvas_reminder(
        &self,
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = Self::get_canvas_from(&transaction, entry_id)?;
        let reminder_id = entry
            .reminder_id
            .ok_or(RepositoryError::CanvasEntryHasNoReminder(entry_id))?;
        let reminder = Self::get_from(&transaction, reminder_id)?;
        entry.message = crate::model::validate_message(message)?;
        entry.reminder_id = None;
        entry.working_text = None;
        entry.updated_at = now;
        transaction.execute(
            "UPDATE canvas_entries SET
                message = ?2, reminder_id = NULL, working_text = NULL, updated_at = ?3
             WHERE id = ?1",
            params![
                entry.id.to_string(),
                entry.message,
                entry.updated_at.timestamp(),
            ],
        )?;
        transaction.execute(
            "DELETE FROM reminders WHERE id = ?1",
            [reminder_id.to_string()],
        )?;
        transaction.commit()?;
        Ok((entry, reminder))
    }

    fn complete_canvas_reminder(
        &self,
        reminder_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Reminder, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut reminder = Self::get_from(&transaction, reminder_id)?;
        reminder.complete(now);
        Self::save(&transaction, &reminder)?;
        transaction.execute(
            "DELETE FROM canvas_entries WHERE reminder_id = ?1",
            [reminder_id.to_string()],
        )?;
        transaction.commit()?;
        Ok(reminder)
    }

    fn delete_canvas_entry(&self, id: Uuid) -> Result<DeletedCanvasItem, RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let entry = Self::get_canvas_from(&transaction, id)?;
        let reminder = entry
            .reminder_id
            .map(|reminder_id| Self::get_from(&transaction, reminder_id))
            .transpose()?;
        if let Some(reminder) = &reminder {
            transaction.execute(
                "DELETE FROM reminders WHERE id = ?1",
                [reminder.id.to_string()],
            )?;
        } else {
            transaction.execute("DELETE FROM canvas_entries WHERE id = ?1", [id.to_string()])?;
        }
        transaction.commit()?;
        Ok(DeletedCanvasItem { entry, reminder })
    }

    fn restore_canvas_item(&self, item: &DeletedCanvasItem) -> Result<(), RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let position_is_taken: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM canvas_entries WHERE position = ?1)",
            [item.entry.position],
            |row| row.get(0),
        )?;
        if position_is_taken {
            transaction.execute(
                "UPDATE canvas_entries SET position = position + 1 WHERE position >= ?1",
                [item.entry.position],
            )?;
        }
        if let Some(reminder) = &item.reminder {
            Self::insert_reminder(&transaction, reminder)?;
        }
        Self::insert_canvas(&transaction, &item.entry)?;
        transaction.commit()?;
        Ok(())
    }

    fn update_canvas_note(
        &self,
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> Result<CanvasEntry, RepositoryError> {
        let mut entry = Self::get_canvas_from(&self.connection, entry_id)?;
        if entry.reminder_id.is_some() {
            return Err(RepositoryError::CanvasEntryHasReminder(entry_id));
        }
        entry.message = crate::model::validate_message(message)?;
        entry.working_text = None;
        entry.updated_at = now;
        self.connection.execute(
            "UPDATE canvas_entries SET message = ?2, working_text = NULL, updated_at = ?3
             WHERE id = ?1",
            params![
                entry.id.to_string(),
                entry.message,
                entry.updated_at.timestamp(),
            ],
        )?;
        Ok(entry)
    }

    fn rename_canvas_reminder(
        &self,
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError> {
        self.mutate_canvas_reminder(entry_id, |reminder| reminder.rename(message, now))
    }

    fn reschedule_canvas_reminder(
        &self,
        entry_id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError> {
        self.mutate_canvas_reminder(entry_id, |reminder| reminder.edit(message, due_at, now))
    }
}

impl SqliteReminderRepository {
    fn mutate_canvas_reminder(
        &self,
        entry_id: Uuid,
        mutation: impl FnOnce(&mut Reminder) -> Result<(), ReminderError>,
    ) -> Result<(CanvasEntry, Reminder), RepositoryError> {
        let transaction = self.connection.unchecked_transaction()?;
        let mut entry = Self::get_canvas_from(&transaction, entry_id)?;
        let reminder_id = entry
            .reminder_id
            .ok_or(RepositoryError::CanvasEntryHasNoReminder(entry_id))?;
        let mut reminder = Self::get_from(&transaction, reminder_id)?;
        mutation(&mut reminder)?;
        Self::save(&transaction, &reminder)?;
        entry.message = reminder.message.clone();
        entry.working_text = None;
        entry.updated_at = reminder.updated_at;
        transaction.execute(
            "UPDATE canvas_entries SET message = ?2, working_text = NULL, updated_at = ?3
             WHERE id = ?1",
            params![
                entry.id.to_string(),
                entry.message,
                entry.updated_at.timestamp(),
            ],
        )?;
        transaction.commit()?;
        Ok((entry, reminder))
    }
}

fn canvas_entry_from_row(row: &Row<'_>) -> rusqlite::Result<CanvasEntry> {
    let id_text: String = row.get(0)?;
    let id = Uuid::parse_str(&id_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
    })?;
    let reminder_id = row
        .get::<_, Option<String>>(3)?
        .map(|value| {
            Uuid::parse_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
            })
        })
        .transpose()?;
    Ok(CanvasEntry {
        id,
        position: row.get(1)?,
        message: row.get(2)?,
        reminder_id,
        working_text: row.get(4)?,
        created_at: timestamp_from_row(row, 5)?,
        updated_at: timestamp_from_row(row, 6)?,
    })
}

fn reminder_from_row(row: &Row<'_>) -> rusqlite::Result<Reminder> {
    let id_text: String = row.get(0)?;
    let id = Uuid::parse_str(&id_text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
    })?;

    Ok(Reminder {
        id,
        message: row.get(1)?,
        due_at: timestamp_from_row(row, 2)?,
        created_at: timestamp_from_row(row, 3)?,
        updated_at: timestamp_from_row(row, 4)?,
        notified_at: optional_timestamp_from_row(row, 5)?,
        completed_at: optional_timestamp_from_row(row, 6)?,
    })
}

fn timestamp_from_row(row: &Row<'_>, index: usize) -> rusqlite::Result<DateTime<Utc>> {
    let timestamp: i64 = row.get(index)?;
    DateTime::from_timestamp(timestamp, 0).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Integer,
            format!("invalid UTC timestamp {timestamp}").into(),
        )
    })
}

fn optional_timestamp_from_row(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<DateTime<Utc>>> {
    row.get::<_, Option<i64>>(index)?
        .map(|timestamp| {
            DateTime::from_timestamp(timestamp, 0).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    Type::Integer,
                    format!("invalid UTC timestamp {timestamp}").into(),
                )
            })
        })
        .transpose()
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("reminder {0} was not found")]
    NotFound(Uuid),
    #[error("canvas entry {0} was not found")]
    CanvasEntryNotFound(Uuid),
    #[error("canvas entry {0} is not linked to a reminder")]
    CanvasEntryHasNoReminder(Uuid),
    #[error("canvas entry {0} is already linked to a reminder")]
    CanvasEntryHasReminder(Uuid),
    #[error("database schema version {0} is not supported")]
    UnsupportedSchema(i64),
    #[error(transparent)]
    InvalidReminder(#[from] ReminderError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}
