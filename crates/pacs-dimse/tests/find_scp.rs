//! pacs-rs as a Query SCP, driven by a bare dicom-ul client: C-FIND on the Study Root
//! model (PS3.4 C.4.1.3, C.6.2; PS3.7 9.1.2), its matching (C.2.2.2), its failures, who is
//! answered and what a project's AE title shows.

mod common;

use std::sync::Arc;

use common::*;
use dicom_core::value::{DataSetSequence, Value};
use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_deid::{Keys, Profile};
use pacs_dimse::command::{Command, NO_DATA_SET, field, tag};
use pacs_dimse::message::{self, Incoming};
use pacs_dimse::query::STUDY_ROOT_FIND;
use pacs_dimse::{AeTitle, Config, Destination, Project, Scp, Status};

/// A configuration that answers the queries of VIEWER.
fn answering() -> Config {
    let mut config = config();
    config.directory = pacs_dimse::Directory::new(
        vec!["viewer=VIEWER@127.0.0.1:11112".parse().unwrap()],
        vec![],
    );
    config
}

async fn viewer(addr: std::net::SocketAddr, called: &str) -> Client {
    Client::connect(
        addr,
        "VIEWER",
        called,
        &[STUDY_ROOT_FIND],
        &[EXPLICIT_VR_LE, IMPLICIT_VR_LE],
    )
    .await
    .unwrap()
}

/// Three studies: Doe's CT of two series (seeds 301 and 304), Roe's MR, Smith's CT.
async fn three_studies(archive: &Archive) -> [RealSeries; 4] {
    let series = [
        RealSeries::new(301, 2)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000"),
        RealSeries::new(302, 1)
            .with_patient("Roe^Richard", None)
            .at("20260106", "120000")
            .of_modality("MR"),
        RealSeries::new(303, 1)
            .with_patient("Smith^Al", None)
            .at("20260201", "235959"),
        RealSeries::new(304, 3)
            .in_study(301, 2)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000")
            .of_modality("SR"),
    ];
    for one in &series {
        ingest(archive, one).await;
    }
    series
}

/// The keys a viewer's study list asks for, all universal.
const STUDY_LIST: &[(Tag, VR, &str)] = &[
    (tags::STUDY_DATE, VR::DA, ""),
    (tags::STUDY_TIME, VR::TM, ""),
    (tags::ACCESSION_NUMBER, VR::SH, ""),
    (tags::MODALITIES_IN_STUDY, VR::CS, ""),
    (tags::PATIENT_NAME, VR::PN, ""),
    (tags::PATIENT_ID, VR::LO, ""),
    (tags::STUDY_INSTANCE_UID, VR::UI, ""),
    (tags::STUDY_ID, VR::SH, ""),
    (tags::NUMBER_OF_STUDY_RELATED_SERIES, VR::IS, ""),
    (tags::NUMBER_OF_STUDY_RELATED_INSTANCES, VR::IS, ""),
];

/// `keys` over the study list's, a key given here replacing its universal one.
fn study_query(keys: &[(Tag, VR, &str)]) -> InMemDicomObject {
    let mut all: Vec<(Tag, VR, &str)> = STUDY_LIST
        .iter()
        .filter(|(tag, ..)| !keys.iter().any(|(given, ..)| given == tag))
        .copied()
        .collect();
    all.extend_from_slice(keys);
    identifier("STUDY", &all)
}

fn sorted(mut values: Vec<Option<String>>) -> Vec<String> {
    values.sort();
    values.into_iter().map(Option::unwrap_or_default).collect()
}

/// PS3.4 C.4.1.3.1: a Pending response for each match, each with one match's values, then
/// Success without an Identifier; the Response Identifier of C.4.1.1.3.2.
#[tokio::test]
async fn a_study_query_answers_one_pending_response_per_study() {
    let archive = archive().await;
    let [doe, ..] = three_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;

    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING; 3]);
    assert_eq!(
        sorted(answer.values(tags::PATIENT_NAME)),
        ["Doe^Jane", "Roe^Richard", "Smith^Al"]
    );
    let found = answer
        .matches
        .iter()
        .find(|found| value(found, tags::STUDY_INSTANCE_UID) == Some(doe.study_uid()))
        .unwrap();
    for (tag, expected) in [
        (tags::QUERY_RETRIEVE_LEVEL, "STUDY"),
        // found here, retrieved from here
        (tags::RETRIEVE_AE_TITLE, "PACSRS"),
        (tags::STUDY_DATE, "20260105"),
        (tags::STUDY_TIME, "100000"),
        (tags::ACCESSION_NUMBER, "ACC301"),
        (tags::PATIENT_ID, "NEMA-301"),
        (tags::STUDY_ID, "S301"),
        (tags::MODALITIES_IN_STUDY, "CT\\SR"),
        (tags::NUMBER_OF_STUDY_RELATED_SERIES, "2"),
        (tags::NUMBER_OF_STUDY_RELATED_INSTANCES, "5"),
    ] {
        assert_eq!(value(found, tag).as_deref(), Some(expected), "{tag}");
    }
    // nothing that was not asked for (C.4.1.1.3.2), and no character set for plain text
    assert!(found.get(tags::STUDY_DESCRIPTION).is_none());
    assert!(found.get(tags::PATIENT_BIRTH_DATE).is_none());
    assert!(found.get(tags::SPECIFIC_CHARACTER_SET).is_none());

    // no match: Success alone
    let none = client
        .find(
            STUDY_ROOT_FIND,
            &study_query(&[(tags::PATIENT_ID, VR::LO, "NOBODY")]),
        )
        .await;
    assert_eq!(none.status(), Status::SUCCESS);
    assert!(none.matches.is_empty());
    assert!(none.last.get(tag::ERROR_COMMENT).is_none());
    client.release().await;
}

/// PS3.4 C.2.2.2: single value, wild card, range, list of UID and universal matching, on
/// the keys of the Study level.
#[tokio::test]
async fn study_keys_are_matched_by_their_kind() {
    let archive = archive().await;
    let [doe, roe, smith, _] = three_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let uids = format!("{}\\{}", roe.study_uid(), smith.study_uid());
    for (keys, expected, why) in [
        (
            vec![(tags::PATIENT_ID, VR::LO, "NEMA-302")],
            vec![&roe],
            "single value",
        ),
        (
            vec![(tags::PATIENT_ID, VR::LO, "nema-302")],
            vec![],
            "text is case-sensitive",
        ),
        (
            vec![(tags::PATIENT_NAME, VR::PN, "doe^*")],
            vec![&doe],
            "a name, without regard to case",
        ),
        (
            vec![(tags::PATIENT_NAME, VR::PN, "?oe^*")],
            vec![&doe, &roe],
            "wild cards",
        ),
        (
            vec![(tags::STUDY_DATE, VR::DA, "20260105-20260106")],
            vec![&doe, &roe],
            "a range of dates",
        ),
        (
            vec![(tags::STUDY_DATE, VR::DA, "20260106-")],
            vec![&roe, &smith],
            "an open range",
        ),
        (
            vec![(tags::STUDY_TIME, VR::TM, "1000-1200")],
            vec![&doe, &roe],
            "a range of times",
        ),
        (
            vec![(tags::STUDY_INSTANCE_UID, VR::UI, uids.as_str())],
            vec![&roe, &smith],
            "a list of UID",
        ),
        (
            vec![(tags::MODALITIES_IN_STUDY, VR::CS, "SR")],
            vec![&doe],
            "any modality of the study",
        ),
        (
            vec![(tags::NUMBER_OF_STUDY_RELATED_INSTANCES, VR::IS, "1")],
            vec![&roe, &smith],
            "what is counted",
        ),
        (
            vec![
                (tags::PATIENT_NAME, VR::PN, "*oe^*"),
                (tags::MODALITIES_IN_STUDY, VR::CS, "MR"),
            ],
            vec![&roe],
            "every key at once",
        ),
        (
            vec![(tags::PATIENT_NAME, VR::PN, "*")],
            vec![&doe, &roe, &smith],
            "a lone star asks nothing",
        ),
    ] {
        let answer = client.find(STUDY_ROOT_FIND, &study_query(&keys)).await;
        assert_eq!(answer.status(), Status::SUCCESS, "{why}");
        let mut expected: Vec<String> = expected.iter().map(|s| s.study_uid()).collect();
        expected.sort();
        assert_eq!(
            sorted(answer.values(tags::STUDY_INSTANCE_UID)),
            expected,
            "{why}"
        );
    }
    client.release().await;
}

/// PS3.4 C.4.1.3.1.1, the hierarchical search: a study's series, then a series' instances,
/// each response with the Unique Keys of the levels above.
#[tokio::test]
async fn series_and_instances_are_found_under_their_unique_keys() {
    let archive = archive().await;
    let [doe, roe, _, report] = three_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let study = doe.study_uid();

    let series = |keys: &[(Tag, VR, &str)]| {
        let mut all = vec![
            (tags::STUDY_INSTANCE_UID, VR::UI, study.as_str()),
            (tags::SERIES_INSTANCE_UID, VR::UI, ""),
            (tags::SERIES_NUMBER, VR::IS, ""),
            (tags::NUMBER_OF_SERIES_RELATED_INSTANCES, VR::IS, ""),
        ];
        all.extend_from_slice(keys);
        identifier("SERIES", &all)
    };
    let answer = client
        .find(STUDY_ROOT_FIND, &series(&[(tags::MODALITY, VR::CS, "")]))
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING; 2]);
    // by Series Number
    assert_eq!(
        answer.values(tags::SERIES_INSTANCE_UID),
        [Some(doe.series_uid()), Some(report.series_uid())]
    );
    assert_eq!(
        answer.values(tags::MODALITY),
        [Some("CT".into()), Some("SR".into())]
    );
    assert_eq!(
        answer.values(tags::NUMBER_OF_SERIES_RELATED_INSTANCES),
        [Some("2".into()), Some("3".into())]
    );
    for found in &answer.matches {
        assert_eq!(value(found, tags::QUERY_RETRIEVE_LEVEL).unwrap(), "SERIES");
        assert_eq!(value(found, tags::STUDY_INSTANCE_UID).unwrap(), study);
    }
    let reports = client
        .find(STUDY_ROOT_FIND, &series(&[(tags::MODALITY, VR::CS, "S?")]))
        .await;
    assert_eq!(
        reports.values(tags::SERIES_INSTANCE_UID),
        [Some(report.series_uid())]
    );
    let second = client
        .find(
            STUDY_ROOT_FIND,
            &series(&[(tags::SERIES_NUMBER, VR::IS, "2")]),
        )
        .await;
    assert_eq!(
        second.values(tags::SERIES_INSTANCE_UID),
        [Some(report.series_uid())]
    );

    let report_series = report.series_uid();
    let instances = |keys: &[(Tag, VR, &str)]| {
        let mut all = vec![
            (tags::STUDY_INSTANCE_UID, VR::UI, study.as_str()),
            (tags::SERIES_INSTANCE_UID, VR::UI, report_series.as_str()),
            (tags::SOP_INSTANCE_UID, VR::UI, ""),
            (tags::SOP_CLASS_UID, VR::UI, ""),
            (tags::ROWS, VR::US, ""),
        ];
        all.extend_from_slice(keys);
        identifier("IMAGE", &all)
    };
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &instances(&[(tags::INSTANCE_NUMBER, VR::IS, "")]),
        )
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(
        answer.values(tags::SOP_INSTANCE_UID),
        [0, 1, 2].map(|slice| Some(report.instance_uid(slice)))
    );
    assert_eq!(
        answer.values(tags::INSTANCE_NUMBER),
        ["1", "2", "3"].map(|n| Some(n.to_owned()))
    );
    for found in &answer.matches {
        assert_eq!(value(found, tags::QUERY_RETRIEVE_LEVEL).unwrap(), "IMAGE");
        assert_eq!(value(found, tags::STUDY_INSTANCE_UID).unwrap(), study);
        assert_eq!(
            value(found, tags::SERIES_INSTANCE_UID).unwrap(),
            report_series
        );
        assert_eq!(value(found, tags::SOP_CLASS_UID).unwrap(), CT);
        assert_eq!(found.get(tags::ROWS).unwrap().to_int::<u16>().unwrap(), 128);
    }
    let one = client
        .find(
            STUDY_ROOT_FIND,
            &instances(&[(tags::INSTANCE_NUMBER, VR::IS, "2")]),
        )
        .await;
    assert_eq!(
        one.values(tags::SOP_INSTANCE_UID),
        [Some(report.instance_uid(1))]
    );

    // a series that is in another study, and a study that is not there: no match
    let elsewhere = identifier(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, VR::UI, &roe.study_uid()),
            (tags::SERIES_INSTANCE_UID, VR::UI, &report_series),
            (tags::SOP_INSTANCE_UID, VR::UI, ""),
        ],
    );
    let nowhere = identifier(
        "SERIES",
        &[
            (tags::STUDY_INSTANCE_UID, VR::UI, "2.25.999"),
            (tags::SERIES_INSTANCE_UID, VR::UI, ""),
        ],
    );
    for query in [elsewhere, nowhere] {
        let answer = client.find(STUDY_ROOT_FIND, &query).await;
        assert_eq!(answer.status(), Status::SUCCESS);
        assert!(answer.matches.is_empty());
    }
    client.release().await;
}

/// A sequence of one item holding `keys`.
fn sequence(tag: Tag, items: Vec<Vec<(Tag, VR, &str)>>) -> DataElement<InMemDicomObject> {
    let items: Vec<InMemDicomObject> = items
        .into_iter()
        .map(|keys| {
            InMemDicomObject::from_element_iter(
                keys.into_iter()
                    .map(|(tag, vr, value)| DataElement::new(tag, vr, PrimitiveValue::from(value))),
            )
        })
        .collect();
    DataElement::new(tag, VR::SQ, Value::Sequence(DataSetSequence::from(items)))
}

/// A slice of `series` with these requests in its Request Attributes Sequence, as
/// (Scheduled Procedure Step ID, Requested Procedure ID).
fn with_requests(series: &RealSeries, requests: &[(&str, &str)]) -> Vec<u8> {
    let mut object = dicom_object::from_reader(&*series.slice_bytes(0)).unwrap();
    object.put(sequence(
        tags::REQUEST_ATTRIBUTES_SEQUENCE,
        requests
            .iter()
            .map(|(step, procedure)| {
                vec![
                    (tags::SCHEDULED_PROCEDURE_STEP_ID, VR::SH, *step),
                    (tags::REQUESTED_PROCEDURE_ID, VR::SH, *procedure),
                    (tags::SCHEDULED_PROCEDURE_STEP_DESCRIPTION, VR::LO, "Liver"),
                ]
            })
            .collect(),
    ));
    let mut bytes = Vec::new();
    object.write_all(&mut bytes).unwrap();
    bytes
}

/// PS3.4 C.2.2.2.6: a sequence key of one item; a series matches when one of its items
/// answers every key of it, and the matching items come back with the attributes asked for.
#[tokio::test]
async fn a_sequence_is_matched_by_the_keys_of_one_item() {
    let archive = archive().await;
    let first = RealSeries::new(311, 1);
    let second = RealSeries::new(312, 1).in_study(311, 2);
    let bare = RealSeries::new(313, 1).in_study(311, 3);
    for file in [
        with_requests(&first, &[("STEP1", "REQ1"), ("STEP2", "REQ2")]),
        with_requests(&second, &[("STEP3", "REQ1")]),
        bare.slice_bytes(0),
    ] {
        archive.ingest(file.into()).await.unwrap();
    }
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let study = first.study_uid();
    let query = |item: Vec<Vec<(Tag, VR, &'static str)>>| {
        let mut query = identifier(
            "SERIES",
            &[
                (tags::STUDY_INSTANCE_UID, VR::UI, &study),
                (tags::SERIES_INSTANCE_UID, VR::UI, ""),
            ],
        );
        query.put(sequence(tags::REQUEST_ATTRIBUTES_SEQUENCE, item));
        query
    };
    let requests = |found: &InMemDicomObject| -> Vec<(Option<String>, Option<String>, usize)> {
        found
            .get(tags::REQUEST_ATTRIBUTES_SEQUENCE)
            .unwrap()
            .items()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    value(item, tags::SCHEDULED_PROCEDURE_STEP_ID),
                    value(item, tags::REQUESTED_PROCEDURE_ID),
                    item.iter().count(),
                )
            })
            .collect()
    };
    let text = |s: &str| Some(s.to_owned());

    // one key: the series with such an item, and of its items the one that matches
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &query(vec![vec![
                (tags::SCHEDULED_PROCEDURE_STEP_ID, VR::SH, "STEP2"),
                (tags::REQUESTED_PROCEDURE_ID, VR::SH, ""),
            ]]),
        )
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING]);
    assert_eq!(
        answer.values(tags::SERIES_INSTANCE_UID),
        [Some(first.series_uid())]
    );
    assert_eq!(
        requests(&answer.matches[0]),
        [(text("STEP2"), text("REQ2"), 2)]
    );

    // two series have a request REQ1
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &query(vec![vec![(tags::REQUESTED_PROCEDURE_ID, VR::SH, "REQ1")]]),
        )
        .await;
    assert_eq!(
        answer.values(tags::SERIES_INSTANCE_UID),
        [Some(first.series_uid()), Some(second.series_uid())]
    );
    assert_eq!(requests(&answer.matches[0]), [(None, text("REQ1"), 1)]);

    // each key has its item, but no item has both
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &query(vec![vec![
                (tags::SCHEDULED_PROCEDURE_STEP_ID, VR::SH, "STEP1"),
                (tags::REQUESTED_PROCEDURE_ID, VR::SH, "REQ2"),
            ]]),
        )
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert!(answer.matches.is_empty());

    // no item, or an empty one: every series, with the sequence whole, empty where the
    // series has none
    for universal in [vec![], vec![vec![]]] {
        let answer = client.find(STUDY_ROOT_FIND, &query(universal)).await;
        assert_eq!(answer.pending, [Status::PENDING; 3]);
        assert_eq!(
            requests(&answer.matches[0]),
            [
                (text("STEP1"), text("REQ1"), 3),
                (text("STEP2"), text("REQ2"), 3)
            ]
        );
        assert!(requests(&answer.matches[2]).is_empty());
    }

    // a key of the item that pacs-rs does not have: left out, and said (FF01)
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &query(vec![vec![
                (tags::SCHEDULED_PROCEDURE_STEP_ID, VR::SH, "STEP3"),
                (tags::SCHEDULED_PROCEDURE_STEP_DESCRIPTION, VR::LO, ""),
            ]]),
        )
        .await;
    assert_eq!(answer.pending, [Status::PENDING_OPTIONAL_KEYS]);
    assert_eq!(requests(&answer.matches[0]), [(text("STEP3"), None, 1)]);

    // two items are not a key
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &query(vec![
                vec![(tags::SCHEDULED_PROCEDURE_STEP_ID, VR::SH, "STEP1")],
                vec![(tags::SCHEDULED_PROCEDURE_STEP_ID, VR::SH, "STEP2")],
            ]),
        )
        .await;
    assert_eq!(answer.status(), Status::UNABLE_TO_PROCESS);
    client.release().await;
}

/// PS3.4 C.2.2.1.3 and Table C.4-1: a key pacs-rs does not have at the level is not in the
/// responses, whose status says so; with every key supported they are FF00.
#[tokio::test]
async fn a_key_that_is_not_supported_is_left_out_and_said() {
    let archive = archive().await;
    three_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &study_query(&[
                (tags::PATIENT_ID, VR::LO, "NEMA-302"),
                // an Optional Key of the level that pacs-rs does not keep
                (tags::ADMITTING_DIAGNOSES_DESCRIPTION, VR::LO, ""),
                // a key of a lower level
                (tags::MODALITY, VR::CS, "CT"),
                // a private attribute is ignored without a word
                (Tag(0x0009, 0x0010), VR::LO, "SOMEBODY"),
            ]),
        )
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING_OPTIONAL_KEYS]);
    let found = &answer.matches[0];
    assert_eq!(value(found, tags::PATIENT_ID).unwrap(), "NEMA-302");
    assert!(found.get(tags::ADMITTING_DIAGNOSES_DESCRIPTION).is_none());
    assert!(found.get(tags::MODALITY).is_none());
    assert!(found.get(Tag(0x0009, 0x0010)).is_none());
    client.release().await;
}

/// A key the level has, for which a study has no value: there, empty (C.4.1.1.3.2).
#[tokio::test]
async fn a_key_without_a_value_comes_back_empty() {
    let archive = archive().await;
    let series = RealSeries::new(321, 1);
    let mut object = dicom_object::from_reader(&*series.slice_bytes(0)).unwrap();
    object.remove_element(tags::ACCESSION_NUMBER);
    let mut bytes = Vec::new();
    object.write_all(&mut bytes).unwrap();
    archive.ingest(bytes.into()).await.unwrap();
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.pending, [Status::PENDING]);
    let accession = answer.matches[0].get(tags::ACCESSION_NUMBER).unwrap();
    assert_eq!(accession.to_str().unwrap(), "");
    // and such a study has no value to answer a key with
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &study_query(&[(tags::ACCESSION_NUMBER, VR::SH, "ACC*")]),
        )
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert!(answer.matches.is_empty());
    client.release().await;
}

/// Text that is not ASCII comes back in UTF-8, with the character set that says so, and is
/// matched as text whatever the file was written in.
#[tokio::test]
async fn a_name_outside_ascii_is_answered_in_utf_8() {
    let archive = archive().await;
    ingest(
        &archive,
        &RealSeries::new(331, 1).with_patient("Müller^Jürgen", Some("ISO_IR 100")),
    )
    .await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let mut query = study_query(&[(tags::PATIENT_NAME, VR::PN, "Müller*")]);
    query.put(DataElement::new(
        tags::SPECIFIC_CHARACTER_SET,
        VR::CS,
        PrimitiveValue::from("ISO_IR 192"),
    ));
    let mut bytes = Vec::new();
    query
        .write_dataset_with_ts_cs(
            &mut bytes,
            client.transfer_syntax(STUDY_ROOT_FIND),
            dicom_encoding::text::SpecificCharacterSet::ISO_IR_192,
        )
        .unwrap();
    let answer = client.find_bytes(STUDY_ROOT_FIND, &bytes).await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING]);
    let found = &answer.matches[0];
    assert_eq!(
        value(found, tags::SPECIFIC_CHARACTER_SET).unwrap(),
        "ISO_IR 192"
    );
    assert_eq!(value(found, tags::PATIENT_NAME).unwrap(), "Müller^Jürgen");
    client.release().await;
}

/// The Failure statuses of PS3.4 Table C.4-1, each with a comment and no Identifier, and
/// the association going on after each.
#[tokio::test]
async fn an_identifier_that_cannot_be_answered_is_a_failure() {
    let archive = archive().await;
    let [doe, ..] = three_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let study = doe.study_uid();
    let two_studies = format!("{study}\\2.25.5");
    let mut no_level = study_query(&[]);
    no_level.remove_element(tags::QUERY_RETRIEVE_LEVEL);
    for (query, status, offending, why) in [
        (
            no_level,
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            Some(tags::QUERY_RETRIEVE_LEVEL),
            "no Query/Retrieve Level",
        ),
        (
            identifier("PATIENT", &[(tags::PATIENT_ID, VR::LO, "")]),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            Some(tags::QUERY_RETRIEVE_LEVEL),
            "a level the Study Root model lacks",
        ),
        (
            identifier("SERIES", &[(tags::SERIES_INSTANCE_UID, VR::UI, "")]),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            Some(tags::STUDY_INSTANCE_UID),
            "no Unique Key of the level above",
        ),
        (
            identifier(
                "SERIES",
                &[(tags::STUDY_INSTANCE_UID, VR::UI, &two_studies)],
            ),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            Some(tags::STUDY_INSTANCE_UID),
            "two values in the Unique Key of the level above",
        ),
        (
            identifier("IMAGE", &[(tags::STUDY_INSTANCE_UID, VR::UI, &study)]),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            Some(tags::SERIES_INSTANCE_UID),
            "no Series Instance UID at the IMAGE level",
        ),
        (
            study_query(&[(tags::STUDY_DATE, VR::DA, "2026-01-05")]),
            Status::UNABLE_TO_PROCESS,
            Some(tags::STUDY_DATE),
            "a date that is not one",
        ),
        (
            study_query(&[(tags::STUDY_DATE, VR::DA, "20260201-20260101")]),
            Status::UNABLE_TO_PROCESS,
            Some(tags::STUDY_DATE),
            "a range that ends before it starts",
        ),
        (
            study_query(&[(tags::STUDY_TIME, VR::TM, "1800-1000")]),
            Status::UNABLE_TO_PROCESS,
            None,
            "a range of times across midnight",
        ),
        (
            study_query(&[(tags::STUDY_INSTANCE_UID, VR::UI, "1.2.*")]),
            Status::UNABLE_TO_PROCESS,
            Some(tags::STUDY_INSTANCE_UID),
            "a wild card in a UID",
        ),
        (
            study_query(&[(tags::MODALITIES_IN_STUDY, VR::CS, "CT\\MR")]),
            Status::UNABLE_TO_PROCESS,
            Some(tags::MODALITIES_IN_STUDY),
            "two values where multiple value matching is not negotiated",
        ),
    ] {
        let answer = client.find(STUDY_ROOT_FIND, &query).await;
        assert_eq!(answer.status(), status, "{why}");
        assert!(answer.matches.is_empty(), "{why}");
        let comment = answer.last.text(tag::ERROR_COMMENT).unwrap_or_default();
        assert!(!comment.is_empty(), "{why}");
        let offending_element = answer.last.get(tag::OFFENDING_ELEMENT).map(|bytes| {
            Tag(
                u16::from_le_bytes([bytes[0], bytes[1]]),
                u16::from_le_bytes([bytes[2], bytes[3]]),
            )
        });
        assert_eq!(offending_element, offending, "{why}: {comment}");
    }
    // bytes that are not a data set
    let answer = client
        .find_bytes(STUDY_ROOT_FIND, &[0x08, 0x00, 0x52, 0x00, 0xFF, 0xFF])
        .await;
    assert_eq!(answer.status(), Status::UNABLE_TO_PROCESS);
    // a request without an Identifier
    let id = client.message_id();
    let context = client.context(STUDY_ROOT_FIND);
    let rsp = client
        .response(context, &no_data(c_find_rq(id, STUDY_ROOT_FIND)), None)
        .await;
    assert_eq!(
        rsp.status(),
        Ok(Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS)
    );
    // the association still answers
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.pending.len(), 3);
    client.release().await;
}

/// Query and retrieve are answered to the AEs the server is configured with; another
/// caller is told it is not authorized (PS3.7 9.1.2.1.6, 0124H), and can still echo.
#[tokio::test]
async fn a_caller_that_is_not_configured_is_not_answered() {
    let archive = archive().await;
    three_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut stranger = Client::connect(
        addr,
        "STRANGER",
        "PACSRS",
        &[STUDY_ROOT_FIND, pacs_dimse::VERIFICATION],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap();
    let answer = stranger.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.status(), Status::NOT_AUTHORIZED);
    assert!(answer.matches.is_empty());
    assert!(
        answer
            .last
            .text(tag::ERROR_COMMENT)
            .unwrap()
            .contains("STRANGER")
    );
    assert_eq!(stranger.echo().await.status(), Ok(Status::SUCCESS));
    stranger.release().await;

    // with no AE configured, nobody is
    let unconfigured = start(config(), archive_with_a_study().await).await;
    let mut client = viewer(unconfigured, "PACSRS").await;
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.status(), Status::NOT_AUTHORIZED);
    client.release().await;
}

async fn archive_with_a_study() -> Archive {
    let archive = archive().await;
    ingest(&archive, &RealSeries::new(341, 1)).await;
    archive
}

/// Rights per project: an association to a project's AE title sees that project's
/// studies and no other, at every level; one to the server's own sees them all.
#[tokio::test]
async fn a_project_ae_title_shows_only_its_project() {
    let archive = archive().await;
    let received = RealSeries::new(351, 1).with_patient("Identified^Ida", None);
    ingest(&archive, &received).await;
    let liver = archive.clone().for_project("liver-2026");
    let in_liver = RealSeries::new(352, 1).with_patient("PSEUDO-1", None);
    ingest(&liver, &in_liver).await;
    let lung = archive.clone().for_project("lung-2026");
    ingest(
        &lung,
        &RealSeries::new(353, 1).with_patient("PSEUDO-2", None),
    )
    .await;
    let project = |ae_title: &str, name: &str, archive: &Archive| {
        let profile = Profile::from_json(&format!(r#"{{"name": "{name}"}}"#)).unwrap();
        (
            AeTitle::new(ae_title).unwrap(),
            Destination::Project(Arc::new(Project {
                ae_title: AeTitle::new(ae_title).unwrap(),
                profile,
                keys: Keys::new(*b"a project secret of 32 bytes ok!").unwrap(),
                archive: archive.clone(),
                quarantine: None,
            })),
        )
    };
    let mut scp = Scp::new(answering(), Destination::Archive(archive.clone()));
    for (ae_title, destination) in [
        project("LIVER_2026", "liver-2026", &liver),
        project("LUNG_2026", "lung-2026", &lung),
    ] {
        scp = scp.with_route(ae_title, destination);
    }
    let addr = start_scp(scp).await;

    let mut client = viewer(addr, "LIVER_2026").await;
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(
        answer.values(tags::PATIENT_NAME),
        [Some("PSEUDO-1".to_owned())]
    );
    assert_eq!(
        answer.values(tags::RETRIEVE_AE_TITLE),
        [Some("LIVER_2026".to_owned())]
    );
    // a study outside the project is not there, asked for by name or looked into
    for outside in [&received, &RealSeries::new(353, 1)] {
        let study = outside.study_uid();
        let by_uid = client
            .find(
                STUDY_ROOT_FIND,
                &study_query(&[(tags::STUDY_INSTANCE_UID, VR::UI, &study)]),
            )
            .await;
        assert_eq!(by_uid.status(), Status::SUCCESS);
        assert!(by_uid.matches.is_empty());
        let series = identifier(
            "SERIES",
            &[
                (tags::STUDY_INSTANCE_UID, VR::UI, &study),
                (tags::SERIES_INSTANCE_UID, VR::UI, ""),
            ],
        );
        assert!(
            client
                .find(STUDY_ROOT_FIND, &series)
                .await
                .matches
                .is_empty()
        );
        let instances = identifier(
            "IMAGE",
            &[
                (tags::STUDY_INSTANCE_UID, VR::UI, &study),
                (tags::SERIES_INSTANCE_UID, VR::UI, &outside.series_uid()),
                (tags::SOP_INSTANCE_UID, VR::UI, ""),
            ],
        );
        assert!(
            client
                .find(STUDY_ROOT_FIND, &instances)
                .await
                .matches
                .is_empty()
        );
    }
    // its own study is, down to the instance
    let instances = identifier(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, VR::UI, &in_liver.study_uid()),
            (tags::SERIES_INSTANCE_UID, VR::UI, &in_liver.series_uid()),
            (tags::SOP_INSTANCE_UID, VR::UI, ""),
        ],
    );
    assert_eq!(
        client
            .find(STUDY_ROOT_FIND, &instances)
            .await
            .values(tags::SOP_INSTANCE_UID),
        [Some(in_liver.instance_uid(0))]
    );
    client.release().await;

    let mut client = viewer(addr, "PACSRS").await;
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(
        sorted(answer.values(tags::PATIENT_NAME)),
        ["Identified^Ida", "PSEUDO-1", "PSEUDO-2"]
    );
    client.release().await;
}

fn c_cancel_rq(message_id: u16) -> Command {
    Command::new()
        .with_us(tag::COMMAND_FIELD, field::C_CANCEL_RQ)
        .with_us(tag::MESSAGE_ID_BEING_RESPONDED_TO, message_id)
        .with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET)
}

/// PS3.4 C.4.1.3.1: a C-CANCEL-FIND-RQ interrupts the matching, which ends with status
/// Cancel and without the rest of the matches. The server is held in the middle of its
/// answer until the cancel is sent ([`gate`]), so this does not depend on who is faster.
#[tokio::test]
async fn a_cancel_in_the_middle_ends_with_status_cancel() {
    const STUDIES: u32 = 2500;
    let archive = archive().await;
    for seed in 0..STUDIES {
        index_only(&archive, &RealSeries::new(10_000 + seed, 1)).await;
    }
    let server = start(answering(), archive).await;
    // the association's acceptance and a few matches pass; the rest waits for the second
    // of the two cancels below
    let addr = gate(server, 2048, 3).await;
    let mut client = viewer(addr, "PACSRS").await;
    let id = client.message_id();
    let context = client.context(STUDY_ROOT_FIND);
    let mut query = Vec::new();
    study_query(&[])
        .write_dataset_with_ts(&mut query, client.transfer_syntax(STUDY_ROOT_FIND))
        .unwrap();
    message::send(
        &mut client.association,
        context,
        &c_find_rq(id, STUDY_ROOT_FIND),
        Some(&query),
    )
    .await
    .unwrap();
    // a first match arrives: the matching is under way
    match message::receive(&mut client.association, 1 << 20)
        .await
        .unwrap()
    {
        Incoming::Message(first) => assert_eq!(first.command.status(), Ok(Status::PENDING)),
        other => panic!("expected a match, got {other:?}"),
    }
    // a cancel of another operation changes nothing; then the one of this C-FIND
    for cancelled in [id + 100, id] {
        message::send(
            &mut client.association,
            context,
            &c_cancel_rq(cancelled),
            None,
        )
        .await
        .unwrap();
    }
    let answer = client.answer(STUDY_ROOT_FIND).await;
    assert_eq!(answer.status(), Status::CANCEL);
    assert_eq!(answer.last.command_field(), Ok(field::C_FIND_RSP));
    assert_eq!(answer.last.us(tag::MESSAGE_ID_BEING_RESPONDED_TO), Ok(id));
    let sent = answer.matches.len() + 1;
    assert!(
        sent < STUDIES as usize,
        "the matching went to its end: {sent} matches"
    );
    // the association is whole: the next query is answered to its end
    let one = client
        .find(
            STUDY_ROOT_FIND,
            &study_query(&[(tags::PATIENT_ID, VR::LO, "NEMA-10007")]),
        )
        .await;
    assert_eq!(one.status(), Status::SUCCESS);
    assert_eq!(one.pending, [Status::PENDING]);
    client.release().await;
}

/// Between two matches the server looks for a cancel without waiting. A look that waited
/// for the runtime's next timer tick cost a millisecond a match: 1500 matches then took
/// over 1.5 s, whatever the machine, and take a fraction of that without.
#[tokio::test]
async fn matches_follow_each_other_without_a_pause() {
    const STUDIES: u32 = 1500;
    let archive = archive().await;
    for seed in 0..STUDIES {
        index_only(&archive, &RealSeries::new(20_000 + seed, 1)).await;
    }
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let started = std::time::Instant::now();
    let all = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    let took = started.elapsed();
    assert_eq!(all.status(), Status::SUCCESS);
    assert_eq!(all.matches.len(), STUDIES as usize);
    // a millisecond a match is the pause this guards against
    assert!(
        took < std::time::Duration::from_millis(u64::from(STUDIES)),
        "{STUDIES} matches took {took:?}: a pause between them?"
    );
    eprintln!("{STUDIES} matches in {took:?}");
    client.release().await;
}

/// A cancel that comes when nothing is being answered asks for nothing (PS3.7 9.3.2.3).
#[tokio::test]
async fn a_cancel_after_the_final_response_is_ignored() {
    let addr = start(answering(), archive_with_a_study().await).await;
    let mut client = viewer(addr, "PACSRS").await;
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.status(), Status::SUCCESS);
    let context = client.context(STUDY_ROOT_FIND);
    message::send(&mut client.association, context, &c_cancel_rq(2), None)
        .await
        .unwrap();
    let answer = client.find(STUDY_ROOT_FIND, &study_query(&[])).await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending.len(), 1);
    client.release().await;
}

/// Another request while a C-FIND is being answered: no asynchronous operations window is
/// negotiated (PS3.7 D.3.3.3), so the association is aborted.
#[tokio::test]
async fn a_second_request_during_a_find_aborts() {
    let archive = archive().await;
    for seed in 0..2500 {
        index_only(&archive, &RealSeries::new(20_000 + seed, 1)).await;
    }
    let server = start(answering(), archive).await;
    let addr = gate(server, 2048, 2).await;
    let mut client = viewer(addr, "PACSRS").await;
    let context = client.context(STUDY_ROOT_FIND);
    let mut query = Vec::new();
    study_query(&[])
        .write_dataset_with_ts(&mut query, client.transfer_syntax(STUDY_ROOT_FIND))
        .unwrap();
    for id in [5, 6] {
        message::send(
            &mut client.association,
            context,
            &c_find_rq(id, STUDY_ROOT_FIND),
            Some(&query),
        )
        .await
        .unwrap();
    }
    loop {
        match message::receive(&mut client.association, 1 << 20).await {
            Ok(Incoming::Message(message)) => {
                assert_eq!(message.command.status(), Ok(Status::PENDING));
            }
            Ok(Incoming::Aborted) | Err(_) => break,
            Ok(Incoming::Released) => panic!("released, not aborted"),
        }
    }
}

/// PS3.7 9.1.2.1.6: an operation the Information Model's FIND SOP class does not have is
/// Unrecognized operation (0211H).
#[tokio::test]
async fn another_operation_on_the_find_sop_class_is_0211() {
    let addr = start(answering(), archive().await).await;
    let mut client = viewer(addr, "PACSRS").await;
    let context = client.context(STUDY_ROOT_FIND);
    for operation in [field::C_MOVE_RQ, field::C_GET_RQ, field::C_STORE_RQ] {
        let id = client.message_id();
        let request =
            no_data(c_find_rq(id, STUDY_ROOT_FIND)).with_us(tag::COMMAND_FIELD, operation);
        let rsp = client.response(context, &request, None).await;
        assert_eq!(rsp.status(), Ok(Status::UNRECOGNIZED_OPERATION));
    }
    client.release().await;
}
