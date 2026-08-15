use chrono::{DateTime, Duration, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::schedule::ScheduleExpression;

pub const MAX_MESSAGE_CHARS: usize = 280;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReminder {
    pub message: String,
    pub due_at: DateTime<Utc>,
}

impl NewReminder {
    pub fn new(message: impl Into<String>, due_at: DateTime<Utc>) -> Self {
        Self {
            message: message.into(),
            due_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: Uuid,
    pub message: String,
    pub due_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub notified_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasEntry {
    pub id: Uuid,
    pub position: i64,
    pub message: String,
    pub reminder_id: Option<Uuid>,
    pub working_text: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CanvasEntry {
    pub fn create(
        message: impl Into<String>,
        reminder_id: Option<Uuid>,
        position: i64,
        now: DateTime<Utc>,
    ) -> Result<Self, ReminderError> {
        Ok(Self {
            id: Uuid::new_v4(),
            position,
            message: validate_message(&message.into())?,
            reminder_id,
            working_text: None,
            created_at: now,
            updated_at: now,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasItem {
    pub entry: CanvasEntry,
    pub reminder: Option<Reminder>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasSchedule {
    None,
    KeepExisting,
    Replace(ScheduleExpression),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedCanvasItem {
    pub entry: CanvasEntry,
    pub reminder: Option<Reminder>,
}

impl Reminder {
    pub fn create(input: NewReminder, now: DateTime<Utc>) -> Result<Self, ReminderError> {
        let message = validate_message(&input.message)?;
        if input.due_at <= now {
            return Err(ReminderError::DueTimeNotFuture);
        }

        Ok(Self {
            id: Uuid::new_v4(),
            message,
            due_at: input.due_at,
            created_at: now,
            updated_at: now,
            notified_at: None,
            completed_at: None,
        })
    }

    pub fn snooze(&mut self, now: DateTime<Utc>) {
        self.due_at = now + Duration::minutes(10);
        self.updated_at = now;
        self.notified_at = None;
        self.completed_at = None;
    }

    pub fn edit(
        &mut self,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), ReminderError> {
        let message = validate_message(message)?;
        if due_at <= now {
            return Err(ReminderError::DueTimeNotFuture);
        }

        self.message = message;
        self.due_at = due_at;
        self.updated_at = now;
        self.notified_at = None;
        self.completed_at = None;
        Ok(())
    }

    pub fn rename(&mut self, message: &str, now: DateTime<Utc>) -> Result<(), ReminderError> {
        self.message = validate_message(message)?;
        self.updated_at = now;
        Ok(())
    }

    pub fn mark_notified(&mut self, now: DateTime<Utc>) {
        self.notified_at = Some(now);
        self.updated_at = now;
    }

    pub fn complete(&mut self, now: DateTime<Utc>) {
        self.completed_at = Some(now);
        self.updated_at = now;
    }

    pub fn is_active(&self) -> bool {
        self.completed_at.is_none()
    }

    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.is_active() && self.due_at <= now
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReminderError {
    #[error("Enter a reminder message")]
    EmptyMessage,
    #[error("Reminder messages can contain at most 280 characters")]
    MessageTooLong,
    #[error("Choose a time in the future")]
    DueTimeNotFuture,
}

pub fn validate_message(message: &str) -> Result<String, ReminderError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(ReminderError::EmptyMessage);
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(ReminderError::MessageTooLong);
    }
    Ok(message.to_owned())
}
