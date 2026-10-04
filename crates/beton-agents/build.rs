//! Bettet die Built-in-Agents aus `agents/` (Repo-Wurzel) in das Crate ein (AGT-003).
//!
//! Erzeugt `$OUT_DIR/builtins.rs` mit einer sortierten Liste `(relativer Pfad, Inhalt)`.
//! Versteckte Dateien (z. B. `.gitkeep`) gehören nicht dazu.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push((rel.join("/"), path));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest.join("../../agents");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    files.sort();
    let mut code = String::from("pub(crate) static EMBEDDED: &[(&str, &[u8])] = &[\n");
    for (rel, path) in &files {
        let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
        let _ = writeln!(
            code,
            "    ({rel:?}, include_bytes!({:?})),",
            abs.display().to_string()
        );
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("builtins.rs");
    if let Err(e) = std::fs::write(&out, code) {
        panic!("{}: {e}", out.display());
    }
}
