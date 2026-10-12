//! Settings → E-mail: status, test message, templates (save / validate /
//! preview / restore) against the storage, and their use in the prefill.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_issued, id};
use common::smtp::{COMPANY_EMAIL, CONTACT_EMAIL, EmailEnv, FROM, issuable_with_emails};
use common::storage::dead_s3;
use common::{TestDb, call};
use serde_json::{Value, json};

#[tokio::test]
async fn status_and_test_message() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let off = env.unconfigured(db.conn.clone());

    let (status, s) = call(&off, Method::GET, "/api/settings/email", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        s,
        json!({ "configured": false, "disabled": false, "from": null, "replyTo": null })
    );
    let (status, e) = call(&off, Method::POST, "/api/settings/email/test", None).await;
    assert_eq!(
        (status, &e["code"]),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            &json!("smtp_not_configured")
        )
    );

    // No address and no company e-mail yet.
    let (status, e) = call(&app, Method::POST, "/api/settings/email/test", None).await;
    assert_eq!(
        (status, &e["fields"]["to"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("required"))
    );
    let (status, e) = call(
        &app,
        Method::POST,
        "/api/settings/email/test",
        Some(json!({ "to": "nope" })),
    )
    .await;
    assert_eq!(
        (status, &e["fields"]["to"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid"))
    );

    issuable_with_emails(&app).await;
    let (_, s) = call(&app, Method::GET, "/api/settings/email", None).await;
    assert_eq!(
        s,
        json!({ "configured": true, "disabled": false, "from": FROM, "replyTo": COMPANY_EMAIL })
    );

    let (status, _) = call(&app, Method::POST, "/api/settings/email/test", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let mail = env.smtp.last();
    assert_eq!(mail.rcpt, [format!("<{COMPANY_EMAIL}>")]);
    assert!(mail.header("Subject").is_some());
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/settings/email/test",
        Some(json!({ "to": CONTACT_EMAIL })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(env.smtp.last().rcpt, [format!("<{CONTACT_EMAIL}>")]);

    env.smtp.reject_with("554 5.7.1 no thanks");
    let (status, e) = call(&app, Method::POST, "/api/settings/email/test", None).await;
    assert_eq!(
        (status, &e["code"]),
        (StatusCode::BAD_GATEWAY, &json!("smtp_failed"))
    );
    assert!(
        e["detail"].as_str().expect("detail").contains("no thanks"),
        "{e}"
    );
}

fn template(entries: &Value, locale: &str) -> Value {
    entries["templates"]
        .as_array()
        .and_then(|a| a.iter().find(|t| t["locale"] == locale))
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn templates_save_validate_preview_restore() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env.router(db.conn.clone());
    let doc = create_issued(&app, issuable_with_emails(&app).await).await;
    // Every key of the space lives under `spaces/{id}/`.
    let storage = &env
        .pdf
        .storage
        .storage
        .scoped(&db.space.storage_prefix())
        .expect("space storage");

    let (status, list) = call(&app, Method::GET, "/api/settings/email/templates", None).await;
    assert_eq!(status, StatusCode::OK);
    let cs_default = template(&list, "cs");
    assert_eq!(cs_default["custom"], false);
    assert_eq!(
        cs_default["subject"],
        "{{ doc.typeLabel }} {{ doc.number }} – {{ company.name }}\n"
    );
    assert_eq!(template(&list, "en")["custom"], false);

    // Undefined variable on line 3 of the body: rejected, nothing stored.
    let bad = json!({ "subject": "{{ doc.number }}", "body": "a\nb\n{{ doc.nope }}" });
    for uri in [
        "/api/settings/email/templates/cs",
        "/api/settings/email/templates/cs/preview",
    ] {
        let method = if uri.ends_with("preview") {
            Method::POST
        } else {
            Method::PUT
        };
        let (status, e) = call(&app, method, uri, Some(bad.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
        assert_eq!(
            (&e["code"], &e["fields"]),
            (
                &json!("template_invalid"),
                &json!({ "body": "template_invalid" })
            )
        );
        assert!(
            e["detail"]
                .as_str()
                .expect("detail")
                .starts_with("line 3: undefined value"),
            "{e}"
        );
    }
    let (status, e) = call(
        &app,
        Method::PUT,
        "/api/settings/email/templates/cs",
        Some(json!({ "subject": "{% if %}", "body": "{{ doc.nope }}" })),
    )
    .await;
    assert_eq!(
        (status, &e["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "subject": "template_invalid" })
        )
    );
    let (status, e) = call(
        &app,
        Method::PUT,
        "/api/settings/email/templates/cs",
        Some(json!({ "subject": "", "body": "x".repeat(20_001) })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        e["fields"],
        json!({ "subject": "required", "body": "too_long" })
    );
    // Missing parts are 422 `required` too (not a serde 400), save and preview.
    for (method, uri) in [
        (Method::PUT, "/api/settings/email/templates/cs"),
        (Method::POST, "/api/settings/email/templates/cs/preview"),
    ] {
        let (status, e) = call(&app, method, uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
        assert_eq!(
            e,
            json!({ "code": "validation", "fields": { "subject": "required", "body": "required" } }),
            "{uri}"
        );
    }
    assert!(storage.list("email").await.expect("list").is_empty());

    // Unguarded `contact.*` fails on the contactless sample (save and preview).
    let unguarded =
        json!({ "subject": "{{ doc.number }}", "body": "Dobrý den,\n{{ contact.name }}" });
    for (method, uri) in [
        (Method::PUT, "/api/settings/email/templates/en"),
        (Method::POST, "/api/settings/email/templates/en/preview"),
    ] {
        let (status, e) = call(&app, method, uri, Some(unguarded.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
        assert_eq!(
            (&e["code"], &e["fields"]),
            (
                &json!("template_invalid"),
                &json!({ "body": "template_invalid" })
            )
        );
        let detail = e["detail"].as_str().expect("detail");
        assert!(detail.starts_with("without contact: line 2: "), "{detail}");
    }
    // Strict errors inside branches the base sample does not take.
    for (subject, prefix) in [
        (
            "{% if doc.paid %}{{ doc.nope }}{% endif %}x",
            "paid: line 1: ",
        ),
        (
            "{% if doc.cancelled %}{{ doc.nope }}{% endif %}x",
            "cancelled: line 1: ",
        ),
        (
            "{% if doc.originalNumber %}{{ doc.nope }}{% endif %}x",
            "credit note: line 1: ",
        ),
        (
            "{% if not doc.dueDate %}{{ doc.nope }}{% endif %}x",
            "no bank account: line 1: ",
        ),
    ] {
        let (status, e) = call(
            &app,
            Method::PUT,
            "/api/settings/email/templates/cs",
            Some(json!({ "subject": subject, "body": "" })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{subject}: {e}");
        assert_eq!(e["fields"], json!({ "subject": "template_invalid" }));
        let detail = e["detail"].as_str().expect("detail");
        assert!(detail.starts_with(prefix), "{subject}: {detail}");
    }
    assert!(storage.list("email").await.expect("list").is_empty());

    // Preview renders on the sample invoice, stores nothing.
    let custom = json!({
        "subject": "Doklad {{ doc.number }}\n{% if contact %} pro {{ contact.name }}{% endif %}",
        "body": "{{ doc.typeLabel }}: {{ doc.total }}{% if contact and contact.email %} → {{ contact.email }}{% endif %}\n\n"
    });
    let (status, p) = call(
        &app,
        Method::POST,
        "/api/settings/email/templates/cs/preview",
        Some(custom.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{p}");
    let subject = p["subject"].as_str().expect("subject");
    assert!(
        subject.starts_with("Doklad ") && subject.ends_with("0001 pro Ukázkový odběratel s.r.o."),
        "{p}"
    );
    assert!(
        p["body"]
            .as_str()
            .expect("body")
            .ends_with("→ fakturace@example.com"),
        "{p}"
    );
    assert!(storage.list("email").await.expect("list").is_empty());

    // Save → stored, effective, used by the prefill.
    let (status, saved) = call(
        &app,
        Method::PUT,
        "/api/settings/email/templates/cs",
        Some(custom.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved,
        json!({ "locale": "cs", "subject": custom["subject"], "body": custom["body"], "custom": true })
    );
    let keys: Vec<String> = storage
        .list("email")
        .await
        .expect("list")
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(keys, ["email/templates/cs.json"]);
    let stored: Value = serde_json::from_slice(
        &storage
            .get("email/templates/cs.json")
            .await
            .expect("stored override"),
    )
    .expect("JSON object");
    assert_eq!(
        stored,
        json!({ "subject": custom["subject"], "body": custom["body"] })
    );
    let (_, list) = call(&app, Method::GET, "/api/settings/email/templates", None).await;
    assert_eq!(template(&list, "cs"), saved);
    assert_eq!(template(&list, "en")["custom"], false);
    let (_, pre) = call(
        &app,
        Method::GET,
        &format!("/api/documents/{}/email", id(&doc)),
        None,
    )
    .await;
    assert_eq!(
        pre["subject"],
        format!(
            "Doklad {} pro Odběratel a.s.",
            doc["number"].as_str().expect("number")
        )
    );
    assert!(
        pre["body"]
            .as_str()
            .expect("body")
            .ends_with(&format!("→ {CONTACT_EMAIL}")),
        "{pre}"
    );

    // An invalid save keeps the active template.
    let (status, _) = call(
        &app,
        Method::PUT,
        "/api/settings/email/templates/cs",
        Some(bad),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, list) = call(&app, Method::GET, "/api/settings/email/templates", None).await;
    assert_eq!(template(&list, "cs"), saved);

    // Restore default (idempotent).
    for _ in 0..2 {
        let (status, restored) = call(
            &app,
            Method::DELETE,
            "/api/settings/email/templates/cs",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(restored, cs_default);
    }
    assert!(storage.list("email").await.expect("list").is_empty());

    for (method, uri) in [
        (Method::PUT, "/api/settings/email/templates/de"),
        (Method::DELETE, "/api/settings/email/templates/de"),
        (Method::POST, "/api/settings/email/templates/de/preview"),
    ] {
        let body = (method != Method::DELETE).then(|| custom.clone());
        let (status, _) = call(&app, method, uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn templates_need_the_storage() {
    let db = TestDb::new().await;
    let env = EmailEnv::new().await;
    let app = env
        .pdf
        .router_with(db.conn.clone(), &env.pdf.url, dead_s3());
    let (status, e) = call(&app, Method::GET, "/api/settings/email/templates", None).await;
    assert_eq!(
        (status, &e["code"]),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            &json!("storage_unavailable")
        )
    );
    let (status, e) = call(
        &app,
        Method::DELETE,
        "/api/settings/email/templates/en",
        None,
    )
    .await;
    assert_eq!(
        (status, &e["code"]),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            &json!("storage_unavailable")
        )
    );
}
