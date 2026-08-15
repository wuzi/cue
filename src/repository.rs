use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, types::Type};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{Reminder, ReminderError};

const SCHEMA_VERSION: i64 = 1;

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
    fn complete(&self, id: Uuid, now: DateTime<Utc>) -> Result<Reminder, RepositoryError>;
    fn delete(&self, id: Uuid) -> Result<Reminder, RepositoryError>;
    fn clear_history(&self) -> Result<usize, RepositoryError>;
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
                PRAGMA user_version = 1;",
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
}

impl ReminderRepository for SqliteReminderRepository {
    fn insert(&self, reminder: &Reminder) -> Result<(), RepositoryError> {
        self.connection.execute(
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
    #[error("database schema version {0} is not supported")]
    UnsupportedSchema(i64),
    #[error(transparent)]
    InvalidReminder(#[from] ReminderError),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}
