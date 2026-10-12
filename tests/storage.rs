//! The shared storage suite: every case runs against `fs` (a temp dir) and
//! `s3` (a random prefix in the `TEST_S3_*` bucket).

mod common;

use bytes::Bytes;
use common::storage::TestStorage;
use futures_util::TryStreamExt as _;
use invoice::error::AppError;
use invoice::pdf::design::{self, Cache, MAX_FILE_BYTES, Source};
use invoice::storage::transfer::{self, Report};
use invoice::storage::{Error, sha256_hex};

macro_rules! suite {
    ($($case:ident),* $(,)?) => {
        mod fs {
            $(#[tokio::test]
            async fn $case() {
                super::$case(super::TestStorage::fs()).await;
            })*
        }
        mod s3 {
            $(#[tokio::test]
            async fn $case() {
                super::$case(super::TestStorage::s3()).await;
            })*
        }
    };
}

suite!(
    put_get_overwrite_delete,
    list_by_prefix,
    design_overlays_the_default,
    design_cache_downloads_only_changes,
    oversized_design_file_fails_the_render,
    design_layers_and_invalidation,
    migrate_is_verified_and_idempotent,
    design_push_and_pull,
);

async fn put_get_overwrite_delete(t: TestStorage) {
    let s = &t.storage;
    let key = "documents/2026/a.pdf";
    assert!(!s.exists(key).await.expect("exists"));
    assert!(matches!(s.get(key).await, Err(Error::NotFound(_))));
    assert!(s.head(key).await.expect("head").is_none());

    let sha = s
        .put(key, Bytes::from_static(b"%PDF-1"))
        .await
        .expect("put");
    assert_eq!(sha, sha256_hex(b"%PDF-1"));
    let sha = s
        .put(key, Bytes::from_static(b"%PDF-22"))
        .await
        .expect("overwrite");
    assert_eq!(sha, sha256_hex(b"%PDF-22"));
    assert!(s.exists(key).await.expect("exists"));
    assert_eq!(s.get(key).await.expect("get").as_ref(), b"%PDF-22");
    let head = s.head(key).await.expect("head").expect("present");
    assert_eq!((head.key.as_str(), head.version.size), (key, 7));

    let download = s.get_stream(key).await.expect("stream");
    assert_eq!(download.size, 7);
    let chunks: Vec<Bytes> = download.stream.try_collect().await.expect("body");
    assert_eq!(chunks.concat(), b"%PDF-22");
    assert!(matches!(
        s.get_stream("documents/2026/missing.pdf").await,
        Err(Error::NotFound(_))
    ));

    s.delete(key).await.expect("delete");
    s.delete(key).await.expect("delete is idempotent");
    assert!(!s.exists(key).await.expect("exists"));
    assert!(matches!(
        s.put("../x", Bytes::new()).await,
        Err(Error::InvalidKey(_))
    ));
    if t.kind() == "fs" {
        let leftovers = std::fs::read_dir(t.path().join("documents/2026"))
            .expect("dir")
            .count();
        assert_eq!(leftovers, 0, "no temp files left behind");
    }
}

async fn list_by_prefix(t: TestStorage) {
    let s = &t.storage;
    for key in [
        "design/logo.png",
        "design/fonts/b.ttf",
        "design/fonts/a.ttf",
        "designer/x",
        "documents/2026/1.pdf",
    ] {
        s.put(key, Bytes::from(key.as_bytes().to_vec()))
            .await
            .expect("put");
    }
    assert_eq!(
        t.keys("design").await,
        [
            "design/fonts/a.ttf",
            "design/fonts/b.ttf",
            "design/logo.png"
        ]
    );
    let listed = s.list("documents").await.expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].version.size, "documents/2026/1.pdf".len() as u64);
    assert!(s.list("nothing-here").await.expect("empty").is_empty());
}

async fn design_overlays_the_default(t: TestStorage) {
    let s = &t.storage;
    let default = design::load(s, &Cache::default(), "a")
        .await
        .expect("default");
    assert!(default.template.contains("/data.json"));
    assert_eq!(default.logo(), None);
    let listed = design::list(s).await.expect("list");
    assert!(listed.iter().all(|f| f.source == Source::Default));

    for (key, body) in [
        ("design/invoice.typ", "custom /data.json"),
        ("design/logo.png", "png"),
        ("design/logo.svg", "<svg/>"),
        ("design/signature.png", "sig"),
        ("design/qr.svg", "reserved"),
        ("design/.hidden", "x"),
        ("design/.git/config", "x"),
        ("design/fonts/extra/Brand.OTF", "otf"),
        ("design/parts.typ", "#let x = 1"),
    ] {
        s.put(key, Bytes::from(body)).await.expect("put");
    }
    let d = design::load(s, &Cache::default(), "a")
        .await
        .expect("design");
    assert_eq!(d.template, "custom /data.json");
    assert_eq!(d.logo().as_deref(), Some("logo.svg"));
    assert_eq!(d.signature().as_deref(), Some("signature.png"));
    assert!(!d.files.contains_key("qr.svg"));
    assert!(!d.files.keys().any(|k| k.starts_with('.')));
    assert!(d.fonts().contains(&"fonts/extra/Brand.OTF".to_string()));
    assert!(d.fonts().contains(&"fonts/Inter-Regular.ttf".to_string()));
    assert_eq!(d.files["parts.typ"].as_ref(), b"#let x = 1");

    let listed = design::list(s).await.expect("list");
    let find = |p: &str| listed.iter().find(|f| f.path == p).map(|f| f.source);
    assert_eq!(find("invoice.typ"), Some(Source::Custom));
    assert_eq!(find("fonts/OFL.txt"), Some(Source::Default));
    assert_eq!(find(".hidden"), None);
    let paths: Vec<&str> = listed.iter().map(|f| f.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

async fn design_cache_downloads_only_changes(t: TestStorage) {
    let s = &t.storage;
    // Listed on every load: the files below change behind the cache.
    let cache = Cache::default().with_listing_ttl(std::time::Duration::ZERO);
    s.put("design/logo.png", Bytes::from_static(b"v1"))
        .await
        .expect("put");
    s.put("design/fonts/Brand.ttf", Bytes::from_static(b"ttf"))
        .await
        .expect("put");
    let first = design::load(s, &cache, "a").await.expect("load");
    assert_eq!(cache.len(), 2, "only overrides are cached");
    // Another space shares the cache but never evicts this one's entries.
    let empty = TestStorage::fs();
    design::load(&empty.storage, &cache, "b")
        .await
        .expect("load b");
    assert_eq!(cache.len(), 2, "b has no overrides, a's stay");
    let second = design::load(s, &cache, "a").await.expect("load");
    // Same allocation → served from the cache, not downloaded again.
    assert_eq!(
        first.files["fonts/Brand.ttf"].as_ptr(),
        second.files["fonts/Brand.ttf"].as_ptr()
    );

    s.put("design/logo.png", Bytes::from_static(b"v2-longer"))
        .await
        .expect("replace");
    let third = design::load(s, &cache, "a").await.expect("load");
    assert_eq!(third.files["logo.png"].as_ref(), b"v2-longer");
    assert_eq!(
        second.files["fonts/Brand.ttf"].as_ptr(),
        third.files["fonts/Brand.ttf"].as_ptr()
    );

    s.delete("design/logo.png").await.expect("delete");
    let fourth = design::load(s, &cache, "a").await.expect("load");
    assert_eq!(fourth.logo(), None);
    assert_eq!(cache.len(), 1, "removed keys leave the cache");
}

async fn oversized_design_file_fails_the_render(t: TestStorage) {
    let s = &t.storage;
    let big = Bytes::from(vec![0u8; (MAX_FILE_BYTES + 1) as usize]);
    s.put("design/logo.png", big).await.expect("put");
    let cache = Cache::default();
    assert!(matches!(
        design::load(s, &cache, "a").await,
        Err(AppError::PdfRenderFailed(m)) if m.contains("logo.png")
    ));
    assert!(cache.is_empty(), "checked before any download");
}

async fn design_layers_and_invalidation(t: TestStorage) {
    let s = &t.storage;
    for (key, body) in [
        ("design/logo.png", "live"),
        ("design/signature.png", "sig"),
        ("design-draft/logo.png", "draft"),
        ("design-draft/parts.typ", "#let x = 2"),
    ] {
        s.put(key, Bytes::from(body)).await.expect("put");
    }
    let cache = Cache::default();
    design::load(s, &cache, "a").await.expect("live");
    assert_eq!(cache.len(), 2);
    let deleted = vec!["signature.png".to_string()];
    let layers = design::Layers {
        base: "design",
        deleted: &deleted,
        top: Some("design-draft"),
    };
    let d = design::load_layers(s, &cache, "a", layers)
        .await
        .expect("draft");
    assert_eq!(d.files["logo.png"].as_ref(), b"draft");
    assert!(d.files.contains_key("parts.typ"));
    assert_eq!(d.signature(), None, "deleted in the draft");
    assert_eq!(cache.len(), 2, "draft files are not cached");
    cache.invalidate("b");
    assert_eq!(cache.len(), 2, "other spaces untouched");
    cache.invalidate("a");
    assert!(cache.is_empty());

    // Server-side copies are independent of their source afterwards.
    s.copy("design-draft/logo.png", "design/logo.png")
        .await
        .expect("copy");
    s.put("design-draft/logo.png", Bytes::from_static(b"changed"))
        .await
        .expect("overwrite source");
    assert_eq!(
        s.get("design/logo.png").await.expect("get").as_ref(),
        b"draft"
    );
    assert_eq!(
        s.sha256("design/logo.png").await.expect("sha"),
        sha256_hex(b"draft")
    );
    assert!(matches!(
        s.copy("design-draft/none.png", "design/x.png").await,
        Err(Error::NotFound(_))
    ));
}

fn write(root: &std::path::Path, rel: &str, body: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

async fn migrate_is_verified_and_idempotent(t: TestStorage) {
    let s = &t.storage;
    let old = tempfile::tempdir().expect("old storage dir");
    write(old.path(), "documents/2025/a.pdf", b"%PDF-a");
    write(
        old.path(),
        "documents/2026/b-original-12345678.pdf",
        b"%PDF-b",
    );
    write(old.path(), "documents/2026/.0b1c.tmp", b"partial");
    let design_dir = tempfile::tempdir().expect("old design dir");
    write(design_dir.path(), "logo.png", b"png");
    write(design_dir.path(), "fonts/Brand.ttf", b"ttf");
    write(design_dir.path(), ".git/HEAD", b"x");

    let report = transfer::migrate(s, old.path(), Some(design_dir.path()))
        .await
        .expect("migrate");
    assert_eq!(
        report.copied,
        [
            "documents/2025/a.pdf",
            "documents/2026/b-original-12345678.pdf",
            "design/fonts/Brand.ttf",
            "design/logo.png",
        ]
    );
    assert!(report.problems.is_empty(), "{report:?}");
    assert_eq!(
        t.bytes("documents/2025/a.pdf").await.as_deref(),
        Some(&b"%PDF-a"[..])
    );
    assert_eq!(t.bytes("documents/2026/.0b1c.tmp").await, None);
    assert_eq!(
        t.bytes("design/logo.png").await.as_deref(),
        Some(&b"png"[..])
    );

    // Re-run: nothing copied. A stored object with other content is
    // reported and kept; the source is never touched.
    s.put("documents/2025/a.pdf", Bytes::from_static(b"%PDF-other"))
        .await
        .expect("diverge");
    let again = transfer::migrate(s, old.path(), Some(design_dir.path()))
        .await
        .expect("migrate again");
    assert!(again.copied.is_empty(), "{again:?}");
    assert_eq!(again.identical.len(), 3);
    assert_eq!(again.problems.len(), 1);
    assert!(again.problems[0].starts_with("documents/2025/a.pdf"));
    assert_eq!(
        t.bytes("documents/2025/a.pdf").await.as_deref(),
        Some(&b"%PDF-other"[..])
    );
    assert!(old.path().join("documents/2025/a.pdf").is_file());
    #[cfg(unix)]
    {
        // An unreadable entry is reported, the rest still compared.
        std::os::unix::fs::symlink(old.path().join("gone"), old.path().join("dangling"))
            .expect("symlink");
        let skipped = transfer::migrate(s, old.path(), None)
            .await
            .expect("migrate");
        assert_eq!(skipped.identical.len(), 1, "{skipped:?}");
        assert!(
            skipped
                .problems
                .iter()
                .any(|p| p.starts_with("skipped") && p.contains("dangling")),
            "{skipped:?}"
        );
    }
    assert!(
        transfer::migrate(s, &old.path().join("missing"), None)
            .await
            .is_err()
    );
}

async fn design_push_and_pull(t: TestStorage) {
    let s = &t.storage;
    let src = tempfile::tempdir().expect("design dir");
    write(src.path(), "invoice.typ", b"custom");
    write(src.path(), "fonts/Brand.ttf", b"ttf");
    let mut report = Report::default();
    transfer::copy_tree(s, src.path(), Some("design"), true, &mut report)
        .await
        .expect("push");
    assert_eq!(report.copied.len(), 2);

    // Push again with a changed file: replaced, the other one identical.
    write(src.path(), "invoice.typ", b"custom v2");
    let mut report = Report::default();
    transfer::copy_tree(s, src.path(), Some("design"), true, &mut report)
        .await
        .expect("push again");
    assert_eq!(
        (report.copied, report.identical),
        (
            vec!["design/invoice.typ".to_string()],
            vec!["design/fonts/Brand.ttf".to_string()]
        )
    );

    let dest = tempfile::tempdir().expect("pull dir");
    let pulled = transfer::pull(s, "design", dest.path())
        .await
        .expect("pull");
    assert_eq!(pulled, ["fonts/Brand.ttf", "invoice.typ"]);
    assert_eq!(
        std::fs::read(dest.path().join("invoice.typ")).expect("pulled"),
        b"custom v2"
    );
    assert_eq!(
        std::fs::read(dest.path().join("fonts/Brand.ttf")).expect("pulled"),
        b"ttf"
    );

    // rm: a stored file goes, a missing one (or a built-in default) is reported.
    assert!(design::remove(s, "invoice.typ").await.expect("rm"));
    assert!(!design::remove(s, "invoice.typ").await.expect("rm again"));
    assert!(
        !design::remove(s, "fonts/Inter-Regular.ttf")
            .await
            .expect("rm default")
    );
    assert_eq!(t.keys("design").await, ["design/fonts/Brand.ttf"]);
}
