//! Receive DICOM instances by C-STORE (PS3.4 Annex B) into a folder, as they were sent,
//! until stopped; C-ECHO is answered too.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use pacs_dimse::tools::archive_in;
use pacs_dimse::{AeTitle, Config, Destination, Scp};

/// Receive DICOM instances by C-STORE into a folder.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The port to listen on.
    port: u16,
    /// Where the instances go: their files under `files/`, an index beside them.
    #[arg(long, default_value = ".")]
    output: PathBuf,
    /// This AE's title, the called AE title it answers.
    #[arg(long, default_value = "STORESCP")]
    aet: AeTitle,
    /// The address to listen on.
    #[arg(long, default_value = "0.0.0.0")]
    bind: std::net::IpAddr,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let result = async {
        let archive = archive_in(&args.output).await?;
        let listener = tokio::net::TcpListener::bind((args.bind, args.port))
            .await
            .map_err(|e| format!("listening on {}:{}: {e}", args.bind, args.port))?;
        eprintln!(
            "storescp: {} on {}, into {}",
            args.aet,
            listener.local_addr().map_err(|e| e.to_string())?,
            args.output.display()
        );
        let scp = Scp::new(Config::new(args.aet.clone()), Destination::Archive(archive));
        Arc::new(scp)
            .serve(listener, async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await;
        Ok::<_, String>(())
    }
    .await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("storescp: {e}");
            ExitCode::FAILURE
        }
    }
}
