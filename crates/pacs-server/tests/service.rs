//! pacs-server as a Windows service: installed, started, answering, stopped and removed by
//! the real service manager on Windows (an administrator's session, as on CI), and refused
//! elsewhere.

use std::process::{Command, Output};

fn pacs_server(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pacs-server"))
        .args(args)
        .env_remove("PACS_CONFIG")
        .output()
        .expect("running pacs-server")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn installing_needs_a_settings_file() {
    let output = pacs_server(&["--install-service"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--config"), "{}", stderr(&output));
}

#[test]
fn installing_is_not_a_health_check_too() {
    let output = pacs_server(&["--install-service", "--healthcheck", "--config", "x.toml"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot be used with"),
        "{}",
        stderr(&output)
    );
}

#[cfg(not(windows))]
#[test]
fn the_service_flags_are_refused_off_windows() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("pacs.toml");
    std::fs::write(&config, "").unwrap();
    let config = config.to_str().unwrap();
    for flag in [
        "--install-service",
        "--uninstall-service",
        "--windows-service",
    ] {
        let output = pacs_server(&[flag, "--config", config]);
        assert_eq!(output.status.code(), Some(1), "{flag}");
        assert!(
            stderr(&output).contains("for Windows"),
            "{flag}: {}",
            stderr(&output)
        );
    }
}

#[cfg(windows)]
mod windows {
    use std::net::{SocketAddr, TcpListener};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::{pacs_server, stderr};

    /// A service of its own name, and a directory under ProgramData its account can be given
    /// rights to, both removed at the end, also when the test fails.
    struct Installed {
        name: String,
        dir: PathBuf,
    }

    impl Installed {
        fn new(test: &str) -> Self {
            let name = format!("pacs-rs-test-{test}-{}", std::process::id());
            let dir = Path::new(r"C:\ProgramData").join(&name);
            std::fs::create_dir_all(&dir).unwrap();
            Installed { name, dir }
        }

        fn config(&self, text: &str) -> String {
            let path = self.dir.join("pacs.toml");
            std::fs::write(&path, text).unwrap();
            path.to_str().unwrap().to_owned()
        }

        fn install(&self, config: &str) -> std::process::Output {
            pacs_server(&[
                "--install-service",
                "--service-name",
                &self.name,
                "--config",
                config,
            ])
        }

        /// What docs/windows.md says to do after installing.
        fn grant(&self) {
            let account = format!("NT SERVICE\\{}:(OI)(CI)M", self.name);
            let status = Command::new("icacls")
                .arg(&self.dir)
                .args(["/grant", &account])
                .status()
                .unwrap();
            assert!(status.success(), "icacls");
        }

        fn start(&self) {
            let output = Command::new("sc.exe")
                .args(["start", &self.name])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "sc.exe start: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }

        /// `sc.exe query`'s answer, or None once the service is gone.
        fn query(&self) -> Option<String> {
            let output = Command::new("sc.exe")
                .args(["query", &self.name])
                .output()
                .unwrap();
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        }

        fn log(&self) -> String {
            let mut text = String::new();
            let logs = self.dir.join("logs");
            let Ok(entries) = std::fs::read_dir(&logs) else {
                return text;
            };
            for entry in entries {
                text += &std::fs::read_to_string(entry.unwrap().path()).unwrap();
            }
            text
        }
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            // cleaning up after a test that may have failed half-way: the service may be gone
            // already, so the outcome is not checked
            let _ = pacs_server(&["--uninstall-service", "--service-name", &self.name]);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn free_address() -> SocketAddr {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let since = Instant::now();
        while !done() {
            assert!(
                since.elapsed() < Duration::from_secs(90),
                "waited for {what}"
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn installed_started_answering_and_removed() {
        let service = Installed::new("run");
        let address = free_address();
        // the index and files at their defaults, data\ beside the settings
        let config = service.config(&format!("listen = \"{address}\"\n"));
        let installed = service.install(&config);
        assert!(installed.status.success(), "{}", stderr(&installed));
        service.grant();
        let query = service.query().expect("installed");
        assert!(query.contains("STOPPED"), "{query}");
        // restarted 10 s after a failure, a failed start included: the access a restart action
        // needs was once missing, and the install failed on it
        let failure = Command::new("sc.exe")
            .args(["qfailure", &service.name])
            .output()
            .unwrap();
        assert!(failure.status.success());
        let failure = String::from_utf8_lossy(&failure.stdout);
        assert_eq!(
            failure.matches("RESTART -- Delay = 10000").count(),
            3,
            "{failure}"
        );
        let flag = Command::new("sc.exe")
            .args(["qfailureflag", &service.name])
            .output()
            .unwrap();
        assert!(flag.status.success());
        assert!(String::from_utf8_lossy(&flag.stdout).contains("TRUE"));
        service.start();
        let since = Instant::now();
        while !pacs_server::healthy(address).await {
            assert!(
                since.elapsed() < Duration::from_secs(90),
                "no answer on {address}; log:\n{}",
                service.log()
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(service.query().unwrap().contains("RUNNING"));
        assert!(service.dir.join(r"data\index.sqlite").exists());
        // removing stops it first
        let removed = pacs_server(&["--uninstall-service", "--service-name", &service.name]);
        assert!(removed.status.success(), "{}", stderr(&removed));
        assert!(!pacs_server::healthy(address).await);
        wait_for("the service to be gone", || service.query().is_none());
        let log = service.log();
        assert!(log.contains("ready"), "{log}");
        assert!(log.contains("shutting down"), "{log}");
        // what it stored stays
        assert!(service.dir.join(r"data\index.sqlite").exists());
    }

    #[test]
    fn a_service_that_cannot_start_says_why_and_stops() {
        let service = Installed::new("fail");
        // the address taken by someone else
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = taken.local_addr().unwrap();
        let config = service.config(&format!("listen = \"{address}\"\n"));
        let installed = service.install(&config);
        assert!(installed.status.success(), "{}", stderr(&installed));
        service.grant();
        service.start();
        wait_for("the error in the log", || service.log().contains("ERROR"));
        wait_for("the service to stop", || {
            service.query().is_some_and(|q| q.contains("STOPPED"))
        });
        let query = service.query().unwrap();
        // ERROR_SERVICE_SPECIFIC_ERROR, so that the service manager restarts it
        assert!(query.contains("1066"), "{query}");
        drop(taken);
    }

    #[test]
    fn settings_with_a_mistake_install_nothing() {
        let service = Installed::new("bad");
        let config = service.config("lisen = \"127.0.0.1:8080\"\n");
        let installed = service.install(&config);
        assert!(!installed.status.success());
        assert!(
            stderr(&installed).contains("lisen"),
            "{}",
            stderr(&installed)
        );
        assert!(service.query().is_none());
    }

    #[test]
    fn the_same_name_twice_is_refused() {
        let service = Installed::new("twice");
        let config = service.config("");
        assert!(service.install(&config).status.success());
        let again = service.install(&config);
        assert!(!again.status.success());
        assert!(
            stderr(&again).contains("installing the service"),
            "{}",
            stderr(&again)
        );
    }

    #[test]
    fn removing_a_service_that_is_not_there_fails() {
        let output = pacs_server(&[
            "--uninstall-service",
            "--service-name",
            "pacs-rs-test-never-installed",
        ]);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("opening the service"),
            "{}",
            stderr(&output)
        );
    }

    #[test]
    fn a_name_with_a_backslash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("pacs.toml");
        std::fs::write(&config, "").unwrap();
        let output = pacs_server(&[
            "--install-service",
            "--service-name",
            r"a\b",
            "--config",
            config.to_str().unwrap(),
        ]);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("--service-name"),
            "{}",
            stderr(&output)
        );
    }

    #[test]
    fn the_service_mode_from_a_console_fails() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("pacs.toml");
        std::fs::write(&config, "").unwrap();
        let output = pacs_server(&["--windows-service", "--config", config.to_str().unwrap()]);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("not a console"),
            "{}",
            stderr(&output)
        );
    }
}
