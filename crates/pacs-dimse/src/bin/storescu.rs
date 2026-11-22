//! Send DICOM files to a remote AE by C-STORE (PS3.4 Annex B), on one association: each in
//! the transfer syntax it is in when the peer takes it, decoded otherwise.

use std::path::PathBuf;
use std::process::ExitCode;

use bytes::Bytes;
use clap::Parser;
use pacs_dimse::Sender;
use pacs_dimse::tools::{Common, files_under, remote};

/// Send DICOM files, or folders of them, to an AE with C-STORE.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The remote AE: AE_TITLE@host:port.
    remote: String,
    /// Files, and folders whose files are all sent.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    #[command(flatten)]
    common: Common,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match send(&args).await {
        Ok(0) => ExitCode::SUCCESS,
        Ok(failed) => {
            eprintln!("storescu: {failed} not stored");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("storescu: {e}");
            ExitCode::FAILURE
        }
    }
}

/// How many files were not stored.
async fn send(args: &Args) -> Result<usize, String> {
    let remote = remote(&args.remote)?;
    let mut files = Vec::new();
    let mut failed = 0;
    for path in files_under(&args.paths)? {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match pacs_core::Instance::from_bytes(&bytes) {
            Ok(instance) => files.push((path, Bytes::from(bytes), instance.key)),
            Err(e) => {
                eprintln!("{}: not DICOM: {e}", path.display());
                failed += 1;
            }
        }
    }
    if files.is_empty() {
        return Err("no DICOM file to send".into());
    }
    let stored: Vec<(String, String)> = files
        .iter()
        .map(|(_, _, key)| (key.sop_class_uid.clone(), key.transfer_syntax_uid.clone()))
        .collect();
    let mut sender = Sender::open(
        &remote,
        &args.common.aet,
        &stored,
        args.common.timeout(),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut holds = true;
    for (path, bytes, key) in files {
        if !holds {
            eprintln!("{}: not sent, the association is over", path.display());
            failed += 1;
            continue;
        }
        match sender
            .send(
                bytes,
                &key.sop_class_uid,
                &key.sop_instance_uid,
                &key.transfer_syntax_uid,
            )
            .await
        {
            Ok(status) => println!("{}: {status}", path.display()),
            Err(e) => {
                holds = e.association_holds();
                eprintln!("{}: {e}", path.display());
                failed += 1;
            }
        }
    }
    if holds {
        sender.release().await;
    } else {
        sender.abort().await;
    }
    Ok(failed)
}
