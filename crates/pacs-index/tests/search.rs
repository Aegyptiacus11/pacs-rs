//! Searching studies by attribute matching: the same answers on SQLite and on Postgres.

mod common;

use pacs_core::Instance;
use pacs_core::real::RealSeries;
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Index, Key, Kind, Match, NewInstance, StudySearch};

async fn record(index: &Index, series: &SyntheticSeries) {
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let key = format!("dicom/{}/{slice}.dcm", series.seed);
        let new = NewInstance::new(&instance, key, bytes.len()).unwrap();
        index.record(&new).await.unwrap();
    }
}

/// Five studies: seeds 201 to 205.
async fn five(index: &Index) {
    for series in [
        SyntheticSeries::new(201, 1)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000"),
        SyntheticSeries::new(202, 1)
            .with_patient("DOE^JOHN", None)
            .at("20260105", "180100"),
        SyntheticSeries::new(203, 1)
            .with_patient("Roe^Jane", None)
            .at("20260106", "120000")
            .of_modality("MR"),
        SyntheticSeries::new(204, 1)
            .with_patient("Do_e%^Odd", None)
            .at("20260107", "090000"),
        SyntheticSeries::new(205, 1)
            .with_patient("Smith^Al", None)
            .at("20260201", "235959")
            .of_modality("US"),
        // a second series of the first study, of another modality
        SyntheticSeries::new(206, 1)
            .in_study(201, 2)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000")
            .of_modality("SR"),
    ] {
        record(index, &series).await;
    }
}

fn key(tag: &'static str, kind: Kind, value: &str) -> Key {
    Key {
        tag,
        kind,
        asked: Match::parse(kind, value).unwrap().unwrap(),
    }
}

/// The seeds of the studies a search finds, in order.
async fn found(index: &Index, keys: Vec<Key>, combined: bool) -> Vec<u32> {
    let search = StudySearch {
        keys,
        combined_date_time: combined,
        ..StudySearch::default()
    };
    let mut seeds: Vec<u32> = index
        .search_studies(&search)
        .await
        .unwrap()
        .iter()
        .map(|study| {
            let uid = study.study_instance_uid.strip_prefix("2.25.").unwrap();
            uid.strip_suffix("100000").unwrap().parse().unwrap()
        })
        .collect();
    seeds.sort_unstable();
    seeds
}

const NAME: &str = "00100010";
const PATIENT_ID: &str = "00100020";
const DATE: &str = "00080020";
const TIME: &str = "00080030";
const ACCESSION: &str = "00080050";
const MODALITIES: &str = "00080061";
const STUDY_UID: &str = "0020000D";
const STUDY_ID: &str = "00200010";
const REFERRER: &str = "00080090";

#[tokio::test]
async fn studies_by_every_kind_of_match() {
    for index in common::indexes().await {
        five(&index).await;
        for (keys, expected, what) in [
            // names: without regard to case, single value and wild cards
            (
                vec![key(NAME, Kind::Name, "doe^jane")],
                vec![201],
                "a name, in another case",
            ),
            (
                vec![key(NAME, Kind::Name, "DOE*")],
                vec![201, 202],
                "names that start",
            ),
            (
                vec![key(NAME, Kind::Name, "*^jane")],
                vec![201, 203],
                "names that end",
            ),
            (
                vec![key(NAME, Kind::Name, "?oe^*")],
                vec![201, 202, 203],
                "one character",
            ),
            (
                vec![key(NAME, Kind::Name, "Doe")],
                vec![],
                "a single value is the whole value",
            ),
            // what a database's own patterns would trip on is matched as itself
            (
                vec![key(NAME, Kind::Name, "Do_e%^*")],
                vec![204],
                "an underscore and a percent",
            ),
            (
                vec![key(NAME, Kind::Name, "Do?e?^Odd")],
                vec![204],
                "wild cards over them",
            ),
            // text: case-sensitive
            (
                vec![key(PATIENT_ID, Kind::Text, "SYN201")],
                vec![201],
                "an identifier",
            ),
            (
                vec![key(PATIENT_ID, Kind::Text, "syn201")],
                vec![],
                "in another case",
            ),
            (
                vec![key(PATIENT_ID, Kind::Text, "SYN20?")],
                vec![201, 202, 203, 204, 205],
                "identifiers",
            ),
            (
                vec![key(PATIENT_ID, Kind::Text, "syn20?")],
                vec![],
                "a pattern in another case",
            ),
            (
                vec![key(ACCESSION, Kind::Text, "ACC20[1-3]")],
                vec![],
                "no character classes",
            ),
            (
                vec![key(STUDY_ID, Kind::Text, "S20*")],
                vec![201, 202, 203, 204, 205],
                "an attribute the index has no column for",
            ),
            (
                vec![key(REFERRER, Kind::Name, "referring^*")],
                vec![201, 202, 203, 204, 205],
                "a name the index has no column for",
            ),
            // UIDs: one, or a list
            (
                vec![key(STUDY_UID, Kind::Uid, "2.25.203100000")],
                vec![203],
                "a UID",
            ),
            (
                vec![key(
                    STUDY_UID,
                    Kind::Uid,
                    "2.25.203100000,2.25.205100000,2.25.9",
                )],
                vec![203, 205],
                "a list of UIDs",
            ),
            // dates
            (
                vec![key(DATE, Kind::Date, "20260105")],
                vec![201, 202],
                "a date",
            ),
            (
                vec![key(DATE, Kind::Date, "20260105-20260106")],
                vec![201, 202, 203],
                "a range of dates",
            ),
            (
                vec![key(DATE, Kind::Date, "20260107-")],
                vec![204, 205],
                "from a date",
            ),
            (
                vec![key(DATE, Kind::Date, "-20260105")],
                vec![201, 202],
                "to a date",
            ),
            // times, by meaning
            (vec![key(TIME, Kind::Time, "1000")], vec![201], "a minute"),
            (
                vec![key(TIME, Kind::Time, "0900-1200")],
                vec![201, 203, 204],
                "a range of times",
            ),
            (
                vec![key(TIME, Kind::Time, "18-")],
                vec![202, 205],
                "from an hour",
            ),
            // modalities: any series of the study
            (
                vec![key(MODALITIES, Kind::Text, "MR")],
                vec![203],
                "a modality",
            ),
            (
                vec![key(MODALITIES, Kind::Text, "SR")],
                vec![201],
                "the modality of a second series",
            ),
            (
                vec![key(MODALITIES, Kind::Text, "?R")],
                vec![201, 203],
                "modalities by pattern",
            ),
            (
                vec![key(MODALITIES, Kind::Text, "mr")],
                vec![],
                "a modality in another case",
            ),
            // several keys: all of them
            (
                vec![
                    key(NAME, Kind::Name, "doe*"),
                    key(DATE, Kind::Date, "20260105"),
                    key(TIME, Kind::Time, "18-"),
                ],
                vec![202],
                "three keys",
            ),
            (
                vec![
                    key(NAME, Kind::Name, "doe*"),
                    key(MODALITIES, Kind::Text, "MR"),
                ],
                vec![],
                "keys no study answers together",
            ),
        ] {
            assert_eq!(found(&index, keys, false).await, expected, "{what}");
        }
    }
}

#[tokio::test]
async fn a_range_of_dates_with_a_range_of_times_is_one_range_of_moments() {
    for index in common::indexes().await {
        five(&index).await;
        // a range of dates and a range of times, each from an end to an end
        let moments = |from: &str, to: &str| {
            let (d1, t1) = from.split_once(' ').unwrap();
            let (d2, t2) = to.split_once(' ').unwrap();
            vec![
                key(DATE, Kind::Date, &format!("{d1}-{d2}")),
                key(TIME, Kind::Time, &format!("{t1}-{t2}")),
            ]
        };
        // from the 5th at 11:00 to the 7th at 10:00: the evening of the 5th, the 6th, and
        // the morning of the 7th, but not the 5th at 10:00
        assert_eq!(
            found(&index, moments("20260105 1100", "20260107 1000"), true).await,
            [202, 203, 204]
        );
        // the same two ranges read apart: each day from 11:00 to 10:00, which no time is in,
        // and which a search that reads them apart says is wrong
        assert_eq!(
            found(&index, moments("20260105 1100", "20260107 1000"), false).await,
            Vec::<u32>::new()
        );
        let apart = StudySearch {
            keys: moments("20260105 1100", "20260107 1000"),
            combined_date_time: false,
            ..StudySearch::default()
        };
        assert!(
            apart
                .check()
                .unwrap_err()
                .contains("does not cross midnight")
        );
        let together = StudySearch {
            combined_date_time: true,
            ..apart.clone()
        };
        together.check().unwrap();
        // without a date range beside it, combined or not
        let alone = StudySearch {
            keys: vec![key(TIME, Kind::Time, "1100-1000")],
            combined_date_time: true,
            ..StudySearch::default()
        };
        assert!(alone.check().is_err());
        // from the 5th at 10:00 to the 5th at 18:00: the first study only, by a minute
        assert_eq!(
            found(&index, moments("20260105 1000", "20260105 1800"), true).await,
            [201]
        );
        // ranges of different forms are read apart, combined or not
        let apart = vec![
            key(DATE, Kind::Date, "20260105-"),
            key(TIME, Kind::Time, "0900-1200"),
        ];
        assert_eq!(found(&index, apart.clone(), true).await, [201, 203, 204]);
        assert_eq!(found(&index, apart, false).await, [201, 203, 204]);
    }
}

#[tokio::test]
async fn a_search_is_paged_after_matching_and_bounded_by_the_account() {
    for index in common::indexes().await {
        five(&index).await;
        let page = async |limit, offset| {
            let search = StudySearch {
                keys: vec![key(NAME, Kind::Name, "*oe^*")],
                limit,
                offset,
                ..StudySearch::default()
            };
            index.search_studies(&search).await.unwrap().len()
        };
        assert_eq!(page(None, 0).await, 3);
        assert_eq!(page(Some(2), 0).await, 2);
        assert_eq!(page(Some(2), 2).await, 1);
        assert_eq!(page(Some(2), 3).await, 0);
        // no key: every study, paged by the database
        let all = StudySearch {
            limit: Some(2),
            offset: 4,
            ..StudySearch::default()
        };
        assert_eq!(index.search_studies(&all).await.unwrap().len(), 1);

        // an account limited to projects none of these studies is in finds nothing
        index
            .set_account_projects("user:reader", Some(&["liver".to_owned()]))
            .await
            .unwrap();
        let reader = index.limited_to("user:reader");
        assert!(
            found(&reader, vec![key(DATE, Kind::Date, "20260105-")], false)
                .await
                .is_empty()
        );
        assert_eq!(
            found(&index, vec![key(DATE, Kind::Date, "20260105-")], false)
                .await
                .len(),
            5
        );
    }
}

/// Enough studies for a cursor to read the database more than once.
#[tokio::test]
async fn a_cursor_gives_every_match_once_and_no_other() {
    for index in common::indexes().await {
        for seed in 1000..1450 {
            let name = if seed % 3 == 0 { "Doe^Jane" } else { "Roe^Al" };
            // a scanner's CT, as a study of its own: its header is what
            // an index keeps
            let series = RealSeries::new(seed, 1).with_patient(name, None);
            let bytes = series.header_bytes(0);
            let instance = Instance::from_bytes(&bytes).unwrap();
            let new = NewInstance::new(&instance, format!("dicom/{seed}.dcm"), bytes.len());
            index.record(&new.unwrap()).await.unwrap();
        }
        let read = async |index: &Index, keys: Vec<Key>| {
            let mut cursor = index.study_cursor(StudySearch {
                keys,
                // a page is not a cursor's business
                limit: Some(1),
                offset: 7,
                ..StudySearch::default()
            });
            let mut uids = Vec::new();
            while let Some(study) = cursor.next().await.unwrap() {
                uids.push(study.study_instance_uid);
            }
            // the end stays the end
            assert!(cursor.next().await.unwrap().is_none());
            uids
        };
        let all = read(&index, Vec::new()).await;
        assert_eq!(all.len(), 450);
        assert!(
            all.windows(2).all(|pair| pair[0] < pair[1]),
            "in UID order, none twice"
        );

        let does = read(&index, vec![key(NAME, Kind::Name, "doe*")]).await;
        assert_eq!(does.len(), 150);
        let searched = index
            .search_studies(&StudySearch {
                keys: vec![key(NAME, Kind::Name, "doe*")],
                ..StudySearch::default()
            })
            .await
            .unwrap();
        let mut searched: Vec<String> =
            searched.into_iter().map(|s| s.study_instance_uid).collect();
        searched.sort();
        assert_eq!(does, searched, "the cursor and the search agree");

        // what is counted is matched too
        assert_eq!(
            read(&index, vec![key("00201208", Kind::Number, "1")])
                .await
                .len(),
            450
        );
        assert!(
            read(&index, vec![key("00201206", Kind::Number, "2")])
                .await
                .is_empty()
        );
        assert!(
            read(&index, vec![key(NAME, Kind::Name, "nobody")])
                .await
                .is_empty()
        );

        // bounded by the account, like every read
        index
            .set_account_projects("user:reader", Some(&["liver".to_owned()]))
            .await
            .unwrap();
        assert!(
            read(&index.limited_to("user:reader"), Vec::new())
                .await
                .is_empty()
        );
    }
}
