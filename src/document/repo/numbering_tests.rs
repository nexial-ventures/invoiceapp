use super::*;
use crate::document::line::Status;
use crate::export::test_doc::doc;

fn draft(number: Option<&str>, unlocked: Option<&str>, imported: bool) -> document::Model {
    let mut d = doc();
    d.status = Status::Draft;
    d.number = number.map(str::to_string);
    d.unlocked_number = unlocked.map(str::to_string);
    d.number_year = Some(2025);
    d.number_seq = Some(7);
    d.number_code = Some("ACME".into());
    d.imported = imported;
    d
}

fn numbered(number: &str, year: i32, seq: Option<i32>, code: Option<&str>) -> Numbered {
    Numbered {
        number: number.into(),
        year,
        seq,
        code: code.map(str::to_string),
        manual: false,
    }
}

#[test]
fn fresh_native_draft_allocates() {
    let p = plan(&draft(None, None, false), 2026).expect("plan");
    assert_eq!(p, Plan::Allocate);
}

#[test]
fn unlocked_draft_keeps_its_number_year_seq_and_code() {
    // Re-dated into another year: the number still belongs to 2025.
    let p = plan(
        &draft(Some("ACME-2025007"), Some("ACME-2025007"), false),
        2026,
    );
    assert_eq!(
        p.expect("plan"),
        Plan::Keep(numbered("ACME-2025007", 2025, Some(7), Some("ACME")))
    );
}

#[test]
fn any_other_native_number_is_manual() {
    let edited = plan(&draft(Some("X-1"), Some("ACME-2025007"), false), 2026);
    assert_eq!(edited.expect("plan"), Plan::Manual("X-1".into()));
    let fresh = plan(&draft(Some("20260003"), None, false), 2026);
    assert_eq!(fresh.expect("plan"), Plan::Manual("20260003".into()));
}

#[test]
fn imported_keeps_its_number_without_a_sequence() {
    let mut d = draft(Some("EXT-1"), None, true);
    d.number_year = None;
    assert_eq!(
        plan(&d, 2026).expect("plan"),
        Plan::Keep(numbered("EXT-1", 2026, None, None))
    );
    d.number_year = Some(2025);
    assert_eq!(
        plan(&d, 2026).expect("plan"),
        Plan::Keep(numbered("EXT-1", 2025, None, None))
    );
    d.number = None;
    assert!(plan(&d, 2026).is_err());
}

#[test]
fn unmatched_manual_number_has_no_sequence() {
    let m = Matched {
        year: 2025,
        seq: 4,
        code: None,
    };
    let p = placed("20250004".into(), Some(m), 2026, true);
    assert_eq!((p.year, p.seq, p.manual), (2025, Some(4), true));
    let p = placed("RUČNĚ".into(), None, 2026, true);
    assert_eq!((p.year, p.seq, p.code), (2026, None, None));
}

#[test]
fn reissue_tracks_the_replaced_number_and_derived_symbol() {
    // Unchanged: nothing replaced, the symbol stays.
    assert_eq!(
        reissue_symbols(Some("20260001"), "20260001", Some("20260001")),
        (None, "20260001".to_string())
    );
    // Renumbered: the old number is kept, a derived symbol follows.
    assert_eq!(
        reissue_symbols(Some("20260001"), "20260005", Some("20260001")),
        (Some("20260001".into()), "20260005".to_string())
    );
    // A hand-entered symbol is kept.
    assert_eq!(
        reissue_symbols(Some("20260001"), "20260005", Some("777")),
        (Some("20260001".into()), "777".to_string())
    );
    // A first issue derives an empty symbol.
    assert_eq!(
        reissue_symbols(None, "ACME-2026001", None),
        (None, "2026001".to_string())
    );
    assert_eq!(
        reissue_symbols(None, "ACME-2026001", Some("2026001")),
        (None, "2026001".to_string())
    );
}
