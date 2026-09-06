//! Calendar arithmetic for the docket, without a date dependency.
//!
//! The workspace has no date/time crate and this module is why it still does
//! not need one. A docket needs four things — parse a stored date, step by days
//! or weeks, find the start of a week, and name a weekday — and all four fall
//! out of the civil-date algorithm below in about forty lines that are
//! deterministic and testable, where a crate would be a dependency in the
//! eventual server build too.
//!
//! Dates are the same text the database stores: `YYYY-MM-DD`, which is also a
//! `DATE` in PostgreSQL when the office schema moves behind a server.
//!
//! There is exactly one clock in the office layer, [`today`], and it reads the
//! **local** date. That is deliberate and is not an inconsistency with the UTC
//! `datetime('now')` defaults on `created_at` columns: a court setting happens
//! on the day the courthouse says it does, while an audit trail is only
//! comparable if every row on it is stamped in one zone.

use crate::error::{Error, Result};

/// Days from the civil epoch 1970-01-01 for the first day of March, which is
/// where the shifted-year algorithm counts from.
const DAYS_FROM_EPOCH_TO_MARCH: i64 = 719_468;

/// Days in the 400-year Gregorian cycle.
const DAYS_PER_ERA: i64 = 146_097;

/// A calendar date, held as its three civil fields.
///
/// Ordering is chronological because the fields are declared most significant
/// first, which is also the order the text form sorts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    year: i32,
    month: u32,
    day: u32,
}

impl CivilDate {
    /// Builds a date, returning `None` unless it is a real day of a real month.
    ///
    /// This is the check the schema cannot make: a `GLOB` pattern accepts
    /// `2026-02-30` because it only sees the shape.
    pub const fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Reads the stored `YYYY-MM-DD` form.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let year: i32 = text.get(0..4)?.parse().ok()?;
        let month: u32 = text.get(5..7)?.parse().ok()?;
        let day: u32 = text.get(8..10)?.parse().ok()?;
        Self::new(year, month, day)
    }

    /// Reads the stored form, naming the field when it is not a real date.
    pub fn require(text: &str) -> Result<Self> {
        Self::parse(text).ok_or_else(|| Error::InvalidDate {
            value: text.to_owned(),
            expected: "YYYY-MM-DD calendar date",
        })
    }

    /// Renders the stored `YYYY-MM-DD` form.
    pub fn to_text(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The calendar year.
    pub const fn year(self) -> i32 {
        self.year
    }

    /// The month, 1 through 12.
    pub const fn month(self) -> u32 {
        self.month
    }

    /// The day of the month, from 1.
    pub const fn day(self) -> u32 {
        self.day
    }

    /// Days since 1970-01-01, negative before it.
    ///
    /// Howard Hinnant's `days_from_civil`: shifting the year to start in March
    /// puts the leap day at the end, so the day-of-year term is a single linear
    /// expression with no month table and no special case for February.
    pub const fn days_from_epoch(self) -> i64 {
        let year = self.year as i64 - if self.month <= 2 { 1 } else { 0 };
        let era = if year >= 0 { year } else { year - 399 } / 400;
        let year_of_era = year - era * 400;
        let month = self.month as i64;
        let day_of_year =
            (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + self.day as i64 - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * DAYS_PER_ERA + day_of_era - DAYS_FROM_EPOCH_TO_MARCH
    }

    /// The date that many days after 1970-01-01. The inverse of
    /// [`Self::days_from_epoch`].
    pub const fn from_days_since_epoch(days: i64) -> Self {
        let shifted = days + DAYS_FROM_EPOCH_TO_MARCH;
        let era = if shifted >= 0 {
            shifted
        } else {
            shifted - DAYS_PER_ERA + 1
        } / DAYS_PER_ERA;
        let day_of_era = shifted - era * DAYS_PER_ERA;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = shifted_month + if shifted_month < 10 { 3 } else { -9 };
        Self {
            year: (year + if month <= 2 { 1 } else { 0 }) as i32,
            month: month as u32,
            day: day as u32,
        }
    }

    /// The date this many days later. A negative delta steps backwards.
    #[must_use]
    pub const fn add_days(self, delta: i64) -> Self {
        Self::from_days_since_epoch(self.days_from_epoch() + delta)
    }

    /// The date this many weeks later. A negative delta steps backwards.
    #[must_use]
    pub const fn add_weeks(self, delta: i64) -> Self {
        self.add_days(delta * 7)
    }

    /// Whole days from this date to another, negative when the other is earlier.
    pub const fn days_until(self, other: Self) -> i64 {
        other.days_from_epoch() - self.days_from_epoch()
    }

    /// The day of the week.
    pub const fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday, so the epoch offset is four days past
        // Sunday. The extra `+ 7` keeps the remainder non-negative for dates
        // before the epoch, where Rust's `%` would otherwise return a negative.
        Weekday::from_index(((self.days_from_epoch() % 7 + 11) % 7) as u32)
    }

    /// The Monday of this date's week.
    ///
    /// The court week is the unit a defender plans in, and it starts on Monday
    /// regardless of where the calendar widget thinks a week begins.
    #[must_use]
    pub const fn week_start(self) -> Self {
        self.add_days(-(self.weekday().days_since_monday() as i64))
    }

    /// The Sunday closing this date's week.
    #[must_use]
    pub const fn week_end(self) -> Self {
        self.week_start().add_days(6)
    }
}

impl std::fmt::Display for CivilDate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// Days in a month, accounting for the Gregorian leap rule.
const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// A day of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weekday {
    /// Sunday.
    Sunday,
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
}

impl Weekday {
    /// Builds a weekday from its index, counting Sunday as zero.
    const fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Sunday,
            1 => Self::Monday,
            2 => Self::Tuesday,
            3 => Self::Wednesday,
            4 => Self::Thursday,
            5 => Self::Friday,
            _ => Self::Saturday,
        }
    }

    /// The weekday's full name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sunday => "Sunday",
            Self::Monday => "Monday",
            Self::Tuesday => "Tuesday",
            Self::Wednesday => "Wednesday",
            Self::Thursday => "Thursday",
            Self::Friday => "Friday",
            Self::Saturday => "Saturday",
        }
    }

    /// Days elapsed since the Monday of the same week.
    pub const fn days_since_monday(self) -> u32 {
        match self {
            Self::Monday => 0,
            Self::Tuesday => 1,
            Self::Wednesday => 2,
            Self::Thursday => 3,
            Self::Friday => 4,
            Self::Saturday => 5,
            Self::Sunday => 6,
        }
    }

    /// Returns whether a court is ordinarily sitting on this day.
    pub const fn is_court_day(self) -> bool {
        !matches!(self, Self::Saturday | Self::Sunday)
    }
}

/// Reads a stored `HH:MM` time, returning `None` unless it names a real minute.
pub fn parse_time(text: &str) -> Option<(u32, u32)> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    let hour: u32 = text.get(0..2)?.parse().ok()?;
    let minute: u32 = text.get(3..5)?.parse().ok()?;
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// Reads a stored `HH:MM` time, naming the field when it is not one.
pub fn require_time(text: &str) -> Result<String> {
    parse_time(text)
        .map(|(hour, minute)| format!("{hour:02}:{minute:02}"))
        .ok_or_else(|| Error::InvalidDate {
            value: text.to_owned(),
            expected: "HH:MM time of day",
        })
}

/// Today's **local** date, in the stored `YYYY-MM-DD` form.
///
/// The only clock in the office layer. It is a `SQLite` call rather than a
/// system-time conversion because the database already knows the local zone and
/// converting epoch seconds correctly would need the dependency this module
/// exists to avoid.
pub fn today(connection: &rusqlite::Connection) -> rusqlite::Result<String> {
    connection.query_row("SELECT date('now','localtime')", [], |row| row.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_round_trips_through_its_epoch_day_across_two_centuries() {
        let mut date = CivilDate::new(1899, 12, 31).expect("start date");
        let last = CivilDate::new(2101, 1, 1).expect("end date");
        while date < last {
            let days = date.days_from_epoch();
            assert_eq!(CivilDate::from_days_since_epoch(days), date, "{date}");
            assert_eq!(CivilDate::parse(&date.to_text()), Some(date));
            date = date.add_days(1);
        }
    }

    #[test]
    fn the_epoch_and_its_neighbours_land_where_the_calendar_says() {
        let epoch = CivilDate::new(1970, 1, 1).expect("epoch");
        assert_eq!(epoch.days_from_epoch(), 0);
        assert_eq!(epoch.weekday(), Weekday::Thursday);
        assert_eq!(
            CivilDate::new(1969, 12, 31)
                .expect("day before")
                .days_from_epoch(),
            -1
        );
        assert_eq!(
            CivilDate::new(1969, 12, 31).expect("day before").weekday(),
            Weekday::Wednesday
        );
    }

    #[test]
    fn february_follows_the_gregorian_leap_rule() {
        assert!(CivilDate::new(2024, 2, 29).is_some(), "2024 is a leap year");
        assert!(CivilDate::new(2026, 2, 29).is_none(), "2026 is not");
        assert!(
            CivilDate::new(2000, 2, 29).is_some(),
            "2000 is divisible by 400"
        );
        assert!(CivilDate::new(1900, 2, 29).is_none(), "1900 is not");
        assert_eq!(
            CivilDate::new(2024, 2, 28)
                .expect("leap eve")
                .add_days(1)
                .to_text(),
            "2024-02-29"
        );
        assert_eq!(
            CivilDate::new(2026, 2, 28)
                .expect("common eve")
                .add_days(1)
                .to_text(),
            "2026-03-01"
        );
    }

    #[test]
    fn a_shape_that_is_not_a_calendar_date_is_refused() {
        for text in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-01-00",
            "2026-1-01",
            "20260101",
            "2026-01-01T00:00",
            "",
            "not-a-date",
        ] {
            assert!(CivilDate::parse(text).is_none(), "{text} must not parse");
        }
        assert!(CivilDate::require("2026-02-30").is_err());
    }

    #[test]
    fn stepping_crosses_month_and_year_ends() {
        let year_end = CivilDate::new(2026, 12, 31).expect("year end");
        assert_eq!(year_end.add_days(1).to_text(), "2027-01-01");
        assert_eq!(year_end.add_days(-365).to_text(), "2025-12-31");
        assert_eq!(
            CivilDate::new(2026, 1, 31)
                .expect("january")
                .add_days(1)
                .to_text(),
            "2026-02-01"
        );
        assert_eq!(
            CivilDate::new(2026, 8, 30)
                .expect("today")
                .add_weeks(1)
                .to_text(),
            "2026-09-06"
        );
        assert_eq!(
            CivilDate::new(2026, 8, 30)
                .expect("today")
                .add_weeks(-1)
                .to_text(),
            "2026-08-23"
        );
    }

    #[test]
    fn a_week_starts_on_monday_from_every_day_inside_it() {
        // 2026-08-31 is a Monday; the week it opens closes on 2026-09-06.
        let monday = CivilDate::new(2026, 8, 31).expect("monday");
        assert_eq!(monday.weekday(), Weekday::Monday);
        for offset in 0..7 {
            let day = monday.add_days(offset);
            assert_eq!(day.week_start(), monday, "{day} belongs to this week");
            assert_eq!(day.week_end().to_text(), "2026-09-06");
        }
        assert_eq!(monday.add_days(-1).weekday(), Weekday::Sunday);
        assert_eq!(monday.add_days(-1).week_start().to_text(), "2026-08-24");
    }

    #[test]
    fn weekdays_name_themselves_and_know_the_weekend() {
        let monday = CivilDate::new(2026, 8, 31).expect("monday");
        let names: Vec<&str> = (0..7)
            .map(|n| monday.add_days(n).weekday().as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday"
            ]
        );
        assert!(monday.weekday().is_court_day());
        assert!(!monday.add_days(5).weekday().is_court_day());
        assert!(!monday.add_days(6).weekday().is_court_day());
    }

    #[test]
    fn the_distance_between_two_dates_is_signed() {
        let first = CivilDate::new(2026, 8, 30).expect("first");
        let later = CivilDate::new(2026, 9, 13).expect("later");
        assert_eq!(first.days_until(later), 14);
        assert_eq!(later.days_until(first), -14);
        assert_eq!(first.days_until(first), 0);
    }

    #[test]
    fn a_time_of_day_is_read_only_when_it_names_a_real_minute() {
        assert_eq!(parse_time("09:00"), Some((9, 0)));
        assert_eq!(parse_time("23:59"), Some((23, 59)));
        for text in ["24:00", "09:60", "9:00", "0900", "09:00:00", ""] {
            assert!(parse_time(text).is_none(), "{text} must not parse");
        }
        assert_eq!(require_time("09:05").expect("valid"), "09:05");
        assert!(require_time("25:00").is_err());
    }

    #[test]
    fn the_clock_reads_a_storable_local_date() {
        let connection = rusqlite::Connection::open_in_memory().expect("connection");
        let stamp = today(&connection).expect("today");
        assert!(
            CivilDate::parse(&stamp).is_some(),
            "the clock must return a date the schema accepts: {stamp}"
        );
    }
}
