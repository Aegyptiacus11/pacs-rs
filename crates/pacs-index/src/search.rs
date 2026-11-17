//! Searching studies by attribute matching (PS3.4 C.2.2.2), for QIDO-RS and the REST
//! API's find.
//!
//! One rule decides what matches: [`Match::matches`], applied to the attributes each study
//! is indexed with. The database only narrows what that rule is asked about, by conditions
//! that can let too much through and never too little, so SQLite and Postgres, whose own
//! pattern matching differs, answer the same. A search without matching keys is paged by
//! the database; one with keys is paged after the rule has spoken.

use serde_json::Value;
use sqlx::Row;

use crate::matching::{Kind, Match};
use crate::worklist::Sql;
use crate::{Dialect, Error, Index, Study, StudyFilter};

/// One matching key of a search: the attribute by its tag (`0020000D`), how its values are
/// matched, and what is asked of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub tag: &'static str,
    pub kind: Kind,
    pub asked: Match,
}

/// A search for studies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StudySearch {
    pub keys: Vec<Key>,
    /// Study Date and Study Time given as ranges of the same form are one range of
    /// moments (PS3.4 C.2.2.2.5.4), as QIDO-RS asks (PS3.18 8.3.4.1.1): from the first
    /// date at the first time to the last date at the last time.
    pub combined_date_time: bool,
    pub limit: Option<usize>,
    pub offset: usize,
}

pub const STUDY_DATE: &str = "00080020";
pub const STUDY_TIME: &str = "00080030";
pub const MODALITIES_IN_STUDY: &str = "00080061";

/// The first value of an attribute in DICOM JSON, as text: a string, a number, or the
/// alphabetic group of a person name.
pub fn first_value(attributes: &Value, tag: &str) -> Option<String> {
    match attributes.get(tag)?.get("Value")?.get(0)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Object(name) => name.get("Alphabetic")?.as_str().map(str::to_owned),
        _ => None,
    }
}

/// `text` as a LIKE pattern that matches itself: `%`, `_` and the escape, escaped.
fn like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A wild card pattern as a LIKE pattern: `*` is `%`, `?` is `_`.
fn like_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        match c {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// The column of `studies st` a key's values are also kept in, for the database to narrow
/// by; none for a key only the attributes hold.
fn column(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "0020000D" => "st.study_instance_uid",
        "00100020" => "st.patient_id",
        "00100010" => "st.patient_name",
        "00080050" => "st.accession_number",
        STUDY_DATE => "st.study_date",
        _ => return None,
    })
}

/// A condition that every study matching `key` satisfies; it may let others through.
fn narrowing(key: &Key, sql: &mut Sql) -> Option<String> {
    let column = column(key.tag)?;
    Some(match (&key.asked, key.kind) {
        // a name's column may hold more than the group that is matched: its start decides
        (Match::Is(value), Kind::Name) => {
            let pattern = sql.text(format!("{}%", like_literal(&value.to_lowercase())));
            format!("lower({column}) LIKE {pattern} ESCAPE '\\'")
        }
        (Match::Like(pattern), Kind::Name) => {
            let pattern = sql.text(format!("{}%", like_pattern(&pattern.to_lowercase())));
            format!("lower({column}) LIKE {pattern} ESCAPE '\\'")
        }
        // LIKE ignores case in SQLite and not in Postgres: lowered on both sides it lets
        // through the same, and the rule then tells the cases apart
        (Match::Like(pattern), _) => {
            let pattern = sql.text(like_pattern(&pattern.to_lowercase()));
            format!("lower({column}) LIKE {pattern} ESCAPE '\\'")
        }
        // matched by meaning, which the database's text comparison does not know
        (Match::Is(_), Kind::Number | Kind::Time) => return None,
        (Match::Is(value), _) => format!("{column} = {}", sql.text(value)),
        (Match::AnyOf(values), _) => {
            let list: Vec<String> = values.iter().map(|value| sql.text(value)).collect();
            format!("{column} IN ({})", list.join(", "))
        }
        (Match::Between(from, to), Kind::Date) => {
            let mut ends = Vec::new();
            if let Some(from) = from {
                ends.push(format!("{column} >= {}", sql.text(from)));
            }
            if let Some(to) = to {
                ends.push(format!("{column} <= {}", sql.text(to)));
            }
            format!("({})", ends.join(" AND "))
        }
        (Match::Between(..), _) => return None,
    })
}

/// Whether a study answers every key of a search.
fn answers(study: &Study, search: &StudySearch) -> bool {
    let key = |tag: &str| search.keys.iter().find(|key| key.tag == tag);
    // a range of dates with a range of times of the same form: one range of moments
    let combined = match (key(STUDY_DATE), key(STUDY_TIME)) {
        (Some(date), Some(time)) if search.combined_date_time => match (&date.asked, &time.asked) {
            (Match::Between(d1, d2), Match::Between(t1, t2))
                if d1.is_some() == t1.is_some() && d2.is_some() == t2.is_some() =>
            {
                Some((
                    d1.as_ref().zip(t1.as_ref()).map(|(d, t)| format!("{d}{t}")),
                    d2.as_ref().zip(t2.as_ref()).map(|(d, t)| format!("{d}{t}")),
                ))
            }
            _ => None,
        },
        _ => None,
    };
    if let Some((from, to)) = &combined {
        let date = first_value(&study.attributes, STUDY_DATE);
        let time = first_value(&study.attributes, STUDY_TIME);
        // the study's moment, with the time as the range's ends are written
        let moment = match (date, time) {
            (Some(date), Some(time)) => {
                let Some(Match::Between(Some(at), _)) =
                    Match::parse(Kind::Time, &time).ok().flatten()
                else {
                    return false;
                };
                format!("{}{at}", date.trim())
            }
            _ => return false,
        };
        if from.as_ref().is_some_and(|from| moment < *from)
            || to.as_ref().is_some_and(|to| moment > *to)
        {
            return false;
        }
    }
    search.keys.iter().all(|key| {
        if combined.is_some() && (key.tag == STUDY_DATE || key.tag == STUDY_TIME) {
            return true;
        }
        if key.tag == MODALITIES_IN_STUDY {
            // a study has several: one that matches is enough
            return study
                .modalities
                .iter()
                .any(|modality| key.asked.matches(key.kind, Some(modality)));
        }
        let stored = first_value(&study.attributes, key.tag);
        key.asked.matches(key.kind, stored.as_deref())
    })
}

impl StudySearch {
    /// What is wrong with the search as a whole, if anything: a range of times that ends
    /// before it starts, which has a meaning only as the ends of a range of moments,
    /// beside a range of dates of the same form in a search that combines them.
    pub fn check(&self) -> Result<(), String> {
        let key = |tag: &str| self.keys.iter().find(|key| key.tag == tag);
        let Some(time) = key(STUDY_TIME).filter(|time| time.asked.reversed()) else {
            return Ok(());
        };
        let combined = self.combined_date_time
            && matches!(
                (key(STUDY_DATE).map(|date| &date.asked), &time.asked),
                (
                    Some(Match::Between(Some(_), Some(_))),
                    Match::Between(Some(_), Some(_))
                )
            );
        if combined {
            Ok(())
        } else {
            Err("the range of times ends before it starts: a range does not cross midnight, unless it comes with a range of dates, from a date to a date".into())
        }
    }
}

impl Index {
    /// The studies that answer `search`, newest first, within what this value's account
    /// may see.
    pub async fn search_studies(&self, search: &StudySearch) -> Result<Vec<Study>, Error> {
        if search.keys.is_empty() {
            let filter = StudyFilter {
                limit: search.limit.map(|limit| limit as i64),
                offset: Some(search.offset as i64),
                ..StudyFilter::default()
            };
            return self.studies(&filter).await;
        }
        let mut sql = Sql::default();
        let mut and = vec!["1 = 1".to_owned()];
        and.extend(self.seen_clause(&mut sql));
        for key in &search.keys {
            and.extend(narrowing(key, &mut sql));
        }
        let modalities = match self.dialect {
            Dialect::Sqlite => {
                "(SELECT group_concat(modality, char(92)) FROM
                   (SELECT DISTINCT modality FROM series se
                     WHERE se.study_instance_uid = st.study_instance_uid
                       AND modality IS NOT NULL ORDER BY modality))"
            }
            Dialect::Postgres => {
                "(SELECT string_agg(DISTINCT modality, chr(92) ORDER BY modality) FROM series se
                   WHERE se.study_instance_uid = st.study_instance_uid)"
            }
        };
        sql.text = format!(
            "SELECT st.study_instance_uid, st.public_id, st.attributes,
                    (SELECT count(*) FROM series se
                      WHERE se.study_instance_uid = st.study_instance_uid) AS series_count,
                    (SELECT count(*) FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid) AS instance_count,
                    {modalities} AS modalities
               FROM studies st
              WHERE {}
              ORDER BY st.study_date DESC NULLS LAST, st.study_instance_uid",
            and.join(" AND ")
        );
        // no page here: the rule below may still leave studies out
        let rows = sql.query().fetch_all(&self.pool).await?;
        let mut found = Vec::new();
        for row in rows {
            let modalities: Option<String> = row.try_get("modalities")?;
            let attributes: String = row.try_get("attributes")?;
            let study = Study {
                study_instance_uid: row.try_get("study_instance_uid")?,
                public_id: row.try_get("public_id")?,
                attributes: serde_json::from_str(&attributes)?,
                modalities: modalities
                    .map(|m| m.split('\\').map(str::to_owned).collect())
                    .unwrap_or_default(),
                series_count: row.try_get("series_count")?,
                instance_count: row.try_get("instance_count")?,
            };
            if answers(&study, search) {
                found.push(study);
            }
        }
        let found = found.into_iter().skip(search.offset);
        Ok(match search.limit {
            Some(limit) => found.take(limit).collect(),
            None => found.collect(),
        })
    }
}
