use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};

use crate::model::Reminder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReminderGroup {
    Overdue,
    Today,
    Tomorrow,
    Later,
}

impl ReminderGroup {
    pub const ALL: [Self; 4] = [Self::Overdue, Self::Today, Self::Tomorrow, Self::Later];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Overdue => "Overdue",
            Self::Today => "Today",
            Self::Tomorrow => "Tomorrow",
            Self::Later => "Later",
        }
    }
}

pub fn group_active_reminders<Tz>(
    reminders: Vec<Reminder>,
    now: DateTime<Utc>,
    timezone: &Tz,
) -> BTreeMap<ReminderGroup, Vec<Reminder>>
where
    Tz: TimeZone,
{
    let mut groups = ReminderGroup::ALL
        .into_iter()
        .map(|group| (group, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let today = now.with_timezone(timezone).date_naive();
    let tomorrow = today
        .succ_opt()
        .expect("the next local day is representable");

    for reminder in reminders.into_iter().filter(Reminder::is_active) {
        let group = if reminder.due_at <= now {
            ReminderGroup::Overdue
        } else {
            match reminder.due_at.with_timezone(timezone).date_naive() {
                date if date == today => ReminderGroup::Today,
                date if date == tomorrow => ReminderGroup::Tomorrow,
                _ => ReminderGroup::Later,
            }
        };
        groups.entry(group).or_default().push(reminder);
    }

    groups
}
