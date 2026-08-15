use chrono::{Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::America::New_York;
use remind_me::schedule::{
    DaySpec, ScheduleError, ScheduleExpression, ScheduleParseStatus, parse_english,
    resolve_schedule,
};

fn valid_expression(input: &str) -> ScheduleExpression {
    match parse_english(input).status {
        ScheduleParseStatus::Valid(expression) => expression,
        status => panic!("expected a valid schedule for {input:?}, got {status:?}"),
    }
}

#[test]
fn composer_input_separates_the_final_schedule_and_unescapes_literal_markers() {
    let parsed = parse_english("Email ada@example.com and ping @@Ada");

    assert_eq!(parsed.message, "Email ada@example.com and ping @Ada");
    assert_eq!(parsed.schedule_span, None);
    assert_eq!(parsed.status, ScheduleParseStatus::Default);

    let input = "Ligar para João @Tomorrow 9:30 PM";
    let parsed = parse_english(input);
    let span = parsed.schedule_span.clone().unwrap();

    assert_eq!(parsed.message, "Ligar para João");
    assert_eq!(&input[span], "@Tomorrow 9:30 PM");
    assert_eq!(
        parsed.status,
        ScheduleParseStatus::Valid(ScheduleExpression::Date {
            day: DaySpec::Tomorrow,
            time: Some(NaiveTime::from_hms_opt(21, 30, 0).unwrap()),
        })
    );
}

#[test]
fn parser_accepts_the_documented_relative_day_time_and_date_phrases() {
    let cases = [
        (
            "Take a break @in 15 minutes",
            "Take a break",
            ScheduleExpression::Relative(Duration::minutes(15)),
        ),
        (
            "Call Ada @in an hour",
            "Call Ada",
            ScheduleExpression::Relative(Duration::hours(1)),
        ),
        (
            "Back up files @in 2 days",
            "Back up files",
            ScheduleExpression::Relative(Duration::hours(48)),
        ),
        (
            "Review goals @IN 1 WEEK",
            "Review goals",
            ScheduleExpression::Relative(Duration::weeks(1)),
        ),
        (
            "Send update @today",
            "Send update",
            ScheduleExpression::Date {
                day: DaySpec::Today,
                time: None,
            },
        ),
        (
            "Team sync @Wednesday",
            "Team sync",
            ScheduleExpression::Date {
                day: DaySpec::Weekday {
                    weekday: Weekday::Wed,
                    following_week: false,
                },
                time: None,
            },
        ),
        (
            "Plan review @next Friday at 14:30",
            "Plan review",
            ScheduleExpression::Date {
                day: DaySpec::Weekday {
                    weekday: Weekday::Fri,
                    following_week: true,
                },
                time: Some(NaiveTime::from_hms_opt(14, 30, 0).unwrap()),
            },
        ),
        (
            "Call home @tomorrow morning",
            "Call home",
            ScheduleExpression::Date {
                day: DaySpec::Tomorrow,
                time: Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
            },
        ),
        (
            "Take medicine @tonight",
            "Take medicine",
            ScheduleExpression::Date {
                day: DaySpec::Today,
                time: Some(NaiveTime::from_hms_opt(19, 0, 0).unwrap()),
            },
        ),
        (
            "Start cooking @6pm",
            "Start cooking",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(18, 0, 0).unwrap()),
        ),
        (
            "Breakfast @9:30 AM",
            "Breakfast",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(9, 30, 0).unwrap()),
        ),
        (
            "Standup @14:30",
            "Standup",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(14, 30, 0).unwrap()),
        ),
        (
            "Lunch @noon",
            "Lunch",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(12, 0, 0).unwrap()),
        ),
        (
            "Reset counter @midnight",
            "Reset counter",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
        ),
        (
            "Morning pages @morning",
            "Morning pages",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
        ),
        (
            "Walk @afternoon",
            "Walk",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(13, 0, 0).unwrap()),
        ),
        (
            "Read @evening",
            "Read",
            ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(18, 0, 0).unwrap()),
        ),
        (
            "Renew passport @Aug 20",
            "Renew passport",
            ScheduleExpression::Date {
                day: DaySpec::MonthDay {
                    month: 8,
                    day: 20,
                    year: None,
                },
                time: None,
            },
        ),
        (
            "Dentist @Aug 20 6pm",
            "Dentist",
            ScheduleExpression::Date {
                day: DaySpec::MonthDay {
                    month: 8,
                    day: 20,
                    year: None,
                },
                time: Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap()),
            },
        ),
        (
            "Conference @August 20 2027 noon",
            "Conference",
            ScheduleExpression::Date {
                day: DaySpec::MonthDay {
                    month: 8,
                    day: 20,
                    year: Some(2027),
                },
                time: Some(NaiveTime::from_hms_opt(12, 0, 0).unwrap()),
            },
        ),
        (
            "Release @2027-08-20 09:00",
            "Release",
            ScheduleExpression::Date {
                day: DaySpec::Exact(NaiveDate::from_ymd_opt(2027, 8, 20).unwrap()),
                time: Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
            },
        ),
    ];

    for (input, expected_message, expected_expression) in cases {
        let parsed = parse_english(input);
        assert_eq!(parsed.message, expected_message, "input: {input}");
        assert_eq!(
            parsed.status,
            ScheduleParseStatus::Valid(expected_expression),
            "input: {input}"
        );
    }
}

#[test]
fn parser_distinguishes_default_partial_and_invalid_input_without_guessing() {
    assert_eq!(
        parse_english("Call Ada").status,
        ScheduleParseStatus::Default
    );
    assert!(matches!(
        parse_english("Call Ada @").status,
        ScheduleParseStatus::Partial
    ));
    assert!(matches!(
        parse_english("Call Ada @tomor").status,
        ScheduleParseStatus::Partial
    ));
    for input in [
        "Call Ada @tomorrow at",
        "Call Ada @next Friday at",
        "Call Ada @Aug 20 at",
        "Call Ada @May",
        "Call Ada @tomorrow 9:",
    ] {
        assert!(
            matches!(parse_english(input).status, ScheduleParseStatus::Partial),
            "input: {input}"
        );
    }
    assert!(matches!(
        parse_english("Call Ada @tomorow").status,
        ScheduleParseStatus::Invalid(_)
    ));
    assert!(matches!(
        parse_english("Call Ada @in 0 minutes").status,
        ScheduleParseStatus::Invalid(_)
    ));
    assert!(matches!(
        parse_english("Call Ada @February 30 9am").status,
        ScheduleParseStatus::Invalid(remind_me::schedule::ScheduleParseError::InvalidDate)
    ));
    assert!(matches!(
        parse_english("Call Ada @February 30").status,
        ScheduleParseStatus::Invalid(remind_me::schedule::ScheduleParseError::InvalidDate)
    ));
}

#[test]
fn schedule_suffix_does_not_reduce_the_available_message_length() {
    let message = "x".repeat(280);
    let parsed = parse_english(&format!("{message} @tomorrow"));

    assert_eq!(parsed.message.chars().count(), 280);
    assert!(matches!(parsed.status, ScheduleParseStatus::Valid(_)));
}

#[test]
fn resolver_applies_relative_defaults_and_calendar_rollover_rules() {
    let now = New_York
        .with_ymd_and_hms(2026, 8, 15, 16, 42, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);

    let relative = resolve_schedule(
        &ScheduleExpression::Relative(Duration::hours(24)),
        now,
        &New_York,
    )
    .unwrap();
    assert_eq!(relative, now + Duration::hours(24));

    let tomorrow = resolve_schedule(
        &ScheduleExpression::Date {
            day: DaySpec::Tomorrow,
            time: None,
        },
        now,
        &New_York,
    )
    .unwrap();
    assert_eq!(
        tomorrow,
        New_York
            .with_ymd_and_hms(2026, 8, 16, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );

    let today = resolve_schedule(
        &ScheduleExpression::Date {
            day: DaySpec::Today,
            time: None,
        },
        now,
        &New_York,
    )
    .unwrap();
    assert_eq!(
        today,
        New_York
            .with_ymd_and_hms(2026, 8, 15, 17, 45, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );

    let next_monday = resolve_schedule(
        &ScheduleExpression::Date {
            day: DaySpec::Weekday {
                weekday: Weekday::Mon,
                following_week: true,
            },
            time: None,
        },
        now,
        &New_York,
    )
    .unwrap();
    assert_eq!(
        next_monday,
        New_York
            .with_ymd_and_hms(2026, 8, 17, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );

    let aug_fourteenth = resolve_schedule(
        &ScheduleExpression::Date {
            day: DaySpec::MonthDay {
                month: 8,
                day: 14,
                year: None,
            },
            time: None,
        },
        now,
        &New_York,
    )
    .unwrap();
    assert_eq!(
        aug_fourteenth,
        New_York
            .with_ymd_and_hms(2027, 8, 14, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );
}

#[test]
fn resolver_uses_the_next_future_time_and_bare_weekday_occurrence() {
    let friday_morning = New_York
        .with_ymd_and_hms(2026, 8, 21, 8, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let friday = ScheduleExpression::Date {
        day: DaySpec::Weekday {
            weekday: Weekday::Fri,
            following_week: false,
        },
        time: None,
    };
    assert_eq!(
        resolve_schedule(&friday, friday_morning, &New_York).unwrap(),
        New_York
            .with_ymd_and_hms(2026, 8, 21, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );

    let afternoon = New_York
        .with_ymd_and_hms(2026, 8, 21, 16, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let nine = ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    assert_eq!(
        resolve_schedule(&nine, afternoon, &New_York).unwrap(),
        New_York
            .with_ymd_and_hms(2026, 8, 22, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );
}

#[test]
fn next_weekday_uses_the_following_calendar_week_even_after_todays_time() {
    let friday_afternoon = New_York
        .with_ymd_and_hms(2026, 8, 21, 16, 0, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let next_friday = ScheduleExpression::Date {
        day: DaySpec::Weekday {
            weekday: Weekday::Fri,
            following_week: true,
        },
        time: Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
    };

    assert_eq!(
        resolve_schedule(&next_friday, friday_afternoon, &New_York).unwrap(),
        New_York
            .with_ymd_and_hms(2026, 8, 28, 9, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    );
}

#[test]
fn resolver_rejects_past_and_nonexistent_times_and_uses_earlier_ambiguous_time() {
    let now = New_York
        .with_ymd_and_hms(2026, 8, 15, 16, 42, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    let past = ScheduleExpression::Date {
        day: DaySpec::Exact(NaiveDate::from_ymd_opt(2026, 8, 14).unwrap()),
        time: Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
    };
    assert_eq!(
        resolve_schedule(&past, now, &New_York),
        Err(ScheduleError::DueTimeNotFuture)
    );

    let nonexistent = ScheduleExpression::Date {
        day: DaySpec::Exact(NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()),
        time: Some(NaiveTime::from_hms_opt(2, 30, 0).unwrap()),
    };
    assert_eq!(
        resolve_schedule(&nonexistent, now - Duration::days(365), &New_York),
        Err(ScheduleError::NonexistentLocalTime)
    );

    let ambiguous = ScheduleExpression::Date {
        day: DaySpec::Exact(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()),
        time: Some(NaiveTime::from_hms_opt(1, 30, 0).unwrap()),
    };
    assert_eq!(
        resolve_schedule(&ambiguous, now, &New_York).unwrap(),
        Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0)
            .single()
            .unwrap()
    );
}

#[test]
fn helper_exposes_valid_expression_without_reading_the_clock() {
    assert_eq!(
        valid_expression("Call Ada @noon"),
        ScheduleExpression::TimeOfDay(NaiveTime::from_hms_opt(12, 0, 0).unwrap())
    );
}
