//! C-ECHO a remote AE: whether it answers (PS3.4 Annex A).

use std::process::ExitCode;

use clap::Parser;
use pacs_dimse::tools::{Common, remote};

/// Test the link to a DICOM AE with C-ECHO.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The remote AE: AE_TITLE@host:port.
    remote: String,
    #[command(flatten)]
    common: Common,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let result = async {
        let remote = remote(&args.remote)?;
        let echo = pacs_dimse::echo(&remote, &args.common.aet, args.common.timeout())
            .await
            .map_err(|e| e.to_string())?;
        println!(
            "{} answered C-ECHO in {} ms",
            remote.ae_title,
            echo.elapsed.as_millis()
        );
        Ok::<_, String>(())
    }
    .await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("echoscu: {e}");
            ExitCode::FAILURE
        }
    }
}
