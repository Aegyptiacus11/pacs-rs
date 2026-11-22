use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use http_body_util::BodyExt;
use pacs_archive::{Archive, Compression};
use pacs_core::real::RealSeries;
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
    headers: axum::http::HeaderMap,
    body: Bytes,
}

impl Reply {
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .map(|h| h.to_str().unwrap())
            .unwrap_or_default()
    }
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
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
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
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        content_type,
        headers,
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
async fn two_series_study(app: &Router) -> (RealSeries, RealSeries) {
    let big = RealSeries::new(61, 3).described("Axial series 1");
    let small = RealSeries::new(62, 2)
        .in_study(61, 2)
        .described("Axial series 2");
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
async fn the_requests_a_viewer_makes() {
    let app = app().await;
    let (big, small) = two_series_study(&app).await;
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
    // a series result carries its study's attributes too, and says it is UTF-8
    assert_eq!(value(&series[0], "00100020"), "NEMA-61");
    assert_eq!(value(&series[0], "00080005"), "ISO_IR 192");

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
        // the scanner's own position, the slices being one image
        assert_eq!(instance["00200032"]["Value"][2], "-75.699997");
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
    let (big, _) = two_series_study(&app).await;
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
    assert_eq!(value(&instances[2], "00080056"), "ONLINE");
    assert_eq!(value(&instances[2], "00080060"), "CT");
    assert_eq!(value(&instances[2], "00080050"), "ACC61");

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
    let native = RealSeries::new(63, 1);
    let jls = RealSeries::new(64, 1);
    // the file of ten frames as it is, and as it is in JPEG-LS: a fragment a frame
    let cut = RealSeries::new(65, 1);
    let fragments = RealSeries::jpeg_ls_frames();
    let files = vec![
        native.multiframe_bytes(0),
        jls.jpeg_ls_multiframe_bytes(0),
        // the same codestreams, the first cut short: a frame that does not decode
        cut.refragmented(
            0,
            2,
            vec![],
            vec![fragments[0][..1000].to_vec(), fragments[1].clone()],
        ),
    ];
    assert_eq!(stow(&app, "/studies", files).await.status, StatusCode::OK);

    let frames_uri = |s: &RealSeries, list: &str| {
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
    assert_eq!(parts[0].content, le_bytes(&RealSeries::frame_pixels(2)));
    assert_eq!(parts[1].content, le_bytes(&RealSeries::frame_pixels(0)));

    let reply = get(&app, &frames_uri(&native, "11"), CORNERSTONE_FRAMES).await;
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

    // asked uncompressed, the JPEG-LS frame is decoded: the pixels of the native file
    let uncompressed = r#"multipart/related; type="application/octet-stream""#;
    let reply = get(&app, &frames_uri(&jls, "1"), uncompressed).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.parts()[0].content,
        le_bytes(&RealSeries::frame_pixels(0))
    );

    // a codestream cut short does not decode: nothing uncompressed to answer with
    let reply = get(&app, &frames_uri(&cut, "1"), uncompressed).await;
    assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn stored_as_jpeg_ls_and_transcoded_on_request() {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap());
    let app = router(archive, Config::default());
    let series = RealSeries::new(66, 1);
    let sent = series.multiframe_bytes(0);
    assert_eq!(
        stow(&app, "/studies", vec![sent.clone()]).await.status,
        StatusCode::OK
    );
    let frames_uri = |list: &str| {
        format!(
            "/studies/{}/series/{}/instances/{}/frames/{list}",
            series.study_uid(),
            series.series_uid(),
            series.instance_uid(0)
        )
    };
    let instance_uri = format!(
        "/studies/{}/series/{}/instances/{}",
        series.study_uid(),
        series.series_uid(),
        series.instance_uid(0)
    );
    let expected = [
        le_bytes(&RealSeries::frame_pixels(0)),
        le_bytes(&RealSeries::frame_pixels(1)),
    ];
    let shape = pacs_core::codec::Shape::new(64, 64, 16, 12);

    // the viewer takes whatever is stored: JPEG-LS
    let reply = get(&app, &frames_uri("1,2"), CORNERSTONE_FRAMES).await;
    let parts = reply.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("image/jls; transfer-syntax=1.2.840.10008.1.2.4.80")
    );
    for (part, expected) in parts.iter().zip(&expected) {
        let decoded = pacs_core::codec::decode(JPEG_LS, shape, &part.content).unwrap();
        assert_eq!(&decoded, expected);
    }

    // a client that wants uncompressed frames gets them, exactly as sent
    let reply = get(
        &app,
        &frames_uri("2"),
        r#"multipart/related; type="application/octet-stream""#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let parts = reply.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("application/octet-stream; transfer-syntax=1.2.840.10008.1.2.1")
    );
    assert_eq!(parts[0].content, expected[1]);

    // and one that wants JPEG XL gets JPEG XL
    let reply = get(
        &app,
        &frames_uri("1"),
        r#"multipart/related; type="image/jxl""#,
    )
    .await;
    let parts = reply.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("image/jxl; transfer-syntax=1.2.840.10008.1.2.4.110")
    );
    let decoded =
        pacs_core::codec::decode("1.2.840.10008.1.2.4.110", shape, &parts[0].content).unwrap();
    assert_eq!(decoded, expected[0]);

    // the instance: stored JPEG-LS, or uncompressed when asked, pixels the same either way
    let reply = get(&app, &instance_uri, "application/dicom").await;
    let stored = pacs_core::Instance::from_bytes(&reply.body).unwrap();
    assert_eq!(stored.key.transfer_syntax_uid, JPEG_LS);
    let reply = get(
        &app,
        &instance_uri,
        r#"multipart/related; type="application/dicom"; transfer-syntax=1.2.840.10008.1.2.1"#,
    )
    .await;
    let parts = reply.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("application/dicom; transfer-syntax=1.2.840.10008.1.2.1")
    );
    let native = pacs_core::Instance::from_bytes(&parts[0].content).unwrap();
    assert_eq!(native.key.transfer_syntax_uid, "1.2.840.10008.1.2.1");
    for (n, expected) in (1..=2).zip(&expected) {
        assert_eq!(&native.native_frame(n, shape).unwrap(), expected);
    }

    // nothing it cannot make
    let reply = get(
        &app,
        &instance_uri,
        r#"multipart/related; type="application/dicom"; transfer-syntax=1.2.840.10008.1.2.4.50"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn stow_reports_what_it_stored_and_what_it_did_not() {
    let app = app().await;
    let series = RealSeries::new(65, 2);
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
    let other = RealSeries::new(66, 1);
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
async fn searches_match_as_the_standard_says() {
    let app = app().await;
    two_series_study(&app).await;
    // two more studies, at other dates and times, one of another modality
    for series in [
        RealSeries::new(63, 1)
            .with_patient("Doe^Jane", None)
            .referred_by("REFERRING^PHYSICIAN")
            .at("20260105", "103000"),
        RealSeries::new(64, 1)
            .with_patient("DOE^JOHN", None)
            .referred_by("REFERRING^PHYSICIAN")
            .at("20260107", "090000")
            .of_modality("MR"),
    ] {
        assert_eq!(
            stow(&app, "/studies", series.all_bytes()).await.status,
            StatusCode::OK
        );
    }
    let json = "application/dicom+json";
    // the Patient IDs of the studies a search finds, sorted
    let found = async |query: &str| -> Vec<String> {
        let reply = get(&app, &format!("/studies?{query}"), json).await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{query}: {}",
            String::from_utf8_lossy(&reply.body)
        );
        let mut ids: Vec<String> = reply
            .json()
            .as_array()
            .unwrap()
            .iter()
            .map(|study| value(study, "00100020").as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        ids
    };

    for (query, expected) in [
        ("StudyInstanceUID=2.25.999", vec![]),
        // single value, by keyword and by tag
        ("PatientID=NEMA-61", vec!["NEMA-61"]),
        ("00100020=NEMA-61", vec!["NEMA-61"]),
        // wild cards: case-sensitive for text, not for names
        ("PatientID=NEMA-6?", vec!["NEMA-61", "NEMA-63", "NEMA-64"]),
        ("PatientID=nema-6?", vec![]),
        ("PatientName=doe*", vec!["NEMA-63", "NEMA-64"]),
        ("PatientName=*%5EJane", vec!["NEMA-63"]),
        ("PatientName=Doe%5EJane", vec!["NEMA-63"]),
        // an empty value, and a lone star: everything
        ("PatientName=", vec!["NEMA-61", "NEMA-63", "NEMA-64"]),
        ("PatientName=*", vec!["NEMA-61", "NEMA-63", "NEMA-64"]),
        // ranges of dates and of times
        ("StudyDate=20260105-20260107", vec!["NEMA-63", "NEMA-64"]),
        ("StudyDate=20260106-", vec!["NEMA-64"]),
        ("StudyDate=-20260101", vec!["NEMA-61"]),
        ("StudyDate=20260105", vec!["NEMA-63"]),
        ("StudyTime=0900-1100", vec!["NEMA-63", "NEMA-64"]),
        ("StudyTime=1030", vec!["NEMA-63"]),
        // a range of dates with a range of times: from the 5th at 11:00 to the 7th at 10:00
        (
            "StudyDate=20260105-20260107&StudyTime=1100-1000",
            vec!["NEMA-64"],
        ),
        (
            "StudyDate=20260105-20260107&StudyTime=1000-1000",
            vec!["NEMA-63", "NEMA-64"],
        ),
        // a list of UIDs
        (
            "StudyInstanceUID=2.25.63100000%2C2.25.64100000%2C2.25.9",
            vec!["NEMA-63", "NEMA-64"],
        ),
        // the modalities of a study's series
        ("ModalitiesInStudy=MR", vec!["NEMA-64"]),
        ("ModalitiesInStudy=CT", vec!["NEMA-61", "NEMA-63"]),
        // the other required keys
        (
            "AccessionNumber=ACC6*",
            vec!["NEMA-61", "NEMA-63", "NEMA-64"],
        ),
        ("StudyID=S63", vec!["NEMA-63"]),
        (
            "ReferringPhysicianName=referring*",
            vec!["NEMA-63", "NEMA-64"],
        ),
        // several keys, and a page of the result
        ("PatientName=doe*&ModalitiesInStudy=MR", vec!["NEMA-64"]),
        ("PatientName=doe*&limit=1&offset=1", vec!["NEMA-63"]),
        ("limit=1&offset=3", vec![]),
        // parameters it knows and does nothing with
        (
            "PatientID=NEMA-61&includefield=all&fuzzymatching=false",
            vec!["NEMA-61"],
        ),
    ] {
        assert_eq!(found(query).await, expected, "{query}");
    }

    // what is not a match for its attribute, or not a key here, is refused, with the reason
    for (query, why) in [
        ("StudyDate=2026-01-05", "StudyDate"),
        ("StudyDate=20260107-20260105", "ends before it starts"),
        ("StudyDate=-", "one end at least"),
        ("StudyTime=2500", "is not a time"),
        ("StudyInstanceUID=2.25.*", "is not a UID"),
        ("PatientID=NEMA-61&PatientID=NEMA-63", "given twice"),
        ("PatientID=NEMA-61&00100020=NEMA-63", "given twice"),
        ("PatientWeight=70", "cannot search on PatientWeight"),
        ("limit=x", "limit is not a number"),
        ("fuzzymatching=maybe", "true or false"),
    ] {
        let reply = get(&app, &format!("/studies?{query}"), json).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        // a refusal is what the API's description says it is
        serde_json::from_slice::<pacs_dicomweb::schema::Problem>(&reply.body).unwrap();
        assert!(
            String::from_utf8_lossy(&reply.body).contains(why),
            "{query}: {}",
            String::from_utf8_lossy(&reply.body)
        );
    }

    // kinds of matching asked for and not done: the search is made without, and says so
    let reply = get(
        &app,
        "/studies?PatientName=doe*&fuzzymatching=true&emptyvaluematching=true",
        json,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json().as_array().unwrap().len(), 2);
    let warnings: Vec<&str> = reply
        .headers
        .get_all("warning")
        .iter()
        .map(|w| w.to_str().unwrap())
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].starts_with("299 ") && warnings[0].ends_with("The fuzzymatching parameter is not supported. Only literal matching has been performed."), "{warnings:?}");
    assert!(
        warnings[1].ends_with("Empty Value Matching has not been performed."),
        "{warnings:?}"
    );
    assert!(
        get(&app, "/studies?PatientName=doe*", json)
            .await
            .headers
            .get("warning")
            .is_none()
    );

    let reply = get(&app, "/studies", "application/dicom+xml").await;
    assert_eq!(reply.status, StatusCode::NOT_ACCEPTABLE);

    // a study's series, and a series' instances
    let study = RealSeries::new(61, 1).study_uid();
    let count = async |path: &str| -> usize {
        let reply = get(&app, &format!("/studies/{study}/{path}"), json).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        reply.json().as_array().unwrap().len()
    };
    let (big, small) = (
        RealSeries::new(61, 3),
        RealSeries::new(62, 2).in_study(61, 2),
    );
    for (path, expected) in [
        ("series?Modality=MR".to_owned(), 0),
        ("series?Modality=CT".to_owned(), 2),
        ("series?Modality=C*".to_owned(), 2),
        ("series?Modality=ct".to_owned(), 0),
        ("series?SeriesNumber=2".to_owned(), 1),
        ("series?SeriesNumber=02".to_owned(), 1),
        ("series?SeriesDescription=*series%201".to_owned(), 1),
        (
            format!(
                "series?SeriesInstanceUID={}%2C{}",
                big.series_uid(),
                small.series_uid()
            ),
            2,
        ),
        (
            format!("series?SeriesInstanceUID={}", small.series_uid()),
            1,
        ),
        ("series?limit=1&offset=1".to_owned(), 1),
        (format!("series/{}/instances", big.series_uid()), 3),
        (
            format!("series/{}/instances?InstanceNumber=2", big.series_uid()),
            1,
        ),
        (
            format!(
                "series/{}/instances?SOPInstanceUID={}%2C{}",
                big.series_uid(),
                big.instance_uid(0),
                big.instance_uid(2)
            ),
            2,
        ),
        (
            format!(
                "series/{}/instances?SOPClassUID=1.2.840.10008.5.1.4.1.1.2",
                big.series_uid()
            ),
            3,
        ),
        (
            format!(
                "series/{}/instances?SOPClassUID=1.2.840.10008.5.1.4.1.1.4",
                big.series_uid()
            ),
            0,
        ),
    ] {
        assert_eq!(count(&path).await, expected, "{path}");
    }
    for bad in [
        "series?SeriesNumber=two",
        "series?Modality=CT&00080060=MR",
        "series?PatientID=NEMA-61",
    ] {
        let reply = get(&app, &format!("/studies/{study}/{bad}"), json).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}");
    }
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
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
    let config = Config {
        mount_path: "/dicom-web/".into(),
        ..Config::default()
    };
    let app = router(archive, config);
    let series = RealSeries::new(68, 1);
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

    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
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

/// The client's order decides, not whether the stored form is acceptable somewhere in the
/// list: a viewer asking for JPEG-LS first, anything after, gets JPEG-LS of a series
/// stored uncompressed.
#[tokio::test]
async fn the_clients_first_choice_wins() {
    let app = app().await;
    let series = RealSeries::new(67, 1);
    assert_eq!(
        stow(&app, "/studies", vec![series.multiframe_bytes(0)])
            .await
            .status,
        StatusCode::OK
    );
    let uri = format!(
        "/studies/{}/series/{}/instances/{}/frames/1",
        series.study_uid(),
        series.series_uid(),
        series.instance_uid(0)
    );
    let jls_first = format!(
        r#"multipart/related; type="image/jls"; transfer-syntax={JPEG_LS}, {CORNERSTONE_FRAMES}"#
    );
    let parts = get(&app, &uri, &jls_first).await.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("image/jls; transfer-syntax=1.2.840.10008.1.2.4.80")
    );
    let shape = pacs_core::codec::Shape::new(64, 64, 16, 12);
    let decoded = pacs_core::codec::decode(JPEG_LS, shape, &parts[0].content).unwrap();
    assert_eq!(decoded, le_bytes(&RealSeries::frame_pixels(0)));

    // ranked lower, JPEG-LS loses to the stored form
    let jls_less = format!(r#"multipart/related; type="image/jls"; q=0.5, {CORNERSTONE_FRAMES}"#);
    let parts = get(&app, &uri, &jls_less).await.parts();
    assert_eq!(
        parts[0].content_type.as_deref(),
        Some("application/octet-stream; transfer-syntax=1.2.840.10008.1.2.1")
    );
}

#[tokio::test]
async fn the_browser_keeps_what_cannot_change() {
    let app = app().await;
    let (big, _) = two_series_study(&app).await;
    let frame = format!(
        "/studies/{}/series/{}/instances/{}/frames/1",
        big.study_uid(),
        big.series_uid(),
        big.instance_uid(0)
    );
    let series = format!("/studies/{}/series/{}", big.study_uid(), big.series_uid());
    let metadata = format!("{series}/metadata");

    let first = get(&app, &frame, CORNERSTONE_FRAMES).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.header("cache-control"), "private, max-age=86400");
    assert_eq!(first.header("vary"), "Accept");
    let etag = first.header("etag").to_owned();
    assert!(etag.starts_with(r#"W/""#), "{etag}");

    // asked again with the tag: 304, no body
    let again = send(
        &app,
        "GET",
        &frame,
        &[("accept", CORNERSTONE_FRAMES), ("if-none-match", &etag)],
        Bytes::new(),
    )
    .await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert!(again.body.is_empty());
    assert_eq!(again.header("etag"), etag);

    // another Accept is another response
    let other = send(
        &app,
        "GET",
        &frame,
        &[
            ("accept", r#"multipart/related; type="image/jls""#),
            ("if-none-match", &etag),
        ],
        Bytes::new(),
    )
    .await;
    assert_eq!(other.status, StatusCode::OK);
    assert_ne!(other.header("etag"), etag);

    // a series gains instances, so it is asked about each time
    let accept = r#"multipart/related; type="application/dicom""#;
    let whole = get(&app, &series, accept).await;
    assert_eq!(whole.header("cache-control"), "private, no-cache");
    let etag = whole.header("etag").to_owned();
    let headers = [("accept", accept), ("if-none-match", etag.as_str())];
    let reply = send(&app, "GET", &series, &headers, Bytes::new()).await;
    assert_eq!(reply.status, StatusCode::NOT_MODIFIED);

    let meta = get(&app, &metadata, "application/dicom+json").await;
    assert_eq!(meta.header("cache-control"), "private, no-cache");
    let etag = meta.header("etag").to_owned();
    let headers = [
        ("accept", "application/dicom+json"),
        ("if-none-match", etag.as_str()),
    ];
    let reply = send(&app, "GET", &metadata, &headers, Bytes::new()).await;
    assert_eq!(reply.status, StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn overwritten_instances_are_asked_about() {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off)
        .with_overwrite(true);
    let app = router(archive, Config::default());
    let series = RealSeries::new(68, 1);
    assert_eq!(
        stow(&app, "/studies", series.all_bytes()).await.status,
        StatusCode::OK
    );
    let uri = format!(
        "/studies/{}/series/{}/instances/{}/frames/1",
        series.study_uid(),
        series.series_uid(),
        series.instance_uid(0)
    );
    let first = get(&app, &uri, CORNERSTONE_FRAMES).await;
    assert_eq!(first.header("cache-control"), "private, no-cache");
    let etag = first.header("etag").to_owned();

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    assert_eq!(
        stow(&app, "/studies", series.all_bytes()).await.status,
        StatusCode::OK
    );
    let headers = [
        ("accept", CORNERSTONE_FRAMES),
        ("if-none-match", etag.as_str()),
    ];
    let reply = send(&app, "GET", &uri, &headers, Bytes::new()).await;
    assert_eq!(reply.status, StatusCode::OK, "a new copy, a new tag");
}

/// STOW-RS takes standard elements in UN and shows them under their VR in metadata; a length that
/// lies is refused, and only that part.
#[tokio::test]
async fn stow_reads_un_and_refuses_a_lying_length() {
    let app = app().await;
    let lying = pacs_core::real::AlteredFile::new("1.2.840.10008.1.2.1")
        .identifiers(68, b"UI")
        .long_declared((0x0043, 0x1001), b"UN", 0xC000_0000, b"short")
        .bytes();
    let reply = stow(
        &app,
        "/studies",
        vec![pacs_core::real::un_sequence_file(67), lying],
    )
    .await;
    assert_eq!(reply.status, StatusCode::ACCEPTED);
    let json = reply.json();
    assert_eq!(json["00081199"]["Value"].as_array().unwrap().len(), 1);
    assert_eq!(json["00081198"]["Value"].as_array().unwrap().len(), 1);
    let reply = get(
        &app,
        "/studies/2.25.671/series/2.25.672/instances/2.25.673/metadata",
        "application/dicom+json",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json()[0]["00081110"]["vr"], "SQ");
}

/// The same answer in what a client reads of it: the boundary of a multipart body is new
/// at each answer, its parts are not.
fn assert_same_answer(alone: &Reply, whole: &Reply, what: &str) {
    assert_eq!(alone.status, whole.status, "{what}");
    // (the validator is not compared: it holds when each archive received the file)
    assert_eq!(
        alone.header("cache-control"),
        whole.header("cache-control"),
        "{what}"
    );
    if whole.status != StatusCode::OK {
        assert_eq!(alone.json(), whole.json(), "{what}");
    } else if whole.content_type.starts_with("multipart/") {
        assert_eq!(alone.parts(), whole.parts(), "{what}");
        assert!(!whole.parts().is_empty(), "{what}");
    } else {
        assert_eq!(alone.content_type, whole.content_type, "{what}");
        assert_eq!(alone.body, whole.body, "{what}");
    }
}

#[tokio::test]
async fn frames_read_alone_are_answered_as_frames_cut_from_the_file() {
    // one archive cuts every frame from the whole file, as for the small files of the other
    // tests; the other reads frames alone, as for large files
    let whole = app().await;
    let alone = router(
        Archive::new(Store::memory(), Index::in_memory().await.unwrap())
            .with_compression(Compression::Off)
            .with_frames_alone_from(0),
        Config::default(),
    );
    let fragments = RealSeries::jpeg_ls_frames();
    let (head, tail) = fragments[0].split_at(1000);
    let three = vec![head.to_vec(), tail.to_vec(), fragments[1].clone()];
    let offset = (8 + head.len() + 8 + tail.len()) as u32;
    let unaltered = |seed| RealSeries::new(seed, 1);
    let altered = |seed, table: Vec<u32>, fragments: &[Vec<u8>]| {
        let series = RealSeries::new(seed, 1);
        let file = series.refragmented(0, 2, table, fragments.to_vec());
        (series, file)
    };
    // the first codestream cut short: a frame that does not decode
    let cut = [fragments[0][..1000].to_vec(), fragments[1].clone()];
    let (native, jls, single) = (unaltered(91), unaltered(92), unaltered(93));
    let (tabled, tabled_file) = altered(94, vec![0, offset], &three);
    let (untabled, untabled_file) = altered(95, vec![], &three);
    let (mistabled, mistabled_file) = altered(96, vec![0, offset - 4], &three);
    let (cut, cut_file) = altered(97, vec![], &cut);
    let files: Vec<(&str, RealSeries, Vec<u8>)> = vec![
        ("native", native.clone(), native.multiframe_bytes(0)),
        (
            "a fragment a frame",
            jls.clone(),
            jls.jpeg_ls_multiframe_bytes(0),
        ),
        ("one frame", single.clone(), single.slice_bytes(0)),
        ("an offset table", tabled, tabled_file),
        ("no table to go by", untabled, untabled_file),
        ("a wrong offset table", mistabled, mistabled_file),
        ("a codestream cut short", cut, cut_file),
    ];
    for app in [&whole, &alone] {
        let sent = files.iter().map(|(_, _, file)| file.clone()).collect();
        assert_eq!(stow(app, "/studies", sent).await.status, StatusCode::OK);
    }

    let uncompressed = r#"multipart/related; type="application/octet-stream""#;
    let mut answered = 0;
    for (name, series, _) in &files {
        for list in ["1", "2", "2,1", "10", "3,1,3", "11", "0", "1,11", "x"] {
            for accept in [CORNERSTONE_FRAMES, uncompressed, "application/octet-stream"] {
                let uri = format!(
                    "/studies/{}/series/{}/instances/{}/frames/{list}",
                    series.study_uid(),
                    series.series_uid(),
                    series.instance_uid(0)
                );
                let what = format!("{name}: frames {list} as {accept}");
                let from_whole = get(&whole, &uri, accept).await;
                let from_alone = get(&alone, &uri, accept).await;
                assert_same_answer(&from_alone, &from_whole, &what);
                answered += usize::from(from_whole.status == StatusCode::OK);
                // and again, by the layout the first request left
                let again = get(&alone, &uri, accept).await;
                assert_same_answer(&again, &from_whole, &what);
            }
        }
    }
    assert!(answered > 40, "{answered} requests answered with frames");
}

#[tokio::test]
async fn a_frame_of_a_large_file_is_read_without_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(
        Archive::new(
            Store::local(dir.path(), "").unwrap(),
            Index::in_memory().await.unwrap(),
        )
        .with_compression(Compression::Off)
        .with_frames_alone_from(0),
        Config::default(),
    );
    let series = RealSeries::new(98, 1);
    let file = series.multiframe_bytes(0);
    assert_eq!(
        stow(&app, "/studies", vec![file.clone()]).await.status,
        StatusCode::OK
    );
    // the stored file with nothing left of its pixel data but frame 3
    let frame_len = le_bytes(&RealSeries::frame_pixels(0)).len();
    let pixel_data = file.len() - frame_len * RealSeries::FRAMES as usize;
    let third = pixel_data + 2 * frame_len..pixel_data + 3 * frame_len;
    let mut holed = file.clone();
    holed[pixel_data..third.start].fill(0);
    holed[third.end..].fill(0);
    // the instance's file, under a key of its own: <instance UID>.<random>.dcm
    let folder = dir
        .path()
        .join(series.study_uid())
        .join(series.series_uid());
    let mut files: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let path = files.remove(0);
    let name = path.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with(&format!("{}.", series.instance_uid(0))),
        "{name}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), file);
    std::fs::write(&path, &holed).unwrap();

    let frame = |number: u32| {
        format!(
            "/studies/{}/series/{}/instances/{}/frames/{number}",
            series.study_uid(),
            series.series_uid(),
            series.instance_uid(0)
        )
    };
    let reply = get(&app, &frame(3), CORNERSTONE_FRAMES).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.parts()[0].content,
        le_bytes(&RealSeries::frame_pixels(2))
    );
    // the file is gone: reading the whole file reports it, as it did before
    std::fs::remove_file(&path).unwrap();
    let reply = get(&app, &frame(4), CORNERSTONE_FRAMES).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

/// PS3.18 8.3.4.1 and PS3.4 C.2.2.2.6: a key of a sequence's item is named
/// `Sequence.Attribute`; a series answers when one item of its sequence answers every key
/// asked of that sequence.
#[tokio::test]
async fn a_series_is_found_by_the_keys_of_one_request() {
    let app = app().await;
    let first = RealSeries::new(71, 1).requested(&[("STEP1", "REQ1"), ("STEP2", "REQ2")]);
    let second = RealSeries::new(72, 1)
        .in_study(71, 2)
        .requested(&[("STEP3", "REQ1")]);
    let bare = RealSeries::new(73, 1).in_study(71, 3);
    let mut files = first.all_bytes();
    files.extend(second.all_bytes());
    files.extend(bare.all_bytes());
    assert_eq!(stow(&app, "/studies", files).await.status, StatusCode::OK);
    let json = "application/dicom+json";
    let study = first.study_uid();
    let found = async |query: &str| -> Vec<String> {
        let reply = get(&app, &format!("/studies/{study}/series?{query}"), json).await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{query}: {}",
            String::from_utf8_lossy(&reply.body)
        );
        let mut numbers: Vec<String> = reply
            .json()
            .as_array()
            .unwrap()
            .iter()
            .map(|series| value(series, "00200011").to_string())
            .collect();
        numbers.sort();
        numbers
    };
    const STEP: &str = "RequestAttributesSequence.ScheduledProcedureStepID";
    const PROCEDURE: &str = "RequestAttributesSequence.RequestedProcedureID";
    for (query, expected) in [
        (format!("{STEP}=STEP1"), vec!["1"]),
        (format!("{PROCEDURE}=REQ1"), vec!["1", "2"]),
        (format!("{STEP}=STEP*"), vec!["1", "2"]),
        // both keys of the one item: STEP1 and REQ2 are in two items of the first series
        (format!("{STEP}=STEP1&{PROCEDURE}=REQ2"), vec![]),
        (format!("{STEP}=STEP2&{PROCEDURE}=REQ2"), vec!["1"]),
        ("00400275.00400009=STEP3".to_owned(), vec!["2"]),
        (
            "00400275.00401001=REQ1&Modality=CT".to_owned(),
            vec!["1", "2"],
        ),
        // universal: the sequence or a key of it, empty
        ("RequestAttributesSequence=".to_owned(), vec!["1", "2", "3"]),
        (format!("{STEP}="), vec!["1", "2", "3"]),
    ] {
        assert_eq!(found(&query).await, expected, "{query}");
    }
    // the series that answers carries its requests
    let reply = get(&app, &format!("/studies/{study}/series?{STEP}=STEP3"), json).await;
    let items = reply.json()[0]["00400275"]["Value"].clone();
    assert_eq!(items[0]["00400009"]["Value"][0], "STEP3", "{items}");

    for bad in [
        "RequestAttributesSequence=STEP1".to_owned(),
        "RequestAttributesSequence.PatientID=NEMA-71".to_owned(),
        "RequestAttributesSequence.ScheduledProcedureStepID.Other=1".to_owned(),
        "OtherSequence.ScheduledProcedureStepID=STEP1".to_owned(),
        format!("{STEP}=STEP1&00400275.00400009=STEP2"),
    ] {
        let reply = get(&app, &format!("/studies/{study}/series?{bad}"), json).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}");
    }
    // a key of the series, not of the study or the instance
    for path in [
        format!("/studies?{STEP}=STEP1"),
        format!(
            "/studies/{study}/series/{}/instances?{STEP}=STEP1",
            first.series_uid()
        ),
    ] {
        let reply = get(&app, &path, json).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{path}");
    }
}

/// `includefield` (PS3.18 8.3.4.3): the attributes asked for, by keyword or tag, from the
/// metadata of what is found; `all` those of the level's modules; an unknown name is 400.
#[tokio::test]
async fn includefield_brings_the_attributes_asked_for() {
    let app = app().await;
    let (big, _) = two_series_study(&app).await;
    let json = "application/dicom+json";
    let study = big.study_uid();
    let answer = async |path: String| -> Vec<Value> {
        let reply = get(&app, &path, json).await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{path}: {}",
            String::from_utf8_lossy(&reply.body)
        );
        reply.json().as_array().unwrap().clone()
    };
    let has = |found: &Value, tag: &str| found.get(tag).is_some();

    // not kept at the study level, so only when asked for
    let plain = answer("/studies".to_owned()).await;
    assert!(!has(&plain[0], "00080070"));
    for asked in [
        "Manufacturer",
        "00080070",
        "00080070,StudyDescription",
        "00080070&includefield=KVP",
    ] {
        let found = answer(format!("/studies?includefield={asked}")).await;
        assert_eq!(
            value(&found[0], "00080070"),
            "GE MEDICAL SYSTEMS",
            "{asked}"
        );
    }
    // an instance's own attribute, at the instance level
    let instances = answer(format!(
        "/studies/{study}/series/{}/instances?includefield=SliceThickness",
        big.series_uid()
    ))
    .await;
    assert_eq!(instances.len(), 3);
    assert!(instances.iter().all(|i| has(i, "00180050")));

    // all: the modules of the level
    let series = answer(format!("/studies/{study}/series?includefield=all")).await;
    assert!(
        series.iter().all(|s| has(s, "00080070")),
        "General Equipment"
    );
    assert!(
        series.iter().all(|s| !has(s, "00180050")),
        "no instance attribute"
    );
    let instances = answer(format!(
        "/studies/{study}/series/{}/instances?includefield=all",
        big.series_uid()
    ))
    .await;
    assert!(
        instances
            .iter()
            .all(|i| has(i, "00180050") && has(i, "00080070"))
    );
    // what the answer has is not changed
    assert_eq!(
        answer("/studies?includefield=StudyInstanceUID".to_owned()).await[0]["0020000D"],
        plain[0]["0020000D"]
    );

    for bad in ["NoSuchKeyword", "0008007"] {
        let reply = get(&app, &format!("/studies?includefield={bad}"), json).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// A PNG answer decoded: width, height, samples a pixel, the samples.
fn decoded_png(body: &[u8]) -> (u32, u32, usize, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(body.to_vec()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    (info.width, info.height, info.color_type.samples(), pixels)
}

/// The width and height of a JPEG, from its start-of-frame segment.
fn jpeg_size(body: &[u8]) -> (u16, u16) {
    assert_eq!(&body[..2], [0xFF, 0xD8], "not a JPEG");
    let mut at = 2;
    while at + 9 < body.len() {
        let (marker, len) = (
            body[at + 1],
            u16::from_be_bytes([body[at + 2], body[at + 3]]),
        );
        if (0xC0..=0xC2).contains(&marker) {
            let height = u16::from_be_bytes([body[at + 5], body[at + 6]]);
            let width = u16::from_be_bytes([body[at + 7], body[at + 8]]);
            return (width, height);
        }
        at += 2 + usize::from(len);
    }
    panic!("no start of frame");
}

/// Rendered resources (PS3.18 9.5): JPEG by default, PNG when asked; the window, the
/// viewport and the quality as the query says; what cannot be met refused.
#[tokio::test]
async fn an_instance_is_rendered_as_asked() {
    let app = app().await;
    let (big, _) = two_series_study(&app).await;
    let at = format!(
        "/studies/{}/series/{}/instances/{}",
        big.study_uid(),
        big.series_uid(),
        big.instance_uid(0)
    );
    let jpeg = get(&app, &format!("{at}/rendered"), "*/*").await;
    assert_eq!(jpeg.status, StatusCode::OK);
    assert_eq!(jpeg.content_type, "image/jpeg");
    assert_eq!(jpeg_size(&jpeg.body), (128, 128));
    let small = get(&app, &format!("{at}/rendered?quality=10"), "image/jpeg").await;
    assert!(small.body.len() < jpeg.body.len());

    // as the file says, and the window asked for: all white below it, all black above
    let png = |query: &str| {
        let path = format!("{at}/rendered{query}");
        let app = app.clone();
        async move {
            let reply = get(&app, &path, "image/png").await;
            assert_eq!(reply.status, StatusCode::OK, "{path}");
            assert_eq!(reply.content_type, "image/png");
            decoded_png(&reply.body)
        }
    };
    let (w, h, samples, pixels) = png("").await;
    assert_eq!((w, h, samples), (128, 128, 1));
    assert!(pixels.iter().any(|&p| p > 0) && pixels.iter().any(|&p| p < 255));
    assert!(
        png("?window=-5000,2,linear")
            .await
            .3
            .iter()
            .all(|&p| p == 255)
    );
    assert!(
        png("?window=50000,2,linear")
            .await
            .3
            .iter()
            .all(|&p| p == 0)
    );
    let (w, h, ..) = png("?viewport=64,32").await;
    assert_eq!((w, h), (32, 32));
    let (w, h, ..) = png("?viewport=40,20,0,0,64,32&annotation=patient").await;
    assert_eq!((w, h), (40, 20));
    // a frame by its number
    let one = get(&app, &format!("{at}/frames/1/rendered"), "image/png").await;
    assert_eq!(decoded_png(&one.body).3, pixels);

    for (query, status) in [
        ("/rendered?window=40,400", StatusCode::BAD_REQUEST),
        ("/rendered?viewport=0,10", StatusCode::BAD_REQUEST),
        ("/rendered?quality=0", StatusCode::BAD_REQUEST),
        ("/rendered?colour=red", StatusCode::BAD_REQUEST),
        (
            "/rendered?viewport=10,10,100,100,64,64",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("/frames/2/rendered", StatusCode::NOT_FOUND),
        ("/frames/0/rendered", StatusCode::BAD_REQUEST),
        ("/frames/1,2/rendered", StatusCode::BAD_REQUEST),
    ] {
        let reply = get(&app, &format!("{at}{query}"), "image/jpeg").await;
        assert_eq!(reply.status, status, "{query}");
    }
    let html = get(&app, &format!("{at}/rendered"), "text/html").await;
    assert_eq!(html.status, StatusCode::NOT_ACCEPTABLE);
}

/// Colour rendered as RGB: the ultrasound's own pixels, from the JPEG-LS the archive keeps;
/// palette colour through its lookup tables.
#[tokio::test]
async fn colour_is_rendered_as_rgb() {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap());
    let app = router(archive, Config::default());
    for (name, palette) in [
        ("examples_rgb_color.dcm", false),
        ("examples_palette.dcm", true),
    ] {
        let file = pacs_core::real::file_bytes(name);
        let instance = pacs_core::Instance::from_bytes(&file).unwrap();
        assert_eq!(
            stow(&app, "/studies", vec![file.clone()]).await.status,
            StatusCode::OK
        );
        let key = &instance.key;
        let reply = get(
            &app,
            &format!(
                "/studies/{}/series/{}/instances/{}/rendered",
                key.study_instance_uid, key.series_instance_uid, key.sop_instance_uid
            ),
            "image/png",
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{name}");
        let (w, h, samples, pixels) = decoded_png(&reply.body);
        let shape = instance.shape().unwrap();
        assert_eq!(
            (w, h, samples),
            (u32::from(shape.columns), u32::from(shape.rows), 3),
            "{name}"
        );
        if !palette {
            assert_eq!(pixels, instance.native_frame(1, shape).unwrap(), "{name}");
        } else {
            assert!(
                pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .any(|[r, g, b]| r != g || g != b)
            );
        }
    }
}
