//! MCP endpoint: transport, auth, handshake, tool listing and error mapping.
//! Raw JSON-RPC over HTTP; fictitious data only.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use common::documents::{create_doc, id, issuable, issue};
use common::mcp::{post, request, rpc, tool_error};
use common::{TEST_TOKEN, TestDb, router, send};
use serde_json::{Value, json};

const READ_TOOLS: &[&str] = &[
    "list_documents",
    "get_document",
    "list_contacts",
    "get_contact",
    "lookup_ares",
    "list_catalog_items",
    "get_settings",
    "compute_document",
];
const WRITE_TOOLS: &[&str] = &[
    "create_contact",
    "create_draft",
    "update_draft",
    "issue_document",
    "add_payment",
    "mark_sent",
];

fn initialize() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": { "name": "test-client", "version": "1.0" } } })
}

#[tokio::test]
async fn requires_the_bearer_token() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for bearer in [None, Some("wrong-token")] {
        let resp = send(app.clone(), request(&initialize(), bearer)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(resp.headers()["www-authenticate"], "Bearer");
        assert_eq!(common::json(resp).await, json!({ "code": "unauthorized" }));
    }
}

#[tokio::test]
async fn get_and_delete_are_not_allowed() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    for method in [Method::GET, Method::DELETE] {
        let req = Request::builder()
            .method(method.clone())
            .uri("/api/mcp")
            .header("host", common::TEST_HOST)
            .header("accept", "application/json, text/event-stream")
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .expect("request");
        let resp = send(app.clone(), req).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "{method}");
        assert_eq!(resp.headers()["allow"], "POST");
        assert_eq!(
            common::json(resp).await,
            json!({ "code": "method_not_allowed" })
        );
    }
}

#[tokio::test]
async fn transport_errors_use_the_api_error_shape() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());

    // No `Accept: text/event-stream` → 406.
    let mut req = request(&initialize(), Some(TEST_TOKEN));
    req.headers_mut()
        .insert("accept", "application/json".parse().expect("header"));
    let resp = send(app.clone(), req).await;
    assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(common::json(resp).await, json!({ "code": "bad_request" }));

    // Over the 2 MiB body limit → 413.
    let big = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "compute_document", "arguments": { "padding": "x".repeat(3 << 20) } } });
    let resp = send(app.clone(), request(&big, Some(TEST_TOKEN))).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(common::json(resp).await, json!({ "code": "too_large" }));
}

#[tokio::test]
async fn initialize_is_stateless_and_describes_the_server() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (status, headers, resp) = post(&app, &initialize()).await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(headers.get("mcp-session-id").is_none(), "stateless");
    assert!(
        headers["content-type"]
            .to_str()
            .expect("content type")
            .starts_with("application/json")
    );
    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "invoiceapp");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["capabilities"].get("resources").is_none());
    assert!(result["capabilities"].get("prompts").is_none());
    let instructions = result["instructions"].as_str().expect("instructions");
    assert!(instructions.contains("issue_document"));
    assert!(instructions.contains("decimal strings"));

    // An unknown version falls back to rmcp's newest handshake version.
    let mut old = initialize();
    old["params"]["protocolVersion"] = json!("1999-01-01");
    let (_, _, resp) = post(&app, &old).await;
    assert_eq!(resp["result"]["protocolVersion"], "2025-11-25", "{resp}");

    // The client's follow-up notification is accepted without a body.
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let (status, _, body) = post(&app, &note).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body, Value::Null);
}

#[tokio::test]
async fn tools_list_names_and_annotations() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let resp = rpc(&app, "tools/list", json!({})).await;
    let tools = resp["result"]["tools"].as_array().expect("tools");
    let mut names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();
    names.sort_unstable();
    let mut expected: Vec<&str> = READ_TOOLS.iter().chain(WRITE_TOOLS).copied().collect();
    expected.sort_unstable();
    assert_eq!(names, expected);

    for t in tools {
        let name = t["name"].as_str().expect("name");
        let a = &t["annotations"];
        assert!(
            t["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name}"
        );
        assert_eq!(t["inputSchema"]["type"], "object", "{name}");
        if READ_TOOLS.contains(&name) {
            assert_eq!(a["readOnlyHint"], true, "{name}");
            continue;
        }
        assert_eq!(a["readOnlyHint"], false, "{name}");
        assert_eq!(a["destructiveHint"], name == "issue_document", "{name}");
        assert_eq!(a["idempotentHint"], name == "update_draft", "{name}");
    }

    // Schemas come from the Rust types: camelCase, required fields.
    let schema = |n: &str| {
        tools
            .iter()
            .find(|t| t["name"] == n)
            .map(|t| t["inputSchema"].clone())
            .expect("tool")
    };
    let draft = schema("create_draft");
    for field in [
        "contactId",
        "issueDate",
        "lines",
        "vatMode",
        "bankAccountId",
    ] {
        assert!(draft["properties"].get(field).is_some(), "{field}");
    }
    let list = schema("list_documents");
    assert_eq!(list["required"], json!(["direction"]));
    let update = schema("update_draft");
    assert!(update["properties"].get("id").is_some());
    assert!(
        update["properties"].get("lines").is_some(),
        "flattened input"
    );
    let pay = schema("add_payment");
    let mut required: Vec<&str> = pay["required"]
        .as_array()
        .expect("required")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    required.sort_unstable();
    assert_eq!(required, ["amount", "date", "id"]);
    // Decimal inputs take a string or a number.
    assert_eq!(
        pay["properties"]["amount"]["type"],
        json!(["string", "number"])
    );
    assert!(pay["properties"].get("exchangeRate").is_some());
    let item = &draft["$defs"]["LineInput"]["properties"];
    assert!(item.get("unitPrice").is_some(), "{draft}");
}

#[tokio::test]
async fn unknown_tool_is_a_json_rpc_error_bad_arguments_a_validation_error() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let resp = rpc(
        &app,
        "tools/call",
        json!({ "name": "delete_everything", "arguments": {} }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32602, "{resp}");
    assert!(resp.get("result").is_none());

    // Arguments that do not fit the schema are REST validation errors.
    let err = tool_error(&app, "get_document", json!({ "id": "not-a-uuid" })).await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "id": "invalid" } })
    );
    let err = tool_error(&app, "add_payment", json!({ "id": "not-a-uuid" })).await;
    assert_eq!(err["fields"]["id"], "invalid");
    let err = tool_error(
        &app,
        "add_payment",
        json!({ "id": "00000000-0000-4000-8000-000000000001", "amount": 5 }),
    )
    .await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "date": "required" } })
    );
    let err = tool_error(
        &app,
        "update_draft",
        json!({ "id": "00000000-0000-4000-8000-000000000001",
                "lines": [{ "kind": "item", "unitPrice": [1] }] }),
    )
    .await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "lines.0.unitPrice": "invalid" } })
    );
    let err = tool_error(&app, "update_draft", json!({ "lines": [] })).await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "id": "required" } })
    );
}

#[tokio::test]
async fn domain_errors_are_tool_errors_with_the_rest_json() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let body = issuable(&app).await;

    // Same validation as REST: a credit note is not creatable, a bad rate is caught.
    let mut bad = body.clone();
    bad["docType"] = json!("credit_note");
    bad["lines"][0]["quantity"] = json!("abc");
    let err = tool_error(&app, "create_draft", bad).await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": {
            "docType": "invalid", "lines.0.quantity": "invalid" } })
    );
    let mut received = body.clone();
    received["direction"] = json!("received");
    let err = tool_error(&app, "create_draft", received).await;
    assert_eq!(err["fields"]["docType"], "invalid");
    let mut imported = body.clone();
    imported["imported"] = json!(true);
    let err = tool_error(&app, "create_draft", imported).await;
    assert_eq!(err["fields"]["imported"], "invalid");

    let missing = json!({ "id": "00000000-0000-4000-8000-000000000001" });
    let err = tool_error(&app, "get_document", missing).await;
    assert_eq!(err, json!({ "code": "not_found" }));

    let err = tool_error(&app, "list_documents", json!({ "direction": "both" })).await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "direction": "invalid" } })
    );

    let err = tool_error(&app, "lookup_ares", json!({ "ico": "12345678" })).await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "ico": "invalid_ico" } })
    );

    // An issued document is locked; issuing it again is an invalid state.
    let doc = create_doc(&app, body.clone()).await;
    let (status, _) = issue(&app, &id(&doc)).await;
    assert_eq!(status, StatusCode::OK);
    let mut update = body.clone();
    update["id"] = json!(id(&doc));
    let err = tool_error(&app, "update_draft", update).await;
    assert_eq!(err, json!({ "code": "document_locked" }));
    let err = tool_error(&app, "issue_document", json!({ "id": id(&doc) })).await;
    assert_eq!(err, json!({ "code": "invalid_state" }));

    // A draft cannot be paid or marked sent.
    let draft = create_doc(&app, body).await;
    let pay = json!({ "id": id(&draft), "date": "2026-10-02", "amount": "10" });
    let err = tool_error(&app, "add_payment", pay).await;
    assert_eq!(err, json!({ "code": "invalid_state" }));
    let err = tool_error(&app, "mark_sent", json!({ "id": id(&draft) })).await;
    assert_eq!(err, json!({ "code": "invalid_state" }));
    let pay = json!({ "id": id(&doc), "date": "2026-10-02", "amount": "0" });
    let err = tool_error(&app, "add_payment", pay).await;
    assert_eq!(
        err,
        json!({ "code": "validation", "fields": { "amount": "invalid" } })
    );
}

#[tokio::test]
async fn openapi_lists_the_endpoint() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let resp = send(app, common::get("/api/openapi.json", None)).await;
    let doc = common::json(resp).await;
    assert!(doc["paths"]["/api/mcp"]["post"].is_object());
}
