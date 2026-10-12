//! List paging shared by the REST lists (contacts, documents, catalog) and
//! the search pattern of their `q` filter.

use serde::Deserialize;
use utoipa::IntoParams;

pub const DEFAULT_LIMIT: u64 = 50;
pub const MAX_LIMIT: u64 = 200;
pub const MAX_OFFSET: u64 = i64::MAX as u64;

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
// The `q` wording is the contacts list's (its OpenAPI parameter); documents
// and the catalog document their own filters.
pub struct ListQuery {
    /// Case-insensitive substring of name, IČO, DIČ or city.
    pub q: Option<String>,
    /// Page size, default 50, clamped to 1..=200.
    pub limit: Option<u64>,
    /// Default 0, clamped to 0..=i64::MAX.
    pub offset: Option<u64>,
}

impl ListQuery {
    /// `(search, limit, offset)` with defaults applied and `limit` clamped.
    pub fn normalized(&self) -> (Option<String>, u64, u64) {
        let q = self
            .q
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        // Postgres OFFSET is a signed bigint; a larger value would be a DB error.
        let offset = self.offset.unwrap_or(0).min(MAX_OFFSET);
        (q, limit, offset)
    }
}

/// `%term%` for ILIKE with the LIKE metacharacters escaped (default escape `\`).
pub fn like_pattern(term: &str) -> String {
    let mut out = String::with_capacity(term.len() + 2);
    out.push('%');
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_pattern_escapes_metacharacters() {
        assert_eq!(like_pattern("abc"), "%abc%");
        assert_eq!(like_pattern("50%_a\\b"), "%50\\%\\_a\\\\b%");
    }

    #[test]
    fn list_query_defaults_and_clamps() {
        assert_eq!(ListQuery::default().normalized(), (None, 50, 0));
        let q = ListQuery {
            q: Some("  acme ".into()),
            limit: Some(10_000),
            offset: Some(5),
        };
        assert_eq!(q.normalized(), (Some("acme".into()), 200, 5));
        let q = ListQuery {
            q: Some("   ".into()),
            limit: Some(0),
            offset: None,
        };
        assert_eq!(q.normalized(), (None, 1, 0));
        let q = ListQuery {
            offset: Some(u64::MAX),
            ..Default::default()
        };
        assert_eq!(q.normalized(), (None, 50, MAX_OFFSET));
    }
}
