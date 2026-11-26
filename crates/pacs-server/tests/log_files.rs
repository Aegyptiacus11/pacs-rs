//! The log in daily files (PACS_LOG_DIR, PACS_LOG_DAYS).

use std::path::Path;

#[test]
fn the_log_goes_to_a_daily_file() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    let settings =
        pacs_server::load_settings(["pacs-server", "--log-dir", logs.to_str().unwrap()]).unwrap();
    let files = pacs_server::log_files(&settings).unwrap();
    assert!(files.is_some());
    // made, with today's file in it
    let names: Vec<String> = std::fs::read_dir(&logs)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        names[0].starts_with("pacs-server.") && names[0].ends_with(".log"),
        "{names:?}"
    );
}

#[test]
fn without_a_log_dir_the_log_goes_to_standard_output() {
    let settings = pacs_server::load_settings(["pacs-server"]).unwrap();
    assert!(pacs_server::log_files(&settings).unwrap().is_none());
}

#[test]
fn a_log_dir_that_cannot_be_made_stops_the_start() {
    let dir = tempfile::tempdir().unwrap();
    // a directory inside a file
    let file = dir.path().join("a-file");
    std::fs::write(&file, b"").unwrap();
    let logs = file.join("logs");
    let settings =
        pacs_server::load_settings(["pacs-server", "--log-dir", logs.to_str().unwrap()]).unwrap();
    let error = pacs_server::log_files(&settings).unwrap_err();
    assert!(format!("{error:#}").contains("PACS_LOG_DIR"), "{error:#}");
}

#[test]
fn no_log_days_is_refused() {
    let error = pacs_server::load_settings(["pacs-server", "--log-days", "0"]).unwrap_err();
    assert!(error.to_string().contains("--log-days"), "{error}");
}

#[test]
fn the_log_dir_is_read_from_the_settings_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("pacs.toml");
    std::fs::write(&config, "log_dir = \"logs\"\nlog_days = 7\n").unwrap();
    let settings =
        pacs_server::load_settings(["pacs-server", "--config", config.to_str().unwrap()]).unwrap();
    assert_eq!(settings.log_dir.as_deref(), Some(Path::new("logs")));
    assert_eq!(settings.log_days, 7);
}
