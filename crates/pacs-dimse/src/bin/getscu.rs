//! Retrieve studies from a remote AE by C-GET (PS3.4 C.4.3) into a folder: they come back
//! on the association that asks, so the remote AE needs no address of this one.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use pacs_dimse::Destination;
use pacs_dimse::scu::get::Receiving;
use pacs_dimse::tools::{Common, archive_in, remote};

/// Retrieve studies from a DICOM AE with C-GET into a folder.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The remote AE: AE_TITLE@host:port.
    remote: String,
    /// Study Instance UIDs.
    #[arg(required = true)]
    studies: Vec<String>,
    /// Where the instances go: their files under `files/`, an index beside them.
    #[arg(long, default_value = ".")]
    output: PathBuf,
    /// The studies' Patient ID: needed by an AE that takes Patient Root only.
    #[arg(long)]
    patient_id: Option<String>,
    /// The largest instance taken, in bytes.
    #[arg(long, default_value_t = 1 << 30)]
    max_bytes: usize,
    #[command(flatten)]
    common: Common,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let result = async {
        let remote = remote(&args.remote)?;
        let destination = Destination::Archive(archive_in(&args.output).await?);
        let outcome = pacs_dimse::get_studies(
            &remote,
            &args.common.aet,
            args.patient_id.as_deref(),
            &args.studies,
            Receiving {
                destination: &destination,
                max_data_set_bytes: args.max_bytes,
                observer: None,
            },
            args.common.timeout(),
            args.common.timeout(),
            |_| ControlFlow::Continue(()),
        )
        .await
        .map_err(|e| e.to_string())?;
        let p = outcome.progress;
        println!(
            "{}: {} received, {} failed, {} with a warning, in {}",
            outcome.status,
            p.completed.unwrap_or(0),
            p.failed.unwrap_or(0),
            p.warning.unwrap_or(0),
            args.output.display()
        );
        Ok::<_, String>(p.failed.unwrap_or(0) == 0)
    }
    .await;
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("getscu: {e}");
            ExitCode::FAILURE
        }
    }
}
