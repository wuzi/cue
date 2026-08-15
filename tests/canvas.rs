use chrono::{TimeZone, Utc};
use chrono_tz::America::New_York;
use remind_me::{
    canvas::{
        escape_message, format_schedule_suffix, normalize_registered_suffix,
        normalize_stored_working_suffix, normalize_working_after_due_change,
    },
    schedule::{ScheduleParseStatus, parse_english, resolve_schedule},
    time_utils::ClockFormat,
};

#[test]
fn messages_escape_only_at_signs_that_could_be_schedule_boundaries() {
    let message = "Email ada@example.com, ping @Ada, then write @home";
    let escaped = escape_message(message);

    assert_eq!(
        escaped,
        "Email ada@example.com, ping @@Ada, then write @@home"
    );
    let parsed = parse_english(&escaped);
    assert_eq!(parsed.message, message);
    assert_eq!(parsed.schedule_span, None);
}

#[test]
fn dirty_message_can_refresh_its_registered_relative_suffix_without_rescheduling() {
    assert_eq!(
        normalize_registered_suffix(
            "Call Ada with notes @Tomorrow 9:00 AM",
            "@Tomorrow 9:00 AM",
            "@Today 9:00 AM",
        ),
        "Call Ada with notes @Today 9:00 AM"
    );
    assert_eq!(
        normalize_registered_suffix(
            "Call Ada @next Friday 9am",
            "@Tomorrow 9:00 AM",
            "@Today 9:00 AM",
        ),
        "Call Ada @next Friday 9am"
    );
}

#[test]
fn restored_dirty_message_normalizes_a_suffix_saved_before_midnight() {
    let saved_at = New_York
        .with_ymd_and_hms(2026, 8, 15, 23, 50, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let now = New_York
        .with_ymd_and_hms(2026, 8, 16, 0, 5, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let due_at = New_York
        .with_ymd_and_hms(2026, 8, 16, 9, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);

    assert_eq!(
        normalize_stored_working_suffix(
            "Call Ada with notes @Tomorrow 9:00 AM",
            due_at,
            saved_at,
            now,
            &New_York,
            ClockFormat::TwelveHour,
        ),
        "Call Ada with notes @Today 9:00 AM"
    );
}

#[test]
fn snoozing_normalizes_a_persisted_dirty_suffix_to_the_new_due_time() {
    let now = New_York
        .with_ymd_and_hms(2026, 8, 15, 16, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let previous_due = New_York
        .with_ymd_and_hms(2026, 8, 16, 9, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let snoozed_due = now + chrono::Duration::minutes(10);

    assert_eq!(
        normalize_working_after_due_change(
            "Call Ada with notes @Tomorrow 9:00 AM",
            previous_due,
            snoozed_due,
            now,
            now,
            &New_York,
        ),
        "Call Ada with notes @Today 4:10 PM"
    );
}

#[test]
fn suffix_formatter_uses_notion_style_relative_days_and_clock_preference() {
    let now = New_York
        .with_ymd_and_hms(2026, 8, 15, 16, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let today = New_York
        .with_ymd_and_hms(2026, 8, 15, 17, 41, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let tomorrow = New_York
        .with_ymd_and_hms(2026, 8, 16, 9, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);

    assert_eq!(
        format_schedule_suffix(today, now, &New_York, ClockFormat::TwelveHour),
        "@Today 5:41 PM"
    );
    assert_eq!(
        format_schedule_suffix(tomorrow, now, &New_York, ClockFormat::TwentyFourHour),
        "@Tomorrow 09:00"
    );
}

#[test]
fn generated_future_suffixes_parse_and_resolve_to_the_original_minute() {
    let now = New_York
        .with_ymd_and_hms(2026, 12, 31, 20, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let cases = [
        New_York
            .with_ymd_and_hms(2026, 12, 31, 22, 15, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc),
        New_York
            .with_ymd_and_hms(2027, 1, 1, 9, 30, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc),
        New_York
            .with_ymd_and_hms(2027, 3, 4, 14, 5, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc),
    ];

    for due_at in cases {
        for clock in [ClockFormat::TwelveHour, ClockFormat::TwentyFourHour] {
            let suffix = format_schedule_suffix(due_at, now, &New_York, clock);
            let parsed = parse_english(&format!("Message {suffix}"));
            let ScheduleParseStatus::Valid(expression) = parsed.status else {
                panic!("generated suffix did not parse: {suffix}");
            };
            assert_eq!(
                resolve_schedule(&expression, now, &New_York).unwrap(),
                due_at
            );
        }
    }
}

#[test]
fn overdue_suffix_uses_an_explicit_english_date_that_still_parses() {
    let now = New_York
        .with_ymd_and_hms(2026, 8, 15, 16, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let overdue = New_York
        .with_ymd_and_hms(2025, 11, 2, 9, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let suffix = format_schedule_suffix(overdue, now, &New_York, ClockFormat::TwelveHour);

    assert_eq!(suffix, "@Nov 2 2025 9:00 AM");
    assert!(matches!(
        parse_english(&format!("Message {suffix}")).status,
        ScheduleParseStatus::Valid(_)
    ));
}
