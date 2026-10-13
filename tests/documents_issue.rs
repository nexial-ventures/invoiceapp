//! Issuing (validation, numbering, snapshots) and the post-issue lifecycle.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_bank, create_contact, create_doc, id, issuable, issue, item};
use common::{TestDb, call, router};
use serde_json::{Value, json};

async fn issue_err(app: &axum::Router, body: Value) -> Value {
    let doc = create_doc(app, body).await;
    let (status, err) = issue(app, &id(&doc)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    err["fields"].clone()
}

#[tokio::test]
async fn issue_validations() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let base = issuable(&app).await;
    let with = |patch: Value| {
        let mut b = base.clone();
        if let (Some(o), Some(p)) = (b.as_object_mut(), patch.as_object()) {
            o.extend(p.clone());
        }
        b
    };

    assert_eq!(
        issue_err(&app, with(json!({ "contactId": null }))).await,
        json!({ "contactId": "required" })
    );
    let only_text = json!({ "lines": [{ "kind": "text", "description": "nothing billable" }] });
    assert_eq!(
        issue_err(&app, with(only_text)).await,
        json!({ "lines": "required" })
    );
    let early_due = json!({ "issueDate": "2026-10-10", "dueDate": "2026-10-09" });
    assert_eq!(
        issue_err(&app, with(early_due)).await,
        json!({ "dueDate": "invalid" })
    );

    // PUT keeps a null tax point date (POST would default it).
    let doc = create_doc(&app, base.clone()).await;
    let mut put = doc.clone();
    put["taxPointDate"] = Value::Null;
    put["lines"] = json!([item("1", "1", "21")]);
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{}", id(&doc)),
        Some(put),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, err) = issue(&app, &id(&doc)).await;
    assert_eq!(
        (status, &err["fields"]),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            &json!({ "taxPointDate": "required" })
        )
    );

    // A non-bank-transfer document issues fine.
    let cash = with(json!({ "paymentMethod": "cash" }));
    let doc = create_doc(&app, cash).await;
    assert_eq!(issue(&app, &id(&doc)).await.0, StatusCode::OK);

    // A deleted bank account (FK set null) → required for bank transfer.
    let bank = doc["bankAccountId"]
        .as_str()
        .expect("default bank")
        .to_string();
    let draft = create_doc(&app, base.clone()).await;
    let (status, _) = call(
        &app,
        Method::DELETE,
        &format!("/api/settings/bank-accounts/{bank}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, err) = issue(&app, &id(&draft)).await;
    assert_eq!(err["fields"], json!({ "bankAccountId": "required" }));

    // A document whose contact was deleted cannot be issued either.
    let contact = create_contact(&app, json!({ "ico": "11111119", "name": "Gone" })).await;
    create_bank(&app, "CZK").await;
    let draft = create_doc(&app, with(json!({ "contactId": contact }))).await;
    call(
        &app,
        Method::DELETE,
        &format!("/api/contacts/{contact}"),
        None,
    )
    .await;
    let (_, err) = issue(&app, &id(&draft)).await;
    assert_eq!(err["fields"], json!({ "contactId": "required" }));
}

#[tokio::test]
async fn issue_numbers_snapshots_and_locks() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;

    let first = create_doc(&app, body.clone()).await;
    let second = create_doc(&app, body.clone()).await;
    let (status, a) = issue(&app, &id(&first)).await;
    assert_eq!(status, StatusCode::OK, "{a}");
    let (_, b) = issue(&app, &id(&second)).await;
    assert_eq!(a["number"], "20260001", "numbered from the issueDate year");
    assert_eq!(b["number"], "20260002");
    assert_eq!(a["variableSymbol"], "20260001");
    assert_eq!(a["status"], "issued");
    assert_eq!(a["paymentState"], "unpaid");
    assert_eq!(a["exchangeRate"], Value::Null);
    assert_eq!(
        a["supplier"],
        json!({ "name": "Dodavatel s.r.o.", "ico": "44444443", "dic": "CZ44444443",
                "street": "Hlavní 1", "city": "Praha", "zip": "11000", "country": "CZ",
                "registration": "C 123 vedená u MS v Praze", "vatPayer": true,
                "email": null, "phone": null, "web": null })
    );
    assert_eq!(a["customer"]["name"], "Odběratel a.s.");
    assert_eq!(a["customer"]["registration"], Value::Null);
    assert_eq!(
        a["bankSnapshot"],
        json!({ "accountNumber": "19-2000145399/0800", "iban": null, "bic": "GIBACZPX" })
    );
    assert_eq!(a["totals"]["payable"], "1210.00");

    // An explicit variable symbol is kept.
    let mut vs = body.clone();
    vs["variableSymbol"] = json!("777");
    let (_, c) = issue(&app, &id(&create_doc(&app, vs).await)).await;
    assert_eq!(
        (&c["number"], &c["variableSymbol"]),
        (&json!("20260003"), &json!("777"))
    );

    // Snapshots are frozen: editing the contact does not change the document.
    let contact = body["contactId"].as_str().expect("contact");
    let (status, _) = call(
        &app,
        Method::PUT,
        &format!("/api/contacts/{contact}"),
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let uri = format!("/api/documents/{}", id(&first));
    let (_, got) = call(&app, Method::GET, &uri, None).await;
    assert_eq!(got["customer"]["name"], "Odběratel a.s.");

    // Locked: PUT / DELETE → 409 document_locked; re-issue → invalid_state.
    let (status, err) = call(&app, Method::PUT, &uri, Some(body.clone())).await;
    assert_eq!(
        (status, err),
        (StatusCode::CONFLICT, json!({ "code": "document_locked" }))
    );
    let (status, err) = call(&app, Method::DELETE, &uri, None).await;
    assert_eq!(
        (status, err),
        (StatusCode::CONFLICT, json!({ "code": "document_locked" }))
    );
    let (status, err) = issue(&app, &id(&first)).await;
    assert_eq!(
        (status, err),
        (StatusCode::CONFLICT, json!({ "code": "invalid_state" }))
    );

    let (_, series) = call(&app, Method::GET, "/api/settings/number-series", None).await;
    assert_eq!(
        series[0]["counters"],
        json!([{ "year": 2026, "lastNumber": 3 }])
    );
}

#[tokio::test]
async fn cancel_mark_sent_and_internal_note() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let draft = create_doc(&app, body.clone()).await;
    let did = id(&draft);

    for action in ["cancel", "mark-sent"] {
        let (status, err) = call(
            &app,
            Method::POST,
            &format!("/api/documents/{did}/{action}"),
            None,
        )
        .await;
        assert_eq!(
            (status, err),
            (StatusCode::CONFLICT, json!({ "code": "invalid_state" })),
            "{action}"
        );
    }
    let (status, doc) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{did}/internal-note"),
        Some(json!({ "internalNote": "draft note" })),
    )
    .await;
    assert_eq!(
        (status, &doc["internalNote"]),
        (StatusCode::OK, &json!("draft note"))
    );

    issue(&app, &did).await;
    let (status, sent) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{did}/mark-sent"),
        Some(json!({ "sentAt": "2026-10-02T10:00:00+02:00" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(sent["sentAt"], "2026-10-02T08:00:00Z");
    let (status, again) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{did}/mark-sent"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(again["sentAt"], sent["sentAt"], "mark-sent overwrites");

    let (status, doc) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{did}/internal-note"),
        Some(json!({ "internalNote": "issued note" })),
    )
    .await;
    assert_eq!(
        (status, &doc["internalNote"]),
        (StatusCode::OK, &json!("issued note"))
    );

    let (status, cancelled) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{did}/cancel"),
        Some(json!({ "reason": "duplicate" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["cancelReason"], "duplicate");
    assert!(cancelled["cancelledAt"].is_string());
    assert_eq!(cancelled["paymentState"], Value::Null);
    assert_eq!(cancelled["number"], "20260001", "the number stays used");
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{did}/cancel"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, doc) = call(
        &app,
        Method::PUT,
        &format!("/api/documents/{did}/internal-note"),
        Some(json!({ "internalNote": null })),
    )
    .await;
    assert_eq!(
        (status, &doc["internalNote"]),
        (StatusCode::OK, &Value::Null)
    );
}

#[tokio::test]
async fn lower_counter_is_accepted_and_allocation_skips_issued_numbers() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    for _ in 0..3 {
        let doc = create_doc(&app, body.clone()).await;
        assert_eq!(issue(&app, &id(&doc)).await.0, StatusCode::OK);
    }
    let uri = "/api/settings/number-series/invoice/counters/2026";
    let (status, s) = call(&app, Method::PUT, uri, Some(json!({ "lastNumber": 1 }))).await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(s["counters"][0], json!({ "year": 2026, "lastNumber": 1 }));
    let doc = create_doc(&app, body.clone()).await;
    let (status, issued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    assert_eq!(
        issued["number"], "20260004",
        "past the highest issued number"
    );
    // A counter above the issued numbers wins.
    call(&app, Method::PUT, uri, Some(json!({ "lastNumber": 10 }))).await;
    let doc = create_doc(&app, body).await;
    assert_eq!(issue(&app, &id(&doc)).await.1["number"], "20260011");
}
