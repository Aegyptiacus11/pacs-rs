use clap::Parser;
use pacs_server::{Settings, StorageLocation, app, open_archive, serve, shutdown_signal};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();

    let settings = Settings::parse();
    let archive = open_archive(&settings).await?;
    let app = app(archive, &settings)?;
    let listener = tokio::net::TcpListener::bind(settings.listen).await?;
    tracing::info!(
        listen = %listener.local_addr()?,
        dicomweb = %settings.dicomweb_path,
        storage = ?StorageLocation::parse(&settings.storage)?,
        index = %settings.index.display(),
        "pacs-server {} ready",
        env!("CARGO_PKG_VERSION"),
    );
    serve(listener, app, shutdown_signal()).await
}
