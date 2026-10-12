//! The uploaded original PDF of a received / imported document.

/// Only an actual PDF is accepted.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_magic() {
        assert!(is_pdf(b"%PDF-1.7\n"));
        assert!(!is_pdf(b"%PDF"));
        assert!(!is_pdf(b"<html>"));
        assert!(!is_pdf(b""));
    }
}
