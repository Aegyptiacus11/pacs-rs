//! Ask a remote AE to send studies to an AE by C-MOVE (PS3.4 C.4.2), Study Root or Patient
//! Root as it takes them.

use std::ops::ControlFlow;
use std::process::ExitCode;

use clap::Parser;
use pacs_dimse::AeTitle;
use pacs_dimse::tools::{Common, remote};

/// Ask a DICOM AE to send studies to another AE (or to this one) with C-MOVE.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The remote AE asked: AE_TITLE@host:port.
    remote: String,
    /// Study Instance UIDs.
    #[arg(required = true)]
    studies: Vec<String>,
    /// The AE the studies go to, as the remote AE knows it.
    #[arg(long)]
    destination: AeTitle,
    /// The studies' Patient ID: needed by an AE that takes Patient Root only.
    #[arg(long)]
    patient_id: Option<String>,
    #[command(flatten)]
    common: Common,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let result = async {
        let remote = remote(&args.remote)?;
        let outcome = pacs_dimse::move_studies(
            &remote,
            &args.common.aet,
            args.patient_id.as_deref(),
            &args.studies,
            &args.destination,
            args.common.timeout(),
            args.common.timeout(),
            |progress| {
                eprintln!(
                    "movescu: {} sent, {} failed, {} left",
                    progress.completed.unwrap_or(0),
                    progress.failed.unwrap_or(0),
                    progress.remaining.unwrap_or(0)
                );
                ControlFlow::Continue(())
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        let p = outcome.progress;
        println!(
            "{}: {} sent, {} failed, {} with a warning",
            outcome.status,
            p.completed.unwrap_or(0),
            p.failed.unwrap_or(0),
            p.warning.unwrap_or(0)
        );
        Ok::<_, String>(p.failed.unwrap_or(0) == 0)
    }
    .await;
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("movescu: {e}");
            ExitCode::FAILURE
        }
    }
}
