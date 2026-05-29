//! Date/time serial-number system.
//!
//! Spreadsheets store dates as numbers: the integer part is a day count and the
//! fractional part is the time of day. This module converts between those
//! serials and broken-down `(year, month, day, hour, minute, second)` values,
//! using the 1900 date system with epoch **1899-12-30** so that
//! 1900-01-01 = 1, 2000-01-01 = 36526, etc.
//!
//! Caveat: Excel's 1900 system contains a deliberate leap-year bug (it treats
//! 1900 as a leap year). We use the proleptic Gregorian calendar, so serials
//! for dates **before 1900-03-01** differ from Excel by one. Modern dates match
//! exactly.

/// A broken-down date and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Days from the civil date 1899-12-30 (the serial epoch) to 1970-01-01, used
/// to anchor the Hinnant algorithms.
const EPOCH_OFFSET: i64 = 25_569; // days between 1899-12-30 and 1970-01-01

/// `days_from_civil` per Howard Hinnant: days since 1970-01-01 (can be negative).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Whole-day serial for a calendar date.
pub fn date_to_serial(year: i32, month: u32, day: u32) -> i64 {
    days_from_civil(year, month, day) + EPOCH_OFFSET
}

/// Serial (with time-of-day fraction) for a full date-time.
pub fn datetime_to_serial(dt: DateTime) -> f64 {
    let days = date_to_serial(dt.year, dt.month, dt.day) as f64;
    let secs = dt.hour * 3600 + dt.minute * 60 + dt.second;
    days + secs as f64 / 86_400.0
}

/// Break a serial into its date and time components.
pub fn serial_to_datetime(serial: f64) -> DateTime {
    let mut day_count = serial.floor() as i64;
    let frac = serial - serial.floor();

    // Round to whole seconds, carrying into the day if it rounds up to 24:00.
    let mut total_secs = (frac * 86_400.0).round() as i64;
    if total_secs >= 86_400 {
        total_secs -= 86_400;
        day_count += 1;
    }

    let (year, month, day) = civil_from_days(day_count - EPOCH_OFFSET);
    let hour = (total_secs / 3600) as u32;
    let minute = ((total_secs % 3600) / 60) as u32;
    let second = (total_secs % 60) as u32;

    DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// Day of week for a serial: 0 = Sunday … 6 = Saturday (Gregorian).
pub fn weekday(serial: f64) -> u32 {
    let days = serial.floor() as i64 - EPOCH_OFFSET; // days since 1970-01-01
                                                     // 1970-01-01 was a Thursday (=4 with Sunday=0).
    (((days % 7) + 7 + 4) % 7) as u32
}

const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAYS_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// One token of a date/time format code.
enum Tok {
    /// A run of the same date letter (lowercased) with its length.
    Letter(char, usize),
    /// `AM/PM` (or `A/P`) marker.
    AmPm,
    /// A literal run.
    Lit(String),
}

/// Render a serial through a date/time format code (e.g. `yyyy-mm-dd hh:mm`).
pub fn format_serial(serial: f64, code: &str) -> String {
    let dt = serial_to_datetime(serial);
    let wd = weekday(serial) as usize;
    let toks = tokenize_date(code);
    let has_ampm = toks.iter().any(|t| matches!(t, Tok::AmPm));

    let mut out = String::new();
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Lit(s) => out.push_str(s),
            Tok::AmPm => out.push_str(if dt.hour < 12 { "AM" } else { "PM" }),
            Tok::Letter(ch, count) => {
                let rendered = render_letter(*ch, *count, &dt, wd, has_ampm, &toks, i);
                out.push_str(&rendered);
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_letter(
    ch: char,
    count: usize,
    dt: &DateTime,
    wd: usize,
    has_ampm: bool,
    toks: &[Tok],
    idx: usize,
) -> String {
    match ch {
        'y' => {
            if count <= 2 {
                format!("{:02}", (dt.year % 100).unsigned_abs())
            } else {
                format!("{:04}", dt.year)
            }
        }
        'd' => match count {
            1 => dt.day.to_string(),
            2 => format!("{:02}", dt.day),
            3 => WEEKDAYS_SHORT[wd].to_string(),
            _ => WEEKDAYS_FULL[wd].to_string(),
        },
        'h' => {
            let h = if has_ampm {
                let h12 = dt.hour % 12;
                if h12 == 0 {
                    12
                } else {
                    h12
                }
            } else {
                dt.hour
            };
            if count >= 2 {
                format!("{h:02}")
            } else {
                h.to_string()
            }
        }
        's' => {
            if count >= 2 {
                format!("{:02}", dt.second)
            } else {
                dt.second.to_string()
            }
        }
        'm' => {
            if is_minute_context(toks, idx) {
                if count >= 2 {
                    format!("{:02}", dt.minute)
                } else {
                    dt.minute.to_string()
                }
            } else {
                let mi = (dt.month as usize - 1).min(11);
                match count {
                    1 => dt.month.to_string(),
                    2 => format!("{:02}", dt.month),
                    3 => MONTHS_SHORT[mi].to_string(),
                    _ => MONTHS_FULL[mi].to_string(),
                }
            }
        }
        _ => String::new(),
    }
}

/// `m`/`mm` is minutes when adjacent (ignoring literals) to an hour token before
/// it or a seconds token after it; otherwise it's a month.
fn is_minute_context(toks: &[Tok], idx: usize) -> bool {
    let prev_letter = toks[..idx].iter().rev().find_map(|t| {
        if let Tok::Letter(c, _) = t {
            Some(*c)
        } else {
            None
        }
    });
    let next_letter = toks[idx + 1..].iter().find_map(|t| {
        if let Tok::Letter(c, _) = t {
            Some(*c)
        } else {
            None
        }
    });
    prev_letter == Some('h') || next_letter == Some('s')
}

fn tokenize_date(code: &str) -> Vec<Tok> {
    let chars: Vec<char> = code.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let lower = c.to_ascii_lowercase();
        if matches!(lower, 'y' | 'd' | 'h' | 's' | 'm') {
            let mut n = 0;
            while i < chars.len() && chars[i].to_ascii_lowercase() == lower {
                n += 1;
                i += 1;
            }
            toks.push(Tok::Letter(lower, n));
        } else if matches_ci(&chars, i, "AM/PM") {
            toks.push(Tok::AmPm);
            i += 5;
        } else if matches_ci(&chars, i, "A/P") {
            toks.push(Tok::AmPm);
            i += 3;
        } else if c == '"' {
            i += 1;
            let mut lit = String::new();
            while i < chars.len() && chars[i] != '"' {
                lit.push(chars[i]);
                i += 1;
            }
            i += 1; // closing quote
            toks.push(Tok::Lit(lit));
        } else if c == '\\' {
            i += 1;
            if i < chars.len() {
                toks.push(Tok::Lit(chars[i].to_string()));
                i += 1;
            }
        } else {
            toks.push(Tok::Lit(c.to_string()));
            i += 1;
        }
    }
    toks
}

fn matches_ci(chars: &[char], at: usize, pat: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    if at + pat.len() > chars.len() {
        return false;
    }
    chars[at..at + pat.len()]
        .iter()
        .zip(&pat)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime {
        DateTime {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: s,
        }
    }

    #[test]
    fn known_serials() {
        // From 1900-03-01 onward our serials match Excel exactly (61 == Excel's
        // value, which includes its phantom 1900-02-29 at serial 60).
        assert_eq!(date_to_serial(1900, 3, 1), 61);
        assert_eq!(date_to_serial(2000, 1, 1), 36_526);
        assert_eq!(date_to_serial(2008, 12, 31), 39_813);
    }

    #[test]
    fn roundtrip_date_and_time() {
        let original = dt(2024, 5, 29, 13, 30, 0);
        let serial = datetime_to_serial(original);
        let back = serial_to_datetime(serial);
        assert_eq!(back, original);
    }

    #[test]
    fn time_fraction() {
        // Noon is exactly half a day.
        let serial = datetime_to_serial(dt(2020, 1, 1, 12, 0, 0));
        assert_eq!(serial.fract(), 0.5);
        let midmorning = serial_to_datetime(date_to_serial(2020, 1, 1) as f64 + 0.25);
        assert_eq!((midmorning.hour, midmorning.minute), (6, 0));
    }

    #[test]
    fn weekday_is_correct() {
        // 2024-05-29 was a Wednesday (=3).
        assert_eq!(weekday(date_to_serial(2024, 5, 29) as f64), 3);
        // 2000-01-01 was a Saturday (=6).
        assert_eq!(weekday(date_to_serial(2000, 1, 1) as f64), 6);
    }

    #[test]
    fn formats_dates() {
        let serial = datetime_to_serial(dt(2024, 5, 29, 0, 0, 0));
        assert_eq!(format_serial(serial, "yyyy-mm-dd"), "2024-05-29");
        assert_eq!(format_serial(serial, "m/d/yy"), "5/29/24");
        assert_eq!(
            format_serial(serial, "dddd, mmmm d, yyyy"),
            "Wednesday, May 29, 2024"
        );
        assert_eq!(format_serial(serial, "mmm dd"), "May 29");
    }

    #[test]
    fn formats_times_and_minute_vs_month() {
        let serial = datetime_to_serial(dt(2024, 1, 2, 14, 5, 9));
        // 24-hour with minutes (m after h) and seconds.
        assert_eq!(format_serial(serial, "hh:mm:ss"), "14:05:09");
        // 12-hour with AM/PM.
        assert_eq!(format_serial(serial, "h:mm AM/PM"), "2:05 PM");
        // Same 'mm' is a month here (next to date letters, not h/s).
        assert_eq!(format_serial(serial, "mm/dd"), "01/02");
    }
}
