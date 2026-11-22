//! Writes docs/conformance.md, the Conformance Statement, from the code.

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/conformance.md");
    std::fs::write(path, pacs_dimse::conformance::statement())
        .unwrap_or_else(|e| panic!("writing {path}: {e}"));
    println!("wrote {path}");
}
