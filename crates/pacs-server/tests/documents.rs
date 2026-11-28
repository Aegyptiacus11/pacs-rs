//! PDF documents through the REST API: a real PDF (pacs-core's tests/data/documents/kits19.pdf)
//! added to a study of real images as an Encapsulated PDF instance, given back, and what is
//! refused.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use clap::Parser;
use dicom_dictionary_std::tags;
use http_body_util::BodyExt;
use pacs_core::public_ids;
use pacs_core::real::RealSeries;
use pacs_server::auth::{Right, right_for};
use pacs_server::{Settings, app, open_archive};
use serde_json::Value;
use tower::ServiceExt;

const PDF: &[u8] = include_bytes!("../../pacs-core/tests/data/documents/kits19.pdf");

struct Server {
    app: Router,
    _dir: tempfile::TempDir,
}

async fn server(series: &RealSeries) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.path().join("files").display());
    let settings =
        Settings::try_parse_from(["pacs-server", "--index", &index, "--storage", &storage])
            .unwrap();
    let archive = open_archive(&settings).await.unwrap();
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
    Server {
        app: app(archive, &settings).unwrap(),
        _dir: dir,
    }
}

impl Server {
    async fn call(
        &self,
        method: Method,
        uri: &str,
        body: &[u8],
    ) -> (StatusCode, Option<String>, Vec<u8>) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::from(body.to_vec()))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let kind = response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned());
        (
            status,
            kind,
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
    }
    /// An instance as stored.
    async fn stored(&self, id: &Value) -> pacs_core::Instance {
        let (status, _, bytes) = self
            .call(
                Method::GET,
                &format!("/instances/{}/file", id.as_str().unwrap()),
                b"",
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        pacs_core::Instance::from_bytes(&bytes).unwrap()
    }
    async fn json(&self, method: Method, uri: &str, body: &[u8]) -> (StatusCode, Value) {
        let (status, _, bytes) = self.call(method, uri, body).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

fn study_of(series: &RealSeries) -> String {
    public_ids::study(&series.patient_id(), &series.study_uid())
}

#[tokio::test]
async fn a_pdf_joins_its_study_as_a_document_and_comes_back_as_it_was() {
    let series = RealSeries::new(91, 2).for_patient(91);
    let server = server(&series).await;
    let study = study_of(&series);

    let (status, stored) = server
        .json(
            Method::POST,
            &format!("/studies/{study}/documents?title=KiTS19%20data%20paper"),
            PDF,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    assert_eq!(stored["Status"], "Success");
    assert_eq!(stored["ParentStudy"], study.as_str());

    // a series of its own in the study, after the images' one
    let (_, series_list) = server
        .json(Method::GET, &format!("/studies/{study}/series"), b"")
        .await;
    let list = series_list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    let doc = list
        .iter()
        .find(|s| s["MainDicomTags"]["Modality"] == "DOC")
        .unwrap();
    assert_eq!(
        doc["MainDicomTags"]["SeriesDescription"],
        "KiTS19 data paper"
    );
    let images = list
        .iter()
        .find(|s| s["MainDicomTags"]["Modality"] != "DOC")
        .unwrap();
    let after: u32 = images["MainDicomTags"]["SeriesNumber"]
        .as_str()
        .unwrap()
        .parse::<u32>()
        .unwrap()
        + 1;
    assert_eq!(doc["MainDicomTags"]["SeriesNumber"], after.to_string());
    // the patient's own attributes
    let tags = server.stored(&stored["ID"]).await;
    assert_eq!(tags.string(tags::PATIENT_ID), Some(series.patient_id()));
    assert_eq!(
        tags.string(tags::SOP_CLASS_UID).as_deref(),
        Some("1.2.840.10008.5.1.4.1.1.104.1")
    );
    assert_eq!(
        tags.string(tags::TIMEZONE_OFFSET_FROM_UTC).as_deref(),
        Some("+0000")
    );

    let (status, kind, pdf) = server
        .call(
            Method::GET,
            &format!("/instances/{}/pdf", stored["ID"].as_str().unwrap()),
            b"",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind.as_deref(), Some("application/pdf"));
    assert_eq!(pdf, PDF);
}

#[tokio::test]
async fn a_document_without_a_title_is_called_pdf_document() {
    let series = RealSeries::new(92, 1).for_patient(92);
    let server = server(&series).await;
    let (status, stored) = server
        .json(
            Method::POST,
            &format!("/studies/{}/documents", study_of(&series)),
            PDF,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{stored}");
    let tags = server.stored(&stored["ID"]).await;
    assert_eq!(
        tags.string(tags::DOCUMENT_TITLE).as_deref(),
        Some("PDF document")
    );
}

#[tokio::test]
async fn what_is_not_a_pdf_or_has_no_study_is_refused_and_an_image_has_no_pdf() {
    let series = RealSeries::new(93, 1).for_patient(93);
    let server = server(&series).await;
    let study = study_of(&series);
    let image = series.all_bytes().remove(0);
    for (body, why) in [(&image[..], "%PDF-"), (&b""[..], "empty")] {
        let (status, answer) = server
            .json(Method::POST, &format!("/studies/{study}/documents"), body)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            answer["Details"].as_str().unwrap().contains(why),
            "{answer}"
        );
    }
    assert_eq!(
        server
            .call(Method::POST, "/studies/nobody/documents", PDF)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // nothing was added
    let (_, list) = server
        .json(Method::GET, &format!("/studies/{study}/series"), b"")
        .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    // an image is no document
    let instance = public_ids::instance(
        &series.patient_id(),
        &series.study_uid(),
        &series.series_uid(),
        &series.instance_uid(0),
    );
    assert_eq!(
        server
            .call(Method::GET, &format!("/instances/{instance}/pdf"), b"")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn adding_a_document_needs_write_and_reading_one_read() {
    assert_eq!(
        right_for(&Method::POST, "/studies/s1/documents"),
        Right::Write
    );
    assert_eq!(right_for(&Method::GET, "/instances/i1/pdf"), Right::Read);
}
