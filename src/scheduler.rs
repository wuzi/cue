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
    fn send(&self, id: &str, reminder: &Reminder) -> Result<(), NotificationError>;
    fn withdraw(&self, id: &str);
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

        Ok(RefreshResult {
            delivered,
            next_due: self.next_due()?,
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
    let safety_check = std::time::Duration::from_secs(30);
    let Some(next_due) = next_due else {
        return safety_check;
    };
    if next_due <= now {
        return std::time::Duration::ZERO;
    }
    (next_due - now)
        .to_std()
        .unwrap_or(std::time::Duration::ZERO)
        .min(safety_check)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshResult {
    pub delivered: Vec<Uuid>,
    pub next_due: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NotificationError {
    #[error("notification service unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Notification(#[from] NotificationError),
}
