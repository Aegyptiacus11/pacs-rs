use std::future::Future;

use pacs_server::{
    Settings, StorageLocation, app_with_audit, dimse_config, open_archive, serve_dimse_audited,
    shutdown_signal,
};
use tracing_subscriber::EnvFilter;

#[cfg(windows)]
mod windows_service;

// musl's allocator is slow under many threads (frames ran 25% slower in the static image);
// mimalloc keeps the static binary as fast as the glibc one.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> anyhow::Result<()> {
    let settings = match pacs_server::load_settings(std::env::args_os()) {
        Ok(settings) => settings,
        // --help and --version are answers, not failures
        Err(e) => e.downcast::<clap::Error>()?.exit(),
    };
    if settings.install_service || settings.uninstall_service || settings.windows_service {
        #[cfg(windows)]
        return windows_service::main(settings);
        #[cfg(not(windows))]
        anyhow::bail!(
            "--install-service, --uninstall-service and --windows-service are for Windows; \
             elsewhere run pacs-server under systemd, Docker or another supervisor"
        );
    }
    init_logging(&settings)?;
    runtime()?.block_on(console(settings))
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

/// The log to standard output, or to the daily files of PACS_LOG_DIR.
fn init_logging(settings: &Settings) -> anyhow::Result<()> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let (stdout, file) = match pacs_server::log_files(settings)? {
        Some(files) => (
            None,
            Some(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(files),
            ),
        ),
        None => (Some(tracing_subscriber::fmt::layer()), None),
    };
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .with(stdout)
        .with(file)
        // what is logged at ERROR, counted for the metrics
        .with(pacs_server::metrics::CountErrors)
        .try_init()?;
    Ok(())
}

/// From a console or a container: the one-off checks, or the server until Ctrl-C or SIGTERM.
async fn console(settings: Settings) -> anyhow::Result<()> {
    if settings.healthcheck {
        std::process::exit(if pacs_server::healthy(settings.listen).await {
            0
        } else {
            1
        });
    }
    if settings.consistency {
        let archive = open_archive(&settings).await?;
        pacs_server::consistency::check_quarantine(&settings)?;
        let audit = pacs_server::audit::open_audit(&settings)?;
        let agree = pacs_server::consistency::run(
            &archive,
            &audit,
            settings.repair,
            std::time::Duration::from_secs(settings.grace),
            &mut std::io::stdout(),
        )
        .await?;
        std::process::exit(if agree { 0 } else { 1 });
    }
    if settings.check_files {
        let archive = open_archive(&settings).await?;
        let mut summary = pacs_server::checks::Summary::default();
        let result = pacs_server::checks::check_all(&archive, &mut summary).await;
        for failure in &summary.failures {
            println!("{}: {}", failure.sop_instance_uid, failure.failure);
        }
        println!(
            "{} files checked, {} failed",
            summary.checked,
            summary.failures.len()
        );
        result?;
        std::process::exit(if summary.failures.is_empty() { 0 } else { 1 });
    }
    serve(settings, shutdown_signal(), || Ok(())).await
}

/// The server, until `shutdown` completes. `ready` is called once both listeners are bound,
/// before the first request is taken: when the service manager is told the service runs.
async fn serve(
    settings: Settings,
    shutdown: impl Future<Output = ()> + Send + 'static,
    ready: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let archive = open_archive(&settings).await?;
    let login =
        pacs_server::auth::open_login(&settings, archive.index(), pacs_server::process_env).await?;
    if login.is_none() {
        tracing::warn!(
            "no login (PACS_AUTH is off): anyone who reaches this server can read and delete"
        );
    }
    let audit = pacs_server::audit::open_audit(&settings)?;
    pacs_server::checks::spawn_due_checks(archive.clone(), settings.check_files_days);
    pacs_server::trash::spawn_emptying(archive.clone());
    pacs_server::spawn_job_sweeps(&settings, &archive, &audit)?;
    pacs_server::spawn_forwarding(&settings, &archive, &audit)?;
    pacs_server::spawn_write_back(&settings, &archive, &audit)?;
    let app = app_with_audit(archive.clone(), &settings, login, audit.clone())?;
    let listener = tokio::net::TcpListener::bind(settings.listen).await?;
    if let Some(metrics) = settings.metrics_listen {
        let metrics = tokio::net::TcpListener::bind(metrics).await?;
        tracing::info!(metrics = %metrics.local_addr()?, "metrics at /metrics");
        let routes = pacs_server::metrics::router(archive.clone());
        tokio::spawn(async move {
            if let Err(error) = axum::serve(metrics, routes).await {
                tracing::error!(%error, "the metrics listener stopped");
            }
        });
    }
    // one shutdown signal for both servers
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown.await;
        let _ = stop.send(true);
    });
    let until_stopped = move || {
        let mut stopped = stopped.clone();
        async move {
            let _ = stopped.wait_for(|s| *s).await;
        }
    };
    let mut ae_titles = Vec::new();
    if let Some(dimse_listen) = settings.dimse_listen {
        let config = dimse_config(&settings)?;
        ae_titles.push(config.ae_title.to_string());
        let projects = pacs_server::projects::open_projects_from_env(&settings, &archive)?;
        let dimse = tokio::net::TcpListener::bind(dimse_listen).await?;
        tracing::info!(
            dimse = %dimse.local_addr()?,
            ae_title = %config.ae_title,
            remotes = config.directory.len(),
            projects = projects.len(),
            "DIMSE ready",
        );
        tokio::spawn(serve_dimse_audited(
            dimse,
            archive,
            config,
            projects,
            audit.clone(),
            until_stopped(),
        ));
    }
    tracing::info!(
        listen = %listener.local_addr()?,
        dicomweb = %settings.dicomweb_path,
        storage = ?StorageLocation::parse(&settings.storage)?,
        index = %pacs_server::without_password(&settings.index),
        "pacs-server {} ready",
        env!("CARGO_PKG_VERSION"),
    );
    // the audit file sent on to a repository, until the server has said it stops
    let (said_stop, stop_said) = tokio::sync::oneshot::channel::<()>();
    let forwarding = match (
        pacs_server::audit_forward::open(&settings)?,
        audit.written(),
    ) {
        (Some(forward), Some(written)) => {
            tracing::info!(repository = %forward.repository(), "the audit trail is sent to a repository");
            Some(tokio::spawn(forward.run(written, async {
                let _ = stop_said.await;
            })))
        }
        _ => None,
    };
    ready()?;
    audit.application(true, &ae_titles);
    let limits = pacs_server::limits::Limits::from_settings(&settings);
    let served = pacs_server::limits::serve(listener, app, limits, until_stopped()).await;
    audit.application(false, &ae_titles);
    // the last lines go before the process does, when the repository takes them in time
    let _ = said_stop.send(());
    if let Some(forwarding) = forwarding {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), forwarding).await;
    }
    served
}
