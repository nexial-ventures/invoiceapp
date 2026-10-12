use super::*;

fn q(from: Option<&str>, to: Option<&str>, direction: Option<&str>) -> AccountantQuery {
    AccountantQuery {
        from: from.map(str::to_string),
        to: to.map(str::to_string),
        direction: direction.map(str::to_string),
        format: None,
    }
}

fn fields(r: Result<Accountant, AppError>) -> Vec<(String, &'static str)> {
    match r {
        Err(AppError::Validation(e)) => {
            let mut v: Vec<_> = ["from", "to", "direction", "format"]
                .into_iter()
                .filter_map(|f| e.get(f).map(|r| (f.to_string(), r)))
                .collect();
            v.sort();
            v
        }
        other => panic!("expected validation, got {other:?}"),
    }
}

#[test]
fn accountant_period() {
    let date = |m, d| NaiveDate::from_ymd_opt(2026, m, d).expect("date");
    assert_eq!(
        q(Some("2026-01-01"), Some("2026-01-31"), None)
            .check()
            .expect("ok"),
        Accountant {
            from: date(1, 1),
            to: date(1, 31),
            direction: None,
            format: AccountantFormat::Csv,
        }
    );
    let one_day = q(Some("2026-03-05"), Some("2026-03-05"), Some("received"));
    assert_eq!(one_day.check().expect("ok").direction, Some(RECEIVED));
    assert!(
        q(Some("2026-01-01"), Some("2027-01-02"), Some("both"))
            .check()
            .is_ok()
    );
    let inv = |f: &str| (f.to_string(), "invalid");
    assert_eq!(
        fields(q(Some("2026-01-01"), Some("2027-01-03"), None).check()),
        [inv("from"), inv("to")],
        "367 days"
    );
    assert_eq!(
        fields(q(Some("2026-02-01"), Some("2026-01-31"), None).check()),
        [inv("from"), inv("to")]
    );
    assert_eq!(
        fields(q(None, Some("31.01.2026"), Some("x")).check()),
        [inv("direction"), inv("from"), inv("to")]
    );
}

#[test]
fn accountant_formats() {
    let with = |f: &str| AccountantQuery {
        format: Some(f.into()),
        ..q(Some("2026-01-01"), Some("2026-01-31"), None)
    };
    assert_eq!(
        with("pohoda").check().expect("ok").format,
        AccountantFormat::Xml(Program::Pohoda)
    );
    assert_eq!(
        with(" money").check().expect("ok").format,
        AccountantFormat::Xml(Program::Money)
    );
    assert_eq!(
        with(" csv ").check().expect("ok").format,
        AccountantFormat::Csv
    );
    for bad in ["Money", "xml", ""] {
        assert_eq!(
            fields(with(bad).check()),
            [("format".to_string(), "invalid")]
        );
    }
}

#[test]
fn list_directions() {
    assert_eq!(list_direction(Some("issued")).ok(), Some(ISSUED));
    assert_eq!(list_direction(Some("received")).ok(), Some(RECEIVED));
    assert!(list_direction(None).is_err());
    assert!(list_direction(Some("both")).is_err());
}
