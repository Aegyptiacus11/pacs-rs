//! The same files stored in pacs-rs and in a reference server; the same requests to both,
//! over DICOMweb and over the REST API; the answers compared tag by tag.
//!
//! Runs only when `PACS_TEST_REFERENCE_URL` names the reference server's DICOMweb root,
//! e.g. `http://127.0.0.1:18042/dicom-web` (README, "Comparing with a reference server").
//! Differences that are deliberate are listed, with the reason, in `ALLOWED`; anything
//! else fails the test with the full list of what differs.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dicomweb::media::MediaType;
use pacs_dicomweb::multipart;
use pacs_server::{Settings, app, open_archive, serve};
use serde_json::Value;

const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";
const FRAMES_ACCEPT: &str = "multipart/related; type=application/octet-stream; transfer-syntax=*";
const JSON: &str = "application/dicom+json";

/// (where, tag, which side has it alone, why that is fine)
const ALLOWED: &[(&str, &str, Side, &str)] = &[
    (
        "study",
        "00081030",
        Side::Pacs,
        "Study Description is sent unasked; the reference sends it only for includefield",
    ),
    (
        "series",
        "00081030",
        Side::Pacs,
        "Study Description is sent unasked; the reference sends it only for includefield",
    ),
    (
        "instances",
        "00081030",
        Side::Pacs,
        "Study Description is sent unasked; the reference sends it only for includefield",
    ),
    (
        "metadata",
        "7FE00010",
        Side::Reference,
        "no BulkDataURI for pixel data yet; viewers fetch frames instead",
    ),
    (
        "rest-instance",
        "FileUuid",
        Side::Reference,
        "pacs-rs names files by their UIDs and has no attachment UUIDs",
    ),
];

/// Tags whose values may differ: (where, tag, why).
const ALLOWED_VALUES: &[(&str, &str, &str)] = &[
    (
        "rest-study",
        "LastUpdate",
        "each server's own clock at arrival",
    ),
    (
        "rest-series",
        "LastUpdate",
        "each server's own clock at arrival",
    ),
];

/// Lists compared as sets, their order being a deliberate difference: (where, tag, why).
const ALLOWED_ORDER: &[(&str, &str, &str)] = &[
    (
        "rest-series",
        "Instances",
        "pacs-rs lists a series' instances by Instance Number; the reference by ID",
    ),
    (
        "rest-study",
        "Series",
        "pacs-rs lists a study's series by Series Number; the reference by ID",
    ),
];

fn same_members(a: &Value, b: &Value) -> bool {
    let set = |v: &Value| -> Option<BTreeSet<String>> {
        Some(v.as_array()?.iter().map(|x| x.to_string()).collect())
    };
    set(a).is_some() && set(a) == set(b)
}

/// Part Content-Types that differ on purpose: (reference, pacs-rs, why).
const ALLOWED_PART_TYPES: &[(&str, &str, &str)] = &[(
    "application/dicom",
    "application/dicom; transfer-syntax=1.2.840.10008.1.2.1",
    "pacs-rs names the stored transfer syntax of each instance it sends",
)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Reference,
    Pacs,
}

struct Server {
    name: &'static str,
    base: String,
    client: reqwest::Client,
}

struct Reply {
    status: u16,
    content_type: String,
    body: Bytes,
}

impl Server {
    async fn get(&self, path: &str, accept: &str) -> Reply {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
            .header("accept", accept)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{} GET {path}: {e}", self.name));
        reply(response).await
    }

    async fn post(&self, path: &str, body: Vec<u8>) -> Reply {
        let response = self
            .client
            .post(format!("{}{path}", self.base))
            .body(body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{} POST {path}: {e}", self.name));
        reply(response).await
    }

    async fn stow(&self, files: &[Vec<u8>]) -> Reply {
        let boundary = multipart::new_boundary();
        let body = multipart::body(
            &boundary,
            files
                .iter()
                .map(|f| ("application/dicom".to_owned(), Bytes::from(f.clone()))),
        );
        let response = self
            .client
            .post(format!("{}/studies", self.base))
            .header(
                "content-type",
                format!(r#"multipart/related; type="application/dicom"; boundary={boundary}"#),
            )
            .header("accept", JSON)
            .body(body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{} STOW: {e}", self.name));
        reply(response).await
    }

    /// The JSON of a reply, with this server's base URL replaced by `BASE`.
    fn json(&self, reply: &Reply) -> Value {
        let mut value: Value = serde_json::from_slice(&reply.body).unwrap_or_else(|e| {
            panic!(
                "{}: not JSON ({e}): {}",
                self.name,
                String::from_utf8_lossy(&reply.body)
            )
        });
        rebase(&mut value, &self.base);
        value
    }
}

async fn reply(response: reqwest::Response) -> Reply {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|h| h.to_str().unwrap().to_owned())
        .unwrap_or_default();
    Reply {
        status,
        content_type,
        body: response.bytes().await.unwrap(),
    }
}

fn rebase(value: &mut Value, base: &str) {
    match value {
        Value::String(s) if s.starts_with(base) => *s = format!("BASE{}", &s[base.len()..]),
        Value::Array(items) => items.iter_mut().for_each(|v| rebase(v, base)),
        Value::Object(map) => map.values_mut().for_each(|v| rebase(v, base)),
        _ => {}
    }
}

fn parts(reply: &Reply) -> Vec<multipart::Part> {
    let media = MediaType::parse(&reply.content_type)
        .unwrap_or_else(|| panic!("no content type: {:?}", reply.content_type));
    let boundary = media
        .param("boundary")
        .unwrap_or_else(|| panic!("not multipart: {}", reply.content_type));
    multipart::parse(boundary, &reply.body).unwrap()
}

/// Differences found so far, as readable lines.
#[derive(Default)]
struct Report {
    lines: Vec<String>,
}

impl Report {
    fn push(&mut self, line: String) {
        self.lines.push(line);
    }

    fn allowed(&self, place: &str, tag: &str, side: Side) -> bool {
        ALLOWED
            .iter()
            .any(|(p, t, s, _)| *p == place && *t == tag && *s == side)
    }

    /// Two DICOM JSON data sets.
    fn objects(&mut self, place: &str, what: &str, reference: &Value, pacs: &Value) {
        let (Some(r), Some(p)) = (reference.as_object(), pacs.as_object()) else {
            self.push(format!("{what}: not both objects"));
            return;
        };
        let tags: BTreeSet<&String> = r.keys().chain(p.keys()).collect();
        for tag in tags {
            match (r.get(tag), p.get(tag)) {
                (Some(a), Some(b))
                    if a != b
                        && !ALLOWED_VALUES
                            .iter()
                            .any(|(p, t, _)| *p == place && t == tag)
                        && !(ALLOWED_ORDER
                            .iter()
                            .any(|(p, t, _)| *p == place && t == tag)
                            && same_members(a, b)) =>
                {
                    self.push(format!("{what} {tag}: reference {a} | pacs-rs {b}"))
                }
                (Some(a), None) if !self.allowed(place, tag, Side::Reference) => {
                    self.push(format!("{what} {tag}: only reference: {a}"))
                }
                (None, Some(b)) if !self.allowed(place, tag, Side::Pacs) => {
                    self.push(format!("{what} {tag}: only pacs-rs: {b}"))
                }
                _ => {}
            }
        }
    }

    /// Two arrays of data sets, matched by the UID in `key`.
    fn arrays(&mut self, place: &str, what: &str, key: &str, reference: &Value, pacs: &Value) {
        let by_key = |v: &Value| -> BTreeMap<String, Value> {
            v.as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|i| {
                            (
                                i[key]["Value"][0].as_str().unwrap_or("?").to_owned(),
                                i.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let (r, p) = (by_key(reference), by_key(pacs));
        if r.keys().ne(p.keys()) {
            self.push(format!(
                "{what}: reference has {:?}, pacs-rs has {:?}",
                r.keys().collect::<Vec<_>>(),
                p.keys().collect::<Vec<_>>()
            ));
        }
        for (uid, a) in &r {
            if let Some(b) = p.get(uid) {
                self.objects(place, &format!("{what} [{uid}]"), a, b);
            }
        }
    }

    fn status(&mut self, what: &str, reference: &Reply, pacs: &Reply) -> bool {
        if reference.status != pacs.status {
            self.push(format!(
                "{what}: status reference {} | pacs-rs {}",
                reference.status, pacs.status
            ));
        }
        reference.status == 200 && pacs.status == 200
    }

    /// Two multipart answers: the parts' types and contents.
    fn multipart(&mut self, what: &str, reference: &Reply, pacs: &Reply) {
        let (r, p) = (parts(reference), parts(pacs));
        if r.len() != p.len() {
            self.push(format!("{what}: {} parts | {} parts", r.len(), p.len()));
            return;
        }
        for (i, (a, b)) in r.iter().zip(&p).enumerate() {
            let allowed = ALLOWED_PART_TYPES.iter().any(|(r, p, _)| {
                a.content_type.as_deref() == Some(*r) && b.content_type.as_deref() == Some(*p)
            });
            if a.content_type != b.content_type && !allowed {
                self.push(format!(
                    "{what} part {i} type: reference {:?} | pacs-rs {:?}",
                    a.content_type, b.content_type
                ));
            }
            if a.content != b.content {
                self.push(format!(
                    "{what} part {i}: {} bytes | {} bytes, contents differ",
                    a.content.len(),
                    b.content.len()
                ));
            }
        }
    }
}

async fn start_pacs() -> (Server, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let settings = <Settings as clap::Parser>::try_parse_from([
        "pacs-server".to_owned(),
        "--index".into(),
        dir.path().join("index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.path().join("files").display()),
    ])
    .unwrap();
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/dicom-web", listener.local_addr().unwrap());
    tokio::spawn(serve(listener, app, std::future::pending()));
    let server = Server {
        name: "pacs-rs",
        base,
        client: reqwest::Client::new(),
    };
    (server, dir)
}

/// The same server's REST root: its DICOMweb base without `/dicom-web`.
fn rest_root(dicomweb: &Server) -> Server {
    Server {
        name: dicomweb.name,
        base: dicomweb.base.trim_end_matches("/dicom-web").to_owned(),
        client: dicomweb.client.clone(),
    }
}

fn unzip(reply: &Reply) -> BTreeMap<String, Vec<u8>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(reply.body.to_vec())).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            (file.name().to_owned(), bytes)
        })
        .collect()
}

#[tokio::test]
async fn same_rest_answers_as_the_reference_server() {
    let Ok(reference_base) = std::env::var("PACS_TEST_REFERENCE_URL") else {
        eprintln!("PACS_TEST_REFERENCE_URL not set; skipping");
        return;
    };
    let reference = rest_root(&Server {
        name: "reference",
        base: reference_base.trim_end_matches('/').to_owned(),
        client: reqwest::Client::new(),
    });
    let (pacs_dicomweb, _dir) = start_pacs().await;
    let pacs = rest_root(&pacs_dicomweb);
    let mut report = Report::default();

    let run = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        % 100_000) as u32
        + 100_000;
    let big = SyntheticSeries::new(run * 10 + 6, 3);
    let small = SyntheticSeries::new(run * 10 + 7, 2).in_study(run * 10 + 6, 2);
    let files: Vec<Vec<u8>> = big
        .all_bytes()
        .into_iter()
        .chain(small.all_bytes())
        .collect();

    // uploads, then the same file again
    let mut study_id = String::new();
    for (n, file) in files.iter().chain(files.first()).enumerate() {
        let (r, p) = (
            reference.post("/instances", file.clone()).await,
            pacs.post("/instances", file.clone()).await,
        );
        let what = format!("POST /instances #{n}");
        if report.status(&what, &r, &p) {
            let (r, p) = (reference.json(&r), pacs.json(&p));
            report.objects("rest-upload", &what, &r, &p);
            study_id = r["ParentStudy"].as_str().unwrap_or_default().to_owned();
        }
    }

    let find =
        serde_json::json!({"Level": "Study", "Query": {"StudyInstanceUID": big.study_uid()}});
    let (r, p) = (
        reference
            .post("/tools/find", find.to_string().into_bytes())
            .await,
        pacs.post("/tools/find", find.to_string().into_bytes())
            .await,
    );
    if report.status("/tools/find", &r, &p) && reference.json(&r) != pacs.json(&p) {
        report.push(format!(
            "/tools/find: reference {} | pacs-rs {}",
            reference.json(&r),
            pacs.json(&p)
        ));
    }

    let path = format!("/studies/{study_id}");
    let (r, p) = (
        reference.get(&path, "*/*").await,
        pacs.get(&path, "*/*").await,
    );
    let mut series_ids = Vec::new();
    if report.status(&path, &r, &p) {
        let (r, p) = (reference.json(&r), pacs.json(&p));
        report.objects("rest-study", &path, &r, &p);
        series_ids = r["Series"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
    }
    let path = format!("/studies/{study_id}/statistics");
    let (r, p) = (
        reference.get(&path, "*/*").await,
        pacs.get(&path, "*/*").await,
    );
    if report.status(&path, &r, &p) {
        report.objects(
            "rest-statistics",
            &path,
            &reference.json(&r),
            &pacs.json(&p),
        );
    }
    let path = format!("/studies/{study_id}/instances");
    let (r, p) = (
        reference.get(&path, "*/*").await,
        pacs.get(&path, "*/*").await,
    );
    let mut instance_ids = Vec::new();
    if report.status(&path, &r, &p) {
        let (r, p) = (reference.json(&r), pacs.json(&p));
        let by_id = |v: &Value| -> BTreeMap<String, Value> {
            v.as_array()
                .into_iter()
                .flatten()
                .map(|i| (i["ID"].as_str().unwrap_or("?").to_owned(), i.clone()))
                .collect()
        };
        let (r, p) = (by_id(&r), by_id(&p));
        if r.keys().ne(p.keys()) {
            report.push(format!("{path}: different instance IDs"));
        }
        for (id, a) in &r {
            if let Some(b) = p.get(id) {
                report.objects("rest-instance", &format!("{path} [{id}]"), a, b);
            }
        }
        instance_ids = r.into_keys().collect();
    }
    for id in &series_ids {
        let path = format!("/series/{id}");
        let (r, p) = (
            reference.get(&path, "*/*").await,
            pacs.get(&path, "*/*").await,
        );
        if report.status(&path, &r, &p) {
            report.objects("rest-series", &path, &reference.json(&r), &pacs.json(&p));
        }
    }
    for id in &instance_ids {
        let path = format!("/instances/{id}");
        let (r, p) = (
            reference.get(&path, "*/*").await,
            pacs.get(&path, "*/*").await,
        );
        if report.status(&path, &r, &p) {
            report.objects("rest-instance", &path, &reference.json(&r), &pacs.json(&p));
        }
        let path = format!("/instances/{id}/file");
        let (r, p) = (
            reference.get(&path, "*/*").await,
            pacs.get(&path, "*/*").await,
        );
        if report.status(&path, &r, &p) && r.body != p.body {
            report.push(format!("{path}: files differ"));
        }
    }
    let path = format!("/studies/{study_id}/archive");
    let (r, p) = (
        reference.get(&path, "*/*").await,
        pacs.get(&path, "*/*").await,
    );
    if report.status(&path, &r, &p) {
        let (r, p) = (unzip(&r), unzip(&p));
        if r.keys().ne(p.keys()) {
            report.push(format!(
                "{path}: names reference {:?} | pacs-rs {:?}",
                r.keys().collect::<Vec<_>>(),
                p.keys().collect::<Vec<_>>()
            ));
        }
        for (name, a) in &r {
            if p.get(name).is_some_and(|b| a != b) {
                report.push(format!("{path} {name}: contents differ"));
            }
        }
    }
    for path in ["/studies/unknown", "/series/unknown", "/instances/unknown"] {
        let (r, p) = (
            reference.get(path, "*/*").await,
            pacs.get(path, "*/*").await,
        );
        report.status(path, &r, &p);
    }

    assert!(
        report.lines.is_empty(),
        "{} difference(s) from the reference server:\n{}",
        report.lines.len(),
        report.lines.join("\n")
    );
}

#[tokio::test]
async fn same_answers_as_the_reference_server() {
    let Ok(reference_base) = std::env::var("PACS_TEST_REFERENCE_URL") else {
        eprintln!("PACS_TEST_REFERENCE_URL not set; skipping");
        return;
    };
    let reference = Server {
        name: "reference",
        base: reference_base.trim_end_matches('/').to_owned(),
        client: reqwest::Client::new(),
    };
    let (pacs, _dir) = start_pacs().await;
    let mut report = Report::default();

    // Fresh UIDs every run: the reference server keeps the first copy of an instance it
    // is sent again, so reused UIDs would compare against an older run's files.
    let run = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        % 100_000) as u32
        + 100_000;
    let seed = |n: u32| run * 10 + n;
    // a study of two series, a native multi-frame instance, an encapsulated one, and a
    // patient name in ISO 8859-1
    let big = SyntheticSeries::new(seed(1), 3);
    let small = SyntheticSeries::new(seed(2), 2).in_study(seed(1), 2);
    let multiframe = SyntheticSeries::new(seed(3), 1);
    let jls = SyntheticSeries::new(seed(4), 1);
    let latin1 = SyntheticSeries::new(seed(5), 2).with_patient("MÜLLER^RENÉ", Some("ISO_IR 100"));
    let mut files = big.all_bytes();
    files.extend(small.all_bytes());
    files.extend(latin1.all_bytes());
    files.push(multiframe.multiframe_bytes(0, 3));
    files.push(jls.encapsulated_bytes(0, JPEG_LS, 2, vec![], vec![vec![7u8; 6], vec![9u8; 10]]));

    let (r, p) = (reference.stow(&files).await, pacs.stow(&files).await);
    if report.status("STOW", &r, &p) {
        let (r, p) = (reference.json(&r), pacs.json(&p));
        report.arrays(
            "stow",
            "STOW referenced",
            "00081155",
            &r["00081199"]["Value"],
            &p["00081199"]["Value"],
        );
    }

    for series in [&big, &multiframe, &jls, &latin1] {
        let study = series.study_uid();
        let path = format!("/studies?StudyInstanceUID={study}&includefield=00201208");
        let (r, p) = (
            reference.get(&path, JSON).await,
            pacs.get(&path, JSON).await,
        );
        if report.status(&path, &r, &p) {
            report.arrays(
                "study",
                &path,
                "0020000D",
                &reference.json(&r),
                &pacs.json(&p),
            );
        }
        let path = format!("/studies/{study}/series?includefield=00201209");
        let (r, p) = (
            reference.get(&path, JSON).await,
            pacs.get(&path, JSON).await,
        );
        if report.status(&path, &r, &p) {
            report.arrays(
                "series",
                &path,
                "0020000E",
                &reference.json(&r),
                &pacs.json(&p),
            );
        }
    }

    for series in [&big, &small, &latin1] {
        let root = format!(
            "/studies/{}/series/{}",
            series.study_uid(),
            series.series_uid()
        );
        let path = format!("{root}/instances");
        let (r, p) = (
            reference.get(&path, JSON).await,
            pacs.get(&path, JSON).await,
        );
        if report.status(&path, &r, &p) {
            report.arrays(
                "instances",
                &path,
                "00080018",
                &reference.json(&r),
                &pacs.json(&p),
            );
        }
        let path = format!("{root}/metadata");
        let (r, p) = (
            reference.get(&path, JSON).await,
            pacs.get(&path, JSON).await,
        );
        if report.status(&path, &r, &p) {
            report.arrays(
                "metadata",
                &path,
                "00080018",
                &reference.json(&r),
                &pacs.json(&p),
            );
        }
        for slice in 0..series.slices {
            let instance = format!("{root}/instances/{}", series.instance_uid(slice));
            let path = format!("{instance}/frames/1");
            let (r, p) = (
                reference.get(&path, FRAMES_ACCEPT).await,
                pacs.get(&path, FRAMES_ACCEPT).await,
            );
            if report.status(&path, &r, &p) {
                report.multipart(&path, &r, &p);
            }
            let accept = r#"multipart/related; type="application/dicom"; transfer-syntax=*"#;
            let (r, p) = (
                reference.get(&instance, accept).await,
                pacs.get(&instance, accept).await,
            );
            if report.status(&instance, &r, &p) {
                report.multipart(&instance, &r, &p);
            }
        }
    }

    let frames = |s: &SyntheticSeries, list: &str| {
        format!(
            "/studies/{}/series/{}/instances/{}/frames/{list}",
            s.study_uid(),
            s.series_uid(),
            s.instance_uid(0)
        )
    };
    for path in [
        frames(&multiframe, "1,3"),
        frames(&multiframe, "2"),
        frames(&jls, "2"),
    ] {
        let (r, p) = (
            reference.get(&path, FRAMES_ACCEPT).await,
            pacs.get(&path, FRAMES_ACCEPT).await,
        );
        if report.status(&path, &r, &p) {
            report.multipart(&path, &r, &p);
        }
    }

    assert!(
        report.lines.is_empty(),
        "{} difference(s) from the reference server:\n{}",
        report.lines.len(),
        report.lines.join("\n")
    );
}
