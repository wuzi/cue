use std::rc::Rc;

use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    canvas::normalize_working_after_due_change,
    model::{CanvasItem, CanvasSchedule, DeletedCanvasItem, NewReminder, Reminder, ReminderError},
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

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
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

    pub fn list_canvas(&self) -> Result<Vec<CanvasItem>, ServiceError> {
        self.repository
            .list_canvas_entries()?
            .into_iter()
            .map(|entry| {
                let reminder = entry
                    .reminder_id
                    .map(|id| self.repository.get(id))
                    .transpose()?;
                Ok(CanvasItem { entry, reminder })
            })
            .collect()
    }

    pub fn load_canvas_draft(&self) -> Result<String, ServiceError> {
        Ok(self.repository.load_canvas_draft()?)
    }

    pub fn save_canvas_draft(&self, text: &str) -> Result<(), ServiceError> {
        Ok(self.repository.save_canvas_draft(text)?)
    }

    pub fn save_canvas_working_text(
        &self,
        id: Uuid,
        text: Option<&str>,
    ) -> Result<(), ServiceError> {
        Ok(self
            .repository
            .save_canvas_working_text(id, text, self.clock.now())?)
    }

    pub fn discard_canvas_working_text(&self, id: Uuid) -> Result<(), ServiceError> {
        self.save_canvas_working_text(id, None)
    }

    pub fn commit_canvas_draft<Tz: TimeZone>(
        &self,
        message: impl Into<String>,
        schedule: CanvasSchedule,
        timezone: &Tz,
    ) -> Result<CanvasItem, ServiceError> {
        let now = self.clock.now();
        let message = message.into();
        match schedule {
            CanvasSchedule::None => {
                let entry = self.repository.append_canvas_entry(&message, None, now)?;
                Ok(CanvasItem {
                    entry,
                    reminder: None,
                })
            }
            CanvasSchedule::KeepExisting => Err(CanvasError::KeepWithoutReminder.into()),
            CanvasSchedule::Replace(expression) => {
                let due_at = resolve_schedule(&expression, now, timezone)?;
                let reminder = Reminder::create(NewReminder::new(message, due_at), now)?;
                let entry =
                    self.repository
                        .append_canvas_entry(&reminder.message, Some(&reminder), now)?;
                self.scheduler.refresh()?;
                Ok(CanvasItem {
                    entry,
                    reminder: Some(reminder),
                })
            }
        }
    }

    pub fn commit_canvas_edit<Tz: TimeZone>(
        &self,
        entry_id: Uuid,
        message: impl Into<String>,
        schedule: CanvasSchedule,
        timezone: &Tz,
    ) -> Result<CanvasItem, ServiceError> {
        let now = self.clock.now();
        let message = message.into();
        let existing = self.repository.get_canvas_entry(entry_id)?;
        match schedule {
            CanvasSchedule::None => {
                if let Some(reminder_id) = existing.reminder_id {
                    let (entry, _) = self
                        .repository
                        .detach_canvas_reminder(entry_id, &message, now)?;
                    self.notifier.withdraw(&stable_notification_id(reminder_id));
                    self.scheduler.refresh()?;
                    Ok(CanvasItem {
                        entry,
                        reminder: None,
                    })
                } else {
                    let entry = self
                        .repository
                        .update_canvas_note(entry_id, &message, now)?;
                    Ok(CanvasItem {
                        entry,
                        reminder: None,
                    })
                }
            }
            CanvasSchedule::KeepExisting => {
                if existing.reminder_id.is_none() {
                    return Err(CanvasError::KeepWithoutReminder.into());
                }
                let (entry, reminder) = self
                    .repository
                    .rename_canvas_reminder(entry_id, &message, now)?;
                Ok(CanvasItem {
                    entry,
                    reminder: Some(reminder),
                })
            }
            CanvasSchedule::Replace(expression) => {
                let due_at = resolve_schedule(&expression, now, timezone)?;
                if let Some(reminder_id) = existing.reminder_id {
                    let (entry, reminder) = self
                        .repository
                        .reschedule_canvas_reminder(entry_id, &message, due_at, now)?;
                    self.notifier.withdraw(&stable_notification_id(reminder_id));
                    self.scheduler.refresh()?;
                    Ok(CanvasItem {
                        entry,
                        reminder: Some(reminder),
                    })
                } else {
                    let reminder = Reminder::create(NewReminder::new(message, due_at), now)?;
                    let entry = self
                        .repository
                        .attach_canvas_reminder(entry_id, &reminder, now)?;
                    self.scheduler.refresh()?;
                    Ok(CanvasItem {
                        entry,
                        reminder: Some(reminder),
                    })
                }
            }
        }
    }

    pub fn delete_canvas_entry(&self, id: Uuid) -> Result<DeletedCanvasItem, ServiceError> {
        let deleted = self.repository.delete_canvas_entry(id)?;
        if let Some(reminder) = &deleted.reminder {
            self.notifier.withdraw(&stable_notification_id(reminder.id));
            self.scheduler.refresh()?;
        }
        Ok(deleted)
    }

    pub fn restore_canvas_item(&self, item: &DeletedCanvasItem) -> Result<(), ServiceError> {
        let mut restored = item.clone();
        if let Some(reminder) = restored.reminder.as_mut()
            && reminder.is_active()
        {
            reminder.notified_at = None;
        }
        self.repository.restore_canvas_item(&restored)?;
        self.scheduler.refresh()?;
        Ok(())
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
        let reminder = self
            .repository
            .complete_canvas_reminder(id, self.clock.now())?;
        self.notifier.withdraw(&stable_notification_id(id));
        self.scheduler.refresh()?;
        Ok(reminder)
    }

    pub fn snooze(&self, id: Uuid) -> Result<Reminder, ServiceError> {
        let now = self.clock.now();
        let previous = self.repository.get(id)?;
        let linked = self
            .repository
            .list_canvas_entries()?
            .into_iter()
            .find(|entry| entry.reminder_id == Some(id));
        let mut projected = previous.clone();
        projected.snooze(now);
        let normalized = linked.as_ref().and_then(|entry| {
            let working = entry.working_text.as_deref()?;
            let normalized = normalize_working_after_due_change(
                working,
                previous.due_at,
                projected.due_at,
                entry.updated_at,
                now,
                &Local,
            );
            (normalized != working).then_some((entry.id, normalized))
        });
        let working = normalized
            .as_ref()
            .map(|(entry_id, text)| (*entry_id, text.as_str()));
        let reminder = self.repository.snooze_canvas_reminder(id, now, working)?;
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
    InvalidCanvas(#[from] CanvasError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CanvasError {
    #[error("A saved reminder is required to keep its existing schedule")]
    KeepWithoutReminder,
}
