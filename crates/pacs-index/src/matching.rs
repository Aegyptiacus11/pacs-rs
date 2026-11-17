//! Attribute matching for searches, by PS3.4 C.2.2.2: what a search asks of one attribute,
//! read from the value it gives, and whether a stored value answers it.
//!
//! - **Single value** (C.2.2.2.1): the value itself. Case-sensitive, except for person
//!   names, which the standard lets an implementation match without regard to case; here
//!   they are. A time matches by its meaning: `2230` is every time in that minute.
//! - **List of UID** (C.2.2.2.2): any of several UIDs.
//! - **Universal** (C.2.2.2.3): an empty value, which every entity matches; no [`Match`].
//! - **Wild card** (C.2.2.2.4): `*` for any run of characters, `?` for any one. For text
//!   and names, not for dates, times, numbers or UIDs.
//! - **Range** (C.2.2.2.5): `from-to`, `from-` or `-to`, both ends included, for dates and
//!   times.
//!
//! Sequence, empty value and multiple value matching (C.2.2.2.6 to 8) are not here.

/// How an attribute's values are matched: by its Value Representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// AE, CS, LO, SH and the like: single value or wild card, case-sensitive.
    Text,
    /// PN: single value or wild card, without regard to case.
    Name,
    /// UI: one UID or a list.
    Uid,
    /// DA: one date or a range.
    Date,
    /// TM: one time or a range, by meaning.
    Time,
    /// IS: one whole number.
    Number,
}

/// What a search asks of one attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// This value.
    Is(String),
    /// This pattern, with `*` and `?`.
    Like(String),
    /// One of these.
    AnyOf(Vec<String>),
    /// From the first to the second, both included; an end left out is open.
    Between(Option<String>, Option<String>),
}

fn date(text: &str) -> Result<String, String> {
    let ok = text.len() == 8
        && text.bytes().all(|b| b.is_ascii_digit())
        && (1..=12).contains(&text[4..6].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&text[6..8].parse::<u8>().unwrap_or(0));
    if ok {
        Ok(text.to_owned())
    } else {
        Err(format!("{text:?} is not a date (YYYYMMDD)"))
    }
}

/// A time as given: HH, HHMM or HHMMSS, with a fraction of up to six digits after the
/// seconds (PS3.5 6.2, TM).
fn time(text: &str) -> Result<&str, String> {
    let bad = || format!("{text:?} is not a time (HH, HHMM, HHMMSS or HHMMSS.FFFFFF)");
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !matches!(whole.len(), 2 | 4 | 6) {
        return Err(bad());
    }
    if let Some(fraction) = fraction
        && (whole.len() != 6 || !digits(fraction) || fraction.len() > 6)
    {
        return Err(bad());
    }
    let part = |at: usize, most: u8| {
        whole.len() <= at || whole[at..at + 2].parse::<u8>().is_ok_and(|n| n <= most)
    };
    // a leap second is 60
    if part(0, 23) && part(2, 59) && part(4, 60) {
        Ok(text)
    } else {
        Err(bad())
    }
}

/// The first and the last instant a time names, as HHMMSS.FFFFFF: `2230` is from
/// `223000.000000` to `223059.999999`.
fn instants(text: &str) -> Result<(String, String), String> {
    let text = time(text)?;
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let low = format!("{whole:0<6}.{fraction:0<6}");
    let rest = match whole.len() {
        2 => "5959",
        4 => "59",
        _ => "",
    };
    let high = format!("{whole}{rest}.{fraction:9<6}");
    Ok((low, high))
}

/// A stored time as HHMMSS.FFFFFF, its first instant, for comparing; `None` when it is not
/// a time.
fn stored_time(text: &str) -> Option<String> {
    instants(text.trim()).ok().map(|(low, _)| low)
}

impl Match {
    /// What `value` asks of an attribute of `kind`; `None` for an empty value, which asks
    /// nothing (universal matching). An error says what is wrong with the value.
    pub fn parse(kind: Kind, value: &str) -> Result<Option<Match>, String> {
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        Ok(Some(match kind {
            Kind::Text | Kind::Name => {
                if value == "*" {
                    // "*" alone is universal matching (C.2.2.2.4, note)
                    return Ok(None);
                }
                if value.contains(['*', '?']) {
                    Match::Like(value.to_owned())
                } else {
                    Match::Is(value.to_owned())
                }
            }
            Kind::Uid => {
                let mut uids = Vec::new();
                for uid in value.split(['\\', ',']) {
                    let uid = uid.trim();
                    if !pacs_core::is_valid_uid(uid) {
                        return Err(format!("{uid:?} is not a UID"));
                    }
                    if !uids.iter().any(|known| known == uid) {
                        uids.push(uid.to_owned());
                    }
                }
                match uids.len() {
                    1 => Match::Is(uids.remove(0)),
                    _ => Match::AnyOf(uids),
                }
            }
            Kind::Date => match value.split_once('-') {
                None => Match::Is(date(value)?),
                Some((from, to)) => {
                    let (from, to) = (from.trim(), to.trim());
                    let end = |text: &str| {
                        if text.is_empty() {
                            Ok(None)
                        } else {
                            date(text).map(Some)
                        }
                    };
                    let (from, to) = (end(from)?, end(to)?);
                    match (&from, &to) {
                        (None, None) => return Err("a range needs one end at least".into()),
                        (Some(a), Some(b)) if a > b => {
                            return Err(format!("the range {value} ends before it starts"));
                        }
                        _ => Match::Between(from, to),
                    }
                }
            },
            Kind::Time => {
                let (from, to) = match value.split_once('-') {
                    // one time is every instant it names
                    None => {
                        let (low, high) = instants(value)?;
                        (Some(low), Some(high))
                    }
                    Some((from, to)) => {
                        let (from, to) = (from.trim(), to.trim());
                        let low = if from.is_empty() {
                            None
                        } else {
                            Some(instants(from)?.0)
                        };
                        let high = if to.is_empty() {
                            None
                        } else {
                            Some(instants(to)?.1)
                        };
                        (low, high)
                    }
                };
                match (&from, &to) {
                    (None, None) => return Err("a range needs one end at least".into()),
                    (Some(a), Some(b)) if a > b => {
                        return Err(format!("the range {value} ends before it starts"));
                    }
                    _ => Match::Between(from, to),
                }
            }
            Kind::Number => {
                let number: i64 = value
                    .parse()
                    .map_err(|_| format!("{value:?} is not a whole number"))?;
                Match::Is(number.to_string())
            }
        }))
    }

    /// Whether a stored value answers this. A missing value answers nothing.
    pub fn matches(&self, kind: Kind, stored: Option<&str>) -> bool {
        let Some(stored) = stored else { return false };
        let stored = stored.trim();
        let folded = |text: &str| match kind {
            Kind::Name => text.to_lowercase(),
            _ => text.to_owned(),
        };
        match (self, kind) {
            (Match::Between(from, to), Kind::Time) => {
                let Some(at) = stored_time(stored) else {
                    return false;
                };
                from.as_ref().is_none_or(|from| *from <= at)
                    && to.as_ref().is_none_or(|to| at <= *to)
            }
            (Match::Between(from, to), _) => {
                !stored.is_empty()
                    && from.as_deref().is_none_or(|from| from <= stored)
                    && to.as_deref().is_none_or(|to| stored <= to)
            }
            (Match::Is(value), Kind::Number) => {
                stored.parse::<i64>().ok().map(|n| n.to_string()).as_ref() == Some(value)
            }
            (Match::Is(value), _) => folded(value) == folded(stored),
            (Match::AnyOf(values), _) => values.iter().any(|value| value == stored),
            (Match::Like(pattern), _) => wild(&folded(pattern), &folded(stored)),
        }
    }
}

/// Wild card matching: `*` is any run of characters, none included, `?` any one.
fn wild(pattern: &str, text: &str) -> bool {
    let (pattern, text): (Vec<char>, Vec<char>) =
        (pattern.chars().collect(), text.chars().collect());
    // the classic two-pointer walk, going back to the last `*` on a mismatch
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((at, taken)) = star {
            p = at + 1;
            t = taken + 1;
            star = Some((at, taken + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(kind: Kind, value: &str) -> Match {
        Match::parse(kind, value).unwrap().unwrap()
    }

    #[test]
    fn an_empty_value_or_a_lone_star_asks_nothing() {
        for kind in [
            Kind::Text,
            Kind::Name,
            Kind::Uid,
            Kind::Date,
            Kind::Time,
            Kind::Number,
        ] {
            assert_eq!(Match::parse(kind, "").unwrap(), None);
            assert_eq!(Match::parse(kind, "  ").unwrap(), None);
        }
        assert_eq!(Match::parse(Kind::Text, "*").unwrap(), None);
        assert_eq!(Match::parse(Kind::Name, "*").unwrap(), None);
    }

    #[test]
    fn text_is_case_sensitive_and_names_are_not() {
        let text = asked(Kind::Text, "ACC1");
        assert!(text.matches(Kind::Text, Some("ACC1")));
        assert!(text.matches(Kind::Text, Some("ACC1 ")));
        assert!(!text.matches(Kind::Text, Some("acc1")));
        assert!(!text.matches(Kind::Text, Some("ACC10")));
        assert!(!text.matches(Kind::Text, None));

        let name = asked(Kind::Name, "doe^jane");
        assert!(name.matches(Kind::Name, Some("DOE^JANE")));
        assert!(!name.matches(Kind::Name, Some("DOE^JANET")));
    }

    #[test]
    fn wild_cards() {
        for (pattern, text, matches) in [
            ("DOE*", "DOE^JANE", true),
            ("DOE*", "DOE", true),
            ("DOE*", "ROE^JANE", false),
            ("*JANE", "DOE^JANE", true),
            ("*^J*", "DOE^JANE", true),
            ("D?E^*", "DOE^JANE", true),
            ("D?E^*", "DE^JANE", false),
            ("??", "CT", true),
            ("??", "C", false),
            ("*a*b*c*", "xxaxxbxxc", true),
            ("*a*b*c*", "xxaxxcxxb", false),
            ("a*", "", false),
            ("C*", "CT", true),
        ] {
            let asked = asked(Kind::Text, pattern);
            assert!(matches!(asked, Match::Like(_)), "{pattern}");
            assert_eq!(
                asked.matches(Kind::Text, Some(text)),
                matches,
                "{pattern} on {text}"
            );
        }
        // case-sensitive for text, not for names
        assert!(!asked(Kind::Text, "doe*").matches(Kind::Text, Some("DOE^JANE")));
        assert!(asked(Kind::Name, "doe*").matches(Kind::Name, Some("DOE^JANE")));
        // a character of a regular expression is itself
        assert!(asked(Kind::Text, "a.c*").matches(Kind::Text, Some("a.cd")));
        assert!(!asked(Kind::Text, "a.c*").matches(Kind::Text, Some("abcd")));
    }

    #[test]
    fn a_list_of_uids() {
        assert_eq!(asked(Kind::Uid, "1.2.3"), Match::Is("1.2.3".into()));
        let list = Match::AnyOf(vec!["1.2.3".into(), "1.2.4".into()]);
        // the separator of DIMSE, and that of a URL
        assert_eq!(asked(Kind::Uid, "1.2.3\\1.2.4"), list);
        assert_eq!(asked(Kind::Uid, "1.2.3,1.2.4,1.2.3"), list);
        assert!(list.matches(Kind::Uid, Some("1.2.4")));
        assert!(!list.matches(Kind::Uid, Some("1.2.5")));
        // no wild card in a UID
        for bad in ["1.2.*", "1.2.3,", "abc", "1..2"] {
            assert!(Match::parse(Kind::Uid, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn dates_and_their_ranges() {
        assert_eq!(asked(Kind::Date, "20260115"), Match::Is("20260115".into()));
        let january = asked(Kind::Date, "20260101-20260131");
        for (stored, matches) in [
            ("20260101", true),
            ("20260131", true),
            ("20251231", false),
            ("20260201", false),
            ("", false),
        ] {
            assert_eq!(
                january.matches(Kind::Date, Some(stored)),
                matches,
                "{stored}"
            );
        }
        assert!(!january.matches(Kind::Date, None));
        let from = asked(Kind::Date, "20260101-");
        assert!(
            from.matches(Kind::Date, Some("20991231"))
                && !from.matches(Kind::Date, Some("20251231"))
        );
        let to = asked(Kind::Date, "-20260131");
        assert!(
            to.matches(Kind::Date, Some("19700101")) && !to.matches(Kind::Date, Some("20260201"))
        );
        // spaces around the hyphen, as the standard writes ranges
        assert_eq!(asked(Kind::Date, "20260101 - 20260131"), january);

        for (bad, why) in [
            ("2026", "is not a date"),
            ("20261301", "is not a date"),
            ("20260100", "is not a date"),
            ("2026-01-15", "is not a date"),
            ("-", "one end at least"),
            ("20260131-20260101", "ends before it starts"),
            ("20260101-x", "is not a date"),
        ] {
            let error = Match::parse(Kind::Date, bad).unwrap_err();
            assert!(error.contains(why), "{bad}: {error}");
        }
    }

    #[test]
    fn times_are_matched_by_their_meaning() {
        // one time is every instant it names
        let minute = asked(Kind::Time, "2230");
        for (stored, matches) in [
            ("223000", true),
            ("2230", true),
            ("223059.999", true),
            ("222959", false),
            ("223100", false),
            ("not a time", false),
        ] {
            assert_eq!(
                minute.matches(Kind::Time, Some(stored)),
                matches,
                "{stored}"
            );
        }
        let exact = asked(Kind::Time, "223015");
        assert!(exact.matches(Kind::Time, Some("223015.5")));
        assert!(!exact.matches(Kind::Time, Some("223016")));

        let office = asked(Kind::Time, "1000-1800");
        for (stored, matches) in [
            ("100000", true),
            ("180059.999999", true),
            ("120000", true),
            ("095959", false),
            ("180100", false),
        ] {
            assert_eq!(
                office.matches(Kind::Time, Some(stored)),
                matches,
                "{stored}"
            );
        }
        assert!(asked(Kind::Time, "-12").matches(Kind::Time, Some("125959")));
        assert!(!asked(Kind::Time, "-12").matches(Kind::Time, Some("130000")));
        assert!(asked(Kind::Time, "12-").matches(Kind::Time, Some("235959")));

        for bad in [
            "1",
            "123",
            "2500",
            "1260",
            "120061",
            "12.5",
            "120000.1234567",
            "1800-1000",
            "-",
        ] {
            assert!(Match::parse(Kind::Time, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn numbers_are_whole_and_matched_by_value() {
        let three = asked(Kind::Number, "3");
        assert!(three.matches(Kind::Number, Some("3")));
        assert!(three.matches(Kind::Number, Some(" 03")));
        assert!(!three.matches(Kind::Number, Some("30")));
        assert!(!three.matches(Kind::Number, Some("x")));
        for bad in ["3.5", "three", "1-3", "1*"] {
            assert!(Match::parse(Kind::Number, bad).is_err(), "{bad}");
        }
    }
}
