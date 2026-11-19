//! The REST API's description against the API itself: every answer a handler gives is read
//! into the type the description names for it, and those types refuse a field they do not
//! have. So an answer that changes without its description fails here.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use http_body_util::BodyExt;
use pacs_archive::{Archive, Compression};
use pacs_core::public_ids;
use pacs_core::real::RealSeries;
use pacs_index::Index;
use pacs_rest::schema::{
    Deleted, Error, Found, Instance, Patient, PatientList, Series, Statistics, Stored, Study,
    StudyList,
};
use pacs_store::Store;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower::ServiceExt;
use utoipa::openapi::OpenApi;

async fn api() -> (Router, OpenApi) {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
    pacs_rest::router_with_api(archive, 1 << 30)
}

async fn call(app: &Router, method: &str, uri: &str, body: Bytes) -> (StatusCode, Bytes) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    (
        status,
        response.into_body().collect().await.unwrap().to_bytes(),
    )
}

/// The answer to a request, which must have `status`, as the described type `T`.
async fn answer<T: DeserializeOwned>(
    app: &Router,
    method: &str,
    uri: &str,
    body: impl Into<Bytes>,
    status: StatusCode,
) -> T {
    let (got, bytes) = call(app, method, uri, body.into()).await;
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(got, status, "{method} {uri}: {text}");
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("{method} {uri}: the answer is not what the description says: {e}\n{text}")
    })
}

async fn get<T: DeserializeOwned>(app: &Router, uri: &str) -> T {
    answer(app, "GET", uri, Bytes::new(), StatusCode::OK).await
}

#[tokio::test]
async fn every_answer_is_what_the_description_says() {
    let (app, _) = api().await;
    let series = RealSeries::new(41, 2);
    let (p, st, se) = ("NEMA-41", series.study_uid(), series.series_uid());
    let patient = public_ids::patient(p);
    let study = public_ids::study(p, &st);
    let series_id = public_ids::series(p, &st, &se);
    let instance = public_ids::instance(p, &st, &se, &series.instance_uid(0));

    for bytes in series.all_bytes() {
        let stored: Stored = answer(&app, "POST", "/instances", bytes, StatusCode::OK).await;
        assert_eq!(stored.status, "Success");
    }
    let again: Stored = answer(
        &app,
        "POST",
        "/instances",
        series.slice_bytes(0),
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        (again.status.as_str(), &again.id),
        ("AlreadyStored", &instance)
    );
    let refused: Error = answer(&app, "POST", "/instances", "junk", StatusCode::BAD_REQUEST).await;
    assert_eq!(
        (refused.http_status, refused.details.is_some()),
        (400, true)
    );

    assert!(
        matches!(get(&app, "/patients").await, PatientList::Ids(ids) if ids == [patient.clone()])
    );
    assert!(
        matches!(get(&app, "/patients?expand").await, PatientList::Expanded(found) if found.len() == 1)
    );
    assert!(
        matches!(get(&app, "/studies?limit=5&since=0").await, StudyList::Ids(ids) if ids == [study.clone()])
    );
    assert!(
        matches!(get(&app, "/studies?expand").await, StudyList::Expanded(found) if found.len() == 1)
    );
    let _: Error = answer(
        &app,
        "GET",
        "/studies?limit=x",
        Bytes::new(),
        StatusCode::BAD_REQUEST,
    )
    .await;

    let _: Patient = get(&app, &format!("/patients/{patient}")).await;
    let _: Vec<Study> = get(&app, &format!("/patients/{patient}/studies")).await;
    let _: Vec<Series> = get(&app, &format!("/patients/{patient}/series")).await;
    let found: Vec<Instance> = get(&app, &format!("/patients/{patient}/instances")).await;
    assert_eq!(found.len(), 2);
    let stats: Statistics = get(&app, &format!("/patients/{patient}/statistics")).await;
    assert_eq!((stats.count_studies, stats.count_instances), (Some(1), 2));

    let found: Study = get(&app, &format!("/studies/{study}")).await;
    assert_eq!(found.main_dicom_tags["StudyInstanceUID"], st);
    let _: Vec<Series> = get(&app, &format!("/studies/{study}/series")).await;
    let _: Vec<Instance> = get(&app, &format!("/studies/{study}/instances")).await;
    let stats: Statistics = get(&app, &format!("/studies/{study}/statistics")).await;
    assert_eq!((stats.count_studies, stats.count_series), (None, 1));
    let _: Patient = get(&app, &format!("/studies/{study}/patient")).await;

    let found: Series = get(&app, &format!("/series/{series_id}")).await;
    assert_eq!(found.expected_number_of_instances, None);
    let _: Vec<Instance> = get(&app, &format!("/series/{series_id}/instances")).await;
    let _: Study = get(&app, &format!("/series/{series_id}/study")).await;
    let _: Patient = get(&app, &format!("/series/{series_id}/patient")).await;

    let found: Instance = get(&app, &format!("/instances/{instance}")).await;
    assert_eq!(found.index_in_series, Some(1));
    let _: Series = get(&app, &format!("/instances/{instance}/series")).await;
    let _: Study = get(&app, &format!("/instances/{instance}/study")).await;
    let _: Patient = get(&app, &format!("/instances/{instance}/patient")).await;

    // a search at each level, as identifiers and as resources
    for (level, key, value) in [
        ("Patient", "PatientID", p.to_owned()),
        ("Study", "StudyInstanceUID", st.clone()),
        ("Series", "SeriesInstanceUID", se.clone()),
        ("Instance", "SOPInstanceUID", series.instance_uid(0)),
    ] {
        for expand in [false, true] {
            let search = json!({"Level": level, "Query": {key: value}, "Expand": expand});
            let found: Found = answer(
                &app,
                "POST",
                "/tools/find",
                search.to_string(),
                StatusCode::OK,
            )
            .await;
            let one = match (&found, level, expand) {
                (Found::Ids(ids), _, false) => ids.len() == 1,
                (Found::Patients(found), "Patient", true) => found.len() == 1,
                (Found::Studies(found), "Study", true) => found.len() == 1,
                (Found::Series(found), "Series", true) => found.len() == 1,
                (Found::Instances(found), "Instance", true) => found.len() == 1,
                _ => false,
            };
            assert!(one, "{level} expand={expand}: {found:?}");
        }
    }
    // no Query: every patient or study; a series or instance search needs its UID
    let all = json!({"Level": "Study"}).to_string();
    let found: Found = answer(&app, "POST", "/tools/find", all, StatusCode::OK).await;
    assert!(matches!(found, Found::Ids(ids) if ids.len() == 1));
    for level in ["Series", "Instance"] {
        let all = json!({"Level": level}).to_string();
        let _: Error = answer(&app, "POST", "/tools/find", all, StatusCode::BAD_REQUEST).await;
    }
    // a wild card and a range find the study; a value that is not a date is refused
    let search =
        json!({"Level": "Study", "Query": {"PatientName": "c*", "StudyDate": "20040119-"}});
    let found: Found = answer(
        &app,
        "POST",
        "/tools/find",
        search.to_string(),
        StatusCode::OK,
    )
    .await;
    assert!(matches!(found, Found::Ids(ids) if ids.len() == 1));
    let search = json!({"Level": "Study", "Query": {"StudyDate": "2026-01-01"}});
    let _: Error = answer(
        &app,
        "POST",
        "/tools/find",
        search.to_string(),
        StatusCode::BAD_REQUEST,
    )
    .await;

    // the statuses the description gives the archives
    let asked = json!({"Resources": [instance, "nothing"]}).to_string();
    for (method, uri, body, status) in [
        (
            "GET",
            format!("/studies/{study}/archive"),
            String::new(),
            StatusCode::OK,
        ),
        (
            "GET",
            format!("/patients/{patient}/archive"),
            String::new(),
            StatusCode::OK,
        ),
        (
            "GET",
            format!("/instances/{instance}/file"),
            String::new(),
            StatusCode::OK,
        ),
        (
            "GET",
            format!("/tools/create-archive?resources={instance}"),
            String::new(),
            StatusCode::OK,
        ),
        // no identifier: an empty ZIP, not a refusal
        (
            "GET",
            "/tools/create-archive".to_owned(),
            String::new(),
            StatusCode::OK,
        ),
        (
            "GET",
            "/tools/create-archive?resources=nothing".to_owned(),
            String::new(),
            StatusCode::NOT_FOUND,
        ),
        (
            "POST",
            "/tools/create-archive".to_owned(),
            asked,
            StatusCode::NOT_FOUND,
        ),
        (
            "POST",
            "/tools/create-archive".to_owned(),
            "{}".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/studies/nothing".to_owned(),
            String::new(),
            StatusCode::NOT_FOUND,
        ),
        (
            "DELETE",
            "/series/nothing".to_owned(),
            String::new(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (got, bytes) = call(&app, method, &uri, body.into()).await;
        assert_eq!(
            got,
            status,
            "{method} {uri}: {}",
            String::from_utf8_lossy(&bytes)
        );
        if status == StatusCode::NOT_FOUND {
            assert!(bytes.is_empty(), "{method} {uri}: a 404 has no body");
        }
    }

    let deleted: Deleted = answer(
        &app,
        "DELETE",
        &format!("/instances/{instance}"),
        Bytes::new(),
        StatusCode::OK,
    )
    .await;
    assert_eq!(deleted.remaining_ancestor.unwrap().r#type, "Series");
    for uri in [
        format!("/series/{series_id}"),
        "/studies/x".into(),
        "/patients/x".into(),
    ] {
        let (status, _) = call(&app, "DELETE", &uri, Bytes::new()).await;
        // the series' deletion took its study and patient with it
        assert!(matches!(status, StatusCode::OK | StatusCode::NOT_FOUND));
    }
    let series = RealSeries::new(42, 1);
    let _: Stored = answer(
        &app,
        "POST",
        "/instances",
        series.slice_bytes(0),
        StatusCode::OK,
    )
    .await;
    for level in ["studies", "patients"] {
        let id = match level {
            "studies" => public_ids::study("NEMA-42", &series.study_uid()),
            _ => public_ids::patient("NEMA-42"),
        };
        let (status, bytes) = call(&app, "DELETE", &format!("/{level}/{id}"), Bytes::new()).await;
        if status == StatusCode::OK {
            let deleted: Deleted = serde_json::from_slice(&bytes).unwrap();
            assert!(deleted.remaining_ancestor.is_none());
        }
    }
}

#[test]
fn a_field_the_description_does_not_have_is_refused() {
    let study = json!({
        "ID": "a", "IsStable": false, "Labels": [], "LastUpdate": "20260101T000000",
        "MainDicomTags": {}, "ParentPatient": "b", "PatientMainDicomTags": {}, "Series": [],
        "Type": "Study",
    });
    serde_json::from_value::<Study>(study.clone()).unwrap();

    let mut more = study.clone();
    more["Added"] = json!(1);
    let error = serde_json::from_value::<Study>(more)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown field `Added`"), "{error}");

    let mut less = study.clone();
    less.as_object_mut().unwrap().remove("Series");
    let error = serde_json::from_value::<Study>(less)
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing field `Series`"), "{error}");

    let mut other = study;
    other["IsStable"] = json!("no");
    assert!(serde_json::from_value::<Study>(other).is_err());
}

#[tokio::test]
async fn every_operation_is_described_in_full() {
    let (_, api) = api().await;
    let api = serde_json::to_value(&api).unwrap();
    let paths = api["paths"].as_object().unwrap();
    let mut operations = 0;
    for (path, item) in paths {
        for (method, operation) in item.as_object().unwrap() {
            operations += 1;
            let said = format!("{method} {path}");
            assert!(
                operation["summary"]
                    .as_str()
                    .is_some_and(|s| s.ends_with('.')),
                "{said}: summary"
            );
            assert_eq!(
                operation["tags"].as_array().map(Vec::len),
                Some(1),
                "{said}: one tag"
            );
            let responses = operation["responses"].as_object().unwrap();
            assert!(
                responses.keys().any(|status| status == "200"),
                "{said}: no 200"
            );
            for (status, response) in responses {
                assert!(
                    response["description"]
                        .as_str()
                        .is_some_and(|d| !d.is_empty()),
                    "{said}: {status} is not described"
                );
            }
            for parameter in operation["parameters"].as_array().into_iter().flatten() {
                assert!(
                    parameter["description"]
                        .as_str()
                        .is_some_and(|d| !d.is_empty()),
                    "{said}: parameter {} is not described",
                    parameter["name"]
                );
            }
            // a path's {id} is a described parameter
            if path.contains("{id}") {
                let named = operation["parameters"].as_array().unwrap();
                assert!(
                    named.iter().any(|p| p["name"] == "id" && p["in"] == "path"),
                    "{said}"
                );
            }
        }
    }
    // the routes of `router_with_api`: 26 paths, four of them with DELETE too, one with POST
    assert_eq!((paths.len(), operations), (26, 31));
    let schemas = api["components"]["schemas"].as_object().unwrap();
    for name in [
        "Patient",
        "Study",
        "Series",
        "Instance",
        "Statistics",
        "Stored",
        "Deleted",
        "Error",
        "Find",
        "CreateArchive",
    ] {
        let schema: &Value = &schemas[name];
        assert!(
            schema["description"].as_str().is_some(),
            "{name} is not described"
        );
    }
}
