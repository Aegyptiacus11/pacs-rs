//! Key images through the REST API (`POST /studies/{id}/key-images`) on real CT slices: the
//! Key Object Selection made, read back from its file, and what is refused.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_core::key_images::key_images_of;
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

async fn server(series: &[&RealSeries]) -> Server {
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
    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        (
            status,
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
    }
    async fn json(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, bytes) = self.call(method, uri, body).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn ids(s: &RealSeries, slice: u32) -> String {
    public_ids::instance(
        &s.patient_id(),
        &s.study_uid(),
        &s.series_uid(),
        &s.instance_uid(slice),
    )
}

#[tokio::test]
async fn marked_images_become_a_key_object_selection_of_their_study() {
    let ct = RealSeries::new(71, 4).for_patient(71);
    let server = server(&[&ct]).await;
    let study = public_ids::study(&ct.patient_id(), &ct.study_uid());

    let (status, stored) = server
        .json(
            Method::POST,
            &format!("/studies/{study}/key-images"),
            Some(
                json!({"Images": [ids(&ct, 3), ids(&ct, 1)], "Description": "Mass, right kidney"}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["ParentStudy"], study.as_str());
    let (_, series) = server
        .json(Method::GET, &format!("/studies/{study}/series"), None)
        .await;
    let ko = series
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["MainDicomTags"]["Modality"] == "KO")
        .unwrap();
    assert_eq!(ko["MainDicomTags"]["SeriesDescription"], "Key images");
    // read back from its file as another system would: the two slices, in the order given
    let (_, file) = server
        .call(
            Method::GET,
            &format!("/instances/{}/file", stored["ID"].as_str().unwrap()),
            None,
        )
        .await;
    let object = pacs_core::Instance::from_bytes(&file).unwrap();
    let found = key_images_of(&object.object).unwrap();
    assert_eq!(
        found
            .iter()
            .map(|k| k.sop_instance.clone())
            .collect::<Vec<_>>(),
        vec![ct.instance_uid(3), ct.instance_uid(1)]
    );
    assert_eq!(found[0].series, ct.series_uid());
    assert_eq!(found[0].sop_class, object_class(&ct));
}

/// The SOP Class UID of the series' images, as their files say.
fn object_class(s: &RealSeries) -> String {
    pacs_core::Instance::from_bytes(&s.slice_bytes(0))
        .unwrap()
        .key
        .sop_class_uid
}

#[tokio::test]
async fn a_frame_is_kept_and_what_is_not_of_the_study_is_refused() {
    let (ct, other) = (
        RealSeries::new(72, 2).for_patient(72),
        RealSeries::new(73, 1).for_patient(73),
    );
    let server = server(&[&ct, &other]).await;
    let study = public_ids::study(&ct.patient_id(), &ct.study_uid());
    let uri = format!("/studies/{study}/key-images");

    let (status, stored) = server
        .json(
            Method::POST,
            &uri,
            Some(json!({"Images": [{"ID": ids(&ct, 0), "Frame": 1}]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let (_, file) = server
        .call(
            Method::GET,
            &format!("/instances/{}/file", stored["ID"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(
        key_images_of(&pacs_core::Instance::from_bytes(&file).unwrap().object).unwrap()[0].frame,
        Some(1)
    );

    for (body, why) in [
        (json!({"Images": []}), "no image"),
        (
            json!({"Images": [ids(&other, 0)]}),
            "not an instance of this study",
        ),
        (
            json!({"Images": ["nobody"]}),
            "not an instance of this study",
        ),
        (
            json!({"Images": [{"ID": ids(&ct, 0), "Frame": 0}]}),
            "frames count from 1",
        ),
        (
            json!({"Pictures": [ids(&ct, 0)]}),
            "not a list of key images",
        ),
    ] {
        let (status, answer) = server.json(Method::POST, &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            answer["Details"].as_str().unwrap().contains(why),
            "{body}: {answer}"
        );
    }
    assert_eq!(
        server
            .call(
                Method::POST,
                "/studies/nobody/key-images",
                Some(json!({"Images": [ids(&ct, 0)]}))
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // only the one with a frame was made
    let (_, series) = server
        .json(Method::GET, &format!("/studies/{study}/series"), None)
        .await;
    assert_eq!(
        series
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["MainDicomTags"]["Modality"] == "KO")
            .count(),
        1
    );
}

#[test]
fn marking_key_images_needs_write() {
    assert_eq!(
        right_for(&Method::POST, "/studies/s1/key-images"),
        Right::Write
    );
}
