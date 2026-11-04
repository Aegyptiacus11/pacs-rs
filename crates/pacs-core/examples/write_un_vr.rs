//! Write synthetic files with standard elements in UN, and with lengths that lie.
//!
//! ```bash
//! cargo run -p pacs-core --features synthetic --example write_un_vr -- out/ 16
//! ```
//! `un_sequence_*` and `un_uid_*` are legal (PS3.5 6.2.2); `bad_length_*` are not.

use dicom_dictionary_std::uids;
use pacs_core::synthetic::{RawFile, un_sequence_file, un_study_uid_file};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "un-vr".into());
    let count: u32 = args.next().map_or(16, |s| s.parse().expect("a count"));
    std::fs::create_dir_all(&dir)?;
    for n in 0..count {
        let seed = 9000 + n;
        std::fs::write(format!("{dir}/un_sequence_{n}.dcm"), un_sequence_file(seed))?;
        std::fs::write(
            format!("{dir}/un_uid_{n}.dcm"),
            un_study_uid_file(seed + 1000),
        )?;
        let bad = RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN)
            .identifiers(seed + 2000, b"UI")
            .long_declared((0x0043, 0x1001), b"UN", 0xC000_0000, b"short")
            .bytes();
        std::fs::write(format!("{dir}/bad_length_{n}.dcm"), bad)?;
    }
    Ok(())
}
