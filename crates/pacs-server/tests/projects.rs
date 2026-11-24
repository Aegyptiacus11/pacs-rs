//! Projects as pacs-server reads them: the PACS_PROJECTS file, each project's secret from the
//! environment, the quarantine store, and an instance sent to a project's AE title arriving
//! de-identified in what the REST API serves.

use std::path::Path;

use clap::Parser;
use dicom_core::value::PrimitiveValue;
use dicom_core::{DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};
use dicom_ul::ClientAssociationOptions;
use pacs_core::real::RealSeries;
use pacs_dimse::command::{Command, PRIORITY_MEDIUM, field, tag};
use pacs_dimse::message::{self, Incoming};
use pacs_server::projects::{open_projects, project_configs, secret_variable};
use pacs_server::{Settings, open_archive};

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

/// A C-STORE of `object` (Explicit VR Little Endian) to `called`; the status answered.
async fn c_store(
    addr: std::net::SocketAddr,
    called: &str,
    object: InMemDicomObject,
    sop_instance_uid: &str,
) -> pacs_dimse::Status {
    let mut file = Vec::new();
    object
        .with_meta(
            FileMetaTableBuilder::new()
                .transfer_syntax("1.2.840.10008.1.2.1")
                .media_storage_sop_class_uid(CT)
                .media_storage_sop_instance_uid(sop_instance_uid),
        )
        .unwrap()
        .write_all(&mut file)
        .unwrap();
    let meta = u32::from_le_bytes(file[140..144].try_into().unwrap()) as usize;
    let mut association = ClientAssociationOptions::new()
        .calling_ae_title("WORKSTATION")
        .called_ae_title(called)
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
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, sop_instance_uid);
    message::send(&mut association, context, &rq, Some(&file[144 + meta..]))
        .await
        .unwrap();
    let Incoming::Message(rsp) = message::receive(&mut association, 0).await.unwrap() else {
        panic!("no C-STORE-RSP");
    };
    association.release().await.unwrap();
    rsp.command.status().unwrap()
}

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
    let trail = dir.path().join("audit.log");
    let s = settings(
        dir.path(),
        LIVER,
        &["--audit-file", trail.to_str().unwrap()],
    );
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
    tokio::spawn(pacs_server::serve_dimse_audited(
        listener,
        archive.clone(),
        config,
        projects,
        pacs_server::audit::open_audit(&s).unwrap(),
        std::future::pending(),
    ));

    let series = RealSeries::new(111, 1);
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
    // the study is recorded as the project's
    let project = index
        .study_project(&stored.study_instance_uid)
        .await
        .unwrap();
    assert_eq!(project, Some(Some("liver-2026".into())));

    // the way back is recorded, for a result made on the copy
    let replaced = [
        keys.uid(&series.study_uid()),
        keys.uid(&series.series_uid()),
        keys.uid(&series.instance_uid(0)),
    ];
    let originals = index.original_uids("liver-2026", &replaced).await.unwrap();
    assert_eq!(originals[&replaced[0]], series.study_uid());
    assert_eq!(originals[&replaced[1]], series.series_uid());
    assert_eq!(originals[&replaced[2]], series.instance_uid(0));
    let original = index
        .original_study("liver-2026", &replaced[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.patient["PatientID"], series.patient_id().as_str());

    // The audit trail names what the archive holds, the pseudonyms: who the patient was is
    // not written there either.
    let mut audited = String::new();
    for _ in 0..100 {
        audited = std::fs::read_to_string(&trail).unwrap_or_default();
        if !audited.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(audited.lines().count(), 1, "{audited}");
    assert!(
        audited.contains("<EventID csd-code=\"110104\""),
        "{audited}"
    );
    assert!(
        audited.contains("AlternativeUserID=\"AETITLES=LIVER_2026\""),
        "{audited}"
    );
    assert!(
        audited.contains(&format!(
            "ParticipantObjectID=\"{}\"",
            keys.uid(&series.study_uid())
        )),
        "{audited}"
    );
    for identified in [
        series.study_uid(),
        series.patient_id(),
        "CompressedSamples".to_owned(),
    ] {
        assert!(
            !audited.contains(&identified),
            "the trail holds {identified:?}:\n{audited}"
        );
    }

    // A result made on the copy (here another series of a real CT, in the replaced study)
    // sent to the project's AE title is stored as it comes, not de-identified again.
    let made = RealSeries::new(112, 1);
    let mut result = made.slice_object(0);
    let copy = keys.pseudonym(&format!("{}\\", series.patient_id()), "");
    for (tag, vr, value) in [
        (
            tags::STUDY_INSTANCE_UID,
            VR::UI,
            stored.study_instance_uid.as_str(),
        ),
        (tags::PATIENT_ID, VR::LO, copy.as_str()),
        (tags::PATIENT_NAME, VR::PN, copy.as_str()),
    ] {
        result.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
    }
    assert_eq!(
        c_store(addr, "LIVER_2026", result, &made.instance_uid(0)).await,
        pacs_dimse::Status::SUCCESS
    );
    let kept = index
        .rest_instance_by_uid(&made.instance_uid(0))
        .await
        .unwrap()
        .expect("the result keeps its own UIDs");
    assert_eq!(kept.study_instance_uid, stored.study_instance_uid);
    assert!(
        index
            .rest_instance_by_uid(&keys.uid(&made.instance_uid(0)))
            .await
            .unwrap()
            .is_none()
    );
}

/// A result is for the project it was made in: sent to another project's AE title, it is
/// refused, not de-identified into that project.
#[tokio::test]
async fn a_result_for_another_project_s_study_is_refused() {
    const TWO: &str = r#"[
        {"ae_title": "LIVER_2026", "profile": {"name": "liver-2026"}},
        {"ae_title": "LUNG_2026", "profile": {"name": "lung-2026"}}]"#;
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), TWO, &[]);
    let archive = open_archive(&s).await.unwrap();
    let projects = open_projects(&s, &archive, |name| {
        name.starts_with("PACS_PROJECT_SECRET_")
            .then(|| SECRET.to_owned())
    })
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = pacs_server::dimse_config(&s).unwrap();
    tokio::spawn(pacs_server::serve_dimse_audited(
        listener,
        archive.clone(),
        config,
        projects,
        pacs_server::audit::open_audit(&s).unwrap(),
        std::future::pending(),
    ));
    let series = RealSeries::new(121, 1);
    assert_eq!(
        c_store(
            addr,
            "LIVER_2026",
            series.slice_object(0),
            &series.instance_uid(0)
        )
        .await,
        pacs_dimse::Status::SUCCESS
    );
    let keys = pacs_deid::Keys::new(SECRET.as_bytes().to_vec()).unwrap();
    let made = RealSeries::new(122, 1);
    let mut result = made.slice_object(0);
    result.put(DataElement::new(
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        PrimitiveValue::from(keys.uid(&series.study_uid()).as_str()),
    ));
    let status = c_store(addr, "LUNG_2026", result, &made.instance_uid(0)).await;
    assert_ne!(status, pacs_dimse::Status::SUCCESS);
    let index = archive.index();
    for uid in [made.instance_uid(0), keys.uid(&made.instance_uid(0))] {
        assert!(index.rest_instance_by_uid(&uid).await.unwrap().is_none());
    }
}

/// A profile that keeps the UIDs gives a copy its original's study UID: the study's other
/// instances, still identified, are not results of it, and are de-identified like the first.
#[tokio::test]
async fn with_the_uids_kept_every_instance_is_still_de_identified() {
    const KEPT: &str = r#"[{"ae_title": "LIVER_2026", "profile": {"name": "liver-2026",
        "options": ["retain-uids"]}}]"#;
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), KEPT, &[]);
    let archive = open_archive(&s).await.unwrap();
    let projects = open_projects(
        &s,
        &archive,
        env_with("PACS_PROJECT_SECRET_LIVER_2026", SECRET),
    )
    .unwrap();
    let series = RealSeries::new(131, 3);
    for bytes in series.all_bytes() {
        projects[0].receive(bytes.into()).await.unwrap();
    }
    let index = archive.index();
    for slice in 0..3 {
        let stored = index
            .rest_instance_by_uid(&series.instance_uid(slice))
            .await
            .unwrap()
            .unwrap();
        let file = archive.file_at(&stored.object_key).await.unwrap();
        let instance = pacs_core::Instance::from_bytes(&file).unwrap();
        assert_ne!(
            instance.string(tags::PATIENT_ID).as_deref(),
            Some(series.patient_id().as_str()),
            "slice {slice} is stored identified"
        );
    }
    // sent again as received, past the project's AE title, it is refused as before: neither
    // stored identified in the project nor noted for write-back
    let more = RealSeries::new(132, 1).in_study(131, 2);
    let refused = archive.ingest(more.slice_bytes(0).into()).await;
    assert!(
        matches!(
            refused,
            Err(pacs_archive::IngestError::Index(_) | pacs_archive::IngestError::Conflict(_))
        ),
        "{refused:?}"
    );
    assert!(
        index
            .rest_instance_by_uid(&more.instance_uid(0))
            .await
            .unwrap()
            .is_none()
    );
    assert!(index.claim_write_backs(i64::MAX).await.unwrap().is_empty());
}

/// An override that keeps the SOP Instance UIDs: the copy's images are still the copy's, and
/// not results of it.
#[tokio::test]
async fn with_the_instance_uids_kept_the_copy_is_not_its_own_result() {
    const KEPT: &str = r#"[{"ae_title": "LIVER_2026", "profile": {"name": "liver-2026",
        "overrides": [{"tag": "(0008,0018)", "action": "K", "reason": "test"}]}}]"#;
    let dir = tempfile::tempdir().unwrap();
    let s = settings(dir.path(), KEPT, &[]);
    let archive = open_archive(&s).await.unwrap();
    let projects = open_projects(
        &s,
        &archive,
        env_with("PACS_PROJECT_SECRET_LIVER_2026", SECRET),
    )
    .unwrap();
    let series = RealSeries::new(141, 2);
    for bytes in series.all_bytes() {
        projects[0].receive(bytes.into()).await.unwrap();
    }
    let keys = pacs_deid::Keys::new(SECRET.as_bytes().to_vec()).unwrap();
    let copy = keys.uid(&series.study_uid());
    let index = archive.index();
    assert_eq!(index.study(&copy).await.unwrap().unwrap().instance_count, 2);
    assert!(
        index
            .original_study("liver-2026", &copy)
            .await
            .unwrap()
            .is_some()
    );
    let results = index.results_of("liver-2026", &copy).await.unwrap();
    assert!(results.is_empty(), "{} of the copy", results.len());
}
