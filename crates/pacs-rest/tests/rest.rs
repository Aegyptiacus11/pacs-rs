use std::io::{Cursor, Read};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use http_body_util::BodyExt;
use pacs_archive::{Archive, Compression};
use pacs_core::public_ids;
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::Index;
use pacs_store::Store;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app() -> Router {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
    pacs_rest::router(archive, 1 << 30)
}

async fn call(app: &Router, method: &str, uri: &str, body: Bytes) -> (StatusCode, String, Bytes) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|h| h.to_str().unwrap().to_owned())
        .unwrap_or_default();
    (
        status,
        content_type,
        response.into_body().collect().await.unwrap().to_bytes(),
    )
}

async fn json(app: &Router, method: &str, uri: &str, body: Bytes) -> Value {
    let (status, _, body) = call(app, method, uri, body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{uri}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap()
}

async fn upload_all(app: &Router, series: &SyntheticSeries) {
    for bytes in series.all_bytes() {
        json(app, "POST", "/instances", bytes.into()).await;
    }
}

#[tokio::test]
async fn upload_answers_with_the_ids_and_keeps_the_first_copy() {
    let app = app().await;
    let series = SyntheticSeries::new(91, 1);
    let (p, st, se, i) = (
        "SYN91",
        series.study_uid(),
        series.series_uid(),
        series.instance_uid(0),
    );
    let reply = json(&app, "POST", "/instances", series.slice_bytes(0).into()).await;
    let id = public_ids::instance(p, &st, &se, &i);
    assert_eq!(
        reply,
        json!({
            "ID": id,
            "ParentPatient": public_ids::patient(p),
            "ParentSeries": public_ids::series(p, &st, &se),
            "ParentStudy": public_ids::study(p, &st),
            "Path": format!("/instances/{id}"),
            "Status": "Success",
        })
    );
    let again = json(&app, "POST", "/instances", series.slice_bytes(0).into()).await;
    assert_eq!(again["Status"], "AlreadyStored");

    let (status, _, body) = call(&app, "POST", "/instances", Bytes::from_static(b"junk")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["HttpStatus"], 400);
    assert_eq!(error["Message"], "Bad file format");
}

#[tokio::test]
async fn what_quorum_reads_back() {
    let app = app().await;
    let first = SyntheticSeries::new(92, 3);
    let second = SyntheticSeries::new(93, 2).in_study(92, 2);
    upload_all(&app, &first).await;
    upload_all(&app, &second).await;
    let study_id = public_ids::study("SYN92", &first.study_uid());

    // find the study by its UID
    let find = json!({"Level": "Study", "Query": {"StudyInstanceUID": first.study_uid()}});
    let found = json(&app, "POST", "/tools/find", find.to_string().into()).await;
    assert_eq!(found, json!([study_id]));
    assert_eq!(
        json(&app, "GET", "/studies", Bytes::new()).await,
        json!([study_id])
    );

    let study = json(&app, "GET", &format!("/studies/{study_id}"), Bytes::new()).await;
    assert_eq!(study["Type"], "Study");
    assert_eq!(
        study["MainDicomTags"]["StudyInstanceUID"],
        first.study_uid()
    );
    assert_eq!(
        study["MainDicomTags"]["StudyDescription"],
        "Synthetic study"
    );
    assert_eq!(study["PatientMainDicomTags"]["PatientID"], "SYN92");
    assert_eq!(study["ParentPatient"], public_ids::patient("SYN92"));
    assert_eq!(study["Series"].as_array().unwrap().len(), 2);
    assert_eq!(study["IsStable"], false);

    let stats = json(
        &app,
        "GET",
        &format!("/studies/{study_id}/statistics"),
        Bytes::new(),
    )
    .await;
    let size: usize = first
        .all_bytes()
        .iter()
        .chain(&second.all_bytes())
        .map(Vec::len)
        .sum();
    assert_eq!(stats["CountInstances"], 5);
    assert_eq!(stats["CountSeries"], 2);
    assert_eq!(stats["DicomUncompressedSize"], size.to_string());
    assert_eq!(stats["DiskSizeMB"], 0);

    let instances = json(
        &app,
        "GET",
        &format!("/studies/{study_id}/instances"),
        Bytes::new(),
    )
    .await;
    let instances = instances.as_array().unwrap();
    assert_eq!(instances.len(), 5);
    assert_eq!(
        instances[0]["MainDicomTags"]["SOPInstanceUID"],
        first.instance_uid(0)
    );
    assert_eq!(instances[0]["IndexInSeries"], 1);
    assert_eq!(instances[0]["FileSize"], first.slice_bytes(0).len());

    // the archive: every file, as uploaded, in patient/study/series folders
    let (status, content_type, zip) = call(
        &app,
        "GET",
        &format!("/studies/{study_id}/archive"),
        Bytes::new(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/zip");
    let mut archive = zip::ZipArchive::new(Cursor::new(zip.to_vec())).unwrap();
    assert_eq!(archive.len(), 5);
    let mut file = archive.by_index(0).unwrap();
    assert_eq!(
        file.name(),
        "SYN92 - SYNTHETIC^SERIES/ACC92 - Synthetic study/1 - CT - Synthetic series 1/CT000001.dcm"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, first.slice_bytes(0));
}

#[tokio::test]
async fn series_instances_and_files() {
    let app = app().await;
    let series = SyntheticSeries::new(94, 2);
    upload_all(&app, &series).await;
    let (p, st, se) = ("SYN94", series.study_uid(), series.series_uid());
    let series_id = public_ids::series(p, &st, &se);

    let s = json(&app, "GET", &format!("/series/{series_id}"), Bytes::new()).await;
    assert_eq!(s["Type"], "Series");
    assert_eq!(s["ParentStudy"], public_ids::study(p, &st));
    assert_eq!(s["MainDicomTags"]["SeriesNumber"], "1");
    assert_eq!(s["Status"], "Unknown");
    assert_eq!(s["Instances"].as_array().unwrap().len(), 2);

    let listed = json(
        &app,
        "GET",
        &format!("/series/{series_id}/instances"),
        Bytes::new(),
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 2);

    let id = public_ids::instance(p, &st, &se, &series.instance_uid(1));
    let i = json(&app, "GET", &format!("/instances/{id}"), Bytes::new()).await;
    assert_eq!(i["ParentSeries"], series_id);
    assert_eq!(i["MainDicomTags"]["InstanceNumber"], "2");
    let (status, content_type, file) =
        call(&app, "GET", &format!("/instances/{id}/file"), Bytes::new()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/dicom");
    assert_eq!(file, series.slice_bytes(1));

    for level in ["Series", "Instance"] {
        let (key, uid) = match level {
            "Series" => ("SeriesInstanceUID", se.clone()),
            _ => ("SOPInstanceUID", series.instance_uid(1)),
        };
        let find = json!({"Level": level, "Query": {key: uid}, "Expand": true});
        let found = json(&app, "POST", "/tools/find", find.to_string().into()).await;
        assert_eq!(found.as_array().unwrap().len(), 1);
        assert_eq!(found[0]["Type"], level);
    }
}

#[tokio::test]
async fn unknown_ids_are_a_bare_404_and_unsupported_searches_400() {
    let app = app().await;
    for uri in [
        "/studies/x",
        "/studies/x/statistics",
        "/series/x",
        "/instances/x/file",
    ] {
        let (status, _, body) = call(&app, "GET", uri, Bytes::new()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(body.is_empty());
    }
    for query in [
        json!({"Level": "Study", "Query": {"PatientID": "SYN*"}}),
        json!({"Level": "Study", "Query": {"PatientName": "X"}}),
        json!({"Level": "Patient", "Query": {"PatientName": "X"}}),
    ] {
        let (status, _, _) = call(&app, "POST", "/tools/find", query.to_string().into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
    }
    let empty = json!({"Level": "Study", "Query": {"StudyInstanceUID": "2.25.1"}});
    assert_eq!(
        json(&app, "POST", "/tools/find", empty.to_string().into()).await,
        json!([])
    );
}

#[tokio::test]
async fn the_archive_is_streamed_file_by_file() {
    let app = app().await;
    let series = SyntheticSeries::new(95, 6);
    upload_all(&app, &series).await;
    let study_id = public_ids::study("SYN95", &series.study_uid());
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/studies/{study_id}/archive"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // no Content-Length: the size is not known until the end
    assert!(response.headers().get("content-length").is_none());
    let mut body = response.into_body();
    let mut chunks = 0;
    let mut zip = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.unwrap().into_data() {
            if !data.is_empty() {
                chunks += 1;
            }
            zip.extend_from_slice(&data);
        }
    }
    // one piece per file at least, not one block at the end
    assert!(chunks >= 6, "{chunks} chunks");
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    assert_eq!(archive.len(), 6);
    for (i, expected) in series.all_bytes().iter().enumerate() {
        let mut bytes = Vec::new();
        archive
            .by_index(i)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(&bytes, expected, "entry {i}");
    }
}

/// Patient SYN96: study 96 (series 96 of 2 slices, series 97 of 1) and study 98 (1 slice).
async fn two_studies(app: &Router) -> [SyntheticSeries; 3] {
    let a = SyntheticSeries::new(96, 2);
    let b = SyntheticSeries::new(97, 1).in_study(96, 2).for_patient(96);
    let c = SyntheticSeries::new(98, 1).for_patient(96);
    for s in [&a, &b, &c] {
        upload_all(app, s).await;
    }
    [a, b, c]
}

#[tokio::test]
async fn patients_and_parents() {
    let app = app().await;
    let [a, _, c] = two_studies(&app).await;
    let patient = public_ids::patient("SYN96");
    let study_a = public_ids::study("SYN96", &a.study_uid());
    let study_c = public_ids::study("SYN96", &c.study_uid());

    assert_eq!(
        json(&app, "GET", "/patients", Bytes::new()).await,
        json!([patient])
    );
    let p = json(&app, "GET", &format!("/patients/{patient}"), Bytes::new()).await;
    assert_eq!(p["Type"], "Patient");
    assert_eq!(p["MainDicomTags"]["PatientID"], "SYN96");
    assert_eq!(p["IsProtected"], false);
    let mut studies: Vec<String> = serde_json::from_value(p["Studies"].clone()).unwrap();
    studies.sort();
    let mut expected = vec![study_a.clone(), study_c.clone()];
    expected.sort();
    assert_eq!(studies, expected);
    let expanded = json(&app, "GET", "/patients?expand", Bytes::new()).await;
    assert_eq!(expanded, json!([p]));

    let uri = |tail: &str| format!("/patients/{patient}/{tail}");
    let listed = json(&app, "GET", &uri("studies"), Bytes::new()).await;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[0]["Type"], "Study");
    let listed = json(&app, "GET", &uri("series"), Bytes::new()).await;
    assert_eq!(listed.as_array().unwrap().len(), 3);
    let listed = json(&app, "GET", &uri("instances"), Bytes::new()).await;
    assert_eq!(listed.as_array().unwrap().len(), 4);
    let stats = json(&app, "GET", &uri("statistics"), Bytes::new()).await;
    assert_eq!(
        (
            &stats["CountStudies"],
            &stats["CountSeries"],
            &stats["CountInstances"]
        ),
        (&json!(2), &json!(3), &json!(4))
    );
    let (status, _, zip) = call(&app, "GET", &uri("archive"), Bytes::new()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        zip::ZipArchive::new(Cursor::new(zip.to_vec()))
            .unwrap()
            .len(),
        4
    );

    let find = json!({"Level": "Patient", "Query": {"PatientID": "SYN96"}});
    assert_eq!(
        json(&app, "POST", "/tools/find", find.to_string().into()).await,
        json!([patient])
    );

    // each resource's ancestors
    let series_a = public_ids::series("SYN96", &a.study_uid(), &a.series_uid());
    let instance =
        public_ids::instance("SYN96", &a.study_uid(), &a.series_uid(), &a.instance_uid(0));
    for (uri, id) in [
        (format!("/studies/{study_a}/patient"), &patient),
        (format!("/series/{series_a}/patient"), &patient),
        (format!("/series/{series_a}/study"), &study_a),
        (format!("/instances/{instance}/patient"), &patient),
        (format!("/instances/{instance}/study"), &study_a),
        (format!("/instances/{instance}/series"), &series_a),
    ] {
        assert_eq!(
            json(&app, "GET", &uri, Bytes::new()).await["ID"],
            *id,
            "{uri}"
        );
    }
    for uri in ["/patients/x", "/patients/x/studies", "/studies/x/patient"] {
        let (status, _, _) = call(&app, "GET", uri, Bytes::new()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn deleting_answers_with_the_remaining_ancestor() {
    let app = app().await;
    let [a, b, c] = two_studies(&app).await;
    let patient = public_ids::patient("SYN96");
    let study_a = public_ids::study("SYN96", &a.study_uid());
    let series_a = public_ids::series("SYN96", &a.study_uid(), &a.series_uid());
    let series_b = public_ids::series("SYN96", &b.study_uid(), &b.series_uid());
    let instance = |slice| {
        public_ids::instance(
            "SYN96",
            &a.study_uid(),
            &a.series_uid(),
            &a.instance_uid(slice),
        )
    };

    let reply = json(
        &app,
        "DELETE",
        &format!("/instances/{}", instance(0)),
        Bytes::new(),
    )
    .await;
    assert_eq!(
        reply,
        json!({"RemainingAncestor": {
            "ID": series_a,
            "Path": format!("/series/{series_a}"),
            "Type": "Series",
        }})
    );
    let (status, _, body) = call(
        &app,
        "GET",
        &format!("/instances/{}/file", instance(0)),
        Bytes::new(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty());

    let reply = json(&app, "DELETE", &format!("/series/{series_a}"), Bytes::new()).await;
    assert_eq!(reply["RemainingAncestor"]["ID"], study_a);
    let reply = json(&app, "DELETE", &format!("/series/{series_b}"), Bytes::new()).await;
    assert_eq!(
        reply["RemainingAncestor"],
        json!({"ID": patient, "Path": format!("/patients/{patient}"), "Type": "Patient"})
    );
    let (status, _, body) =
        call(&app, "DELETE", &format!("/studies/{study_a}"), Bytes::new()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty());

    let reply = json(
        &app,
        "DELETE",
        &format!("/patients/{patient}"),
        Bytes::new(),
    )
    .await;
    assert_eq!(reply, json!({"RemainingAncestor": null}));
    assert_eq!(
        json(&app, "GET", "/patients", Bytes::new()).await,
        json!([])
    );
    assert_eq!(json(&app, "GET", "/studies", Bytes::new()).await, json!([]));

    // a deleted study can be uploaded again
    upload_all(&app, &c).await;
    let study_c = public_ids::study("SYN96", &c.study_uid());
    assert_eq!(
        json(&app, "GET", "/studies", Bytes::new()).await,
        json!([study_c])
    );
}
