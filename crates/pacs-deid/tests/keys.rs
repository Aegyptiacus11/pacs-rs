//! Replacement values from a project secret, and date shifts by the calendar.

use pacs_deid::Keys;

fn keys() -> Keys {
    Keys::new(*b"a project secret of 32 bytes ok!").unwrap()
}

/// Keys: the same secret gives the same values, another secret others; a UID is a valid
/// UUID-derived UID; a short secret is refused.
#[test]
fn replacement_values_follow_the_secret() {
    let (a, b) = (keys(), Keys::new([7u8; 32]).unwrap());
    assert_eq!(a.uid("1.2.3"), keys().uid("1.2.3"));
    assert_ne!(a.uid("1.2.3"), b.uid("1.2.3"));
    assert_ne!(a.pseudonym("P1", ""), b.pseudonym("P1", ""));
    let uid = a.uid("1.2.3");
    let n: u128 = uid.strip_prefix("2.25.").unwrap().parse().unwrap();
    assert_eq!((n >> 76) & 0xF, 8, "version 8");
    assert_eq!((n >> 62) & 0x3, 2, "variant 10");
    assert!(a.pseudonym("P1", "LIVER-").starts_with("LIVER-"));
    for patient in ["a", "b", "c", "d"] {
        let shift = a.date_shift(patient, (-30, -1));
        assert!((-30..=-1).contains(&shift));
    }
    assert!(Keys::new([0u8; 31]).is_err());
}

/// Dates: shifts across months, years and leap days; malformed dates are not shifted; ages.
#[test]
fn dates_shift_by_the_calendar() {
    use pacs_deid::dates::{age_years, shift_da, shift_dt};
    assert_eq!(shift_da("20240301", -1).as_deref(), Some("20240229"));
    assert_eq!(shift_da("20230301", -1).as_deref(), Some("20230228"));
    assert_eq!(shift_da("20260101", -365).as_deref(), Some("20250101"));
    assert_eq!(shift_da("20260230", -1), None);
    assert_eq!(shift_da("2026", -1), None);
    assert_eq!(
        shift_dt("20260101093000+0100", -1).as_deref(),
        Some("20251231093000+0100")
    );
    assert_eq!(shift_dt("202601", -1), None);
    assert_eq!(age_years("19700615", "20260614"), Some(55));
    assert_eq!(age_years("19700615", "20260615"), Some(56));
}
