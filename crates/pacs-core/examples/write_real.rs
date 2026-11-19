//! Write a series of the scanner's CT image to a directory, one `.dcm` file per slice: the
//! file of `tests/data/pydicom` under a series' identifiers (`pacs_core::real`).
//!
//! ```bash
//! cargo run -p pacs-core --features real --example write_real -- out/ 7 20
//! ```
//! arguments: directory, seed (default 1), slices (default 3)

use pacs_core::real::RealSeries;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "real".into());
    let seed = args
        .next()
        .map_or(1, |s| s.parse().expect("seed is a number"));
    let slices = args
        .next()
        .map_or(3, |s| s.parse().expect("slices is a number"));
    let series = RealSeries::new(seed, slices);
    std::fs::create_dir_all(&dir)?;
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        std::fs::write(format!("{dir}/{:05}.dcm", slice + 1), bytes)?;
    }
    println!("study {}", series.study_uid());
    println!("series {}", series.series_uid());
    Ok(())
}
