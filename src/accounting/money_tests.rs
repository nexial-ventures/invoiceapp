use serde_json::json;

use super::*;
use crate::accounting::doc::amount;
use crate::accounting::settings::AccountingSettings;
use crate::document::doc_type::DocType;
use crate::document::entity::{document, vat_recap};
use crate::document::line::{Direction, PaymentMethod, VatMode};
use crate::export::test_doc::{d, date, doc, recap, snapshot};

fn ctx(settings: serde_json::Value) -> Ctx {
    let settings: AccountingSettings = serde_json::from_value(settings).expect("settings");
    Ctx {
        ico: Some("44444443".into()),
        settings: settings.full(),
        third: None,
        from: date(3, 1),
        to: date(3, 31),
    }
}

fn plain() -> Ctx {
    ctx(json!({}))
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

/// The item inside a whole file, parsed (well-formedness); its list and
/// text.
fn render(s: &Source, c: &Ctx) -> (List, String) {
    let (list, item) = item(s, c).expect("item");
    let mut lists = Lists::default();
    lists.push(list, &item);
    roxmltree::Document::parse(&lists.finish(c, date(4, 1))).expect("well-formed");
    (list, item)
}

fn has(x: &str, parts: &[&str]) {
    for p in parts {
        assert!(x.contains(p), "{p}\nin\n{x}");
    }
}

#[test]
fn lists() {
    let t = [
        DocType::Invoice,
        DocType::Simplified,
        DocType::CreditNote,
        DocType::DebitNote,
    ];
    for t in t {
        assert_eq!(List::of(t, true).expect("issued"), List::Issued);
        assert_eq!(List::of(t, false).expect("received"), List::Received);
    }
    for t in [DocType::AdvanceTaxDoc, DocType::AdvanceCreditNote] {
        assert_eq!(List::of(t, true).expect("ddpp"), List::IssuedDpp);
        assert_eq!(List::of(t, false).expect("ddpp"), List::ReceivedDpp);
    }
    assert!(List::of(DocType::Proforma, true).is_err());
}

#[test]
fn issued_czk_invoice() {
    let doc = doc();
    let recap = [
        recap(&doc, "21", "1000", "210"),
        recap(&doc, "12", "0.36", "0.04"),
    ];
    let (list, x) = render(&source(&doc, &recap, None), &plain());
    assert_eq!(list, List::Issued);
    assert_eq!(
        x,
        "<FaktVyd><Doklad>2026000001</Doklad><EvCisDokl>2026000001</EvCisDokl>\
         <Popis>Faktura 2026000001</Popis><Vystaveno>2026-03-01</Vystaveno>\
         <DatUcPr>2026-03-01</DatUcPr><PlnenoDPH>2026-03-01</PlnenoDPH>\
         <Splatno>2026-03-15</Splatno><VarSymbol>2026000001</VarSymbol><Druh>N</Druh>\
         <Uhrada>převodem</Uhrada><SazbaDPH1>12</SazbaDPH1><SazbaDPH2>21</SazbaDPH2>\
         <SouhrnDPH><Zaklad0>-0.40</Zaklad0><Zaklad5>0.36</Zaklad5><Zaklad22>1000.00</Zaklad22>\
         <DPH5>0.04</DPH5><DPH22>210.00</DPH22></SouhrnDPH><Celkem>1210.00</Celkem>\
         <DodOdb><ObchNazev>Odběratel a.s.</ObchNazev><ObchAdresa><Ulice>Vzorová 1</Ulice>\
         <Misto>Praha</Misto><PSC>11000</PSC><KodStatu>CZ</KodStatu></ObchAdresa>\
         <ICO>12345679</ICO><DIC>CZ12345679</DIC></DodOdb></FaktVyd>\n"
    );
    let payable = format!("<Celkem>{}</Celkem>", amount(doc.payable));
    assert!(x.contains(&payable), "Money's total = our payable");
}

#[test]
fn codes_and_a_long_number() {
    let c = ctx(json!({ "money": { "codes": [
        { "direction": "issued", "docType": "invoice", "accounting": "3Fv",
          "classificationVat": "UD", "numberSeries": "FV" }]}}));
    let mut doc = doc();
    doc.payment_method = PaymentMethod::Other;
    let (_, x) = render(&source(&doc, &[], None), &c);
    has(
        &x,
        &[
            "<Doklad>2026000001</Doklad><EvCisDokl>2026000001</EvCisDokl><Rada>FV</Rada>",
            "<KodDPH>UD</KodDPH>",
            "<PredKontac>3Fv</PredKontac>",
        ],
    );
    assert!(!x.contains("Uhrada") && !x.contains("Dobropis"), "{x}");

    // Longer than 10: no Doklad (Money numbers it from Rada), ours stays in
    // EvCisDokl.
    doc.number = Some("FA-2026-00001".into());
    let (_, x) = render(&source(&doc, &[], None), &c);
    assert!(
        x.starts_with("<FaktVyd><EvCisDokl>FA-2026-00001</EvCisDokl><Rada>FV</Rada>"),
        "{x}"
    );
    doc.number = Some("1234567890".into());
    assert!(
        render(&source(&doc, &[], None), &c)
            .1
            .contains("<Doklad>1234567890</Doklad>")
    );
}

#[test]
fn credit_note_negative_with_original() {
    let original = doc();
    let mut note = doc();
    note.doc_type = DocType::CreditNote;
    note.number = Some("D-1".into());
    note.rounding = Decimal::ZERO;
    let recap = [recap(&note, "21", "100", "21")];
    let (_, x) = render(&source(&note, &recap, Some(&original)), &plain());
    has(
        &x,
        &[
            "<Popis>Dobropis D-1 k 2026000001</Popis>",
            "<Druh>N</Druh><Dobropis>true</Dobropis>",
            "<SouhrnDPH><Zaklad22>-100.00</Zaklad22><DPH22>-21.00</DPH22></SouhrnDPH>\
             <Celkem>-121.00</Celkem>",
        ],
    );
}

#[test]
fn ddpp_lists() {
    let mut doc = doc();
    doc.doc_type = DocType::AdvanceCreditNote;
    doc.number = Some("ODD-1".into());
    doc.customer_snapshot = None;
    let (list, x) = render(
        &source(&doc, &[recap(&doc, "21", "10", "2.10")], None),
        &plain(),
    );
    assert_eq!(list, List::IssuedDpp);
    has(
        &x,
        &[
            "<FaktVyd_DPP><Doklad>ODD-1</Doklad>",
            "<Popis>Opravný daňový doklad k platbě ODD-1</Popis>",
            "<Druh>D</Druh><Dobropis>true</Dobropis>",
            "<Zaklad0>0.40</Zaklad0><Zaklad22>-10.00</Zaklad22><DPH22>-2.10</DPH22>",
            "<Celkem>-11.70</Celkem></FaktVyd_DPP>",
        ],
    );
    assert!(!x.contains("DodOdb"), "no snapshot: {x}");
    doc.doc_type = DocType::AdvanceTaxDoc;
    doc.direction = Direction::Received;
    let (list, x) = render(&source(&doc, &[], None), &plain());
    assert_eq!(list, List::ReceivedDpp);
    has(&x, &["<FaktPrij_DPP>", "<Druh>D</Druh>"]);
    assert!(!x.contains("Dobropis"));
}

#[test]
fn received_eur_invoice() {
    let mut doc = doc();
    doc.direction = Direction::Received;
    doc.number = Some("PF-7".into());
    doc.supplier_number = Some("FV-42".into());
    doc.currency = "EUR".into();
    doc.exchange_rate = Some(d("24.335125"));
    doc.total = d("121");
    doc.rounding = Decimal::ZERO;
    doc.supplier_snapshot = Some(snapshot("Vzorový Dodavatel a.s.", "87654326"));
    let mut r = recap(&doc, "21", "100", "21");
    r.base_czk = Some(d("2433.50"));
    r.vat_czk = Some(d("511.04"));
    let (list, x) = render(&source(&doc, &[r], None), &plain());
    assert_eq!(list, List::Received);
    has(
        &x,
        &[
            "<FaktPrij><Doklad>PF-7</Doklad><Popis>Přijatá faktura FV-42</Popis>",
            "<DatUcPr>2026-03-02</DatUcPr><PlnenoDPH>2026-03-01</PlnenoDPH>",
            "<Doruceno>2026-03-02</Doruceno>",
            "<PrijatDokl>FV-42</PrijatDokl>",
            "<SouhrnDPH><Zaklad22>2433.50</Zaklad22><DPH22>511.04</DPH22></SouhrnDPH>\
             <Celkem>2944.54</Celkem>",
            "<Valuty><Mena><Kod>EUR</Kod><Mnozstvi>1</Mnozstvi><Kurs>24.3351</Kurs></Mena>\
             <SouhrnDPH><Zaklad22>100.00</Zaklad22><DPH22>21.00</DPH22></SouhrnDPH>\
             <Celkem>121.00</Celkem></Valuty>",
            "<ObchNazev>Vzorový Dodavatel a.s.</ObchNazev>",
        ],
    );
    assert!(!x.contains("EvCisDokl"), "issued only: {x}");

    // Neither a CZK amount nor a rate → unexportable.
    doc.exchange_rate = None;
    let bare = [recap(&doc, "21", "100", "21")];
    assert!(item(&source(&doc, &bare, None), &plain()).is_err());
}

#[test]
fn foreign_rounding_and_implied_rate() {
    let mut doc = doc();
    doc.currency = "EUR".into();
    doc.exchange_rate = Some(d("25"));
    doc.total = d("120.60");
    doc.rounding = d("0.40");
    let mut r = recap(&doc, "21", "100", "20.60");
    r.base_czk = Some(d("2500"));
    r.vat_czk = Some(d("515"));
    let (_, x) = render(&source(&doc, std::slice::from_ref(&r), None), &plain());
    has(
        &x,
        &[
            "<SouhrnDPH><Zaklad0>10.00</Zaklad0><Zaklad22>2500.00</Zaklad22>\
             <DPH22>515.00</DPH22></SouhrnDPH><Celkem>3025.00</Celkem>",
            "<Kurs>25</Kurs></Mena><SouhrnDPH><Zaklad0>0.40</Zaklad0>\
             <Zaklad22>100.00</Zaklad22><DPH22>20.60</DPH22></SouhrnDPH><Celkem>121.00</Celkem>",
        ],
    );

    // No stored rate but CZK amounts: the implied rate (3015 / 120.60).
    doc.exchange_rate = None;
    doc.rounding = Decimal::ZERO;
    let (_, x) = render(&source(&doc, &[r], None), &plain());
    assert!(x.contains("<Kurs>25</Kurs>"), "{x}");
    assert_eq!(kurs(None, d("2944.54"), d("121")), Some(d("24.335")));
    assert_eq!(
        kurs(Some(d("24.335125")), d("1"), d("1")),
        Some(d("24.3351"))
    );
    assert_eq!(kurs(None, d("0"), d("0")), None);
}

#[test]
fn blank_partner_ids_left_out() {
    let mut doc = doc();
    doc.customer_snapshot = Some(json!({ "name": "Bez IČO", "ico": "", "dic": "  ",
        "street": "", "city": "", "zip": "", "country": "", "registration": null,
        "vatPayer": true }));
    let (_, x) = render(&source(&doc, &[], None), &plain());
    assert!(
        x.contains("<DodOdb><ObchNazev>Bez IČO</ObchNazev><PlatceDPH>true</PlatceDPH></DodOdb>"),
        "{x}"
    );
}

#[test]
fn non_deductible_received() {
    let mut doc = doc();
    doc.direction = Direction::Received;
    doc.vat_deductible = false;
    let c = |nd: Option<&str>| {
        ctx(json!({ "money": { "codes": [
            { "direction": "received", "docType": "invoice", "classificationVat": "PD",
              "classificationVatNonDeductible": nd }]}}))
    };
    let src = source(&doc, &[], None);
    assert!(
        render(&src, &c(Some("PN")))
            .1
            .contains("<KodDPH>PN</KodDPH>")
    );
    let (_, x) = render(&src, &c(None));
    assert!(!x.contains("KodDPH"), "never the deductible code: {x}");
    doc.vat_deductible = true;
    let (_, x) = render(&source(&doc, &[], None), &c(Some("PN")));
    assert!(x.contains("<KodDPH>PD</KodDPH>"), "{x}");
}

#[test]
fn vat_free_other_rates_and_limits() {
    let mut doc = doc();
    doc.vat_mode = VatMode::NonPayer;
    doc.rounding = Decimal::ZERO;
    let (_, x) = render(
        &source(&doc, &[recap(&doc, "21", "500", "0")], None),
        &plain(),
    );
    assert!(
        x.contains("<SouhrnDPH><Zaklad0>500.00</Zaklad0></SouhrnDPH>"),
        "{x}"
    );

    doc.vat_mode = VatMode::Standard;
    doc.doc_type = DocType::Simplified;
    let (_, x) = render(
        &source(&doc, &[recap(&doc, "10", "100", "10")], None),
        &plain(),
    );
    has(
        &x,
        &[
            "<ZjednD>true</ZjednD>",
            "<SouhrnDPH><Zaklad0>0.00</Zaklad0><SeznamDalsiSazby><DalsiSazba>\
             <HladinaDPH>1</HladinaDPH><Sazba>10</Sazba><Zaklad>100.00</Zaklad>\
             <DPH>10.00</DPH></DalsiSazba></SeznamDalsiSazby>",
        ],
    );

    doc.variable_symbol = Some("1".repeat(21));
    let e = item(&source(&doc, &[], None), &plain()).expect_err("long VS");
    assert!(
        e.to_string().starts_with("variable symbol longer than 20"),
        "{e}"
    );
}

#[test]
fn head_and_file() {
    let mut c = plain();
    let h = head(&c, date(4, 1));
    assert_eq!(
        h,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MoneyData ICAgendy=\"44444443\" \
         description=\"Export z Invoice 2026-03-01–2026-03-31\" ExpDate=\"2026-04-01\">\n"
    );
    c.ico = None;
    let mut lists = Lists::default();
    lists.push(List::Issued, "<FaktVyd></FaktVyd>\n");
    let f = lists.finish(&c, date(4, 1));
    assert!(f.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MoneyData description="));
    assert!(
        f.ends_with("<SeznamFaktVyd>\n<FaktVyd></FaktVyd>\n</SeznamFaktVyd>\n</MoneyData>\n"),
        "{f}"
    );
    assert!(!f.contains("SeznamFaktPrij"), "empty lists not written");
}
