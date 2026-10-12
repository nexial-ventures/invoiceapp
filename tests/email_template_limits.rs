//! E-mail template limits (email.md, Render limits): templates render in a
//! child process (`invoice render-template`) with memory / CPU / wall-clock
//! limits plus fuel, recursion and output caps. A runaway template is 422
//! `template_invalid` with a readable detail, nothing is stored and the
//! server keeps serving; normal templates and the prefill are unchanged.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::documents::{create_issued, id};
use common::smtp::{EmailEnv, issuable_with_emails};
use common::{TestDb, call};
use invoice::email::sandbox::TemplateSandbox;
use invoice::email::templates::Template;
use invoice::error::AppError;
use serde_json::json;

const SAVE: &str = "/api/settings/email/templates/cs";
const PREVIEW: &str = "/api/settings/email/templates/cs/preview";
const KILLED: &str = "the template needs too much memory or time";

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
        // 100 MB doubled three times: 800 MB, past the child's memory limit.
        (
            "{% set s = 'x' * 100000000 %}{% set s = s ~ s %}{% set s = s ~ s %}\
             {% set s = s ~ s %}{{ s | length }}",
            KILLED,
        ),
        ("{{ 'x' | indent(width=1000000000) }}", KILLED),
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
    // Still serving, nothing stored.
    let (status, list) = call(&app, Method::GET, "/api/settings/email/templates", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        list["templates"]
            .as_array()
            .expect("templates")
            .iter()
            .all(|t| t["custom"] == false),
        "{list}"
    );

    // An ordinary loop saves, and the prefill renders it.
    let ok = json!({
        "subject": "{{ doc.number }}",
        "body": "{% for i in range(3) %}{{ i }} {% endfor %}{{ company.name }}"
    });
    let (status, saved) = call(&app, Method::PUT, SAVE, Some(ok)).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let doc = create_issued(&app, issuable_with_emails(&app).await).await;
    let uri = format!("/api/documents/{}/email?locale=cs", id(&doc));
    let (status, prefill) = call(&app, Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{prefill}");
    assert!(
        prefill["body"]
            .as_str()
            .is_some_and(|b| b.starts_with("0 1 2 ")),
        "{prefill}"
    );
}

#[tokio::test]
async fn template_checks_per_space_and_hour() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let input = json!({ "subject": "{{ doc.number }}", "body": "x" });
    for n in 0..60 {
        let (status, e) = call(&app, Method::POST, PREVIEW, Some(input.clone())).await;
        assert_eq!(status, StatusCode::OK, "{n}: {e}");
    }
    let (status, e) = call(&app, Method::PUT, SAVE, Some(input)).await;
    assert_eq!(
        (status, &e["code"]),
        (StatusCode::TOO_MANY_REQUESTS, &json!("rate_limited"))
    );
}

#[tokio::test]
async fn a_slow_child_is_killed() {
    let sandbox =
        TemplateSandbox::new(env!("CARGO_BIN_EXE_invoice")).with_timeout(Duration::from_millis(1));
    let t = Template {
        subject: "ok".into(),
        body: "x".into(),
    };
    let ctx = minijinja::Value::from_serialize(json!({}));
    match sandbox.validate(&t, &[("paid", ctx.clone())]).await {
        Err(AppError::TemplateInvalid { detail, .. }) => {
            assert!(
                detail.ends_with("rendering took longer than 0 s"),
                "{detail}"
            )
        }
        other => panic!("{other:?}"),
    }
    let sandbox = TemplateSandbox::new(env!("CARGO_BIN_EXE_invoice"));
    let r = sandbox.render(&t, &ctx).await.expect("renders");
    assert_eq!((r.subject.as_str(), r.body.as_str()), ("ok", "x"));
}
