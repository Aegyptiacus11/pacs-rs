//! What the command-line tools share (feature `tools`): the remote AE as `AE@host:port`, the
//! calling AE title, the timeouts, and an archive in a folder for what is received.

use std::path::Path;
use std::time::Duration;

use pacs_archive::{Archive, Compression};
use pacs_index::Index;
use pacs_store::Store;

use crate::ae::{AeTitle, RemoteAe};

/// The options every tool takes.
#[derive(Debug, Clone, clap::Args)]
pub struct Common {
    /// The calling AE title: this tool's.
    #[arg(long, default_value = "PACSRS")]
    pub aet: AeTitle,
    /// Seconds to wait for the remote AE: to connect, and for each answer.
    #[arg(long, default_value_t = 30)]
    pub timeout: u64,
}

impl Common {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout.max(1))
    }
}

/// A remote AE written `AE_TITLE@host:port`.
pub fn remote(text: &str) -> Result<RemoteAe, String> {
    format!("remote={text}")
        .parse::<RemoteAe>()
        .map_err(|e| format!("{text:?}: {e}; a remote AE is AE_TITLE@host:port"))
}

/// An archive in `folder` that keeps what it receives as it was received: the files under
/// `folder/files`, their index in `folder/index.sqlite`.
pub async fn archive_in(folder: &Path) -> Result<Archive, String> {
    let files = folder.join("files");
    std::fs::create_dir_all(&files)
        .map_err(|e| format!("making the folder {}: {e}", files.display()))?;
    let store = Store::local(&files, "").map_err(|e| format!("{}: {e}", files.display()))?;
    let index = Index::open(folder.join("index.sqlite"))
        .await
        .map_err(|e| format!("the index in {}: {e}", folder.display()))?;
    Ok(Archive::new(store, index).with_compression(Compression::Off))
}

/// Every file under `paths`, folders walked whole, in name order.
pub fn files_under(paths: &[std::path::PathBuf]) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        walk(path, &mut files)?;
    }
    Ok(files)
}

fn walk(path: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_file() {
        files.push(path.to_owned());
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    entries.sort();
    for entry in entries {
        walk(&entry, files)?;
    }
    Ok(())
}
