use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};

use crate::time_utils::{ClockFormat, default_due_time, format_clock_time, resolve_local_datetime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickerSelection {
    pub date: NaiveDate,
    pub time: NaiveTime,
}

pub struct SchedulePicker {
    popover: gtk::Popover,
    calendar: gtk::Calendar,
    hour: gtk::SpinButton,
    minute: gtk::SpinButton,
    period: Option<gtk::DropDown>,
    summary: gtk::Label,
    error: gtk::Label,
    apply: gtk::Button,
    clock_format: ClockFormat,
    now: DateTime<Utc>,
    anchor_tick: RefCell<Option<gtk::TickCallbackId>>,
}

impl SchedulePicker {
    pub fn new(
        initial: DateTime<Local>,
        now: DateTime<Utc>,
        clock_format: ClockFormat,
    ) -> Rc<Self> {
        let popover = gtk::Popover::new();
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_accessible_role(gtk::AccessibleRole::Dialog);
        popover.add_css_class("reminder-picker");
        popover.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
            "Choose reminder time",
        ))]);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_size_request(304, -1);
        content.add_css_class("reminder-picker-content");

        let summary = gtk::Label::new(None);
        summary.set_halign(gtk::Align::Fill);
        summary.set_xalign(0.0);
        summary.add_css_class("reminder-picker-summary");
        summary.add_css_class("heading");
        content.append(&summary);

        let shortcuts = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        shortcuts.add_css_class("reminder-picker-shortcuts");
        let today = gtk::Button::with_label(&gettextrs::gettext("Today"));
        today.add_css_class("flat");
        today.set_hexpand(true);
        let tomorrow = gtk::Button::with_label(&gettextrs::gettext("Tomorrow"));
        tomorrow.add_css_class("flat");
        tomorrow.set_hexpand(true);
        shortcuts.append(&today);
        shortcuts.append(&tomorrow);
        content.append(&shortcuts);

        let calendar = gtk::Calendar::new();
        calendar.add_css_class("reminder-calendar");
        if let Some(date) = glib_local_noon(initial.date_naive()) {
            calendar.set_date(&date);
        }
        content.append(&calendar);

        let time_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        time_row.add_css_class("reminder-picker-time");
        let time_label = gtk::Label::new(Some(&gettextrs::gettext("Time")));
        time_label.set_halign(gtk::Align::Start);
        time_label.set_hexpand(true);
        time_row.append(&time_label);

        let (display_hour, period_index) = display_time_fields(initial.hour(), clock_format);
        let hour_range = match clock_format {
            ClockFormat::TwelveHour => (1.0, 12.0),
            ClockFormat::TwentyFourHour => (0.0, 23.0),
        };
        let hour = gtk::SpinButton::with_range(hour_range.0, hour_range.1, 1.0);
        hour.set_value(display_hour as f64);
        hour.set_wrap(true);
        hour.set_width_chars(2);
        hour.set_tooltip_text(Some(&gettextrs::gettext("Hour")));
        hour.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
            "Hour",
        ))]);
        let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
        minute.set_value(initial.minute() as f64);
        minute.set_wrap(true);
        minute.set_width_chars(2);
        minute.set_tooltip_text(Some(&gettextrs::gettext("Minute")));
        minute.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
            "Minute",
        ))]);
        time_row.append(&hour);
        time_row.append(&gtk::Label::new(Some(":")));
        time_row.append(&minute);
        let period = period_index.map(|selected| {
            let am = gettextrs::gettext("AM");
            let pm = gettextrs::gettext("PM");
            let dropdown = gtk::DropDown::from_strings(&[&am, &pm]);
            dropdown.set_selected(selected);
            dropdown.update_property(&[gtk::accessible::Property::Label(&gettextrs::gettext(
                "AM or PM",
            ))]);
            time_row.append(&dropdown);
            dropdown
        });
        content.append(&time_row);

        let error = gtk::Label::new(None);
        error.set_visible(false);
        error.set_halign(gtk::Align::Start);
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_accessible_role(gtk::AccessibleRole::Alert);
        error.add_css_class("error");
        error.add_css_class("caption");
        content.append(&error);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label(&gettextrs::gettext("Cancel"));
        let apply = gtk::Button::with_label(&gettextrs::gettext("Apply"));
        apply.add_css_class("suggested-action");
        actions.append(&cancel);
        actions.append(&apply);
        content.append(&actions);
        popover.set_child(Some(&content));

        let picker = Rc::new(Self {
            popover,
            calendar,
            hour,
            minute,
            period,
            summary,
            error,
            apply,
            clock_format,
            now,
            anchor_tick: RefCell::new(None),
        });
        picker.connect_updates(today, tomorrow, cancel);
        picker.refresh_summary();
        picker
    }

    pub fn popup_at<F>(
        self: &Rc<Self>,
        parent: &gtk::TextView,
        anchor: &gtk::gdk::Rectangle,
        anchor_rect: F,
    ) where
        F: Fn() -> Option<gtk::gdk::Rectangle> + 'static,
    {
        if let Some(previous) = self.anchor_tick.borrow_mut().take() {
            previous.remove();
        }
        self.popover.set_parent(parent);
        self.popover.set_pointing_to(Some(anchor));
        let weak = Rc::downgrade(self);
        let tick = parent.add_tick_callback(move |_, _| {
            let Some(picker) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some(anchor) = anchor_rect() {
                picker.popover.set_pointing_to(Some(&anchor));
            }
            glib::ControlFlow::Continue
        });
        self.anchor_tick.borrow_mut().replace(tick);
        self.popover.popup();
    }

    pub fn popdown_and_unparent(&self) {
        self.popover.popdown();
        self.unparent();
    }

    pub fn connect_closed<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        self.popover.connect_closed(move |_| callback());
    }

    pub fn unparent(&self) {
        if let Some(tick) = self.anchor_tick.borrow_mut().take() {
            tick.remove();
        }
        if self.popover.parent().is_some() {
            self.popover.unparent();
        }
    }

    pub fn connect_apply<F>(self: &Rc<Self>, callback: F)
    where
        F: Fn(PickerSelection) -> Result<(), String> + 'static,
    {
        let weak = Rc::downgrade(self);
        self.apply.connect_clicked(move |_| {
            let Some(picker) = weak.upgrade() else {
                return;
            };
            let selection = match picker.selection() {
                Ok(selection) => selection,
                Err(message) => {
                    picker.show_error(&message);
                    return;
                }
            };
            match callback(selection) {
                Ok(()) => picker.popover.popdown(),
                Err(message) => picker.show_error(&message),
            }
        });
    }

    fn connect_updates(
        self: &Rc<Self>,
        today: gtk::Button,
        tomorrow: gtk::Button,
        cancel: gtk::Button,
    ) {
        let weak = Rc::downgrade(self);
        self.calendar.connect_day_selected(move |_| {
            if let Some(picker) = weak.upgrade() {
                picker.clear_error();
                picker.refresh_summary();
            }
        });
        let weak = Rc::downgrade(self);
        self.hour.connect_value_changed(move |_| {
            if let Some(picker) = weak.upgrade() {
                picker.clear_error();
                picker.refresh_summary();
            }
        });
        let weak = Rc::downgrade(self);
        self.minute.connect_value_changed(move |_| {
            if let Some(picker) = weak.upgrade() {
                picker.clear_error();
                picker.refresh_summary();
            }
        });
        if let Some(period) = &self.period {
            let weak = Rc::downgrade(self);
            period.connect_selected_notify(move |_| {
                if let Some(picker) = weak.upgrade() {
                    picker.clear_error();
                    picker.refresh_summary();
                }
            });
        }

        let weak = Rc::downgrade(self);
        today.connect_clicked(move |_| {
            if let Some(picker) = weak.upgrade() {
                picker.select_today();
            }
        });
        let weak = Rc::downgrade(self);
        tomorrow.connect_clicked(move |_| {
            if let Some(picker) = weak.upgrade() {
                picker.select_tomorrow();
            }
        });
        let popover = self.popover.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
        });
    }

    fn selection(&self) -> Result<PickerSelection, String> {
        let selected = self.calendar.date();
        let date = NaiveDate::from_ymd_opt(
            selected.year(),
            selected.month() as u32,
            selected.day_of_month() as u32,
        )
        .ok_or_else(|| gettextrs::gettext("Choose a valid date"))?;
        let hour = hour_24(
            self.hour.value_as_int() as u32,
            self.period.as_ref().map(|period| period.selected()),
            self.clock_format,
        )
        .ok_or_else(|| gettextrs::gettext("Choose a valid time"))?;
        let time = NaiveTime::from_hms_opt(hour, self.minute.value_as_int() as u32, 0)
            .ok_or_else(|| gettextrs::gettext("Choose a valid time"))?;
        Ok(PickerSelection { date, time })
    }

    fn select_today(&self) {
        let local_now = self.now.with_timezone(&Local);
        let selected_time = self
            .selection()
            .map(|selection| selection.time)
            .unwrap_or_else(|_| default_due_time(self.now).with_timezone(&Local).time());
        let candidate = local_now.date_naive().and_time(selected_time);
        if !resolve_local_datetime(&Local, candidate).is_ok_and(|value| value > self.now) {
            let Some(fallback) = fallback_time_for_today(self.now, &Local) else {
                self.show_error(&gettextrs::gettext(
                    "There is no selectable time left today.",
                ));
                return;
            };
            self.set_time(fallback);
        }
        self.set_date(local_now.date_naive());
        self.clear_error();
        self.refresh_summary();
    }

    fn select_tomorrow(&self) {
        let local_now = self.now.with_timezone(&Local);
        if let Some(date) = local_now.date_naive().succ_opt() {
            self.set_date(date);
        }
        self.clear_error();
        self.refresh_summary();
    }

    fn set_date(&self, date: NaiveDate) {
        if let Some(date) = glib_local_noon(date) {
            self.calendar.set_date(&date);
        }
    }

    fn set_time(&self, time: NaiveTime) {
        let (hour, period) = display_time_fields(time.hour(), self.clock_format);
        self.hour.set_value(hour as f64);
        self.minute.set_value(time.minute() as f64);
        if let (Some(dropdown), Some(period)) = (&self.period, period) {
            dropdown.set_selected(period);
        }
    }

    fn refresh_summary(&self) {
        let Ok(selection) = self.selection() else {
            return;
        };
        let date = glib_local_noon(selection.date)
            .and_then(|value| value.format("%x").ok())
            .map(|value| value.to_string())
            .unwrap_or_else(|| selection.date.to_string());
        self.summary.set_label(&format!(
            "{date}  {}",
            format_clock_time(
                selection.time.hour(),
                selection.time.minute(),
                self.clock_format,
            )
        ));
    }

    fn show_error(&self, message: &str) {
        self.error.set_label(message);
        self.error.set_visible(true);
    }

    fn clear_error(&self) {
        self.error.set_visible(false);
    }
}

fn glib_local_noon(date: NaiveDate) -> Option<glib::DateTime> {
    glib::DateTime::new(
        &glib::TimeZone::local(),
        date.year(),
        date.month() as i32,
        date.day() as i32,
        12,
        0,
        0.0,
    )
    .ok()
}

fn display_time_fields(hour: u32, format: ClockFormat) -> (u32, Option<u32>) {
    match format {
        ClockFormat::TwentyFourHour => (hour, None),
        ClockFormat::TwelveHour => {
            let period = u32::from(hour >= 12);
            let display_hour = match hour % 12 {
                0 => 12,
                value => value,
            };
            (display_hour, Some(period))
        }
    }
}

fn hour_24(hour: u32, period: Option<u32>, format: ClockFormat) -> Option<u32> {
    match format {
        ClockFormat::TwentyFourHour => (hour <= 23).then_some(hour),
        ClockFormat::TwelveHour => match (hour, period) {
            (1..=11, Some(0)) => Some(hour),
            (12, Some(0)) => Some(0),
            (1..=11, Some(1)) => Some(hour + 12),
            (12, Some(1)) => Some(12),
            _ => None,
        },
    }
}

fn fallback_time_for_today<Tz>(now: DateTime<Utc>, timezone: &Tz) -> Option<NaiveTime>
where
    Tz: TimeZone,
{
    let today = now.with_timezone(timezone).date_naive();
    let default = default_due_time(now).with_timezone(timezone);
    if default.date_naive() == today
        && resolve_local_datetime(timezone, today.and_time(default.time()))
            .is_ok_and(|value| value > now)
    {
        return Some(default.time());
    }

    for minutes in 1..=60 {
        let local = (now + Duration::minutes(minutes)).with_timezone(timezone);
        if local.date_naive() != today {
            return None;
        }
        let time = local.time().with_second(0)?.with_nanosecond(0)?;
        if resolve_local_datetime(timezone, today.and_time(time)).is_ok_and(|value| value > now) {
            return Some(time);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveTime, TimeZone, Utc};

    use crate::time_utils::ClockFormat;

    use super::{display_time_fields, fallback_time_for_today, hour_24};

    #[test]
    fn time_fields_round_trip_midnight_noon_and_evening_in_both_clock_formats() {
        assert_eq!(
            display_time_fields(0, ClockFormat::TwelveHour),
            (12, Some(0))
        );
        assert_eq!(
            display_time_fields(12, ClockFormat::TwelveHour),
            (12, Some(1))
        );
        assert_eq!(
            display_time_fields(20, ClockFormat::TwelveHour),
            (8, Some(1))
        );
        assert_eq!(hour_24(12, Some(0), ClockFormat::TwelveHour), Some(0));
        assert_eq!(hour_24(12, Some(1), ClockFormat::TwelveHour), Some(12));
        assert_eq!(hour_24(8, Some(1), ClockFormat::TwelveHour), Some(20));

        assert_eq!(
            display_time_fields(0, ClockFormat::TwentyFourHour),
            (0, None)
        );
        assert_eq!(
            display_time_fields(20, ClockFormat::TwentyFourHour),
            (20, None)
        );
        assert_eq!(hour_24(20, None, ClockFormat::TwentyFourHour), Some(20));
    }

    #[test]
    fn today_fallback_stays_today_or_reports_that_no_full_minute_remains() {
        let ordinary = Utc.with_ymd_and_hms(2026, 8, 15, 22, 50, 0).unwrap();
        assert_eq!(
            fallback_time_for_today(ordinary, &Utc),
            NaiveTime::from_hms_opt(23, 50, 0)
        );

        let late = Utc.with_ymd_and_hms(2026, 8, 15, 23, 30, 45).unwrap();
        assert_eq!(
            fallback_time_for_today(late, &Utc),
            NaiveTime::from_hms_opt(23, 31, 0)
        );

        let no_minute_left = Utc.with_ymd_and_hms(2026, 8, 15, 23, 59, 0).unwrap();
        assert_eq!(fallback_time_for_today(no_minute_left, &Utc), None);
    }
}
