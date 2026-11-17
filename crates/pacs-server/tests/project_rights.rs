//! Rights per project, on the real routes: an account limited to a project is answered as
//! if the rest of the archive were not there, whatever its role allows.

use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use bytes::Bytes;
use clap::Parser;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::public_ids;
use pacs_core::synthetic::SyntheticSeries;
use pacs_server::auth::open_login;
use pacs_server::{Settings, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

const PROJECTS: &str = r#"[
    {"ae_title": "LIVER_2026", "profile": {"name": "liver-2026"}},
    {"ae_title": "LUNG_2026", "profile": {"name": "lung-2026"}}]"#;

fn settings_in(dir: &Path) -> Settings {
    let file = dir.join("projects.json");
    std::fs::write(&file, PROJECTS).unwrap();
    let args = [
        "pacs-server",
        "--index",
        &dir.join("index.sqlite").display().to_string(),
        "--storage",
        &format!("file:{}", dir.join("files").display()),
        "--projects",
        &file.display().to_string(),
        "--auth",
        "true",
        "--auth-cookie-secure",
        "false",
        "--audit-file",
        &dir.join("audit.log").display().to_string(),
    ]
    .map(String::from);
    Settings::try_parse_from(args).unwrap()
}

fn env(name: &str) -> Option<String> {
    match name {
        "PACS_AUTH_SECRET" => Some("a-signing-secret-of-at-least-32-bytes".into()),
        "PACS_ADMIN_PASSWORD" => Some("admin-password-1".into()),
        _ => None,
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
    bytes: Bytes,
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    json: Option<Value>,
    bytes: Option<Vec<u8>>,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "pacs.test");
    if !token.is_empty() {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = match (json, bytes) {
        (Some(json), _) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        (None, Some(bytes)) => Body::from(bytes),
        (None, None) => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        bytes,
    }
}

async fn get(app: &Router, path: &str, token: &str) -> Reply {
    send(app, Method::GET, path, token, None, None).await
}

async fn post(app: &Router, path: &str, token: &str, body: Value) -> Reply {
    send(app, Method::POST, path, token, Some(body), None).await
}

async fn put(app: &Router, path: &str, token: &str, body: Value) -> Reply {
    send(app, Method::PUT, path, token, Some(body), None).await
}

async fn delete(app: &Router, path: &str, token: &str) -> Reply {
    send(app, Method::DELETE, path, token, None, None).await
}

async fn token_of(app: &Router, login: &str, password: &str) -> String {
    let reply = post(
        app,
        "/auth/login",
        "",
        json!({"email": login, "password": password}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{login}: {}", reply.body);
    reply.body["access_token"].as_str().unwrap().to_owned()
}

/// A new user of `role` limited to `projects` (`Value::Null`: not limited): its id and token.
async fn user(
    app: &Router,
    admin: &str,
    login: &str,
    role: &str,
    projects: Value,
) -> (String, String) {
    let password = format!("{login}-password-1");
    let made = post(
        app,
        "/users",
        admin,
        json!({"login": login, "password": password, "role": role, "projects": projects}),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["projects"], projects);
    (
        made.body["id"].as_str().unwrap().to_owned(),
        token_of(app, login, &password).await,
    )
}

struct Ids {
    patient: String,
    study: String,
    series: String,
    instance: String,
    study_uid: String,
    series_uid: String,
    instance_uid: String,
}

fn ids(s: &SyntheticSeries) -> Ids {
    let (p, st, se, i) = (
        s.patient_id(),
        s.study_uid(),
        s.series_uid(),
        s.instance_uid(0),
    );
    Ids {
        patient: public_ids::patient(&p),
        study: public_ids::study(&p, &st),
        series: public_ids::series(&p, &st, &se),
        instance: public_ids::instance(&p, &st, &se, &i),
        study_uid: st,
        series_uid: se,
        instance_uid: i,
    }
}

struct Server {
    app: Router,
    archive: Archive,
    admin: String,
    /// In `liver-2026`.
    liver: SyntheticSeries,
    /// In `lung-2026`, of the same patient as `liver`.
    lung: SyntheticSeries,
    /// Stored as received, of another patient.
    received: SyntheticSeries,
    dir: tempfile::TempDir,
}

async fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path());
    let archive = open_archive(&settings).await.unwrap();
    let login = open_login(&settings, archive.index(), env).await.unwrap();
    let audit = pacs_server::audit::open_audit(&settings).unwrap();
    let app = pacs_server::app_with_audit(archive.clone(), &settings, login, audit).unwrap();
    let liver = SyntheticSeries::new(91, 2).for_patient(90);
    let lung = SyntheticSeries::new(92, 2).for_patient(90);
    let received = SyntheticSeries::new(93, 2);
    for (series, project) in [
        (&liver, Some("liver-2026")),
        (&lung, Some("lung-2026")),
        (&received, None),
    ] {
        let into = match project {
            Some(name) => archive.clone().for_project(name),
            None => archive.clone(),
        };
        for bytes in series.all_bytes() {
            into.ingest(Bytes::from(bytes)).await.unwrap();
        }
    }
    let admin = token_of(&app, "admin", "admin-password-1").await;
    Server {
        app,
        archive,
        admin,
        liver,
        lung,
        received,
        dir,
    }
}

/// The REST API's `GET` routes with an `{id}`, from its own description, for the resource
/// of `ids` at each route's level.
fn rest_routes(ids: &Ids) -> Vec<String> {
    let api = serde_json::to_value(pacs_server::api::description()).unwrap();
    let mut routes = Vec::new();
    for (path, item) in api["paths"].as_object().unwrap() {
        if !path.contains("{id}") || item.get("get").is_none() {
            continue;
        }
        let id = match path.split('/').nth(1).unwrap() {
            "patients" => &ids.patient,
            "studies" => &ids.study,
            "series" => &ids.series,
            "instances" => &ids.instance,
            // not resources of this archive: remote AEs and the moves from them, which the
            // `remote` permission governs
            "modalities" | "jobs" => continue,
            other => panic!("a level this test does not know: {other}"),
        };
        routes.push(path.replace("{id}", id));
    }
    assert!(routes.len() > 20, "{} routes", routes.len());
    routes
}

/// DICOMweb and the web interface's own routes that name a study.
fn other_routes(ids: &Ids) -> Vec<String> {
    let (st, se, i) = (&ids.study_uid, &ids.series_uid, &ids.instance_uid);
    vec![
        format!("/dicom-web/studies/{st}"),
        format!("/dicom-web/studies/{st}/metadata"),
        format!("/dicom-web/studies/{st}/series"),
        format!("/dicom-web/studies/{st}/series/{se}"),
        format!("/dicom-web/studies/{st}/series/{se}/metadata"),
        format!("/dicom-web/studies/{st}/series/{se}/instances"),
        format!("/dicom-web/studies/{st}/series/{se}/instances/{i}"),
        format!("/dicom-web/studies/{st}/series/{se}/instances/{i}/metadata"),
        format!("/dicom-web/studies/{st}/series/{se}/instances/{i}/frames/1"),
        format!("/dicom-web/studies?StudyInstanceUID={st}"),
        format!("/ui/api/annotations/{st}"),
    ]
}

/// Whether an answer shows nothing of the archive: not found, or an empty list.
fn nothing(reply: &Reply) -> bool {
    match reply.status {
        StatusCode::NOT_FOUND | StatusCode::NO_CONTENT => true,
        StatusCode::OK => reply.body == json!([]),
        _ => false,
    }
}

async fn sees_only_liver(s: &Server, token: &str, who: &str) {
    let (liver, lung, received) = (ids(&s.liver), ids(&s.lung), ids(&s.received));
    let app = &s.app;

    // what is hidden: every route that names it answers as for something that is not there
    for hidden in [&lung, &received] {
        for path in rest_routes(hidden) {
            // the patient the two projects share is seen, with the liver study only
            if std::ptr::eq(hidden, &lung) && path.starts_with("/patients/") {
                continue;
            }
            let reply = get(app, &path, token).await;
            assert_eq!(
                reply.status,
                StatusCode::NOT_FOUND,
                "{who}: {path}: {}",
                reply.body
            );
            assert!(reply.bytes.is_empty(), "{who}: {path}");
        }
        for path in other_routes(hidden) {
            let reply = get(app, &path, token).await;
            assert!(
                nothing(&reply),
                "{who}: {path}: {} {}",
                reply.status,
                reply.body
            );
        }
    }
    // what is seen: the same routes answer
    for path in rest_routes(&liver) {
        let reply = get(app, &path, token).await;
        assert_eq!(reply.status, StatusCode::OK, "{who}: {path}");
        assert!(!reply.bytes.is_empty(), "{who}: {path}");
    }
    for path in other_routes(&liver) {
        let reply = get(app, &path, token).await;
        assert!(
            reply.status.is_success() && !reply.bytes.is_empty(),
            "{who}: {path}: {}",
            reply.status
        );
    }

    // lists and searches
    assert_eq!(
        get(app, "/studies", token).await.body,
        json!([liver.study]),
        "{who}"
    );
    assert_eq!(
        get(app, "/patients", token).await.body,
        json!([liver.patient]),
        "{who}"
    );
    let expanded = get(app, "/studies?expand", token).await.body;
    assert_eq!(expanded.as_array().unwrap().len(), 1, "{who}");
    // the shared patient is its liver study
    let patient = get(app, &format!("/patients/{}", liver.patient), token)
        .await
        .body;
    assert_eq!(patient["Studies"], json!([liver.study]), "{who}");
    for (path, count) in [("studies", 1), ("series", 1), ("instances", 2)] {
        let listed = get(app, &format!("/patients/{}/{path}", liver.patient), token)
            .await
            .body;
        assert_eq!(listed.as_array().unwrap().len(), count, "{who}: {path}");
    }
    let stats = get(
        app,
        &format!("/patients/{}/statistics", liver.patient),
        token,
    )
    .await
    .body;
    assert_eq!(
        (&stats["CountStudies"], &stats["CountInstances"]),
        (&json!(1), &json!(2)),
        "{who}"
    );
    let qido = get(app, "/dicom-web/studies", token).await.body;
    assert_eq!(qido.as_array().unwrap().len(), 1, "{who}");
    assert_eq!(qido[0]["0020000D"]["Value"][0], liver.study_uid, "{who}");

    for (level, key, of) in [
        ("Patient", "PatientID", s.received.patient_id()),
        ("Study", "StudyInstanceUID", lung.study_uid.clone()),
        ("Study", "PatientID", s.received.patient_id()),
        ("Series", "SeriesInstanceUID", lung.series_uid.clone()),
        ("Instance", "SOPInstanceUID", received.instance_uid.clone()),
    ] {
        for expand in [false, true] {
            let search = json!({"Level": level, "Query": {key: of}, "Expand": expand});
            let found = post(app, "/tools/find", token, search).await;
            assert_eq!(
                (found.status, &found.body),
                (StatusCode::OK, &json!([])),
                "{who}: {level} {key}"
            );
        }
    }
    let all = post(app, "/tools/find", token, json!({"Level": "Study"}))
        .await
        .body;
    assert_eq!(all, json!([liver.study]), "{who}");

    // an archive of something hidden, alone or beside something seen
    for resources in [
        json!([lung.instance]),
        json!([liver.instance, received.series]),
    ] {
        let asked = post(
            app,
            "/tools/create-archive",
            token,
            json!({"Resources": resources}),
        )
        .await;
        assert_eq!(asked.status, StatusCode::NOT_FOUND, "{who}");
    }
    let link = format!("/tools/create-archive?resources={}", lung.study);
    assert_eq!(
        get(app, &link, token).await.status,
        StatusCode::NOT_FOUND,
        "{who}"
    );

    // the web interface's lists
    let listed = get(app, "/ui/api/studies", token).await.body;
    assert_eq!(listed["Total"], 1, "{who}: {listed}");
    assert_eq!(listed["Studies"][0]["Instances"], 2, "{who}: {listed}");
    let patients = get(app, "/ui/api/patients", token).await.body;
    assert_eq!(patients["Total"], 1, "{who}: {patients}");
    assert_eq!(patients["Patients"][0]["Studies"], 1, "{who}: {patients}");
    let facets = get(app, "/ui/api/studies/facets", token).await.body;
    let studies: i64 = facets["Dates"]
        .as_object()
        .unwrap()
        .values()
        .map(|n| n.as_i64().unwrap())
        .sum();
    assert_eq!(studies, 1, "{who}: {facets}");
}

#[tokio::test]
async fn a_limited_user_and_a_limited_tool_see_their_project_only() {
    let s = server().await;
    let app = &s.app;

    // the administrator, not limited, sees the three studies
    assert_eq!(
        get(app, "/studies", &s.admin)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        get(app, "/projects", &s.admin).await.body,
        json!(["liver-2026", "lung-2026"])
    );

    // a reader, and someone whose role allows everything: the limit is the same for both
    let (_, reader) = user(app, &s.admin, "reader", "reader", json!(["liver-2026"])).await;
    let (_, second_admin) = user(app, &s.admin, "second", "admin", json!(["liver-2026"])).await;
    sees_only_liver(&s, &reader, "reader").await;
    sees_only_liver(&s, &second_admin, "limited admin").await;
    let me = get(app, "/auth/me", &reader).await.body;
    assert_eq!(me["projects"], json!(["liver-2026"]));
    assert_eq!(
        get(app, "/auth/me", &s.admin).await.body["projects"],
        Value::Null
    );
    // the projects' names are for who manages users
    assert_eq!(
        get(app, "/projects", &reader).await.status,
        StatusCode::FORBIDDEN
    );

    // a tool
    let made = post(
        app,
        "/clients",
        &s.admin,
        json!({"name": "Pipeline", "role": "reader", "projects": ["liver-2026"]}),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["projects"], json!(["liver-2026"]));
    let (id, secret) = (
        made.body["id"].as_str().unwrap(),
        made.body["secret"].as_str().unwrap(),
    );
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let request = Request::post("/auth/token")
        .header("authorization", format!("Basic {basic}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("grant_type=client_credentials"))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let token: Value = serde_json::from_slice(&bytes).unwrap();
    let tool = token["access_token"].as_str().unwrap();
    sees_only_liver(&s, tool, "tool").await;

    // a download link is bounded like the user it was made for
    let lung = ids(&s.lung);
    for (study, status) in [
        (&lung.study, StatusCode::NOT_FOUND),
        (&ids(&s.liver).study, StatusCode::OK),
    ] {
        let path = format!("/studies/{study}/archive");
        let link = post(app, "/auth/download-token", &reader, json!({"path": path})).await;
        assert_eq!(link.status, StatusCode::OK, "{}", link.body);
        let url = format!("{path}?token={}", link.body["token"].as_str().unwrap());
        assert_eq!(get(app, &url, "").await.status, status);
    }

    // nothing was changed by all that reading
    assert_eq!(
        get(app, "/studies", &s.admin)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn a_limited_account_writes_and_deletes_within_its_project() {
    let s = server().await;
    let app = &s.app;
    let (liver, lung, received) = (ids(&s.liver), ids(&s.lung), ids(&s.received));
    // a role that allows everything, so that only the limit refuses
    let (_, token) = user(app, &s.admin, "second", "admin", json!(["liver-2026"])).await;

    // it stores nothing over HTTP: what it sent would be in no project
    let file = SyntheticSeries::new(94, 1).slice_bytes(0);
    let upload = send(
        app,
        Method::POST,
        "/instances",
        &token,
        None,
        Some(file.clone()),
    )
    .await;
    assert_eq!(upload.status, StatusCode::FORBIDDEN, "{}", upload.body);
    assert!(
        upload.body["Details"]
            .as_str()
            .unwrap()
            .contains("would be in no project")
    );
    let boundary = "b";
    let mut stow = format!("--{boundary}\r\nContent-Type: application/dicom\r\n\r\n").into_bytes();
    stow.extend(&file);
    stow.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
    let request = Request::post("/dicom-web/studies")
        .header("authorization", format!("Bearer {token}"))
        .header(
            "content-type",
            format!(r#"multipart/related; type="application/dicom"; boundary={boundary}"#),
        )
        .body(Body::from(stow))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(app, "/studies", &s.admin)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        3
    );

    // measurements: on its own study, not on the others, which keep theirs
    let kept = json!([{"keep": true}]);
    for study in [&lung.study_uid, &received.study_uid] {
        let path = format!("/ui/api/annotations/{study}");
        assert!(
            put(app, &path, &s.admin, kept.clone())
                .await
                .status
                .is_success()
        );
        let refused = put(app, &path, &token, json!([{"overwritten": true}])).await;
        assert_eq!(refused.status, StatusCode::NOT_FOUND, "{}", refused.body);
        assert_eq!(get(app, &path, &s.admin).await.body, kept);
    }
    let own = format!("/ui/api/annotations/{}", liver.study_uid);
    assert!(
        put(app, &own, &token, kept.clone())
            .await
            .status
            .is_success()
    );

    // deletions: what it does not see is not there to delete
    for path in [
        format!("/instances/{}", lung.instance),
        format!("/series/{}", lung.series),
        format!("/studies/{}", lung.study),
        format!("/studies/{}", received.study),
        format!("/patients/{}", received.patient),
    ] {
        assert_eq!(
            delete(app, &path, &token).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert_eq!(
        get(app, "/studies", &s.admin)
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for series in [&s.lung, &s.received] {
        let entries = s
            .archive
            .index()
            .instances(&series.study_uid(), &series.series_uid(), None)
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        for entry in &entries {
            s.archive.file(entry).await.unwrap();
        }
    }

    // the patient it shares with the other project: its own study goes, the other stays
    let deleted = delete(app, &format!("/patients/{}", liver.patient), &token).await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    assert_eq!(deleted.body["RemainingAncestor"], Value::Null);
    assert_eq!(get(app, "/studies", &token).await.body, json!([]));
    let left = get(app, "/studies", &s.admin).await.body;
    assert_eq!(left.as_array().unwrap().len(), 2);
    assert!(left.as_array().unwrap().contains(&json!(lung.study)));

    // a remote move: into its own project only
    for (body, why) in [
        (json!({"StudyInstanceUID": "1.2.3"}), "no project"),
        (
            json!({"StudyInstanceUID": "1.2.3", "Project": "lung-2026"}),
            "another project",
        ),
    ] {
        let moved = post(app, "/modalities/hospital/move", &token, body).await;
        assert_eq!(moved.status, StatusCode::FORBIDDEN, "{why}: {}", moved.body);
    }
    // its own project passes the limit: refused further on, as there is no such remote AE
    let moved = post(
        app,
        "/modalities/hospital/move",
        &token,
        json!({"StudyInstanceUID": "1.2.3", "Project": "liver-2026"}),
    )
    .await;
    assert_eq!(moved.status, StatusCode::NOT_FOUND, "{}", moved.body);
}

#[tokio::test]
async fn projects_are_set_changed_and_lifted_by_who_manages_users() {
    let s = server().await;
    let app = &s.app;
    let count = async |token: &str| {
        get(app, "/studies", token)
            .await
            .body
            .as_array()
            .unwrap()
            .len()
    };

    // made without projects: everything
    let (id, token) = user(app, &s.admin, "reader", "reader", Value::Null).await;
    assert_eq!(count(&token).await, 3);
    let path = format!("/users/{id}");

    // limited: from the next request, with the token it already has
    let changed = put(
        app,
        &path,
        &s.admin,
        json!({"projects": ["lung-2026", "liver-2026", "lung-2026"]}),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    assert_eq!(changed.body["projects"], json!(["liver-2026", "lung-2026"]));
    assert_eq!(count(&token).await, 2);

    // another change leaves the projects as they are
    let changed = put(app, &path, &s.admin, json!({"role": "editor"})).await;
    assert_eq!(changed.body["projects"], json!(["liver-2026", "lung-2026"]));
    assert_eq!(count(&token).await, 2);

    // no project at all: nothing
    let changed = put(app, &path, &s.admin, json!({"projects": []})).await;
    assert_eq!(changed.body["projects"], json!([]));
    assert_eq!(count(&token).await, 0);
    let listed = get(app, "/users", &s.admin).await.body;
    let shown = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id.as_str())
        .unwrap();
    assert_eq!(shown["projects"], json!([]));
    let admin = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["login"] == "admin")
        .unwrap();
    assert_eq!(admin["projects"], Value::Null);

    // a project this server does not have is refused, and nothing changes
    for projects in [json!(["liver-2027"]), json!(["liver-2026", ""])] {
        let refused = put(app, &path, &s.admin, json!({"projects": projects})).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            refused.body
        );
        assert!(
            refused.body["detail"]
                .as_str()
                .unwrap()
                .contains("is not a project of this server")
        );
    }
    let refused = post(
        app,
        "/users",
        &s.admin,
        json!({"login": "other", "password": "other-password-1", "role": "reader", "projects": ["nowhere"]}),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count(&token).await, 0);

    // lifted: everything again
    let changed = put(app, &path, &s.admin, json!({"projects": null})).await;
    assert_eq!(changed.body["projects"], Value::Null);
    assert_eq!(count(&token).await, 3);

    // a limited user cannot lift their own limit: accounts are for who manages users
    put(app, &path, &s.admin, json!({"projects": ["liver-2026"]})).await;
    let own = put(app, &path, &token, json!({"projects": null})).await;
    assert_eq!(own.status, StatusCode::FORBIDDEN);
    assert_eq!(count(&token).await, 1);

    // a tool, the same way
    let made = post(
        app,
        "/clients",
        &s.admin,
        json!({"name": "Pipeline", "role": "reader"}),
    )
    .await;
    assert_eq!(made.body["projects"], Value::Null);
    let tool = format!("/clients/{}", made.body["id"].as_str().unwrap());
    let changed = put(app, &tool, &s.admin, json!({"projects": ["lung-2026"]})).await;
    assert_eq!(changed.body["projects"], json!(["lung-2026"]));
    let renamed = put(app, &tool, &s.admin, json!({"name": "Nightly"})).await;
    assert_eq!(renamed.body["projects"], json!(["lung-2026"]));
    let listed = get(app, "/clients", &s.admin).await.body;
    assert_eq!(listed[0]["projects"], json!(["lung-2026"]));
    let refused = put(app, &tool, &s.admin, json!({"projects": ["nowhere"]})).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);

    // removed accounts leave no projects behind for an account of the same id
    assert_eq!(
        delete(app, &path, &s.admin).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        delete(app, &tool, &s.admin).await.status,
        StatusCode::NO_CONTENT
    );
    let index = s.archive.index();
    assert_eq!(
        index.account_projects(&format!("user:{id}")).await.unwrap(),
        None
    );
    let client = tool.trim_start_matches("/clients/");
    assert_eq!(
        index
            .account_projects(&format!("client:{client}"))
            .await
            .unwrap(),
        None
    );
}

/// The message of an audit line is valid against DICOM's schema, when `PACS_TEST_XMLLINT`
/// names `xmllint` (as in CI).
fn valid(line: &str) {
    let Ok(xmllint) = std::env::var("PACS_TEST_XMLLINT") else {
        return;
    };
    let xml = &line[line.find("<AuditMessage>").expect("an audit message")..];
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("message.xml");
    std::fs::write(&file, xml).unwrap();
    let schema =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../pacs-audit/tests/dicom-audit-message.rng");
    let output = std::process::Command::new(&xmllint)
        .arg("--noout")
        .arg("--relaxng")
        .arg(schema)
        .arg(&file)
        .output()
        .unwrap_or_else(|e| panic!("PACS_TEST_XMLLINT names {xmllint:?}, which did not run: {e}"));
    assert!(
        output.status.success(),
        "{}\n{xml}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The lines the audit trail gained since `from`, and the new count.
fn audited(s: &Server, from: &mut usize) -> Vec<String> {
    let trail = std::fs::read_to_string(s.dir.path().join("audit.log")).unwrap();
    let lines: Vec<String> = trail.lines().skip(*from).map(str::to_owned).collect();
    *from += lines.len();
    lines
}

#[tokio::test]
async fn a_request_outside_the_account_s_projects_is_in_the_audit_trail() {
    let s = server().await;
    let app = &s.app;
    let (liver, lung, received) = (ids(&s.liver), ids(&s.lung), ids(&s.received));
    let (_, reader) = user(app, &s.admin, "reader", "reader", json!(["liver-2026"])).await;
    let (_, second) = user(app, &s.admin, "second", "admin", json!(["liver-2026"])).await;
    let mut seen = 0;
    audited(&s, &mut seen);

    const ACCESSED: &str = "csd-code=\"110103\"";
    const REFUSED: &str = "EventOutcomeIndicator=\"4\"";
    const WHY: &str =
        "<EventOutcomeDescription>outside the account&apos;s projects</EventOutcomeDescription>";
    let one = |lines: Vec<String>, what: &str| -> String {
        assert_eq!(lines.len(), 1, "{what}: {lines:#?}");
        valid(&lines[0]);
        lines.into_iter().next().unwrap()
    };

    // a study of another project, asked for by its identifier: refused, and written
    let path = format!("/studies/{}", lung.study);
    assert_eq!(get(app, &path, &reader).await.status, StatusCode::NOT_FOUND);
    let line = one(audited(&s, &mut seen), "a hidden study");
    for part in [
        ACCESSED,
        REFUSED,
        "EventActionCode=\"R\"",
        "UserName=\"reader\"",
        &lung.study_uid,
    ] {
        assert!(line.contains(part), "{part}: {line}");
    }
    assert!(
        line.contains("outside the account") && line.contains("projects</EventOutcomeDescription>"),
        "{WHY}: {line}"
    );
    assert!(!line.contains(&liver.study_uid), "{line}");

    // asked again at once: no second line, as for a study read again
    assert_eq!(get(app, &path, &reader).await.status, StatusCode::NOT_FOUND);
    assert!(audited(&s, &mut seen).is_empty());

    // what is in no study of the archive: a 404 like any other, and no line
    for path in [
        "/studies/nothing",
        "/dicom-web/studies/1.2.3/metadata",
        "/series/nothing/instances",
    ] {
        assert_eq!(
            get(app, path, &reader).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    assert_eq!(
        delete(app, "/studies/nothing", &second).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(audited(&s, &mut seen).is_empty());

    // over DICOMweb, a study in no project
    let path = format!(
        "/dicom-web/studies/{}/series/{}/metadata",
        received.study_uid, received.series_uid
    );
    assert_eq!(get(app, &path, &reader).await.status, StatusCode::NOT_FOUND);
    let line = one(audited(&s, &mut seen), "DICOMweb");
    assert!(
        line.contains(REFUSED) && line.contains(&received.study_uid),
        "{line}"
    );

    // a deletion that found nothing to delete, because the account does not see it
    let path = format!("/series/{}", lung.series);
    assert_eq!(
        delete(app, &path, &second).await.status,
        StatusCode::NOT_FOUND
    );
    let line = one(audited(&s, &mut seen), "a deletion");
    for part in [
        ACCESSED,
        REFUSED,
        "EventActionCode=\"D\"",
        "UserName=\"second\"",
        &lung.study_uid,
    ] {
        assert!(line.contains(part), "{part}: {line}");
    }

    // an archive of something seen and something hidden: the hidden study is named
    let order = json!({"Resources": [liver.instance, received.series]});
    assert_eq!(
        post(app, "/tools/create-archive", &reader, order)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let line = one(audited(&s, &mut seen), "an archive");
    assert!(
        line.contains(REFUSED) && line.contains(&received.study_uid),
        "{line}"
    );
    assert!(!line.contains(&liver.study_uid), "{line}");

    // its own study: a success, as before
    let path = format!("/studies/{}", liver.study);
    assert_eq!(get(app, &path, &reader).await.status, StatusCode::OK);
    let line = one(audited(&s, &mut seen), "its own study");
    assert!(
        line.contains("EventOutcomeIndicator=\"0\"") && line.contains(&liver.study_uid),
        "{line}"
    );

    // an account that is not limited: a 404 is a study that is not there, and no line
    assert_eq!(
        get(app, "/studies/nothing", &s.admin).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(audited(&s, &mut seen).is_empty());
}
