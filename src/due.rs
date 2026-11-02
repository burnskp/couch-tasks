use chrono::{
    DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeDelta,
    TimeZone, Utc,
};

use crate::{config::DueFormat, error::AppError, output::escape_terminal};

/// Local time of day used when a due date is given without a time.
const DEFAULT_DUE_TIME: (u32, u32) = (9, 0);
const DUE_FORMATS: &str =
    "expected YYYY-MM-DD, YY-MM-DD, MM-DD, DD, or <n>w<n>d, optionally followed by HH:MM";

/// Parse user input into a UTC due timestamp, interpreting dates and times in `now`'s zone.
pub fn parse_due<Tz: TimeZone>(input: &str, now: &DateTime<Tz>) -> Result<String, AppError> {
    let invalid = || {
        AppError::User(format!(
            "Invalid due date: {} ({DUE_FORMATS})",
            escape_terminal(input)
        ))
    };

    let mut tokens = input.split_whitespace();
    let date_token = tokens.next().ok_or_else(invalid)?;
    let time = match tokens.next() {
        Some(token) => parse_time(token).ok_or_else(invalid)?,
        None => NaiveTime::from_hms_opt(DEFAULT_DUE_TIME.0, DEFAULT_DUE_TIME.1, 0)
            .expect("default due time is valid"),
    };
    if tokens.next().is_some() {
        return Err(invalid());
    }

    let today = now.date_naive();
    let date = parse_relative_date(date_token, today)
        .or_else(|| parse_calendar_date(date_token, today))
        .ok_or_else(invalid)?;

    let local = date.and_time(time);
    let due = match now.timezone().from_local_datetime(&local) {
        LocalResult::Single(due) => due,
        LocalResult::Ambiguous(earliest, _) => earliest,
        LocalResult::None => {
            return Err(AppError::User(format!(
                "Due time does not exist in local time: {}",
                local.format("%Y-%m-%d %H:%M")
            )));
        }
    };
    Ok(format_due_timestamp(due.with_timezone(&Utc)))
}

/// True when `input` looks like the optional time part of a due date.
pub fn is_due_time(input: &str) -> bool {
    parse_time(input).is_some()
}

/// Canonical storage form: RFC 3339 UTC with second precision.
pub fn format_due_timestamp(due: DateTime<Utc>) -> String {
    due.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parse a stored due timestamp, accepting only the canonical storage form.
pub fn parse_due_timestamp(value: &str) -> Option<DateTime<Utc>> {
    let due = DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&Utc);
    (format_due_timestamp(due) == value).then_some(due)
}

/// How due dates are rendered for one command invocation.
#[derive(Debug, Clone, Copy)]
pub struct DueDisplay {
    pub format: DueFormat,
    pub now: DateTime<Utc>,
    pub to_local: fn(DateTime<Utc>) -> NaiveDateTime,
}

impl DueDisplay {
    pub fn local(format: DueFormat) -> Self {
        Self {
            format,
            now: Utc::now(),
            to_local: |due| due.with_timezone(&chrono::Local).naive_local(),
        }
    }

    /// Due text in the configured format; overdue values are prefixed with `!`.
    pub fn format(&self, due_at: &str, overdue_applies: bool) -> String {
        let Some(due) = parse_due_timestamp(due_at) else {
            return escape_terminal(due_at);
        };
        let marker = if overdue_applies && due < self.now {
            "!"
        } else {
            ""
        };
        match self.format {
            DueFormat::Absolute => format!("{marker}{}", self.absolute_time(due)),
            DueFormat::Relative => format!("{marker}{}", format_duration(due - self.now)),
        }
    }

    /// Absolute local time with the relative time in parentheses.
    pub fn format_both(&self, due_at: &str, overdue_applies: bool) -> String {
        let Some(due) = parse_due_timestamp(due_at) else {
            return escape_terminal(due_at);
        };
        let marker = if overdue_applies && due < self.now {
            "!"
        } else {
            ""
        };
        format!(
            "{} ({marker}{})",
            self.absolute_time(due),
            format_duration(due - self.now)
        )
    }

    pub fn is_overdue(&self, due_at: &str) -> bool {
        parse_due_timestamp(due_at).is_some_and(|due| due < self.now)
    }

    /// Local `YYYY-MM-DD HH:MM`, as shown in lists and accepted by the parser.
    pub fn absolute(&self, due_at: &str) -> String {
        parse_due_timestamp(due_at)
            .map(|due| self.absolute_time(due))
            .unwrap_or_else(|| escape_terminal(due_at))
    }

    fn absolute_time(&self, due: DateTime<Utc>) -> String {
        (self.to_local)(due).format("%Y-%m-%d %H:%M").to_string()
    }
}

/// Compact relative duration, rounded down:
/// weeks and days from 7 days, days from 3 days, days and hours from 1 day,
/// otherwise hours and minutes.
fn format_duration(delta: TimeDelta) -> String {
    let minutes = delta.num_minutes().unsigned_abs();
    let days = minutes / (24 * 60);
    let hours = minutes / 60 % 24;
    let minutes = minutes % 60;

    let parts: [(u64, &str); 2] = if days >= 7 {
        [(days / 7, "w"), (days % 7, "d")]
    } else if days >= 3 {
        [(days, "d"), (0, "")]
    } else if days >= 1 {
        [(days, "d"), (hours, "h")]
    } else {
        [(hours, "h"), (minutes, "m")]
    };

    let output = parts
        .iter()
        .filter(|(value, _)| *value > 0)
        .map(|(value, unit)| format!("{value}{unit}"))
        .collect::<String>();
    if output.is_empty() {
        "0m".to_string()
    } else {
        output
    }
}

fn parse_time(token: &str) -> Option<NaiveTime> {
    let (hour, minute) = token.split_once(':')?;
    if !(1..=2).contains(&hour.len()) || minute.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(parse_digits(hour)?, parse_digits(minute)?, 0)
}

/// `<n>w`, `<n>d`, or `<n>w<n>d`, counted in calendar days from today.
fn parse_relative_date(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    let token = token.to_ascii_lowercase();
    let (weeks, rest) = match token.split_once('w') {
        Some((weeks, rest)) => (Some(parse_digits(weeks)?), rest),
        None => (None, token.as_str()),
    };
    let days = match rest {
        "" => None,
        rest => Some(parse_digits(rest.strip_suffix('d')?)?),
    };
    if weeks.is_none() && days.is_none() {
        return None;
    }

    let total = u64::from(weeks.unwrap_or(0))
        .checked_mul(7)?
        .checked_add(u64::from(days.unwrap_or(0)))?;
    today.checked_add_days(chrono::Days::new(total))
}

fn parse_calendar_date(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    let parts = token.split('-').collect::<Vec<_>>();
    let (year, month, day) = match parts.as_slice() {
        [year, month, day] => {
            let year = match year.len() {
                4 => i32::try_from(parse_digits(year)?).ok()?,
                2 => 2000 + i32::try_from(parse_digits(year)?).ok()?,
                _ => return None,
            };
            (year, parse_day_or_month(month)?, parse_day_or_month(day)?)
        }
        [month, day] => (
            today.year(),
            parse_day_or_month(month)?,
            parse_day_or_month(day)?,
        ),
        [day] => (today.year(), today.month(), parse_day_or_month(day)?),
        _ => return None,
    };
    NaiveDate::from_ymd_opt(year, month, day)
}

fn parse_day_or_month(value: &str) -> Option<u32> {
    if (1..=2).contains(&value.len()) {
        parse_digits(value)
    } else {
        None
    }
}

fn parse_digits(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveDate, Timelike};

    use super::*;

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::west_opt(5 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 11, 2, 15, 30, 0)
            .unwrap()
    }

    fn parse(input: &str) -> Result<String, AppError> {
        parse_due(input, &now())
    }

    #[test]
    fn parses_calendar_formats_at_nine_local() {
        assert_eq!(parse("2026-11-15").unwrap(), "2026-11-15T14:00:00Z");
        assert_eq!(parse("27-01-05").unwrap(), "2027-01-05T14:00:00Z");
        assert_eq!(parse("12-25").unwrap(), "2026-12-25T14:00:00Z");
        assert_eq!(parse("1-5").unwrap(), "2026-01-05T14:00:00Z");
        assert_eq!(parse("15").unwrap(), "2026-11-15T14:00:00Z");
        assert_eq!(parse("1").unwrap(), "2026-11-01T14:00:00Z");
    }

    #[test]
    fn parses_relative_formats_as_calendar_days() {
        assert_eq!(parse("4d").unwrap(), "2026-11-06T14:00:00Z");
        assert_eq!(parse("1w").unwrap(), "2026-11-09T14:00:00Z");
        assert_eq!(parse("2W3D").unwrap(), "2026-11-19T14:00:00Z");
        assert_eq!(parse("0d").unwrap(), "2026-11-02T14:00:00Z");
    }

    #[test]
    fn parses_optional_time() {
        assert_eq!(parse("15 14:30").unwrap(), "2026-11-15T19:30:00Z");
        assert_eq!(parse("2w 8:05").unwrap(), "2026-11-16T13:05:00Z");
        assert_eq!(parse("2026-11-15 23:59").unwrap(), "2026-11-16T04:59:00Z");
    }

    #[test]
    fn rejects_invalid_input() {
        for input in [
            "",
            "3d2w",
            "2w3",
            "w",
            "d",
            "11/15",
            "2-30",
            "31",
            "026-11-15",
            "2026-1-123",
            "15 24:00",
            "15 9:5",
            "15 2pm",
            "15 14:30 extra",
            "-1d",
            "today",
            "+3d",
        ] {
            let error = parse(input).unwrap_err().to_string();
            assert!(error.contains("Invalid due date"), "{input}: {error}");
        }
    }

    #[test]
    fn rejects_nonexistent_local_time_and_picks_earlier_ambiguous_time() {
        #[derive(Clone, Copy, Debug)]
        struct Shifting;
        // UTC-4 before 2026-11-01 02:00, UTC-5 after; spring-forward modelled on 2026-03-08.
        impl TimeZone for Shifting {
            type Offset = FixedOffset;
            fn from_offset(_: &FixedOffset) -> Self {
                Shifting
            }
            fn offset_from_local_date(&self, _: &NaiveDate) -> LocalResult<FixedOffset> {
                unimplemented!()
            }
            fn offset_from_local_datetime(
                &self,
                local: &NaiveDateTime,
            ) -> LocalResult<FixedOffset> {
                let edt = FixedOffset::west_opt(4 * 3600).unwrap();
                let est = FixedOffset::west_opt(5 * 3600).unwrap();
                let fall = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
                let spring = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
                if local.date() == fall && local.hour() == 1 {
                    LocalResult::Ambiguous(edt, est)
                } else if local.date() == spring && local.hour() == 2 {
                    LocalResult::None
                } else {
                    LocalResult::Single(est)
                }
            }
            fn offset_from_utc_date(&self, _: &NaiveDate) -> FixedOffset {
                unimplemented!()
            }
            fn offset_from_utc_datetime(&self, _: &NaiveDateTime) -> FixedOffset {
                FixedOffset::west_opt(5 * 3600).unwrap()
            }
        }

        let now = Shifting.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        assert_eq!(
            parse_due("2026-11-01 01:30", &now).unwrap(),
            "2026-11-01T05:30:00Z"
        );
        let error = parse_due("2026-03-08 02:30", &now).unwrap_err().to_string();
        assert!(error.contains("does not exist"), "{error}");
    }

    #[test]
    fn stored_timestamps_must_be_canonical() {
        assert!(parse_due_timestamp("2026-11-15T14:00:00Z").is_some());
        assert!(parse_due_timestamp("2026-11-15T09:00:00-05:00").is_none());
        assert!(parse_due_timestamp("2026-11-15T14:00:00.000Z").is_none());
        assert!(parse_due_timestamp("tomorrow").is_none());
    }

    fn display(format: DueFormat) -> DueDisplay {
        DueDisplay {
            format,
            now: Utc.with_ymd_and_hms(2026, 11, 2, 12, 0, 0).unwrap(),
            to_local: |due| due.naive_utc(),
        }
    }

    fn relative(due_at: &str) -> String {
        display(DueFormat::Relative).format(due_at, true)
    }

    #[test]
    fn relative_format_uses_bands_and_rounds_down() {
        assert_eq!(relative("2026-11-19T15:00:00Z"), "2w3d");
        assert_eq!(relative("2026-11-16T12:00:00Z"), "2w");
        assert_eq!(relative("2026-11-09T11:59:00Z"), "6d");
        assert_eq!(relative("2026-11-06T23:00:00Z"), "4d");
        assert_eq!(relative("2026-11-05T11:00:00Z"), "2d23h");
        assert_eq!(relative("2026-11-03T12:30:00Z"), "1d");
        assert_eq!(relative("2026-11-03T05:30:00Z"), "17h30m");
        assert_eq!(relative("2026-11-02T15:00:00Z"), "3h");
        assert_eq!(relative("2026-11-02T12:45:00Z"), "45m");
        assert_eq!(relative("2026-11-02T12:00:00Z"), "0m");
    }

    #[test]
    fn overdue_values_are_marked() {
        assert_eq!(relative("2026-10-31T07:00:00Z"), "!2d5h");
        assert_eq!(relative("2026-11-02T11:59:30Z"), "!0m");
        assert_eq!(
            display(DueFormat::Absolute).format("2026-10-30T09:00:00Z", true),
            "!2026-10-30 09:00"
        );
        assert_eq!(
            display(DueFormat::Absolute).format("2026-10-30T09:00:00Z", false),
            "2026-10-30 09:00"
        );
        assert_eq!(
            display(DueFormat::Relative).format_both("2026-11-04T15:30:00Z", true),
            "2026-11-04 15:30 (2d3h)"
        );
        assert_eq!(
            display(DueFormat::Absolute).format_both("2026-11-01T12:00:00Z", true),
            "2026-11-01 12:00 (!1d)"
        );
    }
}
