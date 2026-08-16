use std::rc::Rc;

use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    model::Reminder,
    repository::{ReminderRepository, RepositoryError},
};

pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub trait ReminderNotifier {
    fn availability(&self) -> Result<(), NotificationError> {
        Ok(())
    }

    fn send(&self, id: &str, reminder: &Reminder) -> Result<(), NotificationError>;
    fn withdraw(&self, id: &str);

    fn play_delivery_sound(&self) {}
}

pub struct Scheduler {
    repository: Rc<dyn ReminderRepository>,
    clock: Rc<dyn Clock>,
    notifier: Rc<dyn ReminderNotifier>,
}

impl Scheduler {
    pub fn new<R, C, N>(repository: Rc<R>, clock: Rc<C>, notifier: Rc<N>) -> Self
    where
        R: ReminderRepository + 'static,
        C: Clock + 'static,
        N: ReminderNotifier + 'static,
    {
        Self {
            repository,
            clock,
            notifier,
        }
    }

    pub fn refresh(&self) -> Result<RefreshResult, SchedulerError> {
        let now = self.clock.now();
        let due = self
            .repository
            .list_active()?
            .into_iter()
            .filter(|reminder| reminder.notified_at.is_none() && reminder.is_due(now))
            .collect::<Vec<_>>();

        let mut delivered = Vec::with_capacity(due.len());
        for reminder in due {
            let notification_id = stable_notification_id(reminder.id);
            self.notifier.send(&notification_id, &reminder)?;
            self.repository.mark_notified(reminder.id, now)?;
            delivered.push(reminder.id);
        }
        let next_due = self.next_due()?;

        if !delivered.is_empty() {
            self.notifier.play_delivery_sound();
        }

        Ok(RefreshResult {
            delivered,
            next_due,
        })
    }

    pub fn next_due(&self) -> Result<Option<DateTime<Utc>>, RepositoryError> {
        Ok(self
            .repository
            .list_active()?
            .into_iter()
            .filter(|reminder| reminder.notified_at.is_none())
            .map(|reminder| reminder.due_at)
            .min())
    }
}

pub fn stable_notification_id(id: Uuid) -> String {
    format!("reminder-{id}")
}

pub fn wakeup_delay(now: DateTime<Utc>, next_due: Option<DateTime<Utc>>) -> std::time::Duration {
    let Some(next_due) = next_due else {
        return SAFETY_CHECK_INTERVAL;
    };
    if next_due <= now {
        return std::time::Duration::ZERO;
    }
    (next_due - now)
        .to_std()
        .unwrap_or(std::time::Duration::ZERO)
        .min(SAFETY_CHECK_INTERVAL)
}

pub const SAFETY_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

pub fn refresh_wakeup_delay(
    refresh_succeeded: bool,
    now: DateTime<Utc>,
    next_due: Option<DateTime<Utc>>,
) -> std::time::Duration {
    if refresh_succeeded {
        wakeup_delay(now, next_due)
    } else {
        SAFETY_CHECK_INTERVAL
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshResult {
    pub delivered: Vec<Uuid>,
    pub next_due: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotificationError {
    #[error("desktop entry is not installed: {desktop_id}")]
    MissingDesktopEntry { desktop_id: String },
    #[error("notification service unavailable: {0}")]
    Unavailable(String),
}

impl NotificationError {
    pub fn missing_desktop_entry() -> Self {
        Self::MissingDesktopEntry {
            desktop_id: crate::notifications::CUE_DESKTOP_ID.to_owned(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Notification(#[from] NotificationError),
}
