//! Module layering, checked on the source text (docs/architecture.md):
//! `document::handlers` is used only by the router (`document/mod.rs`) and
//! `openapi.rs`, and the shared wire shapes stay free of database access.

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

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The code of `src` without `//` comments, as one line with whitespace
/// collapsed and dropped around `::`, braces and commas, so a path split
/// over lines (`document::{\n    handlers::x,`) reads as written in one.
fn code(src: &str) -> String {
    let joined: String = src
        .lines()
        .map(|l| l.find("//").map_or(l, |i| &l[..i]))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    for p in ["::", "{", "}", ","] {
        out = out
            .replace(&format!(" {p}"), p)
            .replace(&format!("{p} "), p);
    }
    out
}

/// Whether `needle` occurs in `hay` as whole identifiers.
fn has_path(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// Whether a `document::{…}` group names `handlers` at its top level.
fn grouped_handlers(code: &str) -> bool {
    code.match_indices("document::{").any(|(i, m)| {
        let group = &code[i + m.len()..];
        let mut depth = 1;
        let mut item_start = true;
        for (j, c) in group.char_indices() {
            if depth == 1 && item_start {
                let rest = &group[j..];
                let next = rest.get("handlers".len()..).and_then(|r| r.chars().next());
                if rest.starts_with("handlers") && !next.is_some_and(is_ident) {
                    return true;
                }
            }
            item_start = false;
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                ',' if depth == 1 => item_start = true,
                _ => {}
            }
        }
        false
    })
}

/// Files that reach `document::handlers` though they may not.
fn handler_offenders(files: Vec<(String, String)>) -> Vec<String> {
    let allowed = |f: &str| {
        f.starts_with("src/document/handlers/")
            || [
                "src/document/mod.rs",
                "src/openapi.rs",
                "src/layering_tests.rs",
            ]
            .contains(&f)
    };
    files
        .into_iter()
        .filter(|(f, src)| {
            let c = code(src);
            !allowed(f)
                && (has_path(&c, "document::handlers")
                    || grouped_handlers(&c)
                    || (f.starts_with("src/document/") && has_path(&c, "super::handlers")))
        })
        .map(|(f, _)| f)
        .collect()
}

#[test]
fn document_handlers_are_used_only_by_the_router_and_openapi() {
    let offenders = handler_offenders(sources(&["src", "tests"]));
    assert!(
        offenders.is_empty(),
        "import from document::{{model, snapshot, meta, service, wire}} instead: {offenders:?}"
    );
}

#[test]
fn handler_check_catches_every_spelling() {
    let file = |path: &str, src: &str| vec![(path.to_string(), src.to_string())];
    for (path, src) in [
        ("src/pdf/x.rs", "use crate::document::handlers::dto::Line;"),
        (
            "src/pdf/x.rs",
            "use crate::document::{\n    handlers::dto,\n    wire,\n};",
        ),
        ("src/pdf/x.rs", "use crate::document::{wire, handlers};"),
        (
            "src/pdf/x.rs",
            "let d = crate::document::handlers::documents::get;",
        ),
        ("src/document/service.rs", "use super::handlers::dto;"),
        ("src/document/repo/x.rs", "use super::super::handlers;"),
    ] {
        assert_eq!(handler_offenders(file(path, src)), [path], "{src}");
    }
    for (path, src) in [
        ("src/pdf/x.rs", "// see document::handlers::dto"),
        (
            "src/pdf/x.rs",
            "use crate::document::{wire::dto, repo::handlers_x};",
        ),
        (
            "src/pdf/x.rs",
            "use crate::document::{repo::{handlers}, wire};",
        ),
        ("src/document/mod.rs", "use handlers::{actions, documents};"),
        (
            "src/openapi.rs",
            "crate::document::handlers::documents::list,",
        ),
        (
            "src/document/handlers/actions.rs",
            "use super::dto::CancelInput;",
        ),
    ] {
        assert!(handler_offenders(file(path, src)).is_empty(), "{src}");
    }
}

/// Wire files (fixtures in `*_tests.rs` aside) that reach the database.
fn wire_offenders(files: Vec<(String, String)>) -> Vec<String> {
    const DB: [&str; 7] = [
        "repo",
        "entity",
        "sea_orm",
        "AppState",
        "DatabaseConnection",
        "DatabaseTransaction",
        "ConnectionTrait",
    ];
    files
        .into_iter()
        .filter(|(f, src)| {
            let c = code(src);
            !f.ends_with("_tests.rs") && DB.iter().any(|t| has_path(&c, t))
        })
        .map(|(f, _)| f)
        .collect()
}

#[test]
fn document_wire_has_no_database_access() {
    let offenders = wire_offenders(sources(&["src/document/wire"]));
    assert!(
        offenders.is_empty(),
        "wire DTOs must stay pure: {offenders:?}"
    );
}

#[test]
fn wire_check_ignores_comments_and_fixtures() {
    let file = |path: &str, src: &str| vec![(path.to_string(), src.to_string())];
    assert!(wire_offenders(file("src/document/wire/a.rs", "/// checked by the repo")).is_empty());
    assert!(
        wire_offenders(file(
            "src/document/wire/a_tests.rs",
            "use crate::x::entity;"
        ))
        .is_empty()
    );
    assert!(!wire_offenders(file("src/document/wire/a.rs", "use crate::x::entity::y;")).is_empty());
    assert!(!wire_offenders(file("src/document/wire/a.rs", "use sea_orm::Set;")).is_empty());
}
