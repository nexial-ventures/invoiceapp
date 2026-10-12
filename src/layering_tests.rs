//! Module layering, checked on the source text (docs/architecture.md):
//! nothing outside the REST layer imports `document::handlers`, and the
//! shared wire shapes stay free of database access.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `(path relative to the crate root, contents)` of every `.rs` under `dirs`.
fn sources(dirs: &[&str]) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for d in dirs {
        rust_files(&root.join(d), &mut files);
    }
    files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(root)
                .expect("under the crate root")
                .to_string_lossy()
                .replace('\\', "/");
            (rel, fs::read_to_string(&p).expect("read source"))
        })
        .collect()
}

#[test]
fn document_handlers_are_imported_only_by_the_rest_layer() {
    let allowed = |f: &str| {
        f.starts_with("src/document/handlers/")
            || ["src/app.rs", "src/openapi.rs", "src/layering_tests.rs"].contains(&f)
    };
    let offenders: Vec<String> = sources(&["src", "tests"])
        .into_iter()
        .filter(|(f, src)| !allowed(f) && src.contains("document::handlers"))
        .map(|(f, _)| f)
        .collect();
    assert!(
        offenders.is_empty(),
        "import from document::{{model, snapshot, meta, service, wire}} instead: {offenders:?}"
    );
}

#[test]
fn document_wire_has_no_database_access() {
    let offenders: Vec<String> = sources(&["src/document/wire"])
        .into_iter()
        .filter(|(_, src)| {
            ["::repo", "sea_orm", "AppState", "DatabaseConnection"]
                .iter()
                .any(|needle| src.contains(needle))
        })
        .map(|(f, _)| f)
        .collect();
    assert!(
        offenders.is_empty(),
        "wire DTOs must stay pure: {offenders:?}"
    );
}
