//! Dates shifted by whole days, for the Retain Longitudinal Temporal Information With
//! Modified Dates Option (PS3.15 E.3.6): intervals between a patient's dates survive, and
//! times are kept, since a whole-day shift does not move them.

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse a DA value, `YYYYMMDD` (PS3.5 Table 6.2-1).
pub fn parse(da: &str) -> Option<(i64, i64, i64)> {
    if da.len() != 8 || !da.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (y, m, d) = (
        da[..4].parse().ok()?,
        da[4..6].parse().ok()?,
        da[6..].parse().ok()?,
    );
    let days = days_from_civil(y, m, d);
    (civil_from_days(days) == (y, m, d)).then_some((y, m, d))
}

/// A DA value moved by `days`, or `None` when it is not a whole date.
pub fn shift_da(da: &str, days: i32) -> Option<String> {
    let (y, m, d) = parse(da.trim())?;
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) + days as i64);
    (0..=9999)
        .contains(&y)
        .then(|| format!("{y:04}{m:02}{d:02}"))
}

/// A DT value with its date part moved by `days`; time and offset kept. `None` for a DT
/// without a whole date (`YYYY`, `YYYYMM`), which a day shift cannot move.
pub fn shift_dt(dt: &str, days: i32) -> Option<String> {
    let dt = dt.trim();
    if dt.len() < 8 || !dt.is_char_boundary(8) {
        return None;
    }
    Some(format!("{}{}", shift_da(&dt[..8], days)?, &dt[8..]))
}

/// Whole years between two DA values, for Patient's Age (PS3.15 E.1.1 notes that it may
/// replace Patient's Birth Date).
pub fn age_years(birth: &str, on: &str) -> Option<u32> {
    let (by, bm, bd) = parse(birth.trim())?;
    let (y, m, d) = parse(on.trim())?;
    let mut years = y - by;
    if (m, d) < (bm, bd) {
        years -= 1;
    }
    u32::try_from(years).ok()
}
