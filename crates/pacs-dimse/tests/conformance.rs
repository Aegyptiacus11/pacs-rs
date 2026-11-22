//! The Conformance Statement in the docs is the one the code writes.

#[test]
fn the_statement_in_the_docs_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/conformance.md");
    let written = std::fs::read_to_string(path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        written == pacs_dimse::conformance::statement(),
        "docs/conformance.md is not what the code negotiates: run \
         `cargo run -p pacs-dimse --example conformance` and commit the page"
    );
}
