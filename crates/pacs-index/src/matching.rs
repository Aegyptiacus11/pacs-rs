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
//! - **Sequence** (C.2.2.2.6): the keys of one item, which one item at least of the stored
//!   sequence must answer, all of them; a sequence inside it is matched the same way.
//!
//! An attribute with several values answers when any of them does (C.2.2.3). Empty value
//! and multiple value matching (C.2.2.2.7 and 8) are not here: each needs an extended
//! negotiation that is not made.

use serde_json::Value;

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
    /// SQ: the keys of one item.
    Sequence,
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
    /// An item that answers every one of these; with none, anything, a missing sequence
    /// included.
    Item(Vec<ItemKey>),
}

/// One key of the item a sequence is matched by: an attribute of the item by its tag
/// (`00400009`), how its values are matched, and what is asked of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemKey {
    pub tag: String,
    pub kind: Kind,
    pub asked: Match,
}

/// The values of an attribute in DICOM JSON, as text: strings, numbers, and of a person
/// name its alphabetic group. None for a missing attribute, a sequence or bulk data.
pub fn values(attributes: &Value, tag: &str) -> Vec<String> {
    let Some(values) = attributes
        .get(tag)
        .and_then(|attribute| attribute.get("Value"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Object(name) => name.get("Alphabetic")?.as_str().map(str::to_owned),
            _ => None,
        })
        .collect()
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
                // A range of times that ends before it starts is not refused here: beside a
                // range of dates it is the two ends of one range of moments
                // (C.2.2.2.5.4). On its own it matches nothing, since a range does not
                // cross midnight (C.2.2.2.5.2): see [`Match::reversed`].
                match (&from, &to) {
                    (None, None) => return Err("a range needs one end at least".into()),
                    _ => Match::Between(from, to),
                }
            }
            Kind::Number => {
                let number: i64 = value
                    .parse()
                    .map_err(|_| format!("{value:?} is not a whole number"))?;
                Match::Is(number.to_string())
            }
            Kind::Sequence => {
                return Err("a sequence is matched by the keys of an item, not by a value".into());
            }
        }))
    }

    /// Whether the attribute `tag` of an entity, given as DICOM JSON, answers this: any
    /// of its values (C.2.2.3), or for a sequence any of its items.
    pub fn answered_by(&self, kind: Kind, attributes: &Value, tag: &str) -> bool {
        let Match::Item(keys) = self else {
            return values(attributes, tag)
                .iter()
                .any(|stored| self.matches(kind, Some(stored)));
        };
        if keys.is_empty() {
            return true;
        }
        attributes
            .get(tag)
            .and_then(|attribute| attribute.get("Value"))
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    keys.iter()
                        .all(|key| key.asked.answered_by(key.kind, item, &key.tag))
                })
            })
    }

    /// Whether this is a range that ends before it starts: of times, which only has a
    /// meaning beside a range of dates.
    pub fn reversed(&self) -> bool {
        matches!(self, Match::Between(Some(from), Some(to)) if from > to)
    }

    /// Whether a stored value answers this. A missing value answers nothing, and no value
    /// answers the keys of an item: see [`Match::answered_by`].
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
            (Match::Item(_), _) => false,
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

    fn item(keys: &[(&str, Kind, &str)]) -> Match {
        Match::Item(
            keys.iter()
                .map(|(tag, kind, value)| ItemKey {
                    tag: (*tag).to_owned(),
                    kind: *kind,
                    asked: asked(*kind, value),
                })
                .collect(),
        )
    }

    /// A series with two requests, the second with a coded reason.
    fn series() -> Value {
        serde_json::json!({
            "00080060": {"vr": "CS", "Value": ["CT"]},
            "00080008": {"vr": "CS", "Value": ["ORIGINAL", "PRIMARY", "AXIAL"]},
            "00200011": {"vr": "IS", "Value": [3]},
            "00081070": {"vr": "PN", "Value": [{"Alphabetic": "DOE^JANE"}]},
            "00400275": {"vr": "SQ", "Value": [
                {
                    "00400009": {"vr": "SH", "Value": ["STEP1"]},
                    "00401001": {"vr": "SH", "Value": ["REQ1"]}
                },
                {
                    "00400009": {"vr": "SH", "Value": ["STEP2"]},
                    "00401001": {"vr": "SH", "Value": ["REQ2"]},
                    "0040100A": {"vr": "SQ", "Value": [
                        {"00080100": {"vr": "SH", "Value": ["R-1"]}},
                        {"00080100": {"vr": "SH", "Value": ["R-2"]}}
                    ]}
                }
            ]},
            "00400260": {"vr": "SQ"}
        })
    }

    #[test]
    fn a_sequence_is_answered_by_one_item_that_answers_every_key() {
        const REQUESTS: &str = "00400275";
        let series = series();
        let answered =
            |keys: &[(&str, Kind, &str)]| item(keys).answered_by(Kind::Sequence, &series, REQUESTS);
        assert!(answered(&[("00400009", Kind::Text, "STEP2")]));
        assert!(answered(&[("00400009", Kind::Text, "STEP?")]));
        assert!(answered(&[
            ("00400009", Kind::Text, "STEP2"),
            ("00401001", Kind::Text, "REQ2")
        ]));
        // each key has an item that answers it, but no item answers both
        assert!(!answered(&[
            ("00400009", Kind::Text, "STEP1"),
            ("00401001", Kind::Text, "REQ2")
        ]));
        assert!(!answered(&[("00400009", Kind::Text, "STEP3")]));
        // a key the items do not have
        assert!(!answered(&[("00321060", Kind::Text, "CHEST")]));
    }

    #[test]
    fn a_sequence_inside_an_item_is_matched_the_same_way() {
        let series = series();
        let reason = |code: &str| {
            Match::Item(vec![ItemKey {
                tag: "0040100A".into(),
                kind: Kind::Sequence,
                asked: item(&[("00080100", Kind::Text, code)]),
            }])
        };
        assert!(reason("R-2").answered_by(Kind::Sequence, &series, "00400275"));
        assert!(!reason("R-3").answered_by(Kind::Sequence, &series, "00400275"));
    }

    #[test]
    fn an_item_without_keys_asks_nothing_and_a_missing_sequence_answers_no_key() {
        let series = series();
        let any = Match::Item(Vec::new());
        // there, empty, and not there at all
        for tag in ["00400275", "00400260", "00081032"] {
            assert!(any.answered_by(Kind::Sequence, &series, tag), "{tag}");
        }
        let code = item(&[("00080100", Kind::Text, "R-1")]);
        for tag in ["00400260", "00081032"] {
            assert!(!code.answered_by(Kind::Sequence, &series, tag), "{tag}");
        }
        // a sequence is not a value, and a value is not a sequence
        assert!(!code.matches(Kind::Sequence, Some("R-1")));
        assert!(!code.answered_by(Kind::Sequence, &series, "00080060"));
        assert!(Match::parse(Kind::Sequence, "R-1").is_err());
        assert_eq!(Match::parse(Kind::Sequence, "").unwrap(), None);
    }

    #[test]
    fn an_attribute_with_several_values_answers_when_any_does() {
        let series = series();
        let answered =
            |tag: &str, kind: Kind, value: &str| asked(kind, value).answered_by(kind, &series, tag);
        assert!(answered("00080008", Kind::Text, "PRIMARY"));
        assert!(answered("00080008", Kind::Text, "AX*"));
        assert!(!answered("00080008", Kind::Text, "DERIVED"));
        // a number, and the alphabetic group of a name
        assert!(answered("00200011", Kind::Number, "3"));
        assert!(answered("00081070", Kind::Name, "doe^*"));
        // missing, or a sequence where a value is asked
        assert!(!answered("00080050", Kind::Text, "ACC1"));
        assert!(!answered("00400275", Kind::Text, "STEP1"));
        assert_eq!(values(&series, "00080008").len(), 3);
        assert!(values(&series, "00400275").is_empty());
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
            Kind::Sequence,
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

        // across midnight: no time of a day is in it
        let night = asked(Kind::Time, "1800-1000");
        assert!(night.reversed() && !office.reversed());
        for stored in ["190000", "090000", "120000"] {
            assert!(!night.matches(Kind::Time, Some(stored)), "{stored}");
        }

        for bad in [
            "1",
            "123",
            "2500",
            "1260",
            "120061",
            "12.5",
            "120000.1234567",
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
