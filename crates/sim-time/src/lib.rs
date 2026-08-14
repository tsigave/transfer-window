//! Deterministic simulation time, identifiers, and scheduled event ordering.

use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

pub const MICROS_PER_SECOND: i64 = 1_000_000;
pub const SECONDS_PER_DAY: i64 = 86_400;
pub const MICROS_PER_DAY: i64 = SECONDS_PER_DAY * MICROS_PER_SECOND;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TimeError {
    #[error("INVALID_ID: stable id must be 1-64 lowercase ASCII characters in slug form")]
    InvalidId,
    #[error("INVALID_CALENDAR: {0}")]
    InvalidCalendar(&'static str),
    #[error("TIME_OUT_OF_RANGE: calendar conversion overflowed")]
    OutOfRange,
}

/// A persistence-safe identifier. IDs are deliberately stricter than display names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StableId(String);

impl StableId {
    pub fn new(value: impl Into<String>) -> Result<Self, TimeError> {
        let value = value.into();
        let bytes = value.as_bytes();
        let separators = |byte: u8| matches!(byte, b'-' | b'_' | b'.' | b':');
        let valid = !bytes.is_empty()
            && bytes.len() <= 64
            && bytes[0].is_ascii_lowercase()
            && bytes[bytes.len() - 1].is_ascii_alphanumeric()
            && bytes.iter().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || separators(*byte)
            })
            && !bytes
                .windows(2)
                .any(|pair| separators(pair[0]) && separators(pair[1]));
        valid.then_some(Self(value)).ok_or(TimeError::InvalidId)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for StableId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for StableId {
    type Err = TimeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for StableId {
    type Error = TimeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<StableId> for String {
    fn from(value: StableId) -> Self {
        value.0
    }
}

/// Integer microseconds from J2000 TDB (2000-01-01T12:00:00 TDB).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct TdbInstant(i64);

impl TdbInstant {
    pub const J2000: Self = Self(0);

    pub const fn from_micros_since_j2000(micros: i64) -> Self {
        Self(micros)
    }

    pub const fn micros_since_j2000(self) -> i64 {
        self.0
    }

    pub fn from_utc(value: CalendarDateTime) -> Result<Self, TimeError> {
        let linear = utc_linear_micros(value)?;
        let origin = utc_linear_micros(CalendarDateTime::new(2000, 1, 1, 11, 58, 55, 816_000)?)?;
        linear
            .checked_sub(origin)
            .map(Self)
            .ok_or(TimeError::OutOfRange)
    }

    pub fn to_utc(self) -> Result<CalendarDateTime, TimeError> {
        let origin = utc_linear_micros(CalendarDateTime::new(2000, 1, 1, 11, 58, 55, 816_000)?)?;
        let linear = origin.checked_add(self.0).ok_or(TimeError::OutOfRange)?;
        utc_from_linear_micros(linear)
    }

    pub fn checked_add_micros(self, micros: i64) -> Result<Self, TimeError> {
        self.0
            .checked_add(micros)
            .map(Self)
            .ok_or(TimeError::OutOfRange)
    }
}

/// A computation epoch kept near the problem to avoid subtracting unnecessarily large values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEpoch {
    pub origin_tdb: TdbInstant,
}

impl LocalEpoch {
    pub fn offset_seconds(self, instant: TdbInstant) -> f64 {
        (instant.micros_since_j2000() - self.origin_tdb.micros_since_j2000()) as f64
            / MICROS_PER_SECOND as f64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    /// May be 60 on an inserted UTC leap second.
    pub second: u8,
    pub microsecond: u32,
}

impl CalendarDateTime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        microsecond: u32,
    ) -> Result<Self, TimeError> {
        let value = Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
        };
        validate_calendar(value)?;
        Ok(value)
    }

    pub fn iso8601(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.microsecond
        )
    }
}

// UTC dates that ended with 23:59:60. The table is intentionally versioned in code; future
// entries do not change historic timestamps. No leap second has been announced after 2016.
const LEAP_DATES: &[(i32, u8, u8)] = &[
    (1972, 6, 30),
    (1972, 12, 31),
    (1973, 12, 31),
    (1974, 12, 31),
    (1975, 12, 31),
    (1976, 12, 31),
    (1977, 12, 31),
    (1978, 12, 31),
    (1979, 12, 31),
    (1981, 6, 30),
    (1982, 6, 30),
    (1983, 6, 30),
    (1985, 6, 30),
    (1987, 12, 31),
    (1989, 12, 31),
    (1990, 12, 31),
    (1992, 6, 30),
    (1993, 6, 30),
    (1994, 6, 30),
    (1995, 12, 31),
    (1997, 6, 30),
    (1998, 12, 31),
    (2005, 12, 31),
    (2008, 12, 31),
    (2012, 6, 30),
    (2015, 6, 30),
    (2016, 12, 31),
];

fn validate_calendar(value: CalendarDateTime) -> Result<(), TimeError> {
    if !(1..=12).contains(&value.month) {
        return Err(TimeError::InvalidCalendar("month must be 1..12"));
    }
    let max_day = days_in_month(value.year, value.month);
    if value.day == 0 || value.day > max_day {
        return Err(TimeError::InvalidCalendar("day is invalid for month"));
    }
    if value.hour > 23 || value.minute > 59 || value.second > 60 || value.microsecond >= 1_000_000 {
        return Err(TimeError::InvalidCalendar("time component out of range"));
    }
    if value.second == 60
        && (value.hour != 23
            || value.minute != 59
            || !LEAP_DATES.contains(&(value.year, value.month, value.day)))
    {
        return Err(TimeError::InvalidCalendar(
            "second 60 is not valid at this UTC date",
        ));
    }
    Ok(())
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn leap_count_before_day(day_number: i64) -> i64 {
    LEAP_DATES
        .iter()
        .filter(|(year, month, day)| days_from_civil(*year, *month, *day) < day_number)
        .count() as i64
}

fn day_has_leap(day_number: i64) -> bool {
    let date = civil_from_days(day_number);
    LEAP_DATES.contains(&date)
}

fn utc_linear_micros(value: CalendarDateTime) -> Result<i64, TimeError> {
    validate_calendar(value)?;
    let day_number = days_from_civil(value.year, value.month, value.day);
    let seconds = day_number
        .checked_mul(SECONDS_PER_DAY)
        .and_then(|base| base.checked_add(leap_count_before_day(day_number)))
        .and_then(|base| base.checked_add(i64::from(value.hour) * 3_600))
        .and_then(|base| base.checked_add(i64::from(value.minute) * 60))
        .and_then(|base| base.checked_add(i64::from(value.second)))
        .ok_or(TimeError::OutOfRange)?;
    seconds
        .checked_mul(MICROS_PER_SECOND)
        .and_then(|base| base.checked_add(i64::from(value.microsecond)))
        .ok_or(TimeError::OutOfRange)
}

fn utc_from_linear_micros(linear: i64) -> Result<CalendarDateTime, TimeError> {
    let whole_seconds = linear.div_euclid(MICROS_PER_SECOND);
    let microsecond = linear.rem_euclid(MICROS_PER_SECOND) as u32;
    let estimate = whole_seconds.div_euclid(SECONDS_PER_DAY);
    let day_number = ((estimate - 64)..=(estimate + 1))
        .find(|day| {
            let start = *day * SECONDS_PER_DAY + leap_count_before_day(*day);
            let length = SECONDS_PER_DAY + i64::from(day_has_leap(*day));
            whole_seconds >= start && whole_seconds < start + length
        })
        .ok_or(TimeError::OutOfRange)?;
    let start = day_number * SECONDS_PER_DAY + leap_count_before_day(day_number);
    let seconds_of_day = whole_seconds - start;
    let (hour, minute, second) = if seconds_of_day == SECONDS_PER_DAY {
        (23, 59, 60)
    } else {
        (
            seconds_of_day / 3_600,
            (seconds_of_day % 3_600) / 60,
            seconds_of_day % 60,
        )
    };
    let (year, month, day) = civil_from_days(day_number);
    CalendarDateTime::new(
        year,
        month,
        day,
        hour as u8,
        minute as u8,
        second as u8,
        microsecond,
    )
}

// Howard Hinnant's public-domain civil calendar algorithms, with the Unix epoch as day zero.
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let adjusted_year = year - i32::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = i32::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i32::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era * 146_097 + day_of_era - 719_468)
}

fn civil_from_days(day_number: i64) -> (i32, u8, u8) {
    let z = day_number + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u8, day as u8)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    pub id: StableId,
    pub due_time: TdbInstant,
    /// Lower values run first at the same instant.
    pub priority: i32,
    pub kind: StableId,
    pub payload_version: u32,
    pub payload: serde_json::Value,
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.due_time, &self.priority, &self.id).cmp(&(
            &other.due_time,
            &other.priority,
            &other.id,
        ))
    }
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Default)]
pub struct EventQueue(BinaryHeap<Reverse<ScheduledEvent>>);

impl EventQueue {
    pub fn push(&mut self, event: ScheduledEvent) {
        self.0.push(Reverse(event));
    }

    pub fn peek(&self) -> Option<&ScheduledEvent> {
        self.0.peek().map(|value| &value.0)
    }

    pub fn pop_due(&mut self, time: TdbInstant) -> Option<ScheduledEvent> {
        if self.peek().is_some_and(|event| event.due_time <= time) {
            self.0.pop().map(|value| value.0)
        } else {
            None
        }
    }

    pub fn ordered(&self) -> Vec<ScheduledEvent> {
        let mut values = self.0.clone().into_sorted_vec();
        values.reverse();
        values.into_iter().map(|value| value.0).collect()
    }

    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&ScheduledEvent) -> bool,
    {
        self.0 = self.0.drain().filter(|event| keep(&event.0)).collect();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<ScheduledEvent>> for EventQueue {
    fn from(events: Vec<ScheduledEvent>) -> Self {
        Self(events.into_iter().map(Reverse).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_reject_invalid_forms() {
        for invalid in ["", "Earth", "two words", "-earth", "earth-", "earth--moon"] {
            assert!(StableId::new(invalid).is_err(), "accepted {invalid}");
        }
        assert_eq!(StableId::new("body:earth").unwrap().as_str(), "body:earth");
    }

    #[test]
    fn j2000_and_century_calendar_round_trip() {
        let j2000_utc = CalendarDateTime::new(2000, 1, 1, 11, 58, 55, 816_000).unwrap();
        assert_eq!(TdbInstant::from_utc(j2000_utc).unwrap(), TdbInstant::J2000);
        for year in [1900, 2000, 2100, 2200] {
            let utc = CalendarDateTime::new(year, 7, 3, 18, 2, 1, 123_456).unwrap();
            assert_eq!(TdbInstant::from_utc(utc).unwrap().to_utc().unwrap(), utc);
        }
    }

    #[test]
    fn leap_second_is_displayed_and_round_trips() {
        let before = CalendarDateTime::new(2016, 12, 31, 23, 59, 59, 0).unwrap();
        let leap = CalendarDateTime::new(2016, 12, 31, 23, 59, 60, 0).unwrap();
        let after = CalendarDateTime::new(2017, 1, 1, 0, 0, 0, 0).unwrap();
        let before_tdb = TdbInstant::from_utc(before).unwrap();
        let leap_tdb = TdbInstant::from_utc(leap).unwrap();
        let after_tdb = TdbInstant::from_utc(after).unwrap();
        assert_eq!(
            leap_tdb.micros_since_j2000() - before_tdb.micros_since_j2000(),
            MICROS_PER_SECOND
        );
        assert_eq!(
            after_tdb.micros_since_j2000() - leap_tdb.micros_since_j2000(),
            MICROS_PER_SECOND
        );
        assert_eq!(leap_tdb.to_utc().unwrap(), leap);
    }

    #[test]
    fn same_time_events_sort_by_priority_then_id() {
        let make = |id: &str, priority| ScheduledEvent {
            id: StableId::new(id).unwrap(),
            due_time: TdbInstant::J2000,
            priority,
            kind: StableId::new("test").unwrap(),
            payload_version: 1,
            payload: serde_json::Value::Null,
        };
        let mut queue = EventQueue::default();
        queue.push(make("event:b", 10));
        queue.push(make("event:c", 0));
        queue.push(make("event:a", 10));
        let ids: Vec<_> = std::iter::from_fn(|| queue.pop_due(TdbInstant::J2000))
            .map(|event| event.id.to_string())
            .collect();
        assert_eq!(ids, ["event:c", "event:a", "event:b"]);
    }
}
