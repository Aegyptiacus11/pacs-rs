//! Settings in a file (`PACS_CONFIG`, `--config`): taken when nothing else says otherwise,
//! checked whole at start, secrets refused.

use pacs_server::load_settings;

fn file(text: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pacs.toml");
    std::fs::write(&path, text).unwrap();
    let path = path.display().to_string();
    (dir, path)
}

#[test]
fn the_file_gives_what_the_command_line_and_the_environment_do_not() {
    let (_dir, path) = file(
        r#"
        listen = "0.0.0.0:8099"
        trash_days = 7
        auth = false
        cors_origins = ["https://a.example", "https://b.example"]
        storage = "file:/srv/pacs/files"
        "#,
    );
    let settings = load_settings(["pacs-server", "--config", &path]).unwrap();
    assert_eq!(settings.listen.to_string(), "0.0.0.0:8099");
    assert_eq!(settings.trash_days, 7);
    assert!(!settings.auth);
    assert_eq!(
        settings.cors_origins,
        ["https://a.example", "https://b.example"]
    );
    assert_eq!(settings.storage, "file:/srv/pacs/files");
    // the command line comes first
    let settings = load_settings(["pacs-server", "--config", &path, "--trash-days", "0"]).unwrap();
    assert_eq!(settings.trash_days, 0);
    assert_eq!(settings.listen.to_string(), "0.0.0.0:8099");
    // without a file, the defaults
    assert_eq!(load_settings(["pacs-server"]).unwrap().trash_days, 30);
}

#[test]
fn a_variable_comes_before_the_file() {
    let (_dir, path) = file("check_files_days = 5\n");
    // SAFETY: no other test of this binary reads PACS_CHECK_FILES_DAYS
    unsafe { std::env::set_var("PACS_CHECK_FILES_DAYS", "9") };
    let settings = load_settings(["pacs-server", "--config", &path]).unwrap();
    unsafe { std::env::remove_var("PACS_CHECK_FILES_DAYS") };
    assert_eq!(settings.check_files_days, 9);
    let settings = load_settings(["pacs-server", "--config", &path]).unwrap();
    assert_eq!(settings.check_files_days, 5);
}

#[test]
fn every_problem_of_the_file_is_said_at_once() {
    let (_dir, path) = file(
        r#"
        listen = "nowhere"
        trash_dayz = 3
        auth_secret = "in a file"
        project_secret_LIVER = "neither"
        index = ["a", "b"]
        dimse_remotes = [1.5]
        config = "/etc/other.toml"

        [projects]
        liver = 1
        "#,
    );
    let error = format!(
        "{:#}",
        load_settings(["pacs-server", "--config", &path]).unwrap_err()
    );
    for said in [
        "listen: ",
        "trash_dayz: not a setting (perhaps trash_days)",
        "auth_secret: a secret, read from the environment only (PACS_AUTH_SECRET or PACS_AUTH_SECRET_FILE)",
        "project_secret_LIVER: a secret",
        "index: one value, not a list",
        "dimse_remotes: a list of strings or numbers",
        "config: a file of settings cannot name another",
        "projects: a string, a number, true or false, not table",
    ] {
        assert!(error.contains(said), "{said:?} not in:\n{error}");
    }
    assert!(error.contains(&path), "{error}");
}

#[test]
fn a_file_that_is_not_toml_or_not_there_stops_the_start() {
    let (_dir, path) = file("listen = \n");
    let error = format!(
        "{:#}",
        load_settings(["pacs-server", "--config", &path]).unwrap_err()
    );
    assert!(error.contains("is not TOML"), "{error}");
    let error = format!(
        "{:#}",
        load_settings(["pacs-server", "--config", "/nonexistent/pacs.toml"]).unwrap_err()
    );
    assert!(error.contains("reading /nonexistent/pacs.toml"), "{error}");
}
