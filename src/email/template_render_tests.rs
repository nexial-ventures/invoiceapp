//! Unit tests of `template_render.rs`.

use super::*;
use crate::email::templates::default;
use crate::pdf::format::Locale;
use serde_json::json;

fn ctx(paid: bool, to_pay: bool) -> Value {
    Value::from_serialize(json!({
        "doc": {
            "type": "invoice", "typeLabel": "Faktura – daňový doklad", "number": "20260001",
            "issueDate": "1. 10. 2026", "taxDate": "1. 10. 2026", "dueDate": "15. 10. 2026",
            "total": "1 210,00 Kč", "payable": "1 210,00 Kč", "currency": "CZK",
            "paid": paid, "cancelled": false, "toPay": to_pay, "variableSymbol": "20260001",
            "bankAccount": null, "iban": null, "originalNumber": null
        },
        "company": { "name": "Dodavatel s.r.o.", "email": null, "phone": null, "web": null },
        "contact": null
    }))
}

#[test]
fn defaults_render_on_every_state() {
    for locale in [Locale::Cs, Locale::En] {
        for (paid, to_pay) in [(false, true), (true, false), (false, false)] {
            let r = render(&default(locale), &ctx(paid, to_pay)).expect("renders");
            assert!(r.subject.ends_with("20260001 – Dodavatel s.r.o."), "{r:?}");
            assert!(!r.body.ends_with('\n'));
            assert!(r.body.contains("1 210,00 Kč"));
            assert!(!r.body.contains("\n\n\n"), "{}", r.body);
        }
    }
    let cs = render(&default(Locale::Cs), &ctx(false, true)).expect("renders");
    assert!(
        cs.body
            .contains("uhraďte do 15. 10. 2026 pod variabilním symbolem 20260001.")
    );
    let paid = render(&default(Locale::Cs), &ctx(true, false)).expect("renders");
    assert!(paid.body.contains("Neplaťte – již uhrazeno."));
    assert!(!paid.body.contains("uhraďte"));
}

#[test]
fn none_is_empty_and_no_html_escaping() {
    let t = Template {
        subject: "A{{ doc.bankAccount }}B\n  {{ company.name }} <&>".into(),
        body: "{{ contact }}|{{ '<b>' }}\n\n  ".into(),
    };
    let r = render(&t, &ctx(false, true)).expect("renders");
    assert_eq!(r.subject, "AB Dodavatel s.r.o. <&>");
    assert_eq!(r.body, "|<b>");
}

#[test]
fn errors_name_the_part_and_line() {
    let ctx = ctx(false, true);
    let undefined = Template {
        subject: "ok".into(),
        body: "a\nb\n{{ doc.nope }}".into(),
    };
    match render(&undefined, &ctx) {
        Err(AppError::TemplateInvalid { field, detail }) => {
            assert_eq!(field, "body");
            assert!(detail.starts_with("line 3: undefined value"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    let syntax = Template {
        subject: "{% if %}".into(),
        body: "{{ doc.nope }}".into(),
    };
    assert!(matches!(
        render(&syntax, &ctx),
        Err(AppError::TemplateInvalid {
            field: "subject",
            ..
        })
    ));
    // Attribute of none is undefined too (strict).
    let contact = Template {
        subject: "{{ contact.email }}".into(),
        body: String::new(),
    };
    assert!(render(&contact, &ctx).is_err());
}

#[test]
fn validation_renders_every_sample() {
    let mut base = json!({
        "doc": { "number": "1", "paid": false },
        "contact": { "name": "A", "email": "a@x.cz" }
    });
    let mut samples = vec![("", Value::from_serialize(&base))];
    base["doc"]["paid"] = json!(true);
    samples.push(("paid", Value::from_serialize(&base)));
    base["doc"]["paid"] = json!(false);
    base["contact"] = serde_json::Value::Null;
    samples.push(("without contact", Value::from_serialize(&base)));

    let unguarded = Template {
        subject: "{{ doc.number }}".into(),
        body: "x\n{{ contact.name }}".into(),
    };
    match validate(&unguarded, &samples) {
        Err(AppError::TemplateInvalid { field, detail }) => {
            assert_eq!(field, "body");
            assert!(detail.starts_with("without contact: line 2: "), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    let untaken = Template {
        subject: "{% if doc.paid %}{{ doc.nope }}{% endif %}x".into(),
        body: String::new(),
    };
    assert!(render(&untaken, &samples[0].1).is_ok());
    assert!(matches!(
        validate(&untaken, &samples),
        Err(AppError::TemplateInvalid { field: "subject", detail })
            if detail.starts_with("paid: line 1: undefined value")
    ));
    let guarded = Template {
        subject: "{{ doc.number }}".into(),
        body: "{% if contact %}{{ contact.name }}{% endif %}".into(),
    };
    let r = validate(&guarded, &samples).expect("valid");
    assert_eq!(r.body, "A", "the result is the base sample's rendering");
    // An error on the base sample keeps its plain detail.
    let broken = Template {
        subject: "{{ doc.nope }}".into(),
        body: String::new(),
    };
    assert!(matches!(
        validate(&broken, &samples),
        Err(AppError::TemplateInvalid { field: "subject", detail }) if detail.starts_with("line 1:")
    ));
}

fn invalid_body(r: Result<Rendered, AppError>) -> String {
    match r {
        Err(AppError::TemplateInvalid {
            field: "body",
            detail,
        }) => detail,
        other => panic!("{other:?}"),
    }
}

fn body(body: &str) -> Template {
    Template {
        subject: "ok".into(),
        body: body.into(),
    }
}

#[test]
fn nested_loops_run_out_of_fuel() {
    let t = body("{% for a in range(1000) %}{% for b in range(1000) %}.{% endfor %}{% endfor %}");
    let detail = invalid_body(render(&t, &ctx(false, true)));
    assert_eq!(
        detail,
        format!("line 1: too much work (more than {FUEL} steps)")
    );
}

#[test]
fn huge_output_is_refused() {
    // Few instructions, 2 MB of output.
    let t = body("{% for i in range(100) %}{{ 'x' * 20000 }}{% endfor %}");
    let detail = invalid_body(render(&t, &ctx(false, true)));
    assert_eq!(detail, "the output is larger than 1024 KB");
    // Up to the cap is fine.
    let t = body("{{ 'x' * 1000000 }}");
    let r = render(&t, &ctx(false, true)).expect("under 1 MB");
    assert_eq!(r.body.len(), 1_000_000);
}

#[test]
fn deep_recursion_is_refused() {
    let t = body("{% macro f(n) %}{{ f(n + 1) }}{% endmacro %}{{ f(0) }}");
    let detail = invalid_body(render(&t, &ctx(false, true)));
    assert!(detail.contains("recursion"), "{detail}");
}
