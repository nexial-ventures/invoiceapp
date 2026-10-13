//! Removed and renamed settings refuse the start.

use super::tests::env;
use super::*;

#[test]
fn leftover_design_dir_is_rejected() {
    let err = Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__DESIGN_DIR", "/srv/design"),
    ]))
    .expect_err("removed setting");
    let msg = err.to_string();
    assert!(msg.contains("INVOICEAPP__DESIGN_DIR"), "{msg}");
    assert!(msg.contains("invoiceapp storage migrate"), "{msg}");
    Config::from_source(env(&[
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
        ("INVOICEAPP__DESIGN_DIR", ""),
    ]))
    .expect("an empty leftover is unset");
}

#[test]
fn old_prefix_refuses_the_start() {
    let base = [
        ("INVOICEAPP__DATABASE_URL", "postgres://x"),
        ("INVOICEAPP__PUBLIC_URL", "http://localhost:3000"),
    ];
    let mut pairs = base.to_vec();
    pairs.extend([
        ("INVOICE__SMTP__HOST", "mx"),
        ("invoice__bind", ""),
        ("INVOICE__DESIGN_DIR", "/d"),
    ]);
    let msg = format!(
        "{:#}",
        Config::from_source(env(&pairs)).expect_err("old prefix")
    );
    for part in [
        "INVOICE__SMTP__HOST → INVOICEAPP__SMTP__HOST",
        "invoice__bind → INVOICEAPP__bind",
    ] {
        assert!(msg.contains(part), "{msg}");
    }
    assert!(msg.contains("INVOICE__DESIGN_DIR (removed"), "{msg}");
    let err =
        DbConfig::from_source(env(&[("INVOICE__DATABASE_URL", "postgres://x")])).expect_err("db");
    assert!(
        format!("{err:#}").contains("INVOICE__DATABASE_URL → INVOICEAPP__DATABASE_URL"),
        "{err:#}"
    );
    // INVOICEAPP__ itself and look-alikes are not the old prefix.
    pairs = base.to_vec();
    pairs.extend([("INVOICE_X", "1"), ("INVOICES__X", "1")]);
    Config::from_source(env(&pairs)).expect("no old prefix");
}
