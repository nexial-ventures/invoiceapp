//! Every route in `/api/openapi.json` has an explicit access decision here
//! (spaces.md, 4a) — a new route without one fails this test. Each decision
//! is then probed: a space route refuses anonymous callers (401) and every
//! role below its minimum (403 `forbidden`) and answers every role from its
//! minimum up with neither 401 nor 403; an account route refuses anonymous
//! callers; a public route does not ask for credentials; a session-only
//! route refuses a token of a sufficient role (403). The success paths per route group
//! are in `spaces_roles.rs`.

mod common;

use std::collections::BTreeSet;

use axum::Router;
use axum::http::{Method, StatusCode};
use common::auth::{As, member, on};
use common::documents::dead_url;
use common::members::{session_on, user_id};
use common::{BASE_HOST, TEST_HOST, TEST_TOKEN, TestDb, call, router_with};
use invoice::space::Role;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
enum Access {
    /// No credentials needed (auth flows, health, OpenAPI, host context).
    Public,
    /// A signed-in user on any known host, outside a space's role matrix
    /// (own account, own spaces, sign-out).
    Account,
    /// A space route with this minimum role.
    Space(Role),
    /// Authenticated, but the decision is made deeper; the reason says where.
    Exempt(&'static str),
}

use Access::{Account, Exempt, Public, Space};
use Role::{Accountant as Acc, Admin, Member as Mem, Owner};

/// (method, OpenAPI path, decision). `POST /api/space/leave` stays last: its
/// probe makes the accountant leave.
const ROUTES: &[(&str, &str, Access)] = &[
    ("GET", "/api/health", Public),
    ("GET", "/api/context", Public),
    ("POST", "/api/auth/register", Public),
    ("POST", "/api/auth/verify", Public),
    ("POST", "/api/auth/verify/resend", Public),
    ("POST", "/api/auth/login", Public),
    ("POST", "/api/auth/login/mfa", Public),
    ("POST", "/api/auth/password-reset", Public),
    ("POST", "/api/auth/password-reset/confirm", Public),
    ("GET", "/api/auth/google/start", Public),
    ("GET", "/api/auth/google/callback", Public),
    ("GET", "/api/auth/google/finish", Public),
    ("GET", "/api/invites/accept", Public),
    ("POST", "/api/invites/accept", Public),
    ("POST", "/api/auth/logout", Account),
    ("GET", "/api/auth/me", Account),
    ("POST", "/api/auth/sessions/revoke-others", Account),
    ("POST", "/api/account/password", Account),
    ("GET", "/api/account/mfa", Account),
    ("POST", "/api/account/mfa/setup", Account),
    ("POST", "/api/account/mfa/enable", Account),
    ("POST", "/api/account/mfa/disable", Account),
    ("POST", "/api/account/mfa/recovery-codes", Account),
    ("POST", "/api/account/google/link", Account),
    ("DELETE", "/api/account/google", Account),
    ("GET", "/api/spaces", Account),
    ("POST", "/api/spaces", Account),
    (
        "POST",
        "/api/mcp",
        Exempt("per tool: spaces_roles.rs mcp_tools_by_role"),
    ),
    ("GET", "/api/space", Space(Acc)),
    ("PUT", "/api/space", Space(Admin)),
    ("DELETE", "/api/space", Space(Owner)),
    ("GET", "/api/tokens", Space(Acc)),
    ("POST", "/api/tokens", Space(Acc)),
    ("DELETE", "/api/tokens/{id}", Space(Acc)),
    ("GET", "/api/members", Space(Admin)),
    ("PUT", "/api/members/{userId}", Space(Admin)),
    ("DELETE", "/api/members/{userId}", Space(Admin)),
    ("GET", "/api/invites", Space(Admin)),
    ("POST", "/api/invites", Space(Admin)),
    ("DELETE", "/api/invites/{id}", Space(Admin)),
    ("POST", "/api/invites/{id}/resend", Space(Admin)),
    ("GET", "/api/settings/company", Space(Acc)),
    ("PUT", "/api/settings/company", Space(Admin)),
    ("GET", "/api/settings/bank-accounts", Space(Acc)),
    ("POST", "/api/settings/bank-accounts", Space(Admin)),
    ("PUT", "/api/settings/bank-accounts/{id}", Space(Admin)),
    ("DELETE", "/api/settings/bank-accounts/{id}", Space(Admin)),
    ("GET", "/api/settings/vat-rates", Space(Acc)),
    ("POST", "/api/settings/vat-rates", Space(Admin)),
    ("PUT", "/api/settings/vat-rates/{id}", Space(Admin)),
    ("DELETE", "/api/settings/vat-rates/{id}", Space(Admin)),
    ("GET", "/api/settings/number-series", Space(Acc)),
    ("PUT", "/api/settings/number-series/{docType}", Space(Admin)),
    (
        "PUT",
        "/api/settings/number-series/{docType}/counters/{year}",
        Space(Admin),
    ),
    ("GET", "/api/settings/categories", Space(Acc)),
    ("POST", "/api/settings/categories", Space(Admin)),
    ("PUT", "/api/settings/categories/{id}", Space(Admin)),
    ("DELETE", "/api/settings/categories/{id}", Space(Admin)),
    ("GET", "/api/settings/custom-fields", Space(Acc)),
    ("POST", "/api/settings/custom-fields", Space(Admin)),
    ("PUT", "/api/settings/custom-fields/{id}", Space(Admin)),
    ("DELETE", "/api/settings/custom-fields/{id}", Space(Admin)),
    ("GET", "/api/settings/accounting", Space(Acc)),
    ("PUT", "/api/settings/accounting", Space(Admin)),
    ("GET", "/api/settings/email", Space(Acc)),
    ("POST", "/api/settings/email/test", Space(Admin)),
    ("GET", "/api/settings/email/templates", Space(Acc)),
    (
        "PUT",
        "/api/settings/email/templates/{locale}",
        Space(Admin),
    ),
    (
        "DELETE",
        "/api/settings/email/templates/{locale}",
        Space(Admin),
    ),
    (
        "POST",
        "/api/settings/email/templates/{locale}/preview",
        Space(Mem),
    ),
    ("GET", "/api/contacts", Space(Acc)),
    ("POST", "/api/contacts", Space(Mem)),
    ("GET", "/api/contacts/{id}", Space(Acc)),
    ("PUT", "/api/contacts/{id}", Space(Mem)),
    ("DELETE", "/api/contacts/{id}", Space(Mem)),
    ("GET", "/api/ares/{ico}", Space(Acc)),
    ("GET", "/api/exchange-rates/{currency}", Space(Acc)),
    ("GET", "/api/documents", Space(Acc)),
    ("POST", "/api/documents", Space(Mem)),
    ("POST", "/api/documents/compute", Space(Mem)),
    ("GET", "/api/documents/isdoc", Space(Acc)),
    ("GET", "/api/documents/{id}", Space(Acc)),
    ("PUT", "/api/documents/{id}", Space(Mem)),
    ("DELETE", "/api/documents/{id}", Space(Mem)),
    ("POST", "/api/documents/{id}/issue", Space(Mem)),
    ("POST", "/api/documents/{id}/cancel", Space(Mem)),
    ("POST", "/api/documents/{id}/mark-sent", Space(Mem)),
    ("POST", "/api/documents/{id}/settle", Space(Mem)),
    ("POST", "/api/documents/{id}/credit-note", Space(Mem)),
    ("POST", "/api/documents/{id}/debit-note", Space(Mem)),
    ("PUT", "/api/documents/{id}/internal-note", Space(Mem)),
    ("PUT", "/api/documents/{id}/metadata", Space(Mem)),
    ("PUT", "/api/documents/{id}/original", Space(Mem)),
    ("DELETE", "/api/documents/{id}/original", Space(Mem)),
    ("GET", "/api/documents/{id}/payments", Space(Acc)),
    ("POST", "/api/documents/{id}/payments", Space(Mem)),
    (
        "DELETE",
        "/api/documents/{id}/payments/{paymentId}",
        Space(Mem),
    ),
    ("GET", "/api/documents/{id}/pdf", Space(Acc)),
    ("GET", "/api/documents/{id}/isdoc", Space(Acc)),
    ("GET", "/api/documents/{id}/email", Space(Acc)),
    ("POST", "/api/documents/{id}/email", Space(Mem)),
    ("GET", "/api/documents/{id}/emails", Space(Acc)),
    ("GET", "/api/catalog/items", Space(Acc)),
    ("POST", "/api/catalog/items", Space(Mem)),
    ("GET", "/api/catalog/items/{id}", Space(Acc)),
    ("PUT", "/api/catalog/items/{id}", Space(Mem)),
    ("DELETE", "/api/catalog/items/{id}", Space(Mem)),
    ("GET", "/api/catalog/groups", Space(Acc)),
    ("POST", "/api/catalog/groups", Space(Mem)),
    ("GET", "/api/catalog/groups/{id}", Space(Acc)),
    ("PUT", "/api/catalog/groups/{id}", Space(Mem)),
    ("DELETE", "/api/catalog/groups/{id}", Space(Mem)),
    ("POST", "/api/import/isdoc/preview", Space(Mem)),
    ("POST", "/api/import/isdoc/confirm", Space(Mem)),
    ("GET", "/api/import/csv/sample", Space(Acc)),
    ("POST", "/api/import/csv/preview", Space(Mem)),
    ("POST", "/api/import/csv/confirm", Space(Mem)),
    ("GET", "/api/export/csv", Space(Acc)),
    ("GET", "/api/export/accountant", Space(Acc)),
    ("GET", "/api/pdf/preview", Space(Acc)),
    ("GET", "/api/pdf/design", Space(Acc)),
    ("GET", "/api/pdf/design/file", Space(Admin)),
    ("PUT", "/api/pdf/design/draft/file", Space(Admin)),
    ("DELETE", "/api/pdf/design/draft/file", Space(Admin)),
    ("DELETE", "/api/pdf/design/draft", Space(Admin)),
    ("GET", "/api/pdf/design/draft/preview", Space(Admin)),
    ("POST", "/api/pdf/design/activate", Space(Admin)),
    ("POST", "/api/pdf/design/reset", Space(Admin)),
    ("GET", "/api/pdf/design/versions", Space(Admin)),
    (
        "POST",
        "/api/pdf/design/versions/{id}/restore",
        Space(Admin),
    ),
    ("POST", "/api/space/leave", Space(Acc)),
];

/// Routes that refuse a token even of a sufficient role (`require_session()`
/// on every path): credentials, TOTP, Google link, leaving / deleting the
/// space. Each must also be in [`ROUTES`]. (`PUT /api/space` needs a session
/// only for `requireMfa`, covered in `mfa_policy.rs`; logout takes either.)
const SESSION_ONLY: &[(&str, &str)] = &[
    ("POST", "/api/account/password"),
    ("POST", "/api/auth/sessions/revoke-others"),
    ("GET", "/api/account/mfa"),
    ("POST", "/api/account/mfa/setup"),
    ("POST", "/api/account/mfa/enable"),
    ("POST", "/api/account/mfa/disable"),
    ("POST", "/api/account/mfa/recovery-codes"),
    ("POST", "/api/account/google/link"),
    ("DELETE", "/api/account/google"),
    ("DELETE", "/api/space"),
    ("POST", "/api/space/leave"),
];

/// A concrete URI for an OpenAPI path: every `{param}` gets a well-formed
/// value that matches nothing (nil ids). The bodies are empty objects, so
/// most probes stop at validation — not all: a sufficient role really runs
/// the handler (e.g. `POST /api/space/leave` makes the accountant leave,
/// `PUT /api/space` with `{}` saves nothing new). Run on a throwaway space.
fn concrete(path: &str) -> String {
    let nil = uuid::Uuid::nil().to_string();
    path.split('/')
        .map(|seg| match seg {
            "{locale}" => "cs".to_string(),
            "{ico}" => "12345679".to_string(),
            "{currency}" => "EUR".to_string(),
            "{docType}" => "invoice".to_string(),
            "{year}" => "2026".to_string(),
            s if s.starts_with('{') => nil.clone(),
            s => s.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The caller was refused for who they are (not for what they sent).
fn refused(j: &Value) -> bool {
    matches!(j["code"].as_str(), Some("unauthorized" | "forbidden"))
}

fn body(m: &Method) -> Option<Value> {
    (m != Method::GET).then(|| json!({}))
}

async fn openapi_routes(app: &Router) -> BTreeSet<(String, String)> {
    let (s, doc) = call(app, Method::GET, "/api/openapi.json", None).await;
    assert_eq!(s, StatusCode::OK);
    let mut out = BTreeSet::new();
    for (path, ops) in doc["paths"].as_object().expect("paths") {
        for method in ops.as_object().expect("operations").keys() {
            out.insert((method.to_uppercase(), path.clone()));
        }
    }
    out
}

#[test]
fn every_route_is_listed_once() {
    let mut seen = BTreeSet::new();
    for (m, p, _) in ROUTES {
        assert!(seen.insert((*m, *p)), "{m} {p} listed twice");
    }
}

#[tokio::test]
async fn every_route_has_an_access_decision_that_holds() {
    let db = TestDb::new().await;
    let app = router_with(db.conn.clone(), &dead_url(), &dead_url());

    let listed: BTreeSet<(String, String)> = ROUTES
        .iter()
        .map(|(m, p, _)| (m.to_string(), p.to_string()))
        .collect();
    let documented = openapi_routes(&app).await;
    let missing: Vec<_> = documented.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "routes without an access decision in ROUTES: {missing:?}"
    );
    let stale: Vec<_> = listed.difference(&documented).collect();
    assert!(
        stale.is_empty(),
        "ROUTES not in the OpenAPI document: {stale:?}"
    );

    // One session per role (session-only routes refuse tokens).
    let mut sessions = vec![(Owner, session_on(&db.conn, db.space, db.user_id).await)];
    for (role, email) in [
        (Acc, "acc@example.com"),
        (Mem, "mem@example.com"),
        (Admin, "adm@example.com"),
    ] {
        member(&db.conn, db.space, email, role).await;
        let id = user_id(&db.conn, email).await;
        sessions.push((role, session_on(&db.conn, db.space, id).await));
    }

    for (method, path, access) in ROUTES {
        let m = Method::from_bytes(method.as_bytes()).expect("method");
        let uri = concrete(path);
        let what = format!("{method} {path}");
        match access {
            Public => {
                let host = if path.starts_with("/api/invites/") {
                    TEST_HOST
                } else {
                    BASE_HOST
                };
                let (s, j) = on(&app, m.clone(), host, &uri, As::Nobody, body(&m)).await;
                assert!(!refused(&j), "{what}: public, got {s} {j}");
            }
            Account => {
                let (s, _) = on(&app, m.clone(), BASE_HOST, &uri, As::Nobody, body(&m)).await;
                assert_eq!(s, StatusCode::UNAUTHORIZED, "{what}: anonymous");
            }
            Exempt(why) => {
                let (s, _) = on(&app, m.clone(), TEST_HOST, &uri, As::Nobody, body(&m)).await;
                assert_eq!(s, StatusCode::UNAUTHORIZED, "{what} ({why}): anonymous");
            }
            Space(min) => {
                let (s, _) = on(&app, m.clone(), TEST_HOST, &uri, As::Nobody, body(&m)).await;
                assert_eq!(s, StatusCode::UNAUTHORIZED, "{what}: anonymous");
                for (role, cookie) in &sessions {
                    // Leaving is probed by the accountant only (the owner
                    // could not leave; an admin leaving would end its probes).
                    if *path == "/api/space/leave" && *role != Acc {
                        continue;
                    }
                    let (s, j) = on(
                        &app,
                        m.clone(),
                        TEST_HOST,
                        &uri,
                        As::Cookie(cookie),
                        body(&m),
                    )
                    .await;
                    if role < min {
                        assert_eq!(
                            (s, j),
                            (StatusCode::FORBIDDEN, json!({ "code": "forbidden" })),
                            "{what} as {role:?}"
                        );
                    } else {
                        assert!(
                            s != StatusCode::UNAUTHORIZED && s != StatusCode::FORBIDDEN,
                            "{what} as {role:?}: got {s} {j}"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn session_only_routes_refuse_tokens() {
    let decided: BTreeSet<(&str, &str)> = ROUTES.iter().map(|(m, p, _)| (*m, *p)).collect();
    let db = TestDb::new().await;
    // Google configured, else its account routes are 404 before the check.
    let (app, _oidc) = common::google::app(&db.conn, false);
    // Shaped like the real inputs, so the request reaches the session check.
    let body = json!({ "slug": "x", "password": "x", "currentPassword": "x",
                       "newPassword": "x", "code": "123456" });
    for (method, path) in SESSION_ONLY {
        assert!(
            decided.contains(&(*method, *path)),
            "{method} {path} not in ROUTES"
        );
        let m = Method::from_bytes(method.as_bytes()).expect("method");
        let b = (m != Method::GET).then(|| body.clone());
        // The owner's token: every role check passes, only the session one is left.
        let (s, j) = on(&app, m, TEST_HOST, path, As::Bearer(TEST_TOKEN), b).await;
        assert_eq!(
            (s, j),
            (StatusCode::FORBIDDEN, json!({ "code": "forbidden" })),
            "{method} {path} with a token"
        );
    }
}
