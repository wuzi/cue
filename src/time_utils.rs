use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, TimeZone, Utc};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockFormat {
    TwelveHour,
    TwentyFourHour,
}

pub fn format_clock_time(hour: u32, minute: u32, format: ClockFormat) -> String {
    match format {
        ClockFormat::TwentyFourHour => format!("{hour:02}:{minute:02}"),
        ClockFormat::TwelveHour => {
            let period = if hour < 12 {
                gettextrs::gettext("AM")
            } else {
                gettextrs::gettext("PM")
            };
            let display_hour = match hour % 12 {
                0 => 12,
                value => value,
            };
            format!("{display_hour}:{minute:02} {period}")
        }
    }
}

pub fn default_due_time(now: DateTime<Utc>) -> DateTime<Utc> {
    let candidate = now + Duration::hours(1);
    let seconds = candidate.timestamp();
    let interval = Duration::minutes(5).num_seconds();
    let rounded = if seconds.rem_euclid(interval) == 0 && candidate.timestamp_subsec_nanos() == 0 {
        seconds
    } else {
        seconds.div_euclid(interval) * interval + interval
    };
    DateTime::from_timestamp(rounded, 0).expect("rounded timestamps stay representable")
}

pub fn resolve_local_datetime<Tz>(
    timezone: &Tz,
    local: NaiveDateTime,
) -> Result<DateTime<Utc>, LocalTimeError>
where
    Tz: TimeZone,
{
    match timezone.from_local_datetime(&local) {
        LocalResult::None => Err(LocalTimeError::Nonexistent),
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => {
            let first = first.with_timezone(&Utc);
            let second = second.with_timezone(&Utc);
            Ok(first.min(second))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LocalTimeError {
    #[error("That local time does not exist because the clock changes then")]
    Nonexistent,
}
