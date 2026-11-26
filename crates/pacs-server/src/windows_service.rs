//! pacs-server as a Windows service: installed, removed, and run under the service manager
//! (docs/windows.md).
//!
//! The service runs as its own virtual account, `NT SERVICE\<name>`, which has no password
//! and only the rights given to it, rather than as LocalSystem. Its command line holds
//! `--windows-service`, its name and the absolute path of its settings file; its working
//! directory is the settings file's, so the relative paths of the defaults (`data/...`) and
//! of the file land beside it. It starts with the machine and again 10 seconds after a
//! failure, three times a day at most.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use pacs_server::Settings;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

/// What the service manager waits for between two reports while the server starts (the
/// index's migrations) or stops (open requests, the audit trail sent on).
const WAIT_HINT: Duration = Duration::from_secs(120);

pub fn main(settings: Settings) -> anyhow::Result<()> {
    check_name(&settings.service_name)?;
    if settings.install_service {
        install(&settings)
    } else if settings.uninstall_service {
        uninstall(&settings.service_name)
    } else {
        run(settings)
    }
}

/// A name the service manager takes: 1 to 256 characters, no slash, no backslash.
fn check_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty()
        || name.chars().count() > 256
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        bail!(
            "--service-name {name:?}: 1 to 256 characters, without a slash, a backslash or a \
             control character"
        );
    }
    Ok(())
}

fn install(settings: &Settings) -> anyhow::Result<()> {
    let name = &settings.service_name;
    let config = settings
        .config
        .as_ref()
        .context("--install-service needs --config, the service's settings file")?;
    // load_settings has read and checked it; the service starts elsewhere, so the full path
    let config =
        std::fs::canonicalize(config).with_context(|| format!("--config: {}", config.display()))?;
    let config = without_verbatim_prefix(config);
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("opening the service manager: --install-service needs an administrator")?;
    let info = ServiceInfo {
        name: name.into(),
        display_name: if name == "pacs-rs" {
            "pacs-rs".into()
        } else {
            format!("pacs-rs ({name})").into()
        },
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe().context("the path of pacs-server.exe")?,
        launch_arguments: vec![
            OsString::from("--windows-service"),
            OsString::from("--service-name"),
            OsString::from(name),
            OsString::from("--config"),
            config.clone().into_os_string(),
        ],
        dependencies: vec![],
        account_name: Some(account(name).into()),
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::DELETE)
        .with_context(|| format!("installing the service {name}"))?;
    let configured = (|| -> anyhow::Result<()> {
        service.set_description(
            "pacs-rs: a PACS, with DICOMweb, a REST API and DIMSE (https://github.com/Aegyptiacus11/pacs-rs)",
        )?;
        service.update_failure_actions(ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(10),
                };
                3
            ]),
        })?;
        // a start that fails on its settings counts too, not only a crash
        service.set_failure_actions_on_non_crash_failures(true)?;
        Ok(())
    })();
    if let Err(error) = configured {
        // half a service is worse than none: the next --install-service starts clean
        service
            .delete()
            .with_context(|| format!("removing the service {name} after: {error:#}"))?;
        return Err(error.context(format!("configuring the service {name}")));
    }
    println!(
        "installed the service {name}, running as {account} with the settings of {config}.\n\
         {account} must be able to change the directories of its index, files and log:\n  \
         icacls <directory> /grant \"{account}:(OI)(CI)M\"\n\
         then start it with: sc.exe start {name}",
        account = account(name),
        config = config.display(),
    );
    Ok(())
}

/// The virtual account a service of this name runs as.
fn account(name: &str) -> String {
    format!("NT SERVICE\\{name}")
}

/// `\\?\C:\x` as `C:\x`: canonicalize gives the first, which the service's command line and
/// the messages need not carry. A path the prefix is needed for (a long one) keeps it.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.len() < 248 && rest.as_bytes().get(1) == Some(&b':') => {
            PathBuf::from(rest)
        }
        _ => path,
    }
}

fn uninstall(name: &str) -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("opening the service manager: --uninstall-service needs an administrator")?;
    let service = manager
        .open_service(
            name,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .with_context(|| format!("opening the service {name}"))?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        service
            .stop()
            .with_context(|| format!("stopping the service {name}"))?;
        let since = Instant::now();
        while service.query_status()?.current_state != ServiceState::Stopped {
            if since.elapsed() > WAIT_HINT {
                bail!(
                    "the service {name} has not stopped after {} s; not removed",
                    WAIT_HINT.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    service
        .delete()
        .with_context(|| format!("removing the service {name}"))?;
    println!(
        "removed the service {name}; its settings, index, files and log are left as they were"
    );
    Ok(())
}

/// The settings `service_main` runs with: it takes no arguments of its own.
static SETTINGS: OnceLock<Settings> = OnceLock::new();
/// Whether the service stopped on an error, for the process's exit code.
static FAILED: AtomicBool = AtomicBool::new(false);

fn run(mut settings: Settings) -> anyhow::Result<()> {
    let config = settings
        .config
        .clone()
        .context("--windows-service needs --config")?;
    let dir = config
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .context("--windows-service: --config must be the settings file's full path")?;
    // the service manager starts services in System32; relative paths go beside the settings
    std::env::set_current_dir(dir)
        .with_context(|| format!("--windows-service: entering {}", dir.display()))?;
    // no console to write to
    if settings.log_dir.is_none() {
        settings.log_dir = Some(PathBuf::from("logs"));
    }
    crate::init_logging(&settings)?;
    let name = settings.service_name.clone();
    if SETTINGS.set(settings).is_err() {
        bail!("--windows-service: started twice in one process");
    }
    service_dispatcher::start(&name, ffi_service_main).context(
        "--windows-service is how the service manager starts pacs-server, not a console: \
         install the service with --install-service and start it with sc.exe start",
    )?;
    if FAILED.load(Ordering::SeqCst) {
        bail!("the service {name} stopped on an error, said in its log");
    }
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = serve_as_service() {
        tracing::error!("the service stopped: {error:#}");
        FAILED.store(true, Ordering::SeqCst);
    }
}

fn serve_as_service() -> anyhow::Result<()> {
    let settings = SETTINGS
        .get()
        .context("the service started without its settings")?
        .clone();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let mut stop = Some(stop);
    let handler = move |control| match control {
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        ServiceControl::Stop | ServiceControl::Shutdown => {
            // a second Stop while stopping finds the sender taken
            if let Some(stop) = stop.take() {
                let _ = stop.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status = service_control_handler::register(&settings.service_name, handler)
        .context("registering with the service manager")?;
    let report = |state, controls_accepted, exit_code, wait_hint| {
        status
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted,
                exit_code,
                checkpoint: 0,
                wait_hint,
                process_id: None,
            })
            .context("reporting to the service manager")
    };
    report(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::NO_ERROR,
        WAIT_HINT,
    )?;
    let served = crate::runtime().and_then(|runtime| {
        runtime.block_on(crate::serve(
            settings,
            async move {
                // the sender dropped without a Stop is a stop too: nothing can stop it after
                let _ = stopped.await;
                tracing::info!("shutting down: the service manager asked");
            },
            || {
                report(
                    ServiceState::Running,
                    ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                    ServiceExitCode::NO_ERROR,
                    Duration::ZERO,
                )
            },
        ))
    });
    // a service-specific code, so that the service manager counts a failure and restarts it
    let exit_code = match served {
        Ok(()) => ServiceExitCode::NO_ERROR,
        Err(_) => ServiceExitCode::ServiceSpecific(1),
    };
    if let Err(error) = &served {
        tracing::error!("{error:#}");
    }
    report(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit_code,
        Duration::ZERO,
    )?;
    if served.is_err() {
        FAILED.store(true, Ordering::SeqCst);
    }
    Ok(())
}
