use chrono::{NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::America::New_York;
use remind_me::time_utils::{
    ClockFormat, LocalTimeError, default_due_time, format_clock_time, resolve_local_datetime,
};

#[test]
fn default_due_time_is_one_hour_ahead_rounded_up_to_five_minutes() {
    let now = Utc.with_ymd_and_hms(2027, 1, 3, 10, 2, 31).unwrap();

    let due_at = default_due_time(now);

    assert_eq!(due_at, Utc.with_ymd_and_hms(2027, 1, 3, 11, 5, 0).unwrap());
    assert_eq!(due_at.second(), 0);
}

#[test]
fn nonexistent_spring_forward_time_is_rejected() {
    let local = NaiveDate::from_ymd_opt(2025, 3, 9)
        .unwrap()
        .and_hms_opt(2, 30, 0)
        .unwrap();

    assert_eq!(
        resolve_local_datetime(&New_York, local).unwrap_err(),
        LocalTimeError::Nonexistent
    );
}

#[test]
fn ambiguous_fall_back_time_uses_the_earlier_occurrence() {
    let local = NaiveDate::from_ymd_opt(2025, 11, 2)
        .unwrap()
        .and_hms_opt(1, 30, 0)
        .unwrap();

    let resolved = resolve_local_datetime(&New_York, local).unwrap();

    assert_eq!(
        resolved,
        Utc.with_ymd_and_hms(2025, 11, 2, 5, 30, 0).unwrap()
    );
}

#[test]
fn clock_display_respects_twelve_and_twenty_four_hour_preferences() {
    assert_eq!(
        format_clock_time(0, 5, ClockFormat::TwentyFourHour),
        "00:05"
    );
    assert_eq!(
        format_clock_time(18, 5, ClockFormat::TwentyFourHour),
        "18:05"
    );
    assert_eq!(format_clock_time(0, 5, ClockFormat::TwelveHour), "12:05 AM");
    assert_eq!(format_clock_time(18, 5, ClockFormat::TwelveHour), "6:05 PM");
}
