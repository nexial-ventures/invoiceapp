use serde_json::json;

use super::*;
use crate::accounting::settings::AccountingSettings;
use crate::document::doc_type::DocType;
use crate::document::entity::{document, vat_recap};
use crate::document::line::{Direction, PaymentMethod, VatMode};
use crate::export::test_doc::{d, date, doc, recap, snapshot};

fn ctx(settings: AccountingSettings) -> Ctx {
    Ctx {
        ico: Some("44444443".into()),
        settings: settings.full(),
        third: None,
        from: date(3, 1),
        to: date(3, 31),
    }
}

fn source<'a>(
    doc: &'a document::Model,
    recap: &'a [vat_recap::Model],
    parent: Option<&'a document::Model>,
) -> Source<'a> {
    Source {
        doc,
        recap,
        payments: &[],
        parent,
        category: None,
    }
}

/// The item inside a whole file, parsed (well-formedness) and returned.
fn render(s: &Source, c: &Ctx) -> String {
    let item = item(s, c).expect("item");
    let file = format!("{}{item}{TAIL}", head(c));
    roxmltree::Document::parse(&file.replace("encoding=\"Windows-1250\"", ""))
        .expect("well-formed");
    item
}

#[test]
fn agendas() {
    let inv = |t| (Ns::Inv, Some(t));
    let cases = [
        (DocType::Invoice, "issuedInvoice", "receivedInvoice"),
        (DocType::Simplified, "issuedInvoice", "receivedInvoice"),
        (
            DocType::CreditNote,
            "issuedCreditNotice",
            "receivedCreditNotice",
        ),
        (DocType::DebitNote, "issuedDebitNote", "receivedDebitNote"),
    ];
    for (t, issued, received) in cases {
        assert_eq!(agenda(t, true).expect("issued"), inv(issued));
        assert_eq!(agenda(t, false).expect("received"), inv(received));
    }
    for t in [DocType::AdvanceTaxDoc, DocType::AdvanceCreditNote] {
        assert_eq!(agenda(t, true).expect("ddpp"), (Ns::Int, None));
        assert_eq!(agenda(t, false).expect("ddpp"), (Ns::Int, None));
    }
    assert!(agenda(DocType::Proforma, true).is_err());
}

#[test]
fn issued_czk_invoice() {
    let mut doc = doc();
    doc.bank_snapshot =
        Some(json!({ "accountNumber": "19-123456789/0800", "iban": null, "bic": null }));
    let recap = [
        recap(&doc, "21", "1000", "210"),
        recap(&doc, "12", "0.36", "0.04"),
    ];
    let x = render(
        &source(&doc, &recap, None),
        &ctx(AccountingSettings::default()),
    );
    for part in [
        &format!(
            "<dat:dataPackItem id=\"{}\" version=\"2.0\"><inv:invoice version=\"2.0\">",
            doc.id
        ),
        "<inv:invoiceType>issuedInvoice</inv:invoiceType>",
        "<inv:number><typ:numberRequested>2026000001</typ:numberRequested></inv:number>",
        "<inv:symVar>2026000001</inv:symVar><inv:originalDocument>2026000001</inv:originalDocument>",
        "<inv:date>2026-03-01</inv:date><inv:dateTax>2026-03-01</inv:dateTax>\
         <inv:dateAccounting>2026-03-01</inv:dateAccounting><inv:dateDue>2026-03-15</inv:dateDue>",
        "<inv:text>Faktura 2026000001</inv:text>",
        "<inv:partnerIdentity><typ:address><typ:company>Odběratel a.s.</typ:company>\
         <typ:city>Praha</typ:city><typ:street>Vzorová 1</typ:street><typ:zip>11000</typ:zip>\
         <typ:ico>12345679</typ:ico><typ:dic>CZ12345679</typ:dic>\
         <typ:country><typ:ids>CZ</typ:ids></typ:country></typ:address></inv:partnerIdentity>",
        "<inv:paymentType><typ:paymentType>draft</typ:paymentType></inv:paymentType>",
        "<inv:account><typ:accountNo>19-123456789</typ:accountNo><typ:bankCode>0800</typ:bankCode></inv:account>",
        "<inv:roundingDocument>math2one</inv:roundingDocument><inv:homeCurrency>\
         <typ:priceLow>0.36</typ:priceLow><typ:priceLowVAT>0.04</typ:priceLowVAT>\
         <typ:priceHigh>1000.00</typ:priceHigh><typ:priceHighVAT>210.00</typ:priceHighVAT>\
         <typ:round><typ:priceRound>-0.40</typ:priceRound></typ:round></inv:homeCurrency>",
    ] {
        assert!(x.contains(part), "{part}\nin\n{x}");
    }
    assert!(
        !x.contains("accounting>") && !x.contains("foreignCurrency"),
        "{x}"
    );
}

#[test]
fn codes_from_settings() {
    let settings: AccountingSettings = serde_json::from_value(json!({ "pohoda": { "codes": [
        { "direction": "issued", "docType": "invoice", "accounting": "3Fv",
          "classificationVat": "UD", "numberSeries": "FV" }]}}))
    .expect("settings");
    let mut doc = doc();
    doc.payment_method = PaymentMethod::Other;
    let x = render(&source(&doc, &[], None), &ctx(settings));
    assert!(
        x.contains(
            "<inv:number><typ:ids>FV</typ:ids>\
             <typ:numberRequested>2026000001</typ:numberRequested></inv:number>"
        ),
        "{x}"
    );
    assert!(x.contains("<inv:accounting><typ:ids>3Fv</typ:ids></inv:accounting>"));
    assert!(x.contains("<inv:classificationVAT><typ:ids>UD</typ:ids></inv:classificationVAT>"));
    assert!(!x.contains("paymentType"), "other → Pohoda default");
}

#[test]
fn credit_note_negative_with_original() {
    let original = doc();
    let mut note = doc();
    note.doc_type = DocType::CreditNote;
    note.number = Some("D-1".into());
    note.rounding = Decimal::ZERO;
    let recap = [recap(&note, "21", "100", "21")];
    let x = render(
        &source(&note, &recap, Some(&original)),
        &ctx(AccountingSettings::default()),
    );
    assert!(x.contains("<inv:invoiceType>issuedCreditNotice</inv:invoiceType>"));
    assert!(
        x.contains("<inv:text>Dobropis D-1 k 2026000001</inv:text>"),
        "{x}"
    );
    assert!(x.contains(
        "<typ:priceHigh>-100.00</typ:priceHigh><typ:priceHighVAT>-21.00</typ:priceHighVAT>"
    ));
    assert!(x.contains("<typ:priceRound>0.00</typ:priceRound>"));
}

#[test]
fn received_eur_invoice() {
    let mut doc = doc();
    doc.direction = Direction::Received;
    doc.supplier_number = Some("FV-42".into());
    doc.currency = "EUR".into();
    doc.exchange_rate = Some(d("24.335000"));
    doc.total = d("121");
    doc.rounding = Decimal::ZERO;
    doc.bank_snapshot = Some(json!({ "accountNumber": "1/0100" }));
    doc.supplier_snapshot = Some(snapshot("Vzorový Dodavatel a.s.", "87654326"));
    let mut r = recap(&doc, "21", "100", "21");
    r.base_czk = Some(d("2433.50"));
    r.vat_czk = Some(d("511.04"));
    let x = render(
        &source(&doc, &[r], None),
        &ctx(AccountingSettings::default()),
    );
    for part in [
        "<inv:invoiceType>receivedInvoice</inv:invoiceType>",
        "<inv:originalDocument>FV-42</inv:originalDocument>",
        "<inv:dateAccounting>2026-03-02</inv:dateAccounting>",
        "<inv:text>Přijatá faktura FV-42</inv:text>",
        "<typ:company>Vzorový Dodavatel a.s.</typ:company>",
        "<typ:priceHigh>2433.50</typ:priceHigh><typ:priceHighVAT>511.04</typ:priceHighVAT></inv:homeCurrency>",
        "<inv:foreignCurrency><typ:currency><typ:ids>EUR</typ:ids></typ:currency>\
         <typ:rate>24.335</typ:rate><typ:amount>1</typ:amount><typ:priceSum>121.00</typ:priceSum>",
    ] {
        assert!(x.contains(part), "{part}\nin\n{x}");
    }
    assert!(!x.contains("inv:account>"), "received: no own account");
    assert!(
        x.contains("<inv:roundingDocument>none</inv:roundingDocument>"),
        "{x}"
    );
}

#[test]
fn non_deductible_received() {
    let mut doc = doc();
    doc.direction = Direction::Received;
    doc.vat_deductible = false;
    let settings = |nd: Option<&str>| -> AccountingSettings {
        serde_json::from_value(json!({ "pohoda": { "codes": [
            { "direction": "received", "docType": "invoice", "classificationVat": "PD",
              "classificationVatNonDeductible": nd }]}}))
        .expect("settings")
    };
    let src = source(&doc, &[], None);
    let x = render(&src, &ctx(settings(Some("PN"))));
    assert!(
        x.contains("<inv:classificationVAT><typ:ids>PN</typ:ids></inv:classificationVAT>"),
        "{x}"
    );
    let x = render(&src, &ctx(settings(None)));
    assert!(
        x.contains(
            "<inv:classificationVAT><typ:classificationVATType>nonSubsume\
             </typ:classificationVATType></inv:classificationVAT>"
        ),
        "never the deductible code, never nothing: {x}"
    );
    doc.vat_deductible = true;
    let x = render(&source(&doc, &[], None), &ctx(settings(Some("PN"))));
    assert!(
        x.contains("<typ:ids>PD</typ:ids>") && !x.contains("PN"),
        "{x}"
    );
    // Odd rounding no Pohoda mode reproduces: roundingDocument left out.
    doc.rounding = d("0.60");
    let x = render(&source(&doc, &[], None), &ctx(settings(None)));
    assert!(
        !x.contains("roundingDocument") && x.contains("<typ:priceRound>0.60<"),
        "{x}"
    );
}

#[test]
fn ddpp_goes_to_internal_documents() {
    let mut doc = doc();
    doc.doc_type = DocType::AdvanceCreditNote;
    doc.number = Some("ODD-1".into());
    doc.customer_snapshot = None;
    let x = render(
        &source(&doc, &[recap(&doc, "21", "10", "2.10")], None),
        &ctx(AccountingSettings::default()),
    );
    assert!(
        x.contains("<int:intDoc version=\"2.0\"><int:intDocHeader>"),
        "{x}"
    );
    assert!(
        x.contains("<int:number><typ:numberRequested>ODD-1</typ:numberRequested></int:number>")
    );
    assert!(x.contains("<int:text>Opravný daňový doklad k platbě ODD-1</int:text>"));
    assert!(x.contains("<int:intDocSummary><int:roundingDocument>math2one</int:roundingDocument>"));
    assert!(x.contains("<typ:priceHigh>-10.00</typ:priceHigh>"));
    for absent in [
        "invoiceType",
        "paymentType",
        "originalDocument",
        "dateDue",
        "partnerIdentity",
    ] {
        assert!(!x.contains(absent), "{absent}");
    }
}

#[test]
fn vat_free_modes_and_skips() {
    let mut doc = doc();
    doc.vat_mode = VatMode::NonPayer;
    let x = render(
        &source(&doc, &[recap(&doc, "21", "500", "0")], None),
        &ctx(AccountingSettings::default()),
    );
    assert!(
        x.contains("<typ:priceNone>500.00</typ:priceNone>") && !x.contains("priceHigh"),
        "{x}"
    );

    doc.vat_mode = VatMode::Standard;
    let ten = [recap(&doc, "10", "100", "10")];
    let c = ctx(AccountingSettings::default());
    let err = item(&source(&doc, &ten, None), &c).expect_err("unmappable");
    assert!(format!("{err:#}").contains("10 %"), "{err:#}");
    let c = Ctx {
        third: Some(d("10")),
        ..c
    };
    assert!(render(&source(&doc, &ten, None), &c).contains("<typ:price3>100.00</typ:price3>"));

    doc.number = Some("X".repeat(33));
    assert!(
        item(&source(&doc, &[], None), &c).is_err(),
        "number too long"
    );
}

#[test]
fn head_and_encoding() {
    let mut c = ctx(AccountingSettings::default());
    let h = head(&c);
    assert!(
        h.starts_with("<?xml version=\"1.0\" encoding=\"Windows-1250\"?>\n<dat:dataPack "),
        "{h}"
    );
    for part in [
        "xmlns:inv=\"http://www.stormware.cz/schema/version_2/invoice.xsd\"",
        "id=\"invoice-2026-03-01-2026-03-31\" ico=\"44444443\" application=\"InvoiceApp\" version=\"2.0\"",
        "note=\"Export z Invoice 2026-03-01–2026-03-31\">",
    ] {
        assert!(h.contains(part), "{part}");
    }
    c.ico = None;
    assert!(!head(&c).contains("ico="));
    // Czech letters and € are in 1250; Cyrillic is not → a character reference.
    assert_eq!(encode("Žluť €"), [0x8e, b'l', b'u', 0x9d, b' ', 0x80]);
    assert_eq!(encode("Ж<"), b"&#1046;<");
}
