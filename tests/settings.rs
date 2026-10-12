//! Company profile and bank accounts.

mod common;

use axum::http::{Method, StatusCode};
use common::{TestDb, call, router};
use serde_json::{Value, json};

fn company_body() -> Value {
    json!({
        "name": " Jan Novák ",
        "ico": "270 74 358",
        "dic": "cz27074358",
        "vatPayer": true,
        "street": "Dlouhá 1",
        "city": "Praha",
        "zip": "11000",
        "country": "cz",
        "email": "jan@example.cz",
        "phone": null,
        "web": "",
        "registration": "Fyzická osoba zapsaná v živnostenském rejstříku",
        "defaultDueDays": 30,
        "defaultLocale": "en"
    })
}

#[tokio::test]
async fn company_is_seeded_and_replaceable() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());

    let (status, seeded) = call(&app, Method::GET, "/api/settings/company", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seeded["name"], "");
    assert_eq!(seeded["country"], "CZ");
    assert_eq!(seeded["defaultDueDays"], 14);
    assert_eq!(seeded["defaultLocale"], "cs");
    assert_eq!(seeded["vatPayer"], false);

    let (status, saved) = call(
        &app,
        Method::PUT,
        "/api/settings/company",
        Some(company_body()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["name"], "Jan Novák");
    assert_eq!(saved["ico"], "27074358");
    assert_eq!(saved["dic"], "CZ27074358");
    assert_eq!(saved["country"], "CZ");
    assert_eq!(saved["web"], Value::Null);
    assert_eq!(saved["defaultLocale"], "en");

    let (_, again) = call(&app, Method::GET, "/api/settings/company", None).await;
    assert_eq!(again, saved);
}

#[tokio::test]
async fn company_validation_is_422_with_fields() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = company_body();
    body["name"] = json!("  ");
    body["ico"] = json!("12345678");
    body["defaultDueDays"] = json!(400);
    body["defaultLocale"] = json!("de");
    body["registration"] = json!("x".repeat(301));

    let (status, err) = call(&app, Method::PUT, "/api/settings/company", Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err,
        json!({
            "code": "validation",
            "fields": {
                "name": "required",
                "ico": "invalid_ico",
                "defaultDueDays": "invalid",
                "defaultLocale": "invalid",
                "registration": "too_long"
            }
        })
    );
}

#[tokio::test]
async fn missing_required_company_fields_are_422() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let mut body = company_body();
    for f in ["vatPayer", "defaultDueDays", "defaultLocale"] {
        body.as_object_mut().expect("object").remove(f);
    }
    let (status, err) = call(&app, Method::PUT, "/api/settings/company", Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err,
        json!({
            "code": "validation",
            "fields": {
                "vatPayer": "required",
                "defaultDueDays": "required",
                "defaultLocale": "required"
            }
        })
    );
}

#[tokio::test]
async fn malformed_json_is_400() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let (status, err) = call(&app, Method::PUT, "/api/settings/company", Some(json!([1]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(err, json!({ "code": "bad_request" }));
}

async fn create_account(app: &axum::Router, body: Value) -> Value {
    let (status, acc) = call(app, Method::POST, "/api/settings/bank-accounts", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{acc}");
    acc
}

async fn list_accounts(app: &axum::Router) -> Vec<Value> {
    let (status, list) = call(app, Method::GET, "/api/settings/bank-accounts", None).await;
    assert_eq!(status, StatusCode::OK);
    list.as_array().expect("array").clone()
}

fn defaults(list: &[Value], currency: &str) -> Vec<String> {
    list.iter()
        .filter(|a| a["currency"] == currency && a["isDefault"] == true)
        .map(|a| a["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn bank_account_crud_and_default_per_currency() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());

    let first = create_account(
        &app,
        json!({ "label": "Main", "currency": "czk", "accountNumber": "19-2000145399/0800" }),
    )
    .await;
    assert_eq!(first["currency"], "CZK");
    assert_eq!(
        first["isDefault"], true,
        "first of a currency becomes default"
    );
    assert_eq!(first["iban"], Value::Null);

    let second = create_account(
        &app,
        json!({ "currency": "CZK", "iban": "cz65 0800 0000 1920 0014 5399", "isDefault": true }),
    )
    .await;
    assert_eq!(second["iban"], "CZ6508000000192000145399");
    assert_eq!(second["isDefault"], true);

    let eur = create_account(
        &app,
        json!({ "currency": "EUR", "iban": "DE89370400440532013000", "bic": "cobadeffxxx" }),
    )
    .await;
    assert_eq!(eur["isDefault"], true);
    assert_eq!(eur["bic"], "COBADEFFXXX");

    let list = list_accounts(&app).await;
    assert_eq!(list.len(), 3);
    assert_eq!(
        list[0]["id"], second["id"],
        "ordered by currency, default first"
    );
    assert_eq!(list[2]["id"], eur["id"]);
    assert_eq!(
        defaults(&list, "CZK"),
        vec![second["id"].as_str().unwrap_or_default()]
    );

    // Making the first one default again unsets the second.
    let id1 = first["id"].as_str().unwrap_or_default();
    let (status, updated) = call(
        &app,
        Method::PUT,
        &format!("/api/settings/bank-accounts/{id1}"),
        Some(json!({ "label": "Renamed", "currency": "CZK", "accountNumber": "2000145399/0800", "isDefault": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["label"], "Renamed");
    assert_eq!(defaults(&list_accounts(&app).await, "CZK"), vec![id1]);

    // Deleting the default promotes the remaining account.
    let (status, body) = call(
        &app,
        Method::DELETE,
        &format!("/api/settings/bank-accounts/{id1}"),
        None,
    )
    .await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
    let list = list_accounts(&app).await;
    assert_eq!(list.len(), 2);
    assert_eq!(
        defaults(&list, "CZK"),
        vec![second["id"].as_str().unwrap_or_default()]
    );

    // Moving the only EUR account to CZK keeps a single CZK default.
    let eur_id = eur["id"].as_str().unwrap_or_default();
    let (status, moved) = call(
        &app,
        Method::PUT,
        &format!("/api/settings/bank-accounts/{eur_id}"),
        Some(json!({ "currency": "CZK", "iban": "DE89370400440532013000", "isDefault": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(moved["isDefault"], false);
    assert_eq!(defaults(&list_accounts(&app).await, "CZK").len(), 1);
}

#[tokio::test]
async fn bank_account_validation_and_not_found() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());

    let (status, err) = call(
        &app,
        Method::POST,
        "/api/settings/bank-accounts",
        Some(json!({ "currency": "CZKK", "accountNumber": "", "iban": " " })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err["fields"],
        json!({ "currency": "invalid", "accountNumber": "required" })
    );

    let (status, err) = call(
        &app,
        Method::POST,
        "/api/settings/bank-accounts",
        Some(json!({ "currency": "CZK", "accountNumber": "1/0800", "iban": "CZ6508000000192000145398", "bic": "X" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err["fields"],
        json!({ "accountNumber": "invalid", "iban": "invalid", "bic": "invalid" })
    );

    let missing = "/api/settings/bank-accounts/00000000-0000-0000-0000-000000000001";
    let body = json!({ "currency": "CZK", "accountNumber": "12/0800" });
    let (status, err) = call(&app, Method::PUT, missing, Some(body)).await;
    assert_eq!(
        (status, err),
        (StatusCode::NOT_FOUND, json!({ "code": "not_found" }))
    );
    let (status, _) = call(&app, Method::DELETE, missing, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/settings/bank-accounts/nope",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
