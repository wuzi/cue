use std::rc::Rc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    model::{NewReminder, Reminder, ReminderError},
    repository::{ReminderRepository, RepositoryError},
    schedule::{ScheduleError, ScheduleExpression, resolve_schedule},
    scheduler::{
        Clock, RefreshResult, ReminderNotifier, Scheduler, SchedulerError, stable_notification_id,
    },
};

pub struct ReminderService {
    repository: Rc<dyn ReminderRepository>,
    clock: Rc<dyn Clock>,
    notifier: Rc<dyn ReminderNotifier>,
    scheduler: Scheduler,
}

impl ReminderService {
    pub fn new<R, C, N>(repository: Rc<R>, clock: Rc<C>, notifier: Rc<N>) -> Self
    where
        R: ReminderRepository + 'static,
        C: Clock + 'static,
        N: ReminderNotifier + 'static,
    {
        let scheduler = Scheduler::new(repository.clone(), clock.clone(), notifier.clone());
        Self {
            repository,
            clock,
            notifier,
            scheduler,
        }
    }

    pub fn create(&self, input: NewReminder) -> Result<Reminder, ServiceError> {
        let reminder = Reminder::create(input, self.clock.now())?;
        self.persist_created(reminder)
    }

    pub fn create_relative(
        &self,
        message: impl Into<String>,
        delay: Duration,
    ) -> Result<Reminder, ServiceError> {
        let now = self.clock.now();
        let reminder = Reminder::create(NewReminder::new(message, now + delay), now)?;
        self.persist_created(reminder)
    }

    pub fn preview_schedule<Tz: TimeZone>(
        &self,
        schedule: &ScheduleExpression,
        timezone: &Tz,
    ) -> Result<DateTime<Utc>, ServiceError> {
        let now = self.clock.now();
        Ok(resolve_schedule(schedule, now, timezone)?)
    }

    pub fn create_scheduled<Tz: TimeZone>(
        &self,
        message: impl Into<String>,
        schedule: &ScheduleExpression,
        timezone: &Tz,
    ) -> Result<Reminder, ServiceError> {
        let now = self.clock.now();
        let due_at = resolve_schedule(schedule, now, timezone)?;
        let reminder = Reminder::create(NewReminder::new(message, due_at), now)?;
        self.persist_created(reminder)
    }

    fn persist_created(&self, reminder: Reminder) -> Result<Reminder, ServiceError> {
        self.repository.insert(&reminder)?;
        self.scheduler.refresh()?;
        Ok(reminder)
    }

    pub fn edit(
        &self,
        id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
    ) -> Result<Reminder, ServiceError> {
        let reminder = self
            .repository
            .edit(id, message, due_at, self.clock.now())?;
        self.notifier.withdraw(&stable_notification_id(id));
        self.scheduler.refresh()?;
        Ok(reminder)
    }

    pub fn complete(&self, id: Uuid) -> Result<Reminder, ServiceError> {
        let reminder = self.repository.complete(id, self.clock.now())?;
        self.notifier.withdraw(&stable_notification_id(id));
        self.scheduler.refresh()?;
        Ok(reminder)
    }

    pub fn snooze(&self, id: Uuid) -> Result<Reminder, ServiceError> {
        let reminder = self.repository.snooze(id, self.clock.now())?;
        self.notifier.withdraw(&stable_notification_id(id));
        self.scheduler.refresh()?;
        Ok(reminder)
    }

    pub fn delete(&self, id: Uuid) -> Result<Reminder, ServiceError> {
        let reminder = self.repository.delete(id)?;
        self.notifier.withdraw(&stable_notification_id(id));
        self.scheduler.refresh()?;
        Ok(reminder)
    }

    pub fn restore(&self, reminder: &Reminder) -> Result<(), ServiceError> {
        let mut restored = reminder.clone();
        if restored.is_active() {
            restored.notified_at = None;
        }
        self.repository.restore(&restored)?;
        self.scheduler.refresh()?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<usize, ServiceError> {
        let removed = self.repository.clear_history()?;
        self.scheduler.refresh()?;
        Ok(removed)
    }

    pub fn get(&self, id: Uuid) -> Result<Reminder, ServiceError> {
        Ok(self.repository.get(id)?)
    }

    pub fn list_active(&self) -> Result<Vec<Reminder>, ServiceError> {
        Ok(self.repository.list_active()?)
    }

    pub fn list_history(&self) -> Result<Vec<Reminder>, ServiceError> {
        Ok(self.repository.list_history()?)
    }

    pub fn refresh(&self) -> Result<RefreshResult, ServiceError> {
        Ok(self.scheduler.refresh()?)
    }

    pub fn next_due(&self) -> Result<Option<DateTime<Utc>>, ServiceError> {
        Ok(self.scheduler.next_due()?)
    }

    pub fn should_hold_background(&self) -> Result<bool, ServiceError> {
        Ok(!self.repository.list_active()?.is_empty())
    }

    pub fn complete_target(&self, target: &str) -> Result<ActionOutcome, ServiceError> {
        self.run_target_action(target, |service, id| service.complete(id))
    }

    pub fn snooze_target(&self, target: &str) -> Result<ActionOutcome, ServiceError> {
        self.run_target_action(target, |service, id| service.snooze(id))
    }

    pub fn resolve_active_target(&self, target: &str) -> Result<Option<Uuid>, ServiceError> {
        let Ok(id) = Uuid::parse_str(target) else {
            return Ok(None);
        };
        match self.repository.get(id) {
            Ok(reminder) if notification_action_is_current(&reminder) => Ok(Some(id)),
            Ok(_) | Err(RepositoryError::NotFound(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn run_target_action(
        &self,
        target: &str,
        action: impl FnOnce(&Self, Uuid) -> Result<Reminder, ServiceError>,
    ) -> Result<ActionOutcome, ServiceError> {
        let Ok(id) = Uuid::parse_str(target) else {
            return Ok(ActionOutcome::Ignored);
        };
        match self.repository.get(id) {
            Ok(reminder) if notification_action_is_current(&reminder) => {}
            Ok(_) | Err(RepositoryError::NotFound(_)) => return Ok(ActionOutcome::Ignored),
            Err(error) => return Err(error.into()),
        }
        match action(self, id) {
            Ok(_) => Ok(ActionOutcome::Applied),
            Err(ServiceError::Repository(RepositoryError::NotFound(_))) => {
                Ok(ActionOutcome::Ignored)
            }
            Err(error) => Err(error),
        }
    }
}

fn notification_action_is_current(reminder: &Reminder) -> bool {
    reminder.is_active() && reminder.notified_at.is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionOutcome {
    Applied,
    Ignored,
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    InvalidReminder(#[from] ReminderError),
    #[error(transparent)]
    InvalidSchedule(#[from] ScheduleError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
}
