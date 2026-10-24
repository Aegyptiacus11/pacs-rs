//! Write a synthetic CT series to a directory, one `.dcm` file per slice.
//!
//! ```bash
//! cargo run -p pacs-core --features synthetic --example write_synthetic -- out/ 7 20
//! ```
//! arguments: directory, seed (default 1), slices (default 3), rows and columns (default 4)

use pacs_core::synthetic::SyntheticSeries;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "synthetic".into());
    let seed = args
        .next()
        .map_or(1, |s| s.parse().expect("seed is a number"));
    let slices = args
        .next()
        .map_or(3, |s| s.parse().expect("slices is a number"));
    let size = args
        .next()
        .map_or(4, |s| s.parse().expect("size is a number"));
    let mut series = SyntheticSeries::new(seed, slices);
    series.rows = size;
    series.columns = size;
    std::fs::create_dir_all(&dir)?;
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        std::fs::write(format!("{dir}/{:05}.dcm", slice + 1), bytes)?;
    }
    println!("study {}", series.study_uid());
    println!("series {}", series.series_uid());
    Ok(())
}
