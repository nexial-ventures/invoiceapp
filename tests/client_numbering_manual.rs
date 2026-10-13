//! Manual numbers on native drafts and renumbering an unlocked draft
//! (`docs/api/numbering.md`).

mod common;

use axum::http::{Method, StatusCode};
use common::documents::{create_doc, create_issued, id, issuable, issue, unlock};
use common::mcp::{rpc, tool};
use common::numbering::{client_setup, coded_contact, issue_for, put_doc};
use common::{TestDb, call, router};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde_json::{Value, json};

/// `(number_year, number_seq, number_code)` of a document.
async fn placement(db: &DatabaseConnection, doc: &Value) -> (i32, Option<i32>, Option<String>) {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT number_year, number_seq, number_code FROM documents WHERE id = $1",
            [id(doc).parse::<uuid::Uuid>().expect("uuid").into()],
        ))
        .await
        .expect("query")
        .expect("row");
    (
        row.try_get("", "number_year").expect("year"),
        row.try_get("", "number_seq").expect("seq"),
        row.try_get("", "number_code").expect("code"),
    )
}

async fn invoice_counter(app: &axum::Router) -> Value {
    let (_, all) = call(app, Method::GET, "/api/settings/number-series", None).await;
    all.as_array()
        .and_then(|a| a.iter().find(|s| s["docType"] == "invoice"))
        .map(|s| s["counters"].clone())
        .expect("invoice series")
}

fn with_number(body: &Value, number: Value) -> Value {
    let mut b = body.clone();
    b["number"] = number;
    b
}

#[tokio::test]
async fn a_manual_number_fills_a_gap_without_moving_the_counter() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    create_issued(&app, body.clone()).await;
    let uri = "/api/settings/number-series/invoice/counters/2026";
    call(&app, Method::PUT, uri, Some(json!({ "lastNumber": 3 }))).await;
    assert_eq!(
        create_issued(&app, body.clone()).await["number"],
        "20260004"
    );

    // 20260002 and 20260003 were skipped: fill one by hand.
    let doc = create_issued(&app, with_number(&body, json!(" 20260002 "))).await;
    assert_eq!(doc["number"], "20260002");
    assert_eq!(doc["variableSymbol"], "20260002");
    assert_eq!(placement(&db.conn, &doc).await, (2026, Some(2), None));
    assert_eq!(
        invoice_counter(&app).await,
        json!([{ "year": 2026, "lastNumber": 4 }])
    );
    assert_eq!(
        create_issued(&app, body.clone()).await["number"],
        "20260005"
    );

    // A number no pattern renders: no sequence, the issue year.
    let doc = create_issued(&app, with_number(&body, json!("RUCNE-1"))).await;
    assert_eq!(placement(&db.conn, &doc).await, (2026, None, None));
}

#[tokio::test]
async fn a_manual_number_is_unique_and_skipped_by_allocation() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    create_issued(&app, body.clone()).await;

    let (status, err) = call(
        &app,
        Method::POST,
        "/api/documents",
        Some(with_number(&body, json!("20260001"))),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["fields"]["number"], "taken");

    // A draft holding a number ahead: allocation steps over it.
    let held = create_doc(&app, with_number(&body, json!("20260003"))).await;
    assert_eq!(held["status"], "draft");
    for expected in ["20260002", "20260004"] {
        assert_eq!(create_issued(&app, body.clone()).await["number"], expected);
    }
    let (status, issued) = issue(&app, &id(&held)).await;
    assert_eq!(
        (status, &issued["number"]),
        (StatusCode::OK, &json!("20260003"))
    );
}

#[tokio::test]
async fn a_manual_client_number_joins_the_client_sequence() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = client_setup(&app).await;
    let acme = coded_contact(&app, "Acme", Some("ACME")).await;
    let mut b = with_number(&body, json!("acme-2026007"));
    b["contactId"] = json!(acme);
    let doc = create_issued(&app, b).await;
    assert_eq!(
        placement(&db.conn, &doc).await,
        (2026, Some(7), Some("acme".into()))
    );
    // Read back into the customer's sequence, but the counter did not move.
    assert_eq!(issue_for(&app, &body, &acme).await, "ACME-2026001");
}

#[tokio::test]
async fn renumbering_an_unlocked_draft_keeps_the_previous_number() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body.clone()).await;
    assert_eq!(doc["previousNumber"], Value::Null);
    let (status, mut draft) = unlock(&app, &id(&doc), json!("wrong number")).await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    assert_eq!(draft["number"], "20260001");

    draft["number"] = json!("20260042");
    let (status, saved) = put_doc(&app, &draft).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(reissued["number"], "20260042");
    assert_eq!(reissued["previousNumber"], "20260001");
    assert_eq!(reissued["variableSymbol"], "20260042", "derived: follows");

    // A hand-entered symbol stays; the latest replaced number is kept.
    let (_, mut draft) = unlock(&app, &id(&doc), json!("again")).await;
    draft["number"] = json!("20260043");
    draft["variableSymbol"] = json!("777");
    assert_eq!(put_doc(&app, &draft).await.0, StatusCode::OK);
    let (_, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(
        (
            &reissued["number"],
            &reissued["previousNumber"],
            &reissued["variableSymbol"]
        ),
        (&json!("20260043"), &json!("20260042"), &json!("777"))
    );

    // Re-issued unchanged: the previous number stays as it was.
    let (_, draft) = unlock(&app, &id(&doc), json!("once more")).await;
    assert_eq!(put_doc(&app, &draft).await.0, StatusCode::OK);
    let (_, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(
        (&reissued["number"], &reissued["previousNumber"]),
        (&json!("20260043"), &json!("20260042"))
    );
}

#[tokio::test]
async fn a_cleared_number_on_an_unlocked_draft_allocates_a_fresh_one() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body).await;
    let (_, mut draft) = unlock(&app, &id(&doc), json!("renumber")).await;
    draft["number"] = Value::Null;
    let (status, saved) = put_doc(&app, &draft).await;
    assert_eq!((status, &saved["number"]), (StatusCode::OK, &Value::Null));

    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(reissued["number"], "20260002");
    assert_eq!(reissued["previousNumber"], "20260001");
    assert_eq!(reissued["variableSymbol"], "20260002");
}

#[tokio::test]
async fn an_omitted_number_keeps_the_unlocked_drafts_number() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;
    let doc = create_issued(&app, body.clone()).await;
    create_issued(&app, body).await;
    let (_, mut draft) = unlock(&app, &id(&doc), json!("fix lines")).await;
    draft.as_object_mut().expect("object").remove("number");
    let (status, saved) = put_doc(&app, &draft).await;
    assert_eq!(
        (status, &saved["number"]),
        (StatusCode::OK, &json!("20260001"))
    );

    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(reissued["number"], "20260001");
    assert_eq!(reissued["previousNumber"], Value::Null);
}

#[tokio::test]
async fn mcp_draft_tools_never_touch_the_number() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;

    let tools = rpc(&app, "tools/list", json!({})).await;
    for name in ["create_draft", "update_draft"] {
        let t = tools["result"]["tools"]
            .as_array()
            .and_then(|l| l.iter().find(|t| t["name"] == name))
            .expect("tool");
        let schema = &t["inputSchema"];
        assert!(schema["properties"].is_object(), "{schema}");
        assert!(
            schema["properties"].get("number").is_none(),
            "{name} advertises number"
        );
    }

    // A sent number is ignored on create…
    let mut args = body.clone();
    args["number"] = json!("X-1");
    let draft = tool(&app, "create_draft", args).await;
    assert_eq!(draft["number"], Value::Null);

    // …and on update: an unlocked draft keeps its number whatever is sent.
    let doc = create_issued(&app, body).await;
    let (_, draft) = unlock(&app, &id(&doc), json!("via MCP")).await;
    for number in [json!("X-2"), Value::Null] {
        let mut args = draft.clone();
        args["number"] = number;
        let updated = tool(&app, "update_draft", args).await;
        assert_eq!(updated["number"], "20260001", "{updated}");
    }
    let (status, reissued) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK, "{reissued}");
    assert_eq!(
        (&reissued["number"], &reissued["previousNumber"]),
        (&json!("20260001"), &Value::Null)
    );
}
