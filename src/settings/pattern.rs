//! Document number patterns, e.g. `{YYYY}{NNNN}` → `20260001`.
//!
//! Tokens: `{YYYY}` four-digit year, `{YY}` two-digit year, `{N…}` (1–9 `N`s)
//! the zero-padded sequence number, `{C}` the customer's number code (client
//! patterns only). Exactly one year token and exactly one sequence token are
//! required — counters reset per year, so a pattern without the year would
//! repeat numbers. A global pattern has no `{C}`, a client pattern exactly
//! one. Everything else is a literal from `[A-Za-z0-9/_-]`. At most
//! [`MAX_LEN`] characters.

pub const MAX_LEN: usize = 40;
/// Longest customer number code.
pub const CODE_MAX: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Literal(char),
    Year4,
    Year2,
    Seq(usize),
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid number pattern")]
pub struct InvalidPattern;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern(Vec<Token>);

/// What a number string says when read back through a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    pub year: i32,
    pub seq: i32,
    /// The `{C}` part as written in the number (client patterns only).
    pub code: Option<String>,
}

/// A character allowed in literals, codes and manual numbers.
pub fn is_number_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-')
}

/// A customer number code: `[A-Za-z0-9/_-]{1,10}`.
pub fn is_valid_code(s: &str) -> bool {
    (1..=CODE_MAX).contains(&s.len()) && s.chars().all(is_number_char)
}

impl Pattern {
    /// A global pattern (no `{C}`).
    pub fn parse(s: &str) -> Result<Self, InvalidPattern> {
        Self::parse_with(s, 0)
    }

    /// A client pattern (exactly one `{C}`).
    pub fn parse_client(s: &str) -> Result<Self, InvalidPattern> {
        Self::parse_with(s, 1)
    }

    fn parse_with(s: &str, codes_wanted: usize) -> Result<Self, InvalidPattern> {
        if s.chars().count() > MAX_LEN {
            return Err(InvalidPattern);
        }
        let mut tokens = Vec::new();
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            if c == '{' {
                let end = rest.find('}').ok_or(InvalidPattern)?;
                tokens.push(placeholder(&rest[1..end])?);
                rest = &rest[end + 1..];
            } else if is_number_char(c) {
                tokens.push(Token::Literal(c));
                rest = &rest[c.len_utf8()..];
            } else {
                return Err(InvalidPattern);
            }
        }
        let count = |f: fn(&Token) -> bool| tokens.iter().filter(|t| f(t)).count();
        let seqs = count(|t| matches!(t, Token::Seq(_)));
        let years = count(|t| matches!(t, Token::Year4 | Token::Year2));
        let codes = count(|t| matches!(t, Token::Code));
        if seqs != 1 || years != 1 || codes != codes_wanted {
            return Err(InvalidPattern);
        }
        Ok(Self(tokens))
    }

    /// Render the number. A sequence wider than its token keeps every digit;
    /// `{C}` renders `code` (empty when `None`).
    pub fn format(&self, year: i32, n: i64, code: Option<&str>) -> String {
        self.0
            .iter()
            .map(|t| match t {
                Token::Literal(c) => c.to_string(),
                Token::Year4 => format!("{year:04}"),
                Token::Year2 => format!("{:02}", year.rem_euclid(100)),
                Token::Seq(width) => format!("{n:0width$}"),
                Token::Code => code.unwrap_or_default().to_string(),
            })
            .collect()
    }

    /// What every number of `year` (and `code`) starts with: the rendering
    /// up to the sequence token.
    pub fn prefix(&self, year: i32, code: Option<&str>) -> String {
        let head: Vec<Token> = self
            .0
            .iter()
            .take_while(|t| !matches!(t, Token::Seq(_)))
            .cloned()
            .collect();
        Pattern(head).format(year, 0, code)
    }

    /// Read `number` back: year (`{YY}` → 2000 + yy), sequence and code.
    /// `code` pins `{C}` to that code (ASCII case-insensitive); `None` lets
    /// it match any valid code. Only a number this pattern would render
    /// itself matches (no extra leading zeros in the sequence).
    pub fn matches(&self, number: &str, code: Option<&str>) -> Option<Matched> {
        let mut found = None;
        self.walk(
            &self.0,
            number,
            Partial::default(),
            code,
            number,
            &mut found,
        );
        found
    }

    fn walk<'a>(
        &self,
        tokens: &[Token],
        rest: &'a str,
        acc: Partial<'a>,
        pinned: Option<&str>,
        number: &str,
        found: &mut Option<Matched>,
    ) {
        if found.is_some() {
            return;
        }
        let Some((token, tail)) = tokens.split_first() else {
            if rest.is_empty() {
                *found = acc.finish(self, number);
            }
            return;
        };
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        match token {
            Token::Literal(c) => {
                if let Some(r) = rest.strip_prefix(*c) {
                    self.walk(tail, r, acc, pinned, number, found);
                }
            }
            Token::Year4 | Token::Year2 => {
                let len = if *token == Token::Year4 { 4 } else { 2 };
                if digits >= len {
                    let Ok(v) = rest[..len].parse::<i32>() else {
                        return;
                    };
                    let year = if len == 2 { 2000 + v } else { v };
                    let acc = Partial {
                        year: Some(year),
                        ..acc
                    };
                    self.walk(tail, &rest[len..], acc, pinned, number, found);
                }
            }
            Token::Seq(width) => {
                for len in *width..=digits.min(10) {
                    let Ok(seq) = rest[..len].parse::<i32>() else {
                        continue;
                    };
                    let acc = Partial {
                        seq: Some(seq),
                        ..acc
                    };
                    self.walk(tail, &rest[len..], acc, pinned, number, found);
                }
            }
            Token::Code => {
                let allowed = rest.chars().take_while(|c| is_number_char(*c)).count();
                let lens: Vec<usize> = match pinned {
                    Some(p) => (is_valid_code(p)
                        && rest
                            .get(..p.len())
                            .is_some_and(|head| head.eq_ignore_ascii_case(p)))
                    .then_some(p.len())
                    .into_iter()
                    .collect(),
                    None => (1..=allowed.min(CODE_MAX)).collect(),
                };
                for len in lens {
                    let acc = Partial {
                        code: Some(&rest[..len]),
                        ..acc
                    };
                    self.walk(tail, &rest[len..], acc, pinned, number, found);
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Partial<'a> {
    year: Option<i32>,
    seq: Option<i32>,
    code: Option<&'a str>,
}

impl Partial<'_> {
    fn finish(self, pattern: &Pattern, number: &str) -> Option<Matched> {
        let (year, seq) = (self.year?, self.seq?);
        (pattern.format(year, i64::from(seq), self.code) == number).then(|| Matched {
            year,
            seq,
            code: self.code.map(str::to_string),
        })
    }
}

fn placeholder(inner: &str) -> Result<Token, InvalidPattern> {
    match inner {
        "YYYY" => Ok(Token::Year4),
        "YY" => Ok(Token::Year2),
        "C" => Ok(Token::Code),
        n if (1..=9).contains(&n.len()) && n.bytes().all(|b| b == b'N') => Ok(Token::Seq(n.len())),
        _ => Err(InvalidPattern),
    }
}

#[cfg(test)]
#[path = "pattern_tests.rs"]
mod tests;
