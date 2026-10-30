//! Embeds the built web interface (`ui/dist`, made by `npm run build` in `ui/`) into the
//! binary: writes `$OUT_DIR/ui_files.rs`, a list of (path, content type, bytes). Without a
//! build the list is empty and `/ui/` says how to make one; the server works either way.
//! `PACS_UI_DIST` names another directory.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-env-changed=PACS_UI_DIST");
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = env::var_os("PACS_UI_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../ui/dist"));
    // a directory: cargo looks at everything under it
    println!("cargo:rerun-if-changed={}", dist.display());

    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        walk(&dist, &dist, &mut files);
    }
    files.sort();
    let mut out = String::from("pub static FILES: &[(&str, &str, &[u8])] = &[\n");
    for (path, full) in &files {
        out.push_str(&format!(
            "    ({path:?}, {:?}, include_bytes!({:?})),\n",
            content_type(path),
            full.display().to_string()
        ));
    }
    out.push_str("];\n");
    let target = PathBuf::from(env::var("OUT_DIR").unwrap()).join("ui_files.rs");
    fs::write(target, out).unwrap();
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, files);
        } else {
            let relative = path.strip_prefix(root).unwrap();
            let name = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            files.push((name, path.canonicalize().unwrap()));
        }
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "map" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
