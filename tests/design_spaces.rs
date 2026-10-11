//! Two spaces render with their own live design (#10): template and logo
//! come from each space's `design/`, and the shared design cache never hands
//! one space's files to another — on both storage backends.

mod common;

use axum::http::{Method, StatusCode};
use bytes::Bytes;
use common::auth::{As, on, seed_space};
use common::design::template;
use common::documents::item;
use common::mdcast::{PdfEnv, Rendered};
use common::members::{B_HOST, B_TOKEN};
use common::storage::TestStorage;
use common::{TEST_HOST, TEST_TOKEN, TestDb};
use serde_json::{Value, json};

/// One space as the test addresses it.
struct Space {
    host: &'static str,
    token: &'static str,
    /// The tag in its template (`// {marker}`) and in its logo bytes.
    marker: &'static str,
}

const ALPHA: Space = Space {
    host: TEST_HOST,
    token: TEST_TOKEN,
    marker: "alpha",
};
const BETA: Space = Space {
    host: B_HOST,
    token: B_TOKEN,
    marker: "beta",
};

async fn ok(
    app: &axum::Router,
    s: &Space,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> Value {
    let (status, json) = on(app, method, s.host, uri, As::Bearer(s.token), body).await;
    assert!(status.is_success(), "{} {uri}: {status} {json}", s.marker);
    json
}

/// `GET /api/pdf/preview` in `s`; what mdcast received.
async fn preview(app: &axum::Router, env: &PdfEnv, s: &Space) -> Rendered {
    let (status, _) = on(
        app,
        Method::GET,
        s.host,
        "/api/pdf/preview",
        As::Bearer(s.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{} preview", s.marker);
    env.mock.last()
}

/// Issue the same invoice in `s`; what mdcast received.
async fn issue(app: &axum::Router, env: &PdfEnv, s: &Space) -> Rendered {
    let company = json!({
        "name": "Dodavatel s.r.o.", "ico": "44444443", "dic": "CZ44444443", "vatPayer": true,
        "street": "Hlavní 1", "city": "Praha", "zip": "11000", "country": "CZ",
        "defaultDueDays": 14, "defaultLocale": "cs"
    });
    ok(app, s, Method::PUT, "/api/settings/company", Some(company)).await;
    let contact = ok(
        app,
        s,
        Method::POST,
        "/api/contacts",
        Some(json!({ "name": "Odběratel a.s.", "ico": "12345679", "city": "Brno" })),
    )
    .await;
    ok(
        app,
        s,
        Method::POST,
        "/api/settings/bank-accounts",
        Some(json!({ "currency": "CZK", "accountNumber": "19-2000145399/0800" })),
    )
    .await;
    let doc = ok(
        app,
        s,
        Method::POST,
        "/api/documents",
        Some(
            json!({ "contactId": contact["id"], "issueDate": "2026-10-01",
                     "lines": [item("10", "100", "21")] }),
        ),
    )
    .await;
    let uri = format!("/api/documents/{}/issue", doc["id"].as_str().expect("id"));
    let before = env.mock.count();
    ok(app, s, Method::POST, &uri, None).await;
    assert_eq!(env.mock.count(), before + 1, "{} issue rendered", s.marker);
    env.mock.last()
}

/// `r` used `s`'s template and logo (`logo` = the digest of its logo).
fn assert_rendered_with(r: &Rendered, s: &Space, logo: &str) {
    assert!(
        r.template.starts_with(&format!("// {}\n", s.marker)),
        "{} got template {:?}",
        s.marker,
        r.template.lines().next()
    );
    assert_eq!(r.data["assets"]["logo"], "logo.png", "{}", s.marker);
    assert_eq!(r.digests.get("logo.png").map(String::as_str), Some(logo));
}

#[tokio::test]
async fn spaces_render_own_design_fs() {
    spaces_render_own_design(TestStorage::fs()).await;
}

#[tokio::test]
async fn spaces_render_own_design_s3() {
    spaces_render_own_design(TestStorage::s3()).await;
}

async fn spaces_render_own_design(storage: TestStorage) {
    let db = TestDb::new().await;
    let (beta, _) = seed_space(&db.conn, "beta", "beta@example.com", Some(B_TOKEN)).await;
    let env = PdfEnv::with_storage(storage);
    let (app, pdf) = env.router_and_pdf(db.conn.clone());
    let put = |key: String, bytes: Vec<u8>| {
        let storage = env.storage.storage.clone();
        async move {
            storage
                .put(&key, Bytes::from(bytes))
                .await
                .expect("live design file");
        }
    };
    let beta_key = |rel: &str| format!("{}/{rel}", beta.storage_prefix());
    put(db.key("design/invoice.typ"), template("alpha")).await;
    put(db.key("design/logo.png"), b"alpha png".to_vec()).await;
    put(beta_key("design/invoice.typ"), template("beta")).await;
    put(beta_key("design/logo.png"), b"beta png".to_vec()).await;

    // The digest of each logo, learned from the first render of each space.
    let a = preview(&app, &env, &ALPHA).await;
    let alpha_logo = a.digests.get("logo.png").expect("alpha logo").clone();
    let b = preview(&app, &env, &BETA).await;
    let beta_logo = b.digests.get("logo.png").expect("beta logo").clone();
    assert_ne!(alpha_logo, beta_logo, "different logo bytes");
    assert_ne!(a.template, b.template);
    assert_rendered_with(&a, &ALPHA, &alpha_logo);
    assert_rendered_with(&b, &BETA, &beta_logo);
    // Template + logo of each space under its own cache key: a key shared
    // across spaces would hold only 2.
    let cache = pdf.design_cache();
    assert_eq!(cache.len(), 4, "one cache entry per space and file");

    // Alpha → beta → alpha again, with both designs cached by now.
    for (s, logo) in [
        (&ALPHA, &alpha_logo),
        (&BETA, &beta_logo),
        (&ALPHA, &alpha_logo),
    ] {
        assert_rendered_with(&preview(&app, &env, s).await, s, logo);
        assert_eq!(
            cache.len(),
            4,
            "{} render kept both spaces cached",
            s.marker
        );
    }

    // The same invoice issued in both spaces: same data, own design.
    let ia = issue(&app, &env, &ALPHA).await;
    let ib = issue(&app, &env, &BETA).await;
    assert_eq!(ia.data["draft"], false);
    assert_eq!(ib.data["draft"], false);
    assert!(ia.data["number"].is_string(), "{}", ia.data["number"]);
    assert_eq!(ia.data["number"], ib.data["number"]);
    assert_eq!(ia.data["lines"], ib.data["lines"]);
    assert_eq!(ia.data["totals"], ib.data["totals"]);
    assert_rendered_with(&ia, &ALPHA, &alpha_logo);
    assert_rendered_with(&ib, &BETA, &beta_logo);
    assert_rendered_with(&preview(&app, &env, &ALPHA).await, &ALPHA, &alpha_logo);
}
