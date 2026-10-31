//! Projects as pacs-server reads them: the PACS_PROJECTS file, each project's secret from the
//! environment, the quarantine store, and an instance sent to a project's AE title arriving
//! de-identified in what the REST API serves.

use std::path::Path;

use clap::Parser;
use dicom_ul::ClientAssociationOptions;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dimse::command::{Command, PRIORITY_MEDIUM, field, tag};
use pacs_dimse::message::{self, Incoming};
use pacs_server::projects::{open_projects, project_configs, secret_variable};
use pacs_server::{Settings, open_archive, serve_dimse};

const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
const SECRET: &str = "0123456789abcdef0123456789abcdef";

fn settings(dir: &Path, projects: &str, extra: &[&str]) -> Settings {
    let file = dir.join("projects.json");
    std::fs::write(&file, projects).unwrap();
    let mut args = vec![
        "pacs-server".to_owned(),
        "--index".into(),
        dir.join("index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.join("files").display()),
        "--projects".into(),
        file.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    Settings::try_parse_from(args).unwrap()
}

const LIVER: &str = r#"[{"ae_title": "LIVER_2026", "profile": {"name": "liver-2026",
    "options": ["retain-longitudinal-modified-dates", "retain-patient-characteristics"]}}]"#;

fn env_with(variable: &'static str, value: &'static str) -> impl Fn(&str) -> Option<String> {
    move |name| (name == variable).then(|| value.to_owned())
}

#[test]
fn the_secret_variable_is_named_after_the_ae_title() {
    let title = pacs_dimse::AeTitle::new("LIVER-2026 a").unwrap();
    assert_eq!(secret_variable(&title), "PACS_PROJECT_SECRET_LIVER_2026_A");
}

/// The file is read and each profile checked; a bad AE title, a profile pacs-rs cannot
/// apply, a project listed twice, or one on pacs-rs's own AE title is an error.
#[test]
fn the_projects_file_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let projects = project_configs(&settings(dir.path(), LIVER, &[])).unwrap();
    assert_eq!(projects[0].ae_title.as_str(), "LIVER_2026");
    assert_eq!(projects[0].profile.name, "liver-2026");
    for bad in [
        r#"[{"ae_title": "A_TITLE_LONGER_THAN_16", "profile": {"name": "a"}}]"#,
        r#"[{"ae_title": "A", "profile": {"name": "a", "options": ["clean-graphics"]}}]"#,
        r#"[{"ae_title": "A", "profile": {"name": "a"}}, {"ae_title": "A", "profile": {"name": "b"}}]"#,
        r#"[{"ae_title": "PACSRS", "profile": {"name": "a"}}]"#,
        r#"[{"ae_title": "A", "profile": {"name": "a"}, "colour": "blue"}]"#,
        r#"{"not": "a list"}"#,
    ] {
        assert!(
            project_configs(&settings(dir.path(), bad, &[])).is_err(),
            "{bad}"
        );
    }
}

/// A project's secret comes from its variable, or the file named by its _FILE variable; a
/// missing or short secret is an error that names the variable.
#[tokio::test]
async fn each_project_needs_its_secret() {
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), LIVER, &[]);
    let archive = open_archive(&s).await.unwrap();
    let missing = open_projects(&s, &archive, |_| None)
        .unwrap_err()
        .to_string();
    assert!(
        missing.contains("PACS_PROJECT_SECRET_LIVER_2026"),
        "{missing}"
    );
    assert!(
        open_projects(
            &s,
            &archive,
            env_with("PACS_PROJECT_SECRET_LIVER_2026", "short")
        )
        .is_err()
    );
    let projects = open_projects(
        &s,
        &archive,
        env_with("PACS_PROJECT_SECRET_LIVER_2026", SECRET),
    )
    .unwrap();
    assert!(projects[0].quarantine.is_none());
    let file = dir.path().join("secret");
    std::fs::write(&file, format!("{SECRET}\n")).unwrap();
    let path: &'static str = Box::leak(file.display().to_string().into_boxed_str());
    assert!(
        open_projects(
            &s,
            &archive,
            env_with("PACS_PROJECT_SECRET_LIVER_2026_FILE", path)
        )
        .is_ok()
    );
}

/// PACS_QUARANTINE gives every project a quarantine store.
#[tokio::test]
async fn the_quarantine_setting_opens_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let quarantine = format!("file:{}", dir.path().join("quarantine").display());
    let s = settings(dir.path(), LIVER, &["--quarantine", &quarantine]);
    let archive = open_archive(&s).await.unwrap();
    let projects = open_projects(
        &s,
        &archive,
        env_with("PACS_PROJECT_SECRET_LIVER_2026", SECRET),
    )
    .unwrap();
    assert!(projects[0].quarantine.is_some());
    assert!(dir.path().join("quarantine").is_dir());
}

/// The whole path: a C-STORE to the project's AE title, and the study the REST API serves is
/// the de-identified one; the identified one is nowhere.
#[tokio::test]
async fn an_instance_sent_to_a_project_is_served_de_identified() {
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), LIVER, &[]);
    let archive = open_archive(&s).await.unwrap();
    let projects = open_projects(
        &s,
        &archive,
        env_with("PACS_PROJECT_SECRET_LIVER_2026", SECRET),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = pacs_server::dimse_config(&s).unwrap();
    tokio::spawn(serve_dimse(
        listener,
        archive.clone(),
        config,
        projects,
        std::future::pending(),
    ));

    let series = SyntheticSeries::new(111, 1);
    let file = series.slice_bytes(0);
    let meta = u32::from_le_bytes(file[140..144].try_into().unwrap()) as usize;
    let mut association = ClientAssociationOptions::new()
        .calling_ae_title("HOSPITAL")
        .called_ae_title("LIVER_2026")
        .with_presentation_context(CT, vec!["1.2.840.10008.1.2.1"])
        .establish_async(addr)
        .await
        .unwrap();
    let context = association.presentation_contexts()[0].id;
    let rq = Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, CT)
        .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
        .with_us(tag::MESSAGE_ID, 1)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0)
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, &series.instance_uid(0));
    message::send(&mut association, context, &rq, Some(&file[144 + meta..]))
        .await
        .unwrap();
    let Incoming::Message(rsp) = message::receive(&mut association, 0).await.unwrap() else {
        panic!("no C-STORE-RSP");
    };
    assert_eq!(rsp.command.status(), Ok(pacs_dimse::Status::SUCCESS));
    association.release().await.unwrap();

    let index = archive.index();
    assert!(
        index
            .rest_instance_by_uid(&series.instance_uid(0))
            .await
            .unwrap()
            .is_none()
    );
    let keys = pacs_deid::Keys::new(SECRET.as_bytes().to_vec()).unwrap();
    let stored = index
        .rest_instance_by_uid(&keys.uid(&series.instance_uid(0)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.study_instance_uid, keys.uid(&series.study_uid()));
}
