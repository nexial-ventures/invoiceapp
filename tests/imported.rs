//! Manual import of issued documents: own number, issue without numbering
//! or render, duplicates, collisions with native numbering.

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_doc, id, issuable, issue, item, pay};
use common::mdcast::{PdfEnv, get_raw};
use common::received::upload;
use common::{TestDb, call};
use serde_json::{Value, json};

fn imported(base: &Value, doc_type: &str, number: &str) -> Value {
    let mut b = base.clone();
    b["imported"] = json!(true);
    b["docType"] = json!(doc_type);
    b["number"] = json!(number);
    if doc_type == "proforma" {
        b["taxPointDate"] = Value::Null;
    }
    b
}

#[tokio::test]
async fn imported_draft_issues_without_numbering_or_render() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let draft = create_doc(&app, imported(&base, "invoice", "FV-2019/0042")).await;
    assert_eq!(
        (&draft["imported"], &draft["number"], &draft["status"]),
        (&json!(true), &json!("FV-2019/0042"), &json!("draft"))
    );
    let (status, doc) = issue(&app, &id(&draft)).await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    assert_eq!(
        (&doc["number"], &doc["status"]),
        (&json!("FV-2019/0042"), &json!("issued"))
    );
    assert_eq!(doc["variableSymbol"], "20190042");
    assert_eq!((&doc["pdf"], env.mock.count()), (&Value::Null, 0));
    assert_eq!(doc["supplier"]["name"], "Dodavatel s.r.o.");
    // No counter moved.
    let (_, series) = call(&app, Method::GET, "/api/settings/number-series", None).await;
    assert!(
        series
            .as_array()
            .expect("array")
            .iter()
            .all(|s| s["counters"] == json!([]))
    );
    // Its PDF is the uploaded original only.
    let uri = format!("/api/documents/{}/pdf", id(&doc));
    let (status, _, _) = get_raw(&app, &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = upload(&app, &id(&doc), b"%PDF-1.4 original").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, headers, bytes) = get_raw(&app, &uri).await;
    assert_eq!(
        (status, bytes.as_slice()),
        (StatusCode::OK, b"%PDF-1.4 original".as_slice())
    );
    assert_eq!(
        headers["content-disposition"],
        "inline; filename=\"FV-2019_0042.pdf\""
    );
    // Lifecycle still applies to the imported invoice.
    let (status, _) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/mark-sent", id(&doc)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, list) = call(&app, Method::GET, "/api/documents?imported=true", None).await;
    assert_eq!(
        (&list["total"], &list["items"][0]["hasPdf"]),
        (&json!(1), &json!(true))
    );
}

#[tokio::test]
async fn import_rules() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let post = |body: Value| {
        let app = app.clone();
        async move { call(&app, Method::POST, "/api/documents", Some(body)).await }
    };
    let first = create_doc(&app, imported(&base, "invoice", "IMP-1")).await;
    let (status, e) = post(imported(&base, "invoice", "IMP-1")).await;
    assert_eq!(
        (status, &e["fields"]["number"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("duplicate"))
    );
    // Unique per doc type only.
    create_doc(&app, imported(&base, "proforma", "IMP-1")).await;
    let mut b = imported(&base, "invoice", "");
    let (_, e) = post(b.clone()).await;
    assert_eq!(e["fields"]["number"], "required");
    // A native draft takes a manual number, from the pattern alphabet only.
    b["imported"] = json!(false);
    b["number"] = json!("X 1");
    let (_, e) = post(b).await;
    assert_eq!(e["fields"]["number"], "invalid");
    // Native drafts: explicit null relatedDocumentId / number are fine.
    let mut native = base.clone();
    native["relatedDocumentId"] = Value::Null;
    native["number"] = Value::Null;
    native["imported"] = json!(false);
    let (status, n) = post(native).await;
    assert_eq!(status, StatusCode::CREATED, "{n}");
    // Native create still refuses DDPP / credit note types; imported allows them.
    let mut ddpp = base.clone();
    ddpp["docType"] = json!("advance_tax_doc");
    let (_, e) = post(ddpp).await;
    assert_eq!(e["fields"]["docType"], "invalid");
    // An imported DDPP has no payment link: it can be cancelled.
    let ddpp = create_doc(&app, imported(&base, "advance_tax_doc", "DP-OLD-1")).await;
    let (status, ddpp) = issue(&app, &id(&ddpp)).await;
    assert_eq!(status, StatusCode::OK, "{ddpp}");
    let (status, c) = call(
        &app,
        Method::POST,
        &format!("/api/documents/{}/cancel", id(&ddpp)),
        None,
    )
    .await;
    assert_eq!(
        (status, &c["status"]),
        (StatusCode::OK, &json!("cancelled"))
    );
    let mut cn = imported(&base, "credit_note", "D-OLD-1");
    cn["correctionReason"] = json!("Sleva");
    cn["relatedDocumentId"] = first["id"].clone();
    let (status, e) = post(cn).await;
    assert_eq!(
        (status, &e["fields"]["relatedDocumentId"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid")),
        "draft target"
    );
    // No advance lines on imported documents.
    let mut adv = imported(&base, "invoice", "IMP-2");
    adv["lines"] =
        json!([item("1", "1", "21"), { "kind": "advance", "advanceDocumentId": id(&first) }]);
    let (_, e) = post(adv).await;
    assert_eq!(e["fields"]["lines.1.kind"], "invalid");
    // The flag is immutable; omitted on PUT keeps it.
    let uri = format!("/api/documents/{}", id(&first));
    let mut put = imported(&base, "invoice", "IMP-1B");
    put["imported"] = json!(false);
    let (_, e) = call(&app, Method::PUT, &uri, Some(put.clone())).await;
    assert_eq!(e["fields"]["imported"], "invalid");
    put.as_object_mut().expect("object").remove("imported");
    for f in ["currency", "locale", "vatMode", "paymentMethod", "dueDate"] {
        put[f] = json!(match f {
            "currency" => "CZK",
            "locale" => "cs",
            "vatMode" => "standard",
            "paymentMethod" => "bank_transfer",
            _ => "2026-10-15",
        });
    }
    let (status, d) = call(&app, Method::PUT, &uri, Some(put)).await;
    assert_eq!(status, StatusCode::OK, "{d}");
    assert_eq!(
        (&d["number"], &d["imported"]),
        (&json!("IMP-1B"), &json!(true))
    );
    // Compute accepts a DDPP with plain items (imported DDPP editor).
    let (status, c) = call(
        &app,
        Method::POST,
        "/api/documents/compute",
        Some(json!({ "docType": "advance_tax_doc", "lines": [item("1", "100", "21")] })),
    )
    .await;
    assert_eq!(
        (status, &c["totals"]["payable"]),
        (StatusCode::OK, &json!("121.00"))
    );
}

#[tokio::test]
async fn related_documents_and_proforma_payments() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let pf = create_doc(&app, imported(&base, "proforma", "ZF-OLD-7")).await;
    let (status, pf) = issue(&app, &id(&pf)).await;
    assert_eq!(status, StatusCode::OK, "{pf}");
    let (status, p) = pay(&app, &id(&pf), json!({ "amount": "1210" })).await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    assert_eq!(
        p["advanceDocumentId"],
        Value::Null,
        "no DDPP for imported proformas"
    );
    let mut inv = imported(&base, "invoice", "FV-OLD-8");
    inv["relatedDocumentId"] = pf["id"].clone();
    let inv = create_doc(&app, inv).await;
    assert_eq!(inv["parent"]["number"], "ZF-OLD-7");
    let mut cn = imported(&base, "credit_note", "D-OLD-9");
    cn["relatedDocumentId"] = pf["id"].clone();
    let (status, e) = call(&app, Method::POST, "/api/documents", Some(cn)).await;
    assert_eq!(
        (status, &e["fields"]["relatedDocumentId"]),
        (StatusCode::UNPROCESSABLE_ENTITY, &json!("invalid"))
    );
}

#[tokio::test]
async fn native_issue_skips_an_imported_number() {
    let db = TestDb::new().await;
    let env = PdfEnv::new();
    let app = env.router(db.conn.clone());
    let base = issuable(&app).await;
    let imp = create_doc(&app, imported(&base, "invoice", "20260001")).await;
    issue(&app, &id(&imp)).await;
    // The next free number after the counter: the imported one is held.
    let native = create_doc(&app, base.clone()).await;
    let (status, doc) = issue(&app, &id(&native)).await;
    assert_eq!(
        (status, &doc["number"]),
        (StatusCode::OK, &json!("20260002"))
    );
}
