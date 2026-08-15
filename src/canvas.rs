use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};

use crate::time_utils::ClockFormat;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn format_schedule_suffix<Tz>(
    due_at: DateTime<Utc>,
    now: DateTime<Utc>,
    timezone: &Tz,
    clock_format: ClockFormat,
) -> String
where
    Tz: TimeZone,
{
    let local_due = due_at.with_timezone(timezone);
    let local_now = now.with_timezone(timezone);
    let date = if local_due.date_naive() == local_now.date_naive() {
        "Today".to_owned()
    } else if local_now
        .date_naive()
        .succ_opt()
        .is_some_and(|tomorrow| local_due.date_naive() == tomorrow)
    {
        "Tomorrow".to_owned()
    } else {
        format!(
            "{} {} {}",
            MONTHS[local_due.month0() as usize],
            local_due.day(),
            local_due.year()
        )
    };
    let time = format_english_time(local_due.hour(), local_due.minute(), clock_format);
    format!("@{date} {time}")
}

/// Escapes boundary-style at signs so committed messages can be parsed again
/// without accidentally interpreting note content as a schedule.
pub fn escape_message(message: &str) -> String {
    let mut escaped = String::with_capacity(message.len());
    for (index, character) in message.char_indices() {
        if character == '@'
            && (index == 0
                || message[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace))
        {
            escaped.push('@');
        }
        escaped.push(character);
    }
    escaped
}

pub fn normalize_registered_suffix(text: &str, previous: &str, current: &str) -> String {
    text.strip_suffix(previous)
        .map_or_else(|| text.to_owned(), |message| format!("{message}{current}"))
}

pub fn normalize_stored_working_suffix<Tz>(
    text: &str,
    due_at: DateTime<Utc>,
    saved_at: DateTime<Utc>,
    now: DateTime<Utc>,
    timezone: &Tz,
    clock_format: ClockFormat,
) -> String
where
    Tz: TimeZone,
{
    let current = format_schedule_suffix(due_at, now, timezone, clock_format);
    for previous_format in [ClockFormat::TwelveHour, ClockFormat::TwentyFourHour] {
        let previous = format_schedule_suffix(due_at, saved_at, timezone, previous_format);
        let normalized = normalize_registered_suffix(text, &previous, &current);
        if normalized != text {
            return normalized;
        }
    }
    text.to_owned()
}

pub fn normalize_working_after_due_change<Tz>(
    text: &str,
    previous_due_at: DateTime<Utc>,
    due_at: DateTime<Utc>,
    saved_at: DateTime<Utc>,
    now: DateTime<Utc>,
    timezone: &Tz,
) -> String
where
    Tz: TimeZone,
{
    for format in [ClockFormat::TwelveHour, ClockFormat::TwentyFourHour] {
        let previous = format_schedule_suffix(previous_due_at, saved_at, timezone, format);
        let current = format_schedule_suffix(due_at, now, timezone, format);
        let normalized = normalize_registered_suffix(text, &previous, &current);
        if normalized != text {
            return normalized;
        }
    }
    text.to_owned()
}

fn format_english_time(hour: u32, minute: u32, format: ClockFormat) -> String {
    match format {
        ClockFormat::TwentyFourHour => format!("{hour:02}:{minute:02}"),
        ClockFormat::TwelveHour => {
            let period = if hour < 12 { "AM" } else { "PM" };
            let display_hour = match hour % 12 {
                0 => 12,
                value => value,
            };
            format!("{display_hour}:{minute:02} {period}")
        }
    }
}
