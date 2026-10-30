//! The web interface's study list, a page at a time: filtered, sorted, counted and cut in the
//! database, so that the browser never loads the whole archive. Free text matches words in
//! the patient's name and ID, the accession number, the study description, the referring
//! physician and the date; the rest are exact filters. Fields the index keeps only among a
//! study's main tags (description, time, referring physician, sex) are read from that JSON in
//! the query, the dialect's way.

use serde_json::Value;
use sqlx::Row;
use sqlx::any::AnyRow;

use crate::{Dialect, Error, Index, parse};

/// What the list shows.
#[derive(Debug, Clone, Default)]
pub struct WorklistQuery {
    /// Each must appear somewhere in the study (any case).
    pub words: Vec<String>,
    /// Study dates from and to, `YYYYMMDD`, both included; `None` leaves that end open.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Studies with a series of any of these modalities; empty for any.
    pub modalities: Vec<String>,
    /// Patient sex, `O` standing for anything but `F` and `M` (and for none); empty for any.
    pub sexes: Vec<String>,
    /// One patient's studies, by the patient's public ID.
    pub patient: Option<String>,
    pub sort: WorklistSort,
    pub descending: bool,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorklistSort {
    Patient,
    Description,
    Modality,
    #[default]
    Date,
    Series,
    Instances,
    /// When the study's latest instance arrived.
    Received,
}

/// One study of the list, with what a row shows.
#[derive(Debug, Clone, PartialEq)]
pub struct WorklistStudy {
    pub public_id: String,
    pub study_instance_uid: String,
    pub patient_public_id: String,
    pub main_tags: Value,
    pub patient_main_tags: Value,
    pub modalities: Vec<String>,
    pub series: i64,
    pub instances: i64,
    /// When its latest instance arrived, epoch ms.
    pub last_ms: Option<i64>,
}

/// What the whole filtered list holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorklistSummary {
    pub patients: i64,
    pub studies: i64,
    pub series: i64,
    pub instances: i64,
    /// Studies with each modality, the most first.
    pub by_modality: Vec<(String, i64)>,
}

/// The choices the filters offer, with how many studies each has, over every study (or one
/// patient's): modalities, sexes (as in [`WorklistQuery::sexes`]) and study dates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorklistFacets {
    pub modalities: Vec<(String, i64)>,
    pub sexes: Vec<(String, i64)>,
    pub dates: Vec<(String, i64)>,
}

enum Arg {
    Text(String),
    Int(i64),
}

/// SQL text and its arguments, each placed as the next `$n`.
#[derive(Default)]
struct Sql {
    text: String,
    args: Vec<Arg>,
}

impl Sql {
    fn arg(&mut self, arg: Arg) -> String {
        self.args.push(arg);
        format!("${}", self.args.len())
    }
    fn text(&mut self, s: impl Into<String>) -> String {
        self.arg(Arg::Text(s.into()))
    }
    fn query(self) -> sqlx::query::Query<'static, sqlx::Any, sqlx::any::AnyArguments> {
        let mut q = sqlx::query(sqlx::AssertSqlSafe(self.text));
        for arg in self.args {
            q = match arg {
                Arg::Text(s) => q.bind(s),
                Arg::Int(n) => q.bind(n),
            };
        }
        q
    }
}

/// `%` and `_` (and the escape itself) matched as themselves in a LIKE pattern.
fn like_escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 2);
    out.push('%');
    for c in word.to_lowercase().chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl Index {
    /// A main tag of `column` (a JSON object of strings), as SQL text.
    fn main_tag(&self, column: &str, key: &str) -> String {
        match self.dialect {
            Dialect::Sqlite => format!("json_extract({column}, '$.{key}')"),
            Dialect::Postgres => format!("(({column})::jsonb ->> '{key}')"),
        }
    }

    fn sex(&self) -> String {
        format!(
            "coalesce(nullif({}, ''), 'O')",
            self.main_tag("st.patient_main_tags", "PatientSex")
        )
    }

    fn modalities_of_study(&self) -> &'static str {
        match self.dialect {
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
        }
    }

    /// The conditions of `query` on `studies st`, its arguments added to `sql`.
    fn worklist_where(&self, query: &WorklistQuery, sql: &mut Sql) -> String {
        let mut and = vec!["st.public_id IS NOT NULL".to_owned()];
        if let Some(patient) = &query.patient {
            and.push(format!("st.patient_public_id = {}", sql.text(patient)));
        }
        if let Some(from) = &query.from {
            and.push(format!("st.study_date >= {}", sql.text(from)));
        }
        if let Some(to) = &query.to {
            and.push(format!("st.study_date <= {}", sql.text(to)));
        }
        if !query.modalities.is_empty() {
            let list: Vec<String> = query.modalities.iter().map(|m| sql.text(m)).collect();
            and.push(format!(
                "EXISTS (SELECT 1 FROM series se WHERE se.study_instance_uid = st.study_instance_uid
                           AND se.modality IN ({}))",
                list.join(", ")
            ));
        }
        if !query.sexes.is_empty() {
            let list: Vec<String> = query.sexes.iter().map(|s| sql.text(s)).collect();
            let known = "('F', 'M')";
            // "O": whatever is neither F nor M
            let other = query.sexes.iter().any(|s| s == "O");
            and.push(format!(
                "({sex} IN ({list}){or})",
                sex = self.sex(),
                list = list.join(", "),
                or = if other {
                    format!(" OR {} NOT IN {known}", self.sex())
                } else {
                    String::new()
                }
            ));
        }
        let fields = [
            "coalesce(st.patient_name, '')".to_owned(),
            "coalesce(st.patient_id, '')".to_owned(),
            "coalesce(st.accession_number, '')".to_owned(),
            "coalesce(st.study_date, '')".to_owned(),
            format!(
                "coalesce({}, '')",
                self.main_tag("st.main_tags", "StudyDescription")
            ),
            format!(
                "coalesce({}, '')",
                self.main_tag("st.main_tags", "ReferringPhysicianName")
            ),
        ];
        for word in query.words.iter().filter(|w| !w.is_empty()) {
            let pattern = sql.text(like_escape(word));
            let any: Vec<String> = fields
                .iter()
                .map(|f| format!("lower({f}) LIKE {pattern} ESCAPE '\\'"))
                .collect();
            and.push(format!("({})", any.join(" OR ")));
        }
        and.join(" AND ")
    }

    /// One page of the list, and how many studies the whole list has.
    pub async fn worklist(
        &self,
        query: &WorklistQuery,
    ) -> Result<(Vec<WorklistStudy>, i64), Error> {
        let mut count = Sql::default();
        let filter = self.worklist_where(query, &mut count);
        count.text = format!("SELECT count(*) AS n FROM studies st WHERE {filter}");
        let total: i64 = count.query().fetch_one(&self.pool).await?.try_get("n")?;

        let mut sql = Sql::default();
        let filter = self.worklist_where(query, &mut sql);
        let direction = if query.descending { "DESC" } else { "ASC" };
        let key = match query.sort {
            WorklistSort::Patient => "lower(st.patient_name)".to_owned(),
            WorklistSort::Description => format!(
                "lower({})",
                self.main_tag("st.main_tags", "StudyDescription")
            ),
            WorklistSort::Modality => "modalities".to_owned(),
            WorklistSort::Date => format!(
                "st.study_date {direction} NULLS LAST, {}",
                self.main_tag("st.main_tags", "StudyTime")
            ),
            WorklistSort::Series => "series_count".to_owned(),
            WorklistSort::Instances => "instance_count".to_owned(),
            WorklistSort::Received => "last_ms".to_owned(),
        };
        let limit = sql.arg(Arg::Int(query.limit.clamp(1, 1000)));
        let offset = sql.arg(Arg::Int(query.offset.max(0)));
        sql.text = format!(
            "SELECT st.public_id, st.study_instance_uid, st.patient_public_id, st.main_tags,
                    st.patient_main_tags,
                    (SELECT count(*) FROM series se
                      WHERE se.study_instance_uid = st.study_instance_uid) AS series_count,
                    (SELECT count(*) FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid) AS instance_count,
                    (SELECT max(i.received_ms) FROM instances i JOIN series se USING (series_instance_uid)
                      WHERE se.study_instance_uid = st.study_instance_uid) AS last_ms,
                    {modalities} AS modalities
               FROM studies st
              WHERE {filter}
              ORDER BY {key} {direction} NULLS LAST, st.public_id
              LIMIT {limit} OFFSET {offset}",
            modalities = self.modalities_of_study(),
        );
        let studies = sql
            .query()
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(worklist_study)
            .collect::<Result<_, _>>()?;
        Ok((studies, total))
    }

    /// Totals over the whole filtered list (its paging aside).
    pub async fn worklist_summary(&self, query: &WorklistQuery) -> Result<WorklistSummary, Error> {
        let mut sql = Sql::default();
        let filter = self.worklist_where(query, &mut sql);
        let chosen = format!(
            "WITH chosen AS (SELECT st.study_instance_uid, st.patient_public_id FROM studies st WHERE {filter})"
        );
        let args = std::mem::take(&mut sql.args);
        let mut totals = Sql {
            args,
            ..Sql::default()
        };
        totals.text = format!(
            "{chosen}
             SELECT (SELECT count(*) FROM chosen) AS studies,
                    (SELECT count(DISTINCT patient_public_id) FROM chosen) AS patients,
                    (SELECT count(*) FROM series se JOIN chosen c ON c.study_instance_uid = se.study_instance_uid) AS series,
                    (SELECT count(*) FROM instances i JOIN series se USING (series_instance_uid)
                       JOIN chosen c ON c.study_instance_uid = se.study_instance_uid) AS instances"
        );
        let row = totals.query().fetch_one(&self.pool).await?;
        let mut by = Sql::default();
        self.worklist_where(query, &mut by);
        by.text = format!(
            "{chosen}
             SELECT se.modality AS value, count(DISTINCT se.study_instance_uid) AS n
               FROM series se JOIN chosen c ON c.study_instance_uid = se.study_instance_uid
              WHERE se.modality IS NOT NULL
              GROUP BY se.modality ORDER BY n DESC, se.modality"
        );
        Ok(WorklistSummary {
            studies: row.try_get("studies")?,
            patients: row.try_get("patients")?,
            series: row.try_get("series")?,
            instances: row.try_get("instances")?,
            by_modality: counts(by.query().fetch_all(&self.pool).await?)?,
        })
    }

    /// What the filters can choose, with counts, over every study or one patient's.
    pub async fn worklist_facets(&self, patient: Option<&str>) -> Result<WorklistFacets, Error> {
        let scope = |sql: &mut Sql| match patient {
            Some(p) => format!(
                "st.public_id IS NOT NULL AND st.patient_public_id = {}",
                sql.text(p)
            ),
            None => "st.public_id IS NOT NULL".to_owned(),
        };
        let mut modalities = Sql::default();
        let filter = scope(&mut modalities);
        modalities.text = format!(
            "SELECT se.modality AS value, count(DISTINCT st.study_instance_uid) AS n
               FROM series se JOIN studies st ON st.study_instance_uid = se.study_instance_uid
              WHERE {filter} AND se.modality IS NOT NULL
              GROUP BY se.modality ORDER BY se.modality"
        );
        let mut sexes = Sql::default();
        let filter = scope(&mut sexes);
        sexes.text = format!(
            "SELECT CASE WHEN {sex} IN ('F', 'M') THEN {sex} ELSE 'O' END AS value, count(*) AS n
               FROM studies st WHERE {filter} GROUP BY 1 ORDER BY 1",
            sex = self.sex()
        );
        let mut dates = Sql::default();
        let filter = scope(&mut dates);
        dates.text = format!(
            "SELECT st.study_date AS value, count(*) AS n FROM studies st
              WHERE {filter} AND st.study_date IS NOT NULL AND st.study_date <> ''
              GROUP BY st.study_date ORDER BY st.study_date"
        );
        Ok(WorklistFacets {
            modalities: counts(modalities.query().fetch_all(&self.pool).await?)?,
            sexes: counts(sexes.query().fetch_all(&self.pool).await?)?,
            dates: counts(dates.query().fetch_all(&self.pool).await?)?,
        })
    }
}

fn counts(rows: Vec<AnyRow>) -> Result<Vec<(String, i64)>, Error> {
    rows.iter()
        .map(|r| Ok((r.try_get("value")?, r.try_get("n")?)))
        .collect()
}

fn worklist_study(row: AnyRow) -> Result<WorklistStudy, Error> {
    let modalities: Option<String> = row.try_get("modalities")?;
    Ok(WorklistStudy {
        public_id: row.try_get("public_id")?,
        study_instance_uid: row.try_get("study_instance_uid")?,
        patient_public_id: row.try_get("patient_public_id")?,
        main_tags: parse(row.try_get("main_tags")?)?,
        patient_main_tags: parse(row.try_get("patient_main_tags")?)?,
        modalities: modalities
            .map(|m| m.split('\\').map(str::to_owned).collect())
            .unwrap_or_default(),
        series: row.try_get("series_count")?,
        instances: row.try_get("instance_count")?,
        last_ms: row.try_get("last_ms")?,
    })
}
