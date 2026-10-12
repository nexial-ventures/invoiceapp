//! E-mail template render limits (email.md): a template that loops too long
//! or writes too much is refused on save and preview (422 `template_invalid`
//! with a readable detail) and nothing is stored; a normal one still saves.

mod common;

use axum::http::{Method, StatusCode};
use common::smtp::EmailEnv;
use common::{TestDb, call};
use serde_json::json;

const SAVE: &str = "/api/settings/email/templates/cs";
const PREVIEW: &str = "/api/settings/email/templates/cs/preview";

#[tokio::test]
async fn runaway_templates_are_refused() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());

    let cases = [
        (
            "{% for a in range(1000) %}{% for b in range(1000) %}.{% endfor %}{% endfor %}",
            "line 1: too much work (more than 100000 steps)",
        ),
        (
            "{% for i in range(100) %}{{ 'x' * 20000 }}{% endfor %}",
            "the output is larger than 1024 KB",
        ),
    ];
    for (body, detail) in cases {
        let input = json!({ "subject": "{{ doc.number }}", "body": body });
        for (method, uri) in [(Method::PUT, SAVE), (Method::POST, PREVIEW)] {
            let (status, e) = call(&app, method, uri, Some(input.clone())).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
            assert_eq!(
                e,
                json!({
                    "code": "template_invalid",
                    "fields": { "body": "template_invalid" },
                    "detail": detail
                })
            );
        }
    }
    let (_, list) = call(&app, Method::GET, "/api/settings/email/templates", None).await;
    assert!(
        list["templates"]
            .as_array()
            .expect("templates")
            .iter()
            .all(|t| t["custom"] == false),
        "{list}"
    );

    // A loop of ordinary size is fine.
    let ok = json!({
        "subject": "{{ doc.number }}",
        "body": "{% for i in range(50) %}{{ i }} {% endfor %}{{ company.name }}"
    });
    let (status, saved) = call(&app, Method::PUT, SAVE, Some(ok)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
}
