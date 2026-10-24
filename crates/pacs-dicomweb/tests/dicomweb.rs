use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dicomweb::media::MediaType;
use pacs_dicomweb::{Config, multipart, router};
use pacs_index::Index;
use pacs_store::Store;
use serde_json::Value;
use tower::ServiceExt;

/// What the Cornerstone WADO-RS image loader sends for frames.
const CORNERSTONE_FRAMES: &str =
    "multipart/related; type=application/octet-stream; transfer-syntax=*";
const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

struct Reply {
    status: StatusCode,
    content_type: String,
    body: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    fn parts(&self) -> Vec<multipart::Part> {
        let media = MediaType::parse(&self.content_type).unwrap();
        assert_eq!(media.essence, "multipart/related", "{}", self.content_type);
        multipart::parse(media.param("boundary").unwrap(), &self.body).unwrap()
    }
}

async fn app() -> Router {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap());
    router(archive, Config::default())
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Bytes,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "pacs.test");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|h| h.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        content_type,
        body,
    }
}

async fn get(app: &Router, uri: &str, accept: &str) -> Reply {
    send(app, "GET", uri, &[("accept", accept)], Bytes::new()).await
}

async fn stow(app: &Router, uri: &str, files: Vec<Vec<u8>>) -> Reply {
    let boundary = multipart::new_boundary();
    let body = multipart::body(
        &boundary,
        files
            .into_iter()
            .map(|f| ("application/dicom".to_owned(), Bytes::from(f))),
    );
    let content_type =
        format!(r#"multipart/related; type="application/dicom"; boundary={boundary}"#);
    send(app, "POST", uri, &[("content-type", &content_type)], body).await
}

fn le_bytes(pixels: &[u16]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.to_le_bytes()).collect()
}

fn value<'a>(json: &'a Value, tag: &str) -> &'a Value {
    &json[tag]["Value"][0]
}

/// A study of two series, 3 and 2 slices, stored through STOW-RS.
async fn quorum_study(app: &Router) -> (SyntheticSeries, SyntheticSeries) {
    let big = SyntheticSeries::new(61, 3);
    let small = SyntheticSeries::new(62, 2).in_study(61, 2);
    let mut files = small.all_bytes();
    files.extend(big.all_bytes());
    let reply = stow(app, "/studies", files).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    (big, small)
}

#[tokio::test]
async fn the_requests_quorum_makes() {
    let app = app().await;
    let (big, small) = quorum_study(&app).await;
    let study = big.study_uid();

    // 1. the study, with its instance count
    let reply = get(
        &app,
        &format!("/studies?StudyInstanceUID={study}&includefield=00201208"),
        "application/dicom+json",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.content_type, "application/dicom+json");
    let studies = reply.json();
    assert_eq!(studies.as_array().unwrap().len(), 1);
    assert_eq!(value(&studies[0], "0020000D"), &study);
    assert_eq!(value(&studies[0], "00201208"), 5);
    assert_eq!(value(&studies[0], "00201206"), 2);
    assert_eq!(studies[0]["00080061"]["Value"], serde_json::json!(["CT"]));
    assert_eq!(
        value(&studies[0], "00081190"),
        &format!("http://pacs.test/studies/{study}")
    );

    // 2. its series, each with its count
    let reply = get(
        &app,
        &format!("/studies/{study}/series?includefield=00201209"),
        "application/dicom+json",
    )
    .await;
    let series = reply.json();
    let series = series.as_array().unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(value(&series[0], "0020000E"), &big.series_uid());
    assert_eq!(value(&series[0], "00201209"), 3);
    assert_eq!(value(&series[1], "0020000E"), &small.series_uid());
    assert_eq!(value(&series[1], "00201209"), 2);

    // 3. the largest series' metadata, in one response
    let reply = get(
        &app,
        &format!("/studies/{study}/series/{}/metadata", big.series_uid()),
        "application/dicom+json",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let metadata = reply.json();
    let metadata = metadata.as_array().unwrap();
    assert_eq!(metadata.len(), 3);
    for (slice, instance) in metadata.iter().enumerate() {
        assert_eq!(value(instance, "00080018"), &big.instance_uid(slice as u32));
        assert_eq!(value(instance, "00200013"), slice + 1);
        assert_eq!(instance["00200032"]["Value"][2], slice as f64 * 2.5);
        assert!(instance.get("7FE00010").is_none());
    }

    // 4. each slice's first frame, as the Cornerstone loader asks and parses it
    for slice in 0..3 {
        let reply = get(
            &app,
            &format!(
                "/studies/{study}/series/{}/instances/{}/frames/1",
                big.series_uid(),
                big.instance_uid(slice)
            ),
            CORNERSTONE_FRAMES,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK);
        // the loader finds the boundary as the first line starting with "--", the
        // transfer syntax in a line starting exactly with "Content-Type:"
        let text = String::from_utf8_lossy(&reply.body);
        let header_end = text.find("\r\n\r\n").unwrap();
        let header: Vec<&str> = text[..header_end].split("\r\n").collect();
        assert!(header[0].starts_with("--"));
        assert_eq!(
            header[1],
            "Content-Type: application/octet-stream; transfer-syntax=1.2.840.10008.1.2.1"
        );
        let parts = reply.parts();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].content, le_bytes(&big.pixels(slice, 0)));
    }
}

#[tokio::test]
async fn instances_and_series_come_back_byte_for_byte() {
    let app = app().await;
    let (big, _) = quorum_study(&app).await;
    let study = big.study_uid();

    let reply = get(
        &app,
        &format!("/studies/{study}/series/{}/instances", big.series_uid()),
        "application/dicom+json",
    )
    .await;
    let instances = reply.json();
    assert_eq!(instances.as_array().unwrap().len(), 3);
    assert_eq!(value(&instances[2], "00080018"), &big.instance_uid(2));
    assert_eq!(value(&instances[2], "0020000E"), &big.series_uid());

    let reply = get(
        &app,
        &format!(
            "/studies/{study}/series/{}/instances/{}",
            big.series_uid(),
            big.instance_uid(1)
        ),
        r#"multipart/related; type="application/dicom"; transfer-syntax=*"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let parts = reply.parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("application/dicom; transfer-syntax=1.2.840.10008.1.2.1")
    );
    assert_eq!(parts[0].content, big.slice_bytes(1));

    let reply = get(
        &app,
        &format!("/studies/{study}/series/{}", big.series_uid()),
        "multipart/related; type=\"application/dicom\"",
    )
    .await;
    let parts = reply.parts();
    let contents: Vec<_> = parts.iter().map(|p| p.content.to_vec()).collect();
    assert_eq!(contents, big.all_bytes());

    let reply = get(&app, &format!("/studies/{study}"), "*/*").await;
    assert_eq!(reply.parts().len(), 5);

    // a client that asks for a bare application/dicom gets the file itself
    let reply = get(
        &app,
        &format!(
            "/studies/{study}/series/{}/instances/{}",
            big.series_uid(),
            big.instance_uid(0)
        ),
        "application/dicom",
    )
    .await;
    assert_eq!(reply.body, big.slice_bytes(0));
}

#[tokio::test]
async fn multiframe_and_encapsulated_frames() {
    let app = app().await;
    let native = SyntheticSeries::new(63, 1);
    let jls = SyntheticSeries::new(64, 1);
    let fragments = vec![vec![1u8, 1], vec![2, 2, 2, 2]];
    let files = vec![
        native.multiframe_bytes(0, 3),
        jls.encapsulated_bytes(0, JPEG_LS, 2, vec![], fragments.clone()),
    ];
    assert_eq!(stow(&app, "/studies", files).await.status, StatusCode::OK);

    let frames_uri = |s: &SyntheticSeries, list: &str| {
        format!(
            "/studies/{}/series/{}/instances/{}/frames/{list}",
            s.study_uid(),
            s.series_uid(),
            s.instance_uid(0)
        )
    };
    let reply = get(&app, &frames_uri(&native, "3,1"), CORNERSTONE_FRAMES).await;
    let parts = reply.parts();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].content, le_bytes(&native.pixels(0, 2)));
    assert_eq!(parts[1].content, le_bytes(&native.pixels(0, 0)));

    let reply = get(&app, &frames_uri(&native, "4"), CORNERSTONE_FRAMES).await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = get(&app, &frames_uri(&native, "x"), CORNERSTONE_FRAMES).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);

    let reply = get(&app, &frames_uri(&jls, "2"), CORNERSTONE_FRAMES).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parts = reply.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("image/jls; transfer-syntax=1.2.840.10008.1.2.4.80")
    );
    assert_eq!(parts[0].content, fragments[1]);
    assert!(reply.content_type.contains(r#"type="image/jls""#));

    // asking for uncompressed frames of a compressed instance: no transcoding
    let reply = get(
        &app,
        &frames_uri(&jls, "1"),
        r#"multipart/related; type="application/octet-stream""#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn stow_reports_what_it_stored_and_what_it_did_not() {
    let app = app().await;
    let series = SyntheticSeries::new(65, 2);
    let reply = stow(
        &app,
        "/studies",
        vec![
            series.slice_bytes(0),
            b"not dicom".to_vec(),
            series.slice_bytes(1),
        ],
    )
    .await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    let json = reply.json();
    let referenced = json["00081199"]["Value"].as_array().unwrap();
    assert_eq!(referenced.len(), 2);
    assert_eq!(value(&referenced[1], "00081155"), &series.instance_uid(1));
    assert_eq!(
        value(&referenced[1], "00081190"),
        &format!(
            "http://pacs.test/studies/{}/series/{}/instances/{}",
            series.study_uid(),
            series.series_uid(),
            series.instance_uid(1)
        )
    );
    let failed = json["00081198"]["Value"].as_array().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(value(&failed[0], "00081197"), 0xC000);
    assert_eq!(
        value(&json, "00081190"),
        &format!("http://pacs.test/studies/{}", series.study_uid())
    );

    // all refused: 409
    let reply = stow(&app, "/studies", vec![b"no".to_vec()]).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);

    // into another study: refused, with the UIDs of what was refused
    let other = SyntheticSeries::new(66, 1);
    let reply = stow(
        &app,
        &format!("/studies/{}", series.study_uid()),
        vec![other.slice_bytes(0)],
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    let json = reply.json();
    let failed = &json["00081198"]["Value"][0];
    assert_eq!(value(failed, "00081155"), &other.instance_uid(0));
    assert_eq!(value(failed, "00081197"), 0xA900);

    // not multipart/related
    let reply = send(
        &app,
        "POST",
        "/studies",
        &[("content-type", "application/dicom")],
        Bytes::from(series.slice_bytes(0)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn post_instances_takes_one_file() {
    let app = app().await;
    let series = SyntheticSeries::new(67, 1);
    let upload = || {
        send(
            &app,
            "POST",
            "/instances",
            &[("content-type", "application/dicom")],
            Bytes::from(series.slice_bytes(0)),
        )
    };
    let reply = upload().await;
    assert_eq!(reply.status, StatusCode::OK);
    let json = reply.json();
    assert_eq!(json["SOPInstanceUID"], series.instance_uid(0));
    assert_eq!(json["StudyInstanceUID"], series.study_uid());
    assert_eq!(json["Status"], "Added");
    assert_eq!(upload().await.json()["Status"], "Replaced");

    let reply = send(&app, "POST", "/instances", &[], Bytes::from_static(b"junk")).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(reply.json()["error"].is_string());
}

#[tokio::test]
async fn searches_it_cannot_do_exactly_are_refused() {
    let app = app().await;
    quorum_study(&app).await;
    let json = "application/dicom+json";

    let reply = get(&app, "/studies?StudyInstanceUID=2.25.999", json).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), serde_json::json!([]));

    assert_eq!(
        get(&app, "/studies?PatientName=SYN*", json).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&app, "/studies?PatientID=SYN*", json).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&app, "/studies?StudyDate=20200101-20201231", json)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&app, "/studies?limit=x", json).await.status,
        StatusCode::BAD_REQUEST
    );

    let reply = get(&app, "/studies?00100020=SYN61", json).await;
    assert_eq!(reply.json().as_array().unwrap().len(), 1);
    let reply = get(&app, "/studies?limit=1&offset=1", json).await;
    assert_eq!(reply.json(), serde_json::json!([]));

    let reply = get(&app, "/studies", "application/dicom+xml").await;
    assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE);

    let study = SyntheticSeries::new(61, 1).study_uid();
    let reply = get(&app, &format!("/studies/{study}/series?Modality=MR"), json).await;
    assert_eq!(reply.json(), serde_json::json!([]));
    let reply = get(&app, &format!("/studies/{study}/series?Modality=CT"), json).await;
    assert_eq!(reply.json().as_array().unwrap().len(), 2);

    let reply = get(
        &app,
        &format!("/studies/{study}/series/2.25.1/metadata"),
        json,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retrieve_urls_follow_the_proxy_and_the_mount_path() {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap());
    let config = Config {
        mount_path: "/dicom-web/".into(),
        ..Config::default()
    };
    let app = router(archive, config);
    let series = SyntheticSeries::new(68, 1);
    stow(&app, "/studies", series.all_bytes()).await;
    let reply = send(
        &app,
        "GET",
        "/studies",
        &[
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "pacs.example.org, proxy.internal"),
        ],
        Bytes::new(),
    )
    .await;
    assert_eq!(
        value(&reply.json()[0], "00081190"),
        &format!(
            "https://pacs.example.org/dicom-web/studies/{}",
            series.study_uid()
        )
    );

    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap());
    let config = Config {
        base_url: Some("https://fixed.example/root/".into()),
        ..Config::default()
    };
    let app = router(archive, config);
    stow(&app, "/studies", series.all_bytes()).await;
    let reply = get(&app, "/studies", "application/dicom+json").await;
    assert_eq!(
        value(&reply.json()[0], "00081190"),
        &format!("https://fixed.example/root/studies/{}", series.study_uid())
    );
}
