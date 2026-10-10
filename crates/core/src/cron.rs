//! When a CronJob runs next. Follows the schedule syntax Kubernetes accepts:
//! five fields (minute hour day-of-month month day-of-week) with `*`, `?`,
//! ranges, steps, lists and month/day names, plus the `@hourly`-style
//! shorthands.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// Each field as a bit set of the values it matches.
struct Schedule {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    /// Day-of-month and day-of-week match together (AND) when either is `*`,
    /// and either one (OR) when both are restricted, as in Vixie cron.
    any_day: bool,
}

/// The first run after `after`, in `time_zone` (an IANA name; UTC when unset,
/// the usual controller default). `None` if the schedule doesn't parse or
/// never fires within five years.
pub fn next_run(schedule: &str, time_zone: Option<&str>, after: Timestamp) -> Option<Timestamp> {
    let s = parse(schedule)?;
    let tz = match time_zone {
        Some(name) => TimeZone::get(name).ok()?,
        None => TimeZone::UTC,
    };
    let mut date = after.to_zoned(tz.clone()).date();
    for _ in 0..5 * 366 {
        if s.matches_day(date) {
            for hour in (0..24).filter(|h| bit(s.hours, *h)) {
                for minute in (0..60).filter(|m| bit(s.minutes, *m)) {
                    let at = date.at(hour as i8, minute as i8, 0, 0).to_zoned(tz.clone()).ok()?.timestamp();
                    if at > after {
                        return Some(at);
                    }
                }
            }
        }
        date = date.tomorrow().ok()?;
    }
    None
}

impl Schedule {
    fn matches_day(&self, date: Date) -> bool {
        let day = bit(self.days, date.day() as u32);
        let weekday = bit(self.weekdays, date.weekday().to_sunday_zero_offset() as u32);
        bit(self.months, date.month() as u32) && if self.any_day { day && weekday } else { day || weekday }
    }
}

fn bit(set: u64, n: u32) -> bool {
    set & (1 << n) != 0
}

fn parse(schedule: &str) -> Option<Schedule> {
    let schedule = match schedule.trim() {
        "@yearly" | "@annually" => "0 0 1 1 *",
        "@monthly" => "0 0 1 * *",
        "@weekly" => "0 0 * * 0",
        "@daily" | "@midnight" => "0 0 * * *",
        "@hourly" => "0 * * * *",
        s => s,
    };
    let [minute, hour, day, month, weekday] = schedule.split_whitespace().collect::<Vec<_>>().try_into().ok()?;
    let star = |f: &str| f.starts_with('*') || f.starts_with('?');
    let mut weekdays = field(weekday, 0, 7, &DAYS, 0)?;
    // 7 is Sunday too.
    if bit(weekdays, 7) {
        weekdays = (weekdays & !(1 << 7)) | 1;
    }
    Some(Schedule {
        minutes: field(minute, 0, 59, &[], 0)?,
        hours: field(hour, 0, 23, &[], 0)?,
        days: field(day, 1, 31, &[], 0)?,
        months: field(month, 1, 12, &MONTHS, 1)?,
        weekdays,
        any_day: star(day) || star(weekday),
    })
}

/// One field: comma-separated `*`, `n`, `a-b`, each optionally `/step`.
/// `names[i]` stands for `i + first`.
fn field(f: &str, min: u32, max: u32, names: &[&str], first: u32) -> Option<u64> {
    let value = |v: &str| -> Option<u32> {
        let lower = v.to_ascii_lowercase();
        match names.iter().position(|n| *n == lower) {
            Some(i) => Some(i as u32 + first),
            None => v.parse().ok(),
        }
    };
    let mut set = 0u64;
    for part in f.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, Some(s.parse::<u32>().ok().filter(|s| *s > 0)?)),
            None => (part, None),
        };
        let (lo, hi) = match range {
            "*" | "?" => (min, max),
            r => match r.split_once('-') {
                Some((a, b)) => (value(a)?, value(b)?),
                // `5/15` means from 5 to the end, every 15.
                None => value(r).map(|v| (v, if step.is_some() { max } else { v }))?,
            },
        };
        if lo < min || hi > max || lo > hi {
            return None;
        }
        for v in (lo..=hi).step_by(step.unwrap_or(1) as usize) {
            set |= 1 << v;
        }
    }
    Some(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(schedule: &str, tz: Option<&str>, after: &str) -> Option<String> {
        next_run(schedule, tz, after.parse().unwrap()).map(|t| t.to_string())
    }

    #[test]
    fn steps_ranges_and_shorthands() {
        assert_eq!(next("*/15 * * * *", None, "2026-10-10T12:07:30Z").unwrap(), "2026-10-10T12:15:00Z");
        assert_eq!(next("*/15 * * * *", None, "2026-10-10T12:15:00Z").unwrap(), "2026-10-10T12:30:00Z");
        assert_eq!(next("@daily", None, "2026-10-10T12:00:00Z").unwrap(), "2026-10-11T00:00:00Z");
        assert_eq!(next("@hourly", None, "2026-10-10T12:00:00Z").unwrap(), "2026-10-10T13:00:00Z");
        // 2026-10-10 is a Saturday.
        assert_eq!(next("30 3 * * MON-FRI", None, "2026-10-10T12:00:00Z").unwrap(), "2026-10-12T03:30:00Z");
        assert_eq!(next("0 9 * * 7", None, "2026-10-10T12:00:00Z").unwrap(), "2026-10-11T09:00:00Z");
        assert_eq!(next("5/20 1,2 * * *", None, "2026-10-10T01:30:00Z").unwrap(), "2026-10-10T01:45:00Z");
        assert_eq!(next("0 0 29 feb *", None, "2026-10-10T00:00:00Z").unwrap(), "2028-02-29T00:00:00Z");
    }

    #[test]
    fn day_fields_or_when_both_set() {
        // The 13th, or any Friday: Friday 2026-10-16 comes before the 13th of November.
        assert_eq!(next("0 0 13 * 5", None, "2026-10-14T00:00:00Z").unwrap(), "2026-10-16T00:00:00Z");
        // With a star, both must match: Fridays only.
        assert_eq!(next("0 0 * * 5", None, "2026-10-14T00:00:00Z").unwrap(), "2026-10-16T00:00:00Z");
        assert_eq!(next("0 0 13 * *", None, "2026-10-14T00:00:00Z").unwrap(), "2026-11-13T00:00:00Z");
    }

    #[test]
    fn time_zones_and_bad_input() {
        // 09:00 in New York is 13:00 UTC in October (EDT).
        assert_eq!(
            next("0 9 * * *", Some("America/New_York"), "2026-10-10T14:00:00Z").unwrap(),
            "2026-10-11T13:00:00Z"
        );
        assert_eq!(next("61 * * * *", None, "2026-10-10T00:00:00Z"), None);
        assert_eq!(next("* * *", None, "2026-10-10T00:00:00Z"), None);
        assert_eq!(next("0 0 31 2 *", None, "2026-10-10T00:00:00Z"), None);
        assert_eq!(next("0 0 * * *", Some("Not/AZone"), "2026-10-10T00:00:00Z"), None);
    }
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod coverage_tests;
