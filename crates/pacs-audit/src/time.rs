//! Timestamps as RFC 3339 in UTC, to the millisecond: `xsd:dateTime` with its time zone
//! (A.5.2.5) and the TIMESTAMP of RFC 5424.

use std::time::{SystemTime, UNIX_EPOCH};

/// `2026-11-15T20:04:05.123Z`. A time before 1970 is written as 1970-01-01T00:00:00.000Z.
pub fn rfc3339(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (seconds, millis) = (since.as_secs(), since.subsec_millis());
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// The date `days` after 1970-01-01 in the proleptic Gregorian calendar (Howard Hinnant's
/// `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64, millis: u32) -> String {
        rfc3339(UNIX_EPOCH + Duration::new(seconds, millis * 1_000_000))
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(at(86_399, 999), "1970-01-01T23:59:59.999Z");
        // a leap day, and the days around the end of a leap year
        assert_eq!(at(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert_eq!(at(1_709_164_800, 0), "2024-02-29T00:00:00.000Z");
        assert_eq!(at(1_735_689_599, 0), "2024-12-31T23:59:59.000Z");
        assert_eq!(at(1_735_689_600, 0), "2025-01-01T00:00:00.000Z");
        // 2100 is not a leap year
        assert_eq!(at(4_107_542_400, 0), "2100-03-01T00:00:00.000Z");
        assert_eq!(at(1_794_773_045, 123), "2026-11-15T20:04:05.123Z");
        // before the epoch there is nothing to report
        assert_eq!(
            rfc3339(UNIX_EPOCH - Duration::from_secs(1)),
            "1970-01-01T00:00:00.000Z"
        );
    }
}
