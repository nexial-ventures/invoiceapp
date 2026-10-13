//! The design draft (design.md, 4f): upload validation and quota, add /
//! replace / delete live / discard, downloads, the draft preview, roles and
//! space isolation.

mod common;

use axum::http::{Method, StatusCode};
use bytes::Bytes;
use common::auth::{As, member, on, seed_space};
use common::design::{listing, paths, template, upload, upload_as, upload_ok};
use common::mdcast::{PdfEnv, get_raw};
use common::members::{B_HOST, B_TOKEN};
use common::{TEST_HOST, TestDb, call};
use invoiceapp::space::Role;
use serde_json::{Value, json};

const MB: usize = 1024 * 1024;

fn field(err: &Value, name: &str) -> Value {
    assert_eq!(err["code"], "validation", "{err}");
    err["fields"][name].clone()
}

#[tokio::test]
async fn upload_validation_and_quota() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());

    for bad in [
        "Logo.svg",
        "a/b/c.png",
        "qr.svg",
        "logo.gif",
        "..%2Fx.png",
        ".hidden.png",
        "fonts/other.txt",
    ] {
        let (status, err) = upload(&app, bad, b"x".to_vec()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(field(&err, "path"), "invalid", "{bad}");
    }
    let (status, err) = upload(&app, "logo.png", vec![0; 10 * MB + 1]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(field(&err, "file"), "too_large");
    let (status, err) = upload(&app, "invoice.typ", vec![0xff, 0xfe]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(field(&err, "file"), "invalid");
    let at_limit = upload_ok(&app, "fonts/big.ttf", vec![1; 10 * MB]).await;
    assert_eq!(
        at_limit["quota"],
        json!({ "used": 10 * MB, "limit": 50 * MB })
    );

    // Kept versions count: 4 × 9 MB of an old version + the 10 MB draft file.
    for i in 0..4 {
        env.storage
            .storage
            .put(
                &db.key(&format!("design-versions/{}/f{i}.png", uuid::Uuid::nil())),
                Bytes::from(vec![2; 9 * MB]),
            )
            .await
            .expect("put version object");
    }
    let (status, err) = upload(&app, "logo.png", vec![3; 5 * MB]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(
        err,
        json!({ "code": "quota_exceeded", "detail": "46.0 / 50 MB" })
    );
    // Replacing a draft file only counts the difference.
    upload_ok(&app, "fonts/big.ttf", vec![4; 10 * MB]).await;
    assert_eq!(listing(&app).await["quota"]["used"], 46 * MB);
}

#[tokio::test]
async fn draft_add_replace_delete_and_discard() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    assert_eq!(listing(&app).await["draft"], Value::Null);
    env.storage
        .storage
        .put(&db.key("design/logo.svg"), Bytes::from_static(b"<svg/>"))
        .await
        .expect("live custom file");

    upload_ok(&app, "signature.png", b"sig".to_vec()).await;
    upload_ok(&app, "invoice.typ", template("draft v1")).await;
    let l = upload_ok(&app, "invoice.typ", template("draft v2")).await;
    let draft = &l["draft"];
    assert_eq!(
        draft["files"],
        json!([
            { "path": "invoice.typ", "size": template("draft v2").len(), "change": "replaced" },
            { "path": "signature.png", "size": 3, "change": "added" },
        ])
    );
    assert_eq!(draft["deleted"], json!([]));
    assert_eq!(draft["updatedBy"]["email"], common::OWNER_EMAIL);
    assert!(draft["updatedBy"]["displayName"].is_string(), "{draft}");
    assert!(draft["updatedAt"].is_string(), "{draft}");
    assert_eq!(paths(&l, "custom"), ["logo.svg"], "live untouched");

    let del = |path: &'static str| {
        let app = app.clone();
        async move {
            let uri = format!("/api/pdf/design/draft/file?path={path}");
            call(&app, Method::DELETE, &uri, None).await
        }
    };
    // A draft file goes away; a live custom file becomes a draft deletion.
    let (status, l) = del("signature.png").await;
    assert_eq!(status, StatusCode::OK, "{l}");
    let (status, l) = del("logo.svg").await;
    assert_eq!(status, StatusCode::OK, "{l}");
    assert_eq!(l["draft"]["deleted"], json!(["logo.svg"]));
    assert_eq!(l["draft"]["files"].as_array().map(Vec::len), Some(1));
    // Default-only and unknown paths are 404.
    for path in ["fonts/OFL.txt", "nope.png", "signature.png"] {
        assert_eq!(del(path).await.0, StatusCode::NOT_FOUND, "{path}");
    }
    // Uploading a deleted path takes the deletion back.
    let l = upload_ok(&app, "logo.svg", b"<svg id='2'/>".to_vec()).await;
    assert_eq!(l["draft"]["deleted"], json!([]));
    del("logo.svg").await;
    del("logo.svg").await;
    assert_eq!(listing(&app).await["draft"]["deleted"], json!(["logo.svg"]));

    let (status, body) = call(&app, Method::DELETE, "/api/pdf/design/draft", None).await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
    assert_eq!(listing(&app).await["draft"], Value::Null);
    assert!(env.storage.keys(&db.key("design-draft")).await.is_empty());
    assert!(
        env.storage
            .bytes(&db.key("design/logo.svg"))
            .await
            .is_some()
    );
}

#[tokio::test]
async fn downloads_live_draft_and_default_files() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    env.storage
        .storage
        .put(&db.key("design/logo.svg"), Bytes::from_static(b"<svg/>"))
        .await
        .expect("live custom file");
    upload_ok(&app, "fonts/brand.ttf", b"ttf".to_vec()).await;

    let file = |q: &str| format!("/api/pdf/design/file?{q}");
    let (status, headers, body) = get_raw(&app, &file("path=logo.svg&source=live")).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &b"<svg/>"[..]));
    assert_eq!(headers["content-type"], "image/svg+xml");
    assert_eq!(
        headers["content-disposition"],
        "attachment; filename=\"logo.svg\""
    );
    let (status, headers, body) = get_raw(&app, &file("path=fonts/brand.ttf&source=draft")).await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, &b"ttf"[..]));
    assert_eq!(headers["content-type"], "font/ttf");
    let (status, _, body) = get_raw(&app, &file("path=invoice.typ&source=default")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("data.json"));
    // `live` of a path without a custom file is the default one.
    let (status, _, live) = get_raw(&app, &file("path=invoice.typ")).await;
    assert_eq!((status, live), (StatusCode::OK, body));
    let (status, _, _) = get_raw(&app, &file("path=fonts/Inter-Regular.ttf&source=default")).await;
    assert_eq!(status, StatusCode::OK);

    for q in [
        "path=logo.svg&source=draft",
        "path=logo.svg&source=default",
        "path=nope.png",
        "path=../secret",
        "path=.env",
    ] {
        let (status, _, _) = get_raw(&app, &file(q)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{q}");
    }
    let (status, err) = call(&app, Method::GET, &file("path=logo.svg&source=x"), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(field(&err, "source"), "invalid");
}

#[tokio::test]
async fn draft_preview_uses_the_draft_and_live_renders_do_not() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    env.storage
        .storage
        .put(&db.key("design/logo.png"), Bytes::from_static(b"png"))
        .await
        .expect("live custom file");
    upload_ok(&app, "invoice.typ", template("draft template")).await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/pdf/design/draft/file?path=logo.png",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, headers, bytes) = get_raw(&app, "/api/pdf/design/draft/preview?locale=en").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    let render = env.mock.last();
    assert_eq!(bytes, render.pdf);
    assert!(render.template.starts_with("// draft template"));
    assert_eq!(render.data["locale"], "en");
    assert_eq!(
        render.data["assets"]["logo"],
        Value::Null,
        "deleted in the draft"
    );

    let (status, _, _) = get_raw(&app, "/api/pdf/preview").await;
    assert_eq!(status, StatusCode::OK);
    let live = env.mock.last();
    assert!(
        !live.template.starts_with("// draft"),
        "live keeps the default"
    );
    assert_eq!(live.data["assets"]["logo"], "logo.png");

    upload_ok(&app, "invoice.typ", template("mock-fail")).await;
    let (status, err) = call(&app, Method::GET, "/api/pdf/design/draft/preview", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(err["code"], "pdf_render_failed");
    assert_eq!(err["detail"], "error: mock template failure");
}

#[tokio::test]
async fn editor_routes_are_admin_only() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let admin = member(&db.conn, db.space, "admin@example.com", Role::Admin).await;
    let (status, l) = upload_as(&app, &admin, "logo.svg", b"<svg/>".to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{l}");
    assert_eq!(l["draft"]["updatedBy"]["email"], "admin@example.com");

    let worker = member(&db.conn, db.space, "member@example.com", Role::Member).await;
    let (status, _) = upload_as(&app, &worker, "logo.svg", b"<svg/>".to_vec()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    for (method, uri) in [
        (Method::GET, "/api/pdf/design/file?path=invoice.typ"),
        (Method::DELETE, "/api/pdf/design/draft/file?path=logo.svg"),
        (Method::DELETE, "/api/pdf/design/draft"),
        (Method::GET, "/api/pdf/design/draft/preview"),
        (Method::POST, "/api/pdf/design/activate"),
        (Method::GET, "/api/pdf/design/versions"),
        (
            Method::POST,
            "/api/pdf/design/versions/00000000-0000-0000-0000-000000000000/restore",
        ),
        (Method::POST, "/api/pdf/design/reset"),
    ] {
        let (status, _) = on(
            &app,
            method.clone(),
            TEST_HOST,
            uri,
            As::Bearer(&worker),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    // The listing stays readable by every role (1d, spaces.md).
    let accountant = member(&db.conn, db.space, "acc@example.com", Role::Accountant).await;
    let (status, l) = on(
        &app,
        Method::GET,
        TEST_HOST,
        "/api/pdf/design",
        As::Bearer(&accountant),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(l["draft"]["files"][0]["path"], "logo.svg");
}

#[tokio::test]
async fn design_keys_stay_in_their_space() {
    let db = TestDb::new().await;
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    upload_ok(&app, "logo.svg", b"<svg/>".to_vec()).await;
    assert_eq!(
        env.storage.keys(&db.key("design-draft")).await,
        [db.key("design-draft/logo.svg")]
    );

    let as_b = |method: Method, uri: &'static str| {
        let app = app.clone();
        async move { on(&app, method, B_HOST, uri, As::Bearer(B_TOKEN), None).await }
    };
    let (status, l) = as_b(Method::GET, "/api/pdf/design").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (l["draft"].clone(), l["quota"]["used"].clone()),
        (Value::Null, json!(0))
    );
    let (status, _) = as_b(
        Method::GET,
        "/api/pdf/design/file?path=logo.svg&source=draft",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, err) = as_b(Method::POST, "/api/pdf/design/activate").await;
    assert_eq!(
        (status, err["code"].clone()),
        (StatusCode::UNPROCESSABLE_ENTITY, json!("draft_empty"))
    );
    let (status, _) = as_b(Method::DELETE, "/api/pdf/design/draft").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(listing(&app).await["draft"]["files"][0]["path"], "logo.svg");
    let b_prefix = beta.storage_prefix();
    assert!(env.storage.keys(&b_prefix).await.is_empty());
}
