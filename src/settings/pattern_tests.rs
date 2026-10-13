use super::*;

fn format_number(pattern: &str, year: i32, n: i64) -> Result<String, InvalidPattern> {
    Pattern::parse(pattern).map(|p| p.format(year, n, None))
}

fn matched(year: i32, seq: i32, code: Option<&str>) -> Option<Matched> {
    Some(Matched {
        year,
        seq,
        code: code.map(str::to_string),
    })
}

#[test]
fn formats_seeded_patterns() {
    assert_eq!(
        format_number("{YYYY}{NNNN}", 2026, 1),
        Ok("20260001".into())
    );
    assert_eq!(
        format_number("D{YYYY}{NNNN}", 2026, 42),
        Ok("D20260042".into())
    );
    assert_eq!(
        format_number("DP{YYYY}{NNNN}", 2026, 7),
        Ok("DP20260007".into())
    );
    assert_eq!(
        format_number("FV-{YY}/{NNN}", 2026, 5),
        Ok("FV-26/005".into())
    );
    assert_eq!(format_number("{YY}{N}", 2005, 3), Ok("053".into()));
    assert_eq!(
        format_number("{YY}{NNNNNNNNN}", 2026, 1),
        Ok("26000000001".into())
    );
}

#[test]
fn sequence_overflow_keeps_all_digits() {
    assert_eq!(
        format_number("{YYYY}{NN}", 2026, 12345),
        Ok("202612345".into())
    );
}

#[test]
fn year_is_required_and_literals_pass_through() {
    assert_eq!(
        format_number("A_b-1/{YY}/{NNN}", 2026, 9),
        Ok("A_b-1/26/009".into())
    );
    assert_eq!(format_number("A_b-1/{NNN}", 2026, 9), Err(InvalidPattern));
    assert_eq!(
        format_number("{YYYY}{YY}{NNN}", 2026, 9),
        Err(InvalidPattern)
    );
}

#[test]
fn rejects_invalid_patterns() {
    for bad in [
        "",
        "{YYYY}",
        "{NNNN}{NN}",
        "{NNNN}",
        "{YY}{YY}{N}",
        "{YY}{NNNNNNNNNN}",
        "{N",
        "{}",
        "{YYY}{NNN}",
        "{YY}{nnn}",
        "{YY}{X}{NNN}",
        "{YY}FA {NNN}",
        "{YY}FA.{NNN}",
        "{YY}Č{NNN}",
        "{YY}}{NNN}",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA{YY}{NNN}",
    ] {
        assert_eq!(Pattern::parse(bad), Err(InvalidPattern), "{bad:?}");
    }
}

#[test]
fn accepts_max_length() {
    let p = format!("{}{{YY}}{{N}}", "A".repeat(MAX_LEN - 7));
    assert_eq!(p.chars().count(), MAX_LEN);
    assert!(Pattern::parse(&p).is_ok());
    let p = format!("A{p}");
    assert_eq!(Pattern::parse(&p), Err(InvalidPattern));
}

#[test]
fn code_token_only_in_client_patterns() {
    assert_eq!(Pattern::parse("{C}-{YYYY}{NNN}"), Err(InvalidPattern));
    assert_eq!(Pattern::parse_client("{YYYY}{NNN}"), Err(InvalidPattern));
    assert_eq!(
        Pattern::parse_client("{C}{C}{YYYY}{NNN}"),
        Err(InvalidPattern)
    );
    assert_eq!(Pattern::parse_client("{C}-{NNN}"), Err(InvalidPattern));
    let p = Pattern::parse_client("{C}-{YYYY}{NNN}").expect("client pattern");
    assert_eq!(p.format(2026, 1, Some("Acme")), "Acme-2026001");
    assert_eq!(p.format(2026, 1234, Some("B")), "B-20261234");
}

#[test]
fn codes_are_validated() {
    for ok in ["A", "acme", "AB-12/x_", "0123456789"] {
        assert!(is_valid_code(ok), "{ok}");
    }
    for bad in ["", "ACME CORP", "01234567890", "Č", "A.B"] {
        assert!(!is_valid_code(bad), "{bad}");
    }
}

#[test]
fn matches_global_numbers() {
    let p = Pattern::parse("{YYYY}{NNNN}").expect("pattern");
    assert_eq!(p.matches("20260042", None), matched(2026, 42, None));
    // Overflow keeps all digits, so a wider sequence still matches.
    assert_eq!(p.matches("202612345", None), matched(2026, 12345, None));
    // Not what the pattern renders: too short, extra zeros, junk.
    for bad in ["2026042", "2026000042", "X20260042", "20260042X", ""] {
        assert_eq!(p.matches(bad, None), None, "{bad}");
    }
    let p = Pattern::parse("FV-{YY}/{NNN}").expect("pattern");
    assert_eq!(p.matches("FV-26/005", None), matched(2026, 5, None));
    assert_eq!(p.matches("FV-26/5", None), None);
    let p = Pattern::parse("{NNN}/{YYYY}").expect("pattern");
    assert_eq!(p.matches("1234/2026", None), matched(2026, 1234, None));
}

#[test]
fn matches_client_numbers() {
    let p = Pattern::parse_client("{C}-{YYYY}{NNN}").expect("pattern");
    assert_eq!(
        p.matches("ACME-2026001", None),
        matched(2026, 1, Some("ACME"))
    );
    // A code may itself contain the separator.
    assert_eq!(
        p.matches("AC-ME-2026001", None),
        matched(2026, 1, Some("AC-ME"))
    );
    // Pinned to the customer's code, case-insensitively, as written.
    assert_eq!(
        p.matches("acme-2026007", Some("ACME")),
        matched(2026, 7, Some("acme"))
    );
    assert_eq!(p.matches("BETA-2026007", Some("ACME")), None);
    // A code is at most ten characters.
    assert_eq!(p.matches("ABCDEFGHIJK-2026001", None), None);
    let p = Pattern::parse_client("{YY}{C}{NNN}").expect("pattern");
    assert_eq!(p.matches("26AB012", None), matched(2026, 12, Some("AB")));
    assert_eq!(
        p.matches("26AB012", Some("ab")),
        matched(2026, 12, Some("AB"))
    );
}

#[test]
fn matching_never_panics_on_odd_input() {
    let p = Pattern::parse_client("{C}{YYYY}{NNN}").expect("pattern");
    for odd in ["Č2026001", "ŽŽŽ", "{C}", "2026001", "A"] {
        let _ = p.matches(odd, Some("Č"));
        let _ = p.matches(odd, Some("AŽ"));
        let _ = p.matches(odd, None);
    }
}

#[test]
fn prefix_is_the_rendering_before_the_sequence() {
    let p = Pattern::parse_client("{C}-{YYYY}{NNN}").expect("pattern");
    assert_eq!(p.prefix(2026, Some("ACME")), "ACME-2026");
    let p = Pattern::parse("{NNN}/{YYYY}").expect("pattern");
    assert_eq!(p.prefix(2026, None), "");
    let p = Pattern::parse("FV-{YY}/{NNN}").expect("pattern");
    assert_eq!(p.prefix(2026, None), "FV-26/");
}
