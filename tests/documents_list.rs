//! Document list: filters, search, ordering, derived state.

mod common;

use axum::http::{Method, StatusCode};
use chrono::{Days, NaiveDate};
use common::documents::{create_contact, create_doc, id, issuable, issue};
use common::{TestDb, call, router};
use invoiceapp::time::today;
use serde_json::{Value, json};

fn ago(days: u64) -> NaiveDate {
    today().checked_sub_days(Days::new(days)).expect("date")
}

async fn numbers(app: &axum::Router, query: &str) -> (Vec<Value>, Value) {
    let (status, body) = call(app, Method::GET, &format!("/api/documents{query}"), None).await;
    assert_eq!(status, StatusCode::OK, "{query}: {body}");
    let items = body["items"].as_array().cloned().unwrap_or_default();
    (
        items.iter().map(|d| d["number"].clone()).collect(),
        body["total"].clone(),
    )
}

#[tokio::test]
async fn filters_search_and_ordering() {
    let db = TestDb::new().await;
    let app = router(db.conn.clone());
    let base = issuable(&app).await;
    let acme = create_contact(&app, json!({ "name": "ACME Corp", "ico": "11111119" })).await;
    let doc = |issue_date: NaiveDate, due: NaiveDate, contact: &Value| {
        let mut b = base.clone();
        b["issueDate"] = json!(issue_date.to_string());
        b["dueDate"] = json!(due.to_string());
        b["contactId"] = contact.clone();
        b
    };
    let customer = base["contactId"].clone();

    // overdue unpaid (A), overdue partial (B), not yet due (C), paid (D), draft (E), cancelled (F)
    let a = id(&create_doc(&app, doc(ago(40), ago(20), &customer)).await);
    let b = id(&create_doc(&app, doc(ago(30), ago(10), &json!(acme))).await);
    let c = id(&create_doc(&app, doc(ago(5), ago(0), &customer)).await);
    let d = id(&create_doc(&app, doc(ago(5), ago(1), &customer)).await);
    let e = create_doc(&app, doc(ago(5), ago(1), &json!(acme))).await;
    let f = id(&create_doc(&app, doc(ago(50), ago(45), &customer)).await);
    for x in [&a, &b, &c, &d, &f] {
        assert_eq!(issue(&app, x).await.0, StatusCode::OK);
    }
    let pay = |doc: String, amount: &'static str| {
        let app = app.clone();
        async move {
            call(
                &app,
                Method::POST,
                &format!("/api/documents/{doc}/payments"),
                Some(json!({ "date": ago(1).to_string(), "amount": amount })),
            )
            .await
        }
    };
    pay(b.clone(), "100").await;
    pay(d.clone(), "1210").await;
    call(
        &app,
        Method::POST,
        &format!("/api/documents/{f}/cancel"),
        None,
    )
    .await;

    let get_num = |id: &str| {
        let app = app.clone();
        let id = id.to_string();
        async move {
            call(&app, Method::GET, &format!("/api/documents/{id}"), None)
                .await
                .1["number"]
                .clone()
        }
    };
    let (na, nb, nc, nd, nf) = (
        get_num(&a).await,
        get_num(&b).await,
        get_num(&c).await,
        get_num(&d).await,
        get_num(&f).await,
    );

    // Ordering: issueDate desc, number desc nulls first, createdAt desc.
    let (all, total) = numbers(&app, "").await;
    assert_eq!(total, 6);
    assert_eq!(
        all,
        vec![
            Value::Null,
            nd.clone(),
            nc.clone(),
            nb.clone(),
            na.clone(),
            nf.clone()
        ]
    );

    assert_eq!(numbers(&app, "?status=draft").await.0, vec![Value::Null]);
    assert_eq!(numbers(&app, "?status=cancelled").await.0, vec![nf.clone()]);
    assert_eq!(
        numbers(&app, "?paymentState=unpaid").await.0,
        vec![nc.clone(), na.clone()]
    );
    assert_eq!(
        numbers(&app, "?paymentState=partial").await.0,
        vec![nb.clone()]
    );
    assert_eq!(
        numbers(&app, "?paymentState=paid").await.0,
        vec![nd.clone()]
    );
    assert_eq!(
        numbers(&app, "?paymentState=overpaid").await.0,
        Vec::<Value>::new()
    );
    assert_eq!(
        numbers(&app, "?overdue=true").await.0,
        vec![nb.clone(), na.clone()]
    );
    assert_eq!(numbers(&app, "?overdue=false").await.1, 4);
    assert_eq!(
        numbers(&app, &format!("?contactId={acme}")).await.0,
        vec![Value::Null, nb.clone()]
    );
    let range = format!("?from={}&to={}", ago(30), ago(5));
    assert_eq!(
        numbers(&app, &range).await.0,
        vec![Value::Null, nd.clone(), nc.clone(), nb.clone()]
    );
    assert_eq!(
        numbers(&app, "?direction=issued&docType=invoice").await.1,
        6
    );
    assert_eq!(numbers(&app, "?docType=proforma").await.1, 0);

    // q: customer name (snapshot for issued, live for drafts), number, VS.
    assert_eq!(
        numbers(&app, "?q=acme").await.0,
        vec![Value::Null, nb.clone()]
    );
    let q = na.as_str().expect("number");
    assert_eq!(numbers(&app, &format!("?q={q}")).await.0, vec![na.clone()]);
    assert_eq!(
        numbers(&app, "?q=%25").await.1,
        0,
        "LIKE metacharacters match literally"
    );

    let (_, page) = call(&app, Method::GET, "/api/documents?limit=2&offset=1", None).await;
    assert_eq!(page["total"], 6);
    assert_eq!(page["items"].as_array().map(Vec::len), Some(2));

    let (_, body) = call(&app, Method::GET, "/api/documents?overdue=true", None).await;
    let first = &body["items"][0];
    assert_eq!(first["customerName"], "ACME Corp");
    assert_eq!(first["paymentState"], "partial");
    assert_eq!(first["overdue"], true);
    assert_eq!(first["payable"], "1210.00");
    assert_eq!(first["paid"], "100.00");
    let (_, body) = call(&app, Method::GET, "/api/documents?status=draft", None).await;
    assert_eq!(body["items"][0]["customerName"], "ACME Corp");
    assert_eq!(body["items"][0]["id"], e["id"]);
    assert_eq!(body["items"][0]["paymentState"], Value::Null);

    for bad in [
        "?status=sent",
        "?paymentState=x",
        "?overdue=maybe",
        "?from=yesterday",
    ] {
        let (status, _) = call(&app, Method::GET, &format!("/api/documents{bad}"), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
}
