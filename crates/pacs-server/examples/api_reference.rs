//! Writes docs/api-reference.md from the handlers' descriptions:
//!
//! ```bash
//! cargo run -p pacs-server --example api_reference
//! ```
//!
//! `tests/api_reference.rs` fails while the page is not what this writes.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let page = match pacs_server::api::reference(&pacs_server::api::description()) {
        Ok(page) => page,
        Err(e) => {
            eprintln!("the reference cannot be written: {e}");
            return ExitCode::FAILURE;
        }
    };
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/api-reference.md");
    if let Err(e) = std::fs::write(&file, page) {
        eprintln!("{}: {e}", file.display());
        return ExitCode::FAILURE;
    }
    println!("wrote {}", file.display());
    ExitCode::SUCCESS
}
