//! Search a remote AE's studies by C-FIND (PS3.4 C.4.1), Study Root or Patient Root as it
//! takes them: each study found, as a line of DICOM JSON.

use std::ops::ControlFlow;
use std::process::ExitCode;

use clap::Parser;
use pacs_dimse::tools::{Common, remote};
use pacs_dimse::{DateRange, StudyQuery};

/// Search a DICOM AE's studies with C-FIND; one line of DICOM JSON a study.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The remote AE: AE_TITLE@host:port.
    remote: String,
    /// Patient ID, a value or a wild card (`*`, `?`).
    #[arg(long)]
    patient_id: Option<String>,
    /// Patient's Name, a value or a wild card.
    #[arg(long)]
    patient_name: Option<String>,
    /// Accession Number, a value or a wild card.
    #[arg(long)]
    accession_number: Option<String>,
    /// Study Date: YYYYMMDD, or a range YYYYMMDD-YYYYMMDD, -YYYYMMDD, YYYYMMDD-.
    #[arg(long)]
    study_date: Option<DateRange>,
    /// One modality of the study, e.g. CT.
    #[arg(long)]
    modality: Option<String>,
    #[command(flatten)]
    common: Common,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let result = async {
        let remote = remote(&args.remote)?;
        let query = StudyQuery {
            patient_id: args.patient_id.clone(),
            patient_name: args.patient_name.clone(),
            accession_number: args.accession_number.clone(),
            study_date: args.study_date.clone(),
            modality: args.modality.clone(),
        };
        let outcome = pacs_dimse::find_studies(
            &remote,
            &args.common.aet,
            &query,
            args.common.timeout(),
            |found| {
                println!("{}", pacs_dimse::query::to_json(&found));
                ControlFlow::Continue(())
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        eprintln!("findscu: {} found, {}", outcome.matches, outcome.status);
        Ok::<_, String>(())
    }
    .await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("findscu: {e}");
            ExitCode::FAILURE
        }
    }
}
