use std::ops::Range;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use thiserror::Error;

use crate::time_utils::{default_due_time, resolve_local_datetime};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedComposerInput {
    pub message: String,
    pub schedule_span: Option<Range<usize>>,
    pub status: ScheduleParseStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleParseStatus {
    Default,
    Partial,
    Valid(ScheduleExpression),
    Invalid(ScheduleParseError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleExpression {
    Relative(Duration),
    TimeOfDay(NaiveTime),
    Date {
        day: DaySpec,
        time: Option<NaiveTime>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaySpec {
    Today,
    Tomorrow,
    Weekday {
        weekday: Weekday,
        following_week: bool,
    },
    MonthDay {
        month: u32,
        day: u32,
        year: Option<i32>,
    },
    Exact(NaiveDate),
}

pub fn parse_english(input: &str) -> ParsedComposerInput {
    let Some(marker) = final_schedule_marker(input) else {
        return ParsedComposerInput {
            message: unescape_markers(input.trim()),
            schedule_span: None,
            status: ScheduleParseStatus::Default,
        };
    };

    let message = unescape_markers(input[..marker].trim_end());
    let phrase = input[marker + 1..].trim();
    let status = if phrase.is_empty() || looks_partial(phrase) {
        ScheduleParseStatus::Partial
    } else {
        match parse_expression(phrase) {
            Ok(expression) => ScheduleParseStatus::Valid(expression),
            Err(error) => ScheduleParseStatus::Invalid(error),
        }
    };

    ParsedComposerInput {
        message,
        schedule_span: Some(marker..input.len()),
        status,
    }
}

pub fn resolve_schedule<Tz>(
    expression: &ScheduleExpression,
    now: DateTime<Utc>,
    timezone: &Tz,
) -> Result<DateTime<Utc>, ScheduleError>
where
    Tz: TimeZone,
{
    match expression {
        ScheduleExpression::Relative(duration) => {
            let due_at = now
                .checked_add_signed(*duration)
                .ok_or(ScheduleError::OutOfRange)?;
            ensure_future(due_at, now)
        }
        ScheduleExpression::TimeOfDay(time) => {
            let local_now = now.with_timezone(timezone);
            let today = local_now.date_naive();
            let mut due_at = resolve_local(timezone, today, *time)?;
            if due_at <= now {
                let tomorrow = today.succ_opt().ok_or(ScheduleError::OutOfRange)?;
                due_at = resolve_local(timezone, tomorrow, *time)?;
            }
            ensure_future(due_at, now)
        }
        ScheduleExpression::Date { day, time } => {
            if matches!(day, DaySpec::Today) && time.is_none() {
                return ensure_future(default_due_time(now), now);
            }
            resolve_date(day, time.unwrap_or_else(default_day_time), now, timezone)
        }
    }
}

fn parse_expression(phrase: &str) -> Result<ScheduleExpression, ScheduleParseError> {
    let normalized = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = normalized.split_whitespace().collect::<Vec<_>>();

    if words
        .first()
        .is_some_and(|word| word.eq_ignore_ascii_case("in"))
    {
        return parse_relative(&words);
    }

    if words.len() == 1 && words[0].eq_ignore_ascii_case("tonight") {
        return Ok(ScheduleExpression::Date {
            day: DaySpec::Today,
            time: Some(NaiveTime::from_hms_opt(19, 0, 0).expect("19:00 is valid")),
        });
    }

    if words.len() == 2 && parse_month(words[0]).is_some() {
        let day = parse_day(&words)?;
        return Ok(ScheduleExpression::Date { day, time: None });
    }

    if let Some((time, consumed)) = parse_trailing_time(&words) {
        let mut day_words = &words[..words.len() - consumed];
        if day_words
            .last()
            .is_some_and(|word| word.eq_ignore_ascii_case("at"))
        {
            day_words = &day_words[..day_words.len() - 1];
        }
        if day_words.is_empty() {
            return Ok(ScheduleExpression::TimeOfDay(time));
        }
        let day = parse_day(day_words)?;
        return Ok(ScheduleExpression::Date {
            day,
            time: Some(time),
        });
    }

    let day = parse_day(&words)?;
    Ok(ScheduleExpression::Date { day, time: None })
}

fn parse_relative(words: &[&str]) -> Result<ScheduleExpression, ScheduleParseError> {
    if words.len() != 3 {
        return Err(ScheduleParseError::Unsupported);
    }
    let amount = match words[1].to_ascii_lowercase().as_str() {
        "a" | "an" | "one" => 1_i64,
        value => value
            .parse::<i64>()
            .map_err(|_| ScheduleParseError::InvalidAmount)?,
    };
    if amount <= 0 {
        return Err(ScheduleParseError::InvalidAmount);
    }
    let unit_seconds = match words[2].to_ascii_lowercase().as_str() {
        "minute" | "minutes" => 60,
        "hour" | "hours" => 60 * 60,
        "day" | "days" => 24 * 60 * 60,
        "week" | "weeks" => 7 * 24 * 60 * 60,
        _ => return Err(ScheduleParseError::Unsupported),
    };
    let seconds = amount
        .checked_mul(unit_seconds)
        .ok_or(ScheduleParseError::OutOfRange)?;
    Ok(ScheduleExpression::Relative(Duration::seconds(seconds)))
}

fn parse_day(words: &[&str]) -> Result<DaySpec, ScheduleParseError> {
    match words {
        [word] if word.eq_ignore_ascii_case("today") => return Ok(DaySpec::Today),
        [word] if word.eq_ignore_ascii_case("tomorrow") => return Ok(DaySpec::Tomorrow),
        [word] => {
            if let Some(weekday) = parse_weekday(word) {
                return Ok(DaySpec::Weekday {
                    weekday,
                    following_week: false,
                });
            }
            if let Ok(date) = NaiveDate::parse_from_str(word, "%Y-%m-%d") {
                return Ok(DaySpec::Exact(date));
            }
        }
        [next, weekday] if next.eq_ignore_ascii_case("next") => {
            if let Some(weekday) = parse_weekday(weekday) {
                return Ok(DaySpec::Weekday {
                    weekday,
                    following_week: true,
                });
            }
        }
        [month, day] | [month, day, _] => {
            let Some(month) = parse_month(month) else {
                return Err(ScheduleParseError::Unsupported);
            };
            let day = day
                .trim_end_matches(',')
                .parse::<u32>()
                .map_err(|_| ScheduleParseError::InvalidDate)?;
            let year = if words.len() == 3 {
                Some(
                    words[2]
                        .parse::<i32>()
                        .map_err(|_| ScheduleParseError::InvalidDate)?,
                )
            } else {
                None
            };
            let validation_year = year.unwrap_or(2000);
            if NaiveDate::from_ymd_opt(validation_year, month, day).is_none() {
                return Err(ScheduleParseError::InvalidDate);
            }
            return Ok(DaySpec::MonthDay { month, day, year });
        }
        _ => {}
    }
    Err(ScheduleParseError::Unsupported)
}

fn parse_trailing_time(words: &[&str]) -> Option<(NaiveTime, usize)> {
    let last = *words.last()?;
    if let Some(time) = parse_time_word(last, None) {
        return Some((time, 1));
    }
    if words.len() >= 2 {
        let meridiem = last.to_ascii_lowercase();
        if matches!(meridiem.as_str(), "am" | "pm") {
            return parse_time_word(words[words.len() - 2], Some(&meridiem)).map(|time| (time, 2));
        }
    }
    None
}

fn parse_time_word(word: &str, separate_meridiem: Option<&str>) -> Option<NaiveTime> {
    let lower = word.to_ascii_lowercase();
    let fixed = match lower.as_str() {
        "morning" => Some((9, 0)),
        "afternoon" => Some((13, 0)),
        "evening" => Some((18, 0)),
        "tonight" => Some((19, 0)),
        "noon" => Some((12, 0)),
        "midnight" => Some((0, 0)),
        _ => None,
    };
    if let Some((hour, minute)) = fixed {
        return NaiveTime::from_hms_opt(hour, minute, 0);
    }

    let (clock, meridiem) = if let Some(meridiem) = separate_meridiem {
        (lower.as_str(), Some(meridiem))
    } else if let Some(clock) = lower.strip_suffix("am") {
        (clock, Some("am"))
    } else if let Some(clock) = lower.strip_suffix("pm") {
        (clock, Some("pm"))
    } else {
        (lower.as_str(), None)
    };
    let (hour, minute) = if let Some((hour, minute)) = clock.split_once(':') {
        (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?)
    } else {
        (clock.parse::<u32>().ok()?, 0)
    };
    let hour = match meridiem {
        Some("am") if hour == 12 => 0,
        Some("am") if (1..=11).contains(&hour) => hour,
        Some("pm") if hour == 12 => 12,
        Some("pm") if (1..=11).contains(&hour) => hour + 12,
        Some(_) => return None,
        None => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn final_schedule_marker(input: &str) -> Option<usize> {
    input
        .char_indices()
        .filter_map(|(index, character)| {
            if character != '@' {
                return None;
            }
            let starts_boundary = index == 0
                || input[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            let escaped = input[index + 1..].starts_with('@');
            (starts_boundary && !escaped).then_some(index)
        })
        .next_back()
}

fn unescape_markers(message: &str) -> String {
    message.replace("@@", "@")
}

fn looks_partial(phrase: &str) -> bool {
    let lower = phrase.to_ascii_lowercase();
    if parse_month(&lower).is_some() {
        return true;
    }
    if let Some(day) = lower.strip_suffix(" at") {
        let words = day.split_whitespace().collect::<Vec<_>>();
        if parse_day(&words).is_ok() {
            return true;
        }
    }
    if lower.split_whitespace().next_back().is_some_and(|word| {
        word.strip_suffix(':').is_some_and(|hour| {
            !hour.is_empty() && hour.chars().all(|value| value.is_ascii_digit())
        })
    }) {
        return true;
    }
    const STARTS: &[&str] = &[
        "in",
        "today",
        "tomorrow",
        "tonight",
        "morning",
        "afternoon",
        "evening",
        "noon",
        "midnight",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "next",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    if STARTS
        .iter()
        .any(|candidate| candidate.starts_with(&lower) && *candidate != lower)
    {
        return true;
    }
    let words = lower.split_whitespace().collect::<Vec<_>>();
    matches!(words.as_slice(), ["in"] | ["in", _] | ["next"])
        || matches!(words.as_slice(), ["next", weekday] if parse_weekday(weekday).is_none() && ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"].iter().any(|value| value.starts_with(weekday)))
}

fn parse_weekday(word: &str) -> Option<Weekday> {
    match word.to_ascii_lowercase().as_str() {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_month(word: &str) -> Option<u32> {
    match word.to_ascii_lowercase().trim_end_matches('.') {
        "january" | "jan" => Some(1),
        "february" | "feb" => Some(2),
        "march" | "mar" => Some(3),
        "april" | "apr" => Some(4),
        "may" => Some(5),
        "june" | "jun" => Some(6),
        "july" | "jul" => Some(7),
        "august" | "aug" => Some(8),
        "september" | "sep" | "sept" => Some(9),
        "october" | "oct" => Some(10),
        "november" | "nov" => Some(11),
        "december" | "dec" => Some(12),
        _ => None,
    }
}

fn resolve_date<Tz>(
    day: &DaySpec,
    time: NaiveTime,
    now: DateTime<Utc>,
    timezone: &Tz,
) -> Result<DateTime<Utc>, ScheduleError>
where
    Tz: TimeZone,
{
    let local_now = now.with_timezone(timezone);
    let today = local_now.date_naive();
    let due_at = match day {
        DaySpec::Today => resolve_local(timezone, today, time)?,
        DaySpec::Tomorrow => {
            let date = today.succ_opt().ok_or(ScheduleError::OutOfRange)?;
            resolve_local(timezone, date, time)?
        }
        DaySpec::Weekday {
            weekday,
            following_week,
        } => {
            let current = today.weekday().num_days_from_monday() as i64;
            let target = weekday.num_days_from_monday() as i64;
            let days = if *following_week {
                7 - current + target
            } else {
                (target - current).rem_euclid(7)
            };
            let mut date = today
                .checked_add_signed(Duration::days(days))
                .ok_or(ScheduleError::OutOfRange)?;
            let mut candidate = resolve_local(timezone, date, time)?;
            if candidate <= now {
                date = date
                    .checked_add_signed(Duration::days(7))
                    .ok_or(ScheduleError::OutOfRange)?;
                candidate = resolve_local(timezone, date, time)?;
            }
            candidate
        }
        DaySpec::MonthDay { month, day, year } => {
            if let Some(year) = year {
                let date = NaiveDate::from_ymd_opt(*year, *month, *day)
                    .ok_or(ScheduleError::InvalidDate)?;
                resolve_local(timezone, date, time)?
            } else {
                let mut candidate = None;
                for offset in 0..=8 {
                    let Some(year) = today.year().checked_add(offset) else {
                        return Err(ScheduleError::OutOfRange);
                    };
                    let Some(date) = NaiveDate::from_ymd_opt(year, *month, *day) else {
                        continue;
                    };
                    let resolved = resolve_local(timezone, date, time)?;
                    if resolved > now {
                        candidate = Some(resolved);
                        break;
                    }
                }
                candidate.ok_or(ScheduleError::OutOfRange)?
            }
        }
        DaySpec::Exact(date) => resolve_local(timezone, *date, time)?,
    };
    ensure_future(due_at, now)
}

fn resolve_local<Tz>(
    timezone: &Tz,
    date: NaiveDate,
    time: NaiveTime,
) -> Result<DateTime<Utc>, ScheduleError>
where
    Tz: TimeZone,
{
    resolve_local_datetime(timezone, date.and_time(time))
        .map_err(|_| ScheduleError::NonexistentLocalTime)
}

fn ensure_future(
    due_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, ScheduleError> {
    if due_at <= now {
        Err(ScheduleError::DueTimeNotFuture)
    } else {
        Ok(due_at)
    }
}

fn default_day_time() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 0, 0).expect("09:00 is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScheduleParseError {
    #[error("I don't understand that schedule")]
    Unsupported,
    #[error("Use a positive amount of time")]
    InvalidAmount,
    #[error("Choose a valid date")]
    InvalidDate,
    #[error("That schedule is too far away")]
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScheduleError {
    #[error("Choose a time in the future")]
    DueTimeNotFuture,
    #[error("That local time does not exist because the clock changes then")]
    NonexistentLocalTime,
    #[error("Choose a valid date")]
    InvalidDate,
    #[error("That schedule is out of range")]
    OutOfRange,
}
