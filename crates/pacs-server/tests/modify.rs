//! Modifying and merging patients through the REST API (`POST /patients/{id}/modify` and
//! `/merge`) on real series: the answer, the files, the errors, the right it needs, the audit
//! trail, and the undo by `POST /trash/{id}/restore`.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_core::public_ids;
use pacs_core::real::RealSeries;
use pacs_server::auth::{Right, right_for};
use pacs_server::{Settings, app, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Server {
    app: Router,
    _dir: tempfile::TempDir,
}

async fn server(series: &[RealSeries]) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.path().join("files").display());
    let settings =
        Settings::try_parse_from(["pacs-server", "--index", &index, "--storage", &storage])
            .unwrap();
    let archive = open_archive(&settings).await.unwrap();
    for s in series {
        for bytes in s.all_bytes() {
            archive.ingest(bytes.into()).await.unwrap();
        }
    }
    Server {
        app: app(archive, &settings).unwrap(),
        _dir: dir,
    }
}

impl Server {
    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn patient(&self, id: &str) -> (StatusCode, Value) {
        self.call(Method::GET, &format!("/patients/{id}"), None)
            .await
    }
}

#[tokio::test]
async fn a_patient_is_modified_and_answered_under_the_new_id() {
    let series = RealSeries::new(81, 2).for_patient(81);
    let server = server(std::slice::from_ref(&series)).await;
    let old = public_ids::patient(&series.patient_id());

    let (status, done) = server
        .call(
            Method::POST,
            &format!("/patients/{old}/modify"),
            Some(json!({"PatientName": "Benali^Amina", "PatientID": "H-0042", "PatientSex": "F"})),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{done}");
    let new = public_ids::patient("H-0042");
    assert_eq!(done["ID"], new);
    assert_eq!(done["Path"], format!("/patients/{new}"));
    assert_eq!(done["Instances"], 2);
    assert!(done["Trash"].is_string());
    let (status, patient) = server.patient(&new).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(patient["MainDicomTags"]["PatientName"], "Benali^Amina");
    assert_eq!(patient["MainDicomTags"]["PatientSex"], "F");
    assert_eq!(server.patient(&old).await.0, StatusCode::NOT_FOUND);
    // the trash lists it as a modification, with the values before and after
    let (_, trash) = server.call(Method::GET, "/trash", None).await;
    assert_eq!(trash[0]["level"], "Modification");
    assert_eq!(trash[0]["studies"][0]["PatientID"], series.patient_id());
    assert_eq!(trash[0]["studies"][0]["changed"]["PatientID"], "H-0042");
    assert!(trash[0]["studies"][0].get("rekeyed").is_none());
}

#[tokio::test]
async fn restoring_the_trash_entry_undoes_it() {
    let series = RealSeries::new(82, 1).for_patient(82);
    let server = server(std::slice::from_ref(&series)).await;
    let old = public_ids::patient(&series.patient_id());
    let (_, done) = server
        .call(
            Method::POST,
            &format!("/patients/{old}/modify"),
            Some(json!({"PatientID": "H-7"})),
        )
        .await;
    let trash = done["Trash"].as_str().unwrap();

    let (status, restored) = server
        .call(Method::POST, &format!("/trash/{trash}/restore"), None)
        .await;

    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(server.patient(&old).await.0, StatusCode::OK);
    assert_eq!(
        server.patient(&public_ids::patient("H-7")).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_patient_is_merged_into_another() {
    let kept = RealSeries::new(83, 1)
        .for_patient(83)
        .with_patient("KEPT^ONE", None);
    let merged = RealSeries::new(84, 1).for_patient(84);
    let server = server(&[kept.clone(), merged.clone()]).await;
    let into = public_ids::patient(&kept.patient_id());
    let from = public_ids::patient(&merged.patient_id());

    let (status, done) = server
        .call(
            Method::POST,
            &format!("/patients/{from}/merge"),
            Some(json!({"Into": into})),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["ID"], into);
    let (_, studies) = server
        .call(Method::GET, &format!("/patients/{into}/studies"), None)
        .await;
    assert_eq!(studies.as_array().unwrap().len(), 2);
    assert_eq!(server.patient(&from).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bad_requests_are_refused_and_say_why() {
    let series = RealSeries::new(85, 1).for_patient(85);
    let server = server(std::slice::from_ref(&series)).await;
    let id = public_ids::patient(&series.patient_id());
    let modify = format!("/patients/{id}/modify");
    for (body, why) in [
        (json!({}), "nothing to change"),
        (json!({"PatientSex": "X"}), "PatientSex X is not M, F or O"),
        (
            json!({"PatientBirthDate": "2001-02-03"}),
            "is not a date as YYYYMMDD",
        ),
        (json!({"PatientID": ""}), "PatientID may not be empty"),
        (json!({"PatientAge": "042Y"}), "not a patient modification"),
    ] {
        let (status, answer) = server.call(Method::POST, &modify, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            answer["Details"].as_str().unwrap().contains(why),
            "{body}: {answer}"
        );
    }
    let (status, answer) = server
        .call(
            Method::POST,
            &format!("/patients/{id}/merge"),
            Some(json!({"Into": id})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        answer["Details"]
            .as_str()
            .unwrap()
            .contains("merged into themselves")
    );
    assert_eq!(
        server
            .call(
                Method::POST,
                "/patients/nobody/modify",
                Some(json!({"PatientID": "X"}))
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .call(
                Method::POST,
                &format!("/patients/{id}/merge"),
                Some(json!({"Into": "nobody"}))
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // nothing changed
    assert_eq!(server.patient(&id).await.0, StatusCode::OK);
    assert_eq!(server.call(Method::GET, "/trash", None).await.1, json!([]));
}

#[test]
fn only_who_may_modify_patients_does() {
    for path in ["/patients/p1/modify", "/patients/p1/merge"] {
        assert_eq!(right_for(&Method::POST, path), Right::Modify, "{path}");
    }
    // reading a patient, or another POST, is not a modification
    assert_eq!(right_for(&Method::GET, "/patients/p1"), Right::Read);
    assert_eq!(right_for(&Method::POST, "/instances"), Right::Write);
}
