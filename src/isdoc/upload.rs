//! Uploaded files → ISDOC documents: `.zip` unpacked recursively, `.isdocx`
//! opened, plain `.isdoc` taken as is. Zip bombs are bounded by counting the
//! bytes actually decompressed, never the sizes the headers claim.

use std::collections::HashSet;
use std::io::{Cursor, Read};

use super::model::MANIFEST_NS;
use super::parse::{Code, INVALID_XML};
pub use crate::import::form::File;

/// Entry error: a zip nested deeper than the limit.
pub const TOO_DEEP: Code = "too_deep";
/// Entry error: an unreadable `.zip`.
pub const INVALID_ARCHIVE: Code = "invalid_archive";
/// Warning: the PDF of an `.isdocx` exceeds the original limit.
pub const PDF_SKIPPED: Code = "pdf_skipped";

const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_docs: usize,
    pub max_unpacked: u64,
    pub max_depth: usize,
    pub max_pdf: u64,
    /// Entries per archive (a central directory full of empty entries).
    pub max_entries: usize,
}

pub const LIMITS: Limits = Limits {
    max_docs: 500,
    max_unpacked: 200 * MIB,
    max_depth: 3,
    max_pdf: 20 * MIB,
    max_entries: 10_000,
};

/// A whole-upload limit was exceeded (→ 422 `files`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exceeded {
    TooMany,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdf {
    pub name: String,
    /// `None`: larger than the original limit (skipped).
    pub bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub xml: Vec<u8>,
    /// The chosen PDF of an `.isdocx` (see [`choose_pdf`]).
    pub pdf: Option<Pdf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub key: String,
    pub doc: Result<Doc, Code>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Zip,
    Isdocx,
    Isdoc,
    Pdf,
    Other,
}

fn kind(name: &str) -> Kind {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        Kind::Zip
    } else if lower.ends_with(".isdocx") {
        Kind::Isdocx
    } else if lower.ends_with(".isdoc") {
        Kind::Isdoc
    } else if lower.ends_with(".pdf") {
        Kind::Pdf
    } else {
        Kind::Other
    }
}

struct Unpacker {
    limits: Limits,
    unpacked: u64,
    found: Vec<Found>,
    keys: HashSet<String>,
}

type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

impl Unpacker {
    /// Reads entry `i`, at most `cap` bytes (`None` beyond it); every byte
    /// read counts toward the unpacked total.
    fn read(&mut self, zip: &mut Archive, i: usize, cap: u64) -> Result<Option<Vec<u8>>, Exceeded> {
        let Ok(entry) = zip.by_index(i) else {
            return Ok(Some(Vec::new()));
        };
        let left = self.limits.max_unpacked - self.unpacked;
        let mut buf = Vec::new();
        // An undecodable entry reads short; the parser rejects what is left.
        let _ = entry.take(left.min(cap) + 1).read_to_end(&mut buf);
        let n = buf.len() as u64;
        if n > left {
            return Err(Exceeded::TooLarge);
        }
        self.unpacked += n;
        Ok((n <= cap).then_some(buf))
    }

    fn push(&mut self, key: String, doc: Result<Doc, Code>) -> Result<(), Exceeded> {
        if self.found.len() >= self.limits.max_docs {
            return Err(Exceeded::TooMany);
        }
        // Two uploads with the same name: the later key gets `#2`, `#3`, …
        let mut unique = key.clone();
        let mut n = 1;
        while !self.keys.insert(unique.clone()) {
            n += 1;
            unique = format!("{key}#{n}");
        }
        self.found.push(Found { key: unique, doc });
        Ok(())
    }

    fn open<'a>(&self, bytes: &'a [u8]) -> Option<Archive<'a>> {
        zip::ZipArchive::new(Cursor::new(bytes))
            .ok()
            .filter(|z| z.len() <= self.limits.max_entries)
    }

    fn zip(&mut self, key: &str, bytes: &[u8], depth: usize) -> Result<(), Exceeded> {
        if depth > self.limits.max_depth {
            return self.push(key.to_string(), Err(TOO_DEEP));
        }
        let Some(mut zip) = self.open(bytes) else {
            return self.push(key.to_string(), Err(INVALID_ARCHIVE));
        };
        for i in 0..zip.len() {
            let Some((name, is_dir)) = entry_name(&mut zip, i) else {
                continue;
            };
            let k = kind(&name);
            if is_dir || !matches!(k, Kind::Zip | Kind::Isdocx | Kind::Isdoc) {
                continue;
            }
            let inner = self.read(&mut zip, i, u64::MAX)?.unwrap_or_default();
            self.file(&format!("{key}/{name}"), k, &inner, depth + 1)?;
        }
        Ok(())
    }

    fn file(&mut self, key: &str, k: Kind, bytes: &[u8], depth: usize) -> Result<(), Exceeded> {
        match k {
            Kind::Zip => self.zip(key, bytes, depth),
            Kind::Isdocx => {
                let doc = self.isdocx(bytes)?;
                self.push(key.to_string(), doc)
            }
            _ => self.push(
                key.to_string(),
                Ok(Doc {
                    xml: bytes.to_vec(),
                    pdf: None,
                }),
            ),
        }
    }

    /// The main `.isdoc` (named by `manifest.xml`, else the first one) and
    /// every PDF of the archive.
    fn isdocx(&mut self, bytes: &[u8]) -> Result<Result<Doc, Code>, Exceeded> {
        let Some(mut zip) = self.open(bytes) else {
            return Ok(Err(INVALID_XML));
        };
        let names: Vec<String> = (0..zip.len())
            .map(|i| entry_name(&mut zip, i).map(|e| e.0).unwrap_or_default())
            .collect();
        let manifest = match names
            .iter()
            .position(|n| n.eq_ignore_ascii_case("manifest.xml"))
        {
            Some(i) => self.read(&mut zip, i, MIB)?.and_then(|m| main_document(&m)),
            None => None,
        };
        let main = manifest
            .and_then(|m| names.iter().position(|n| *n == m))
            .or_else(|| {
                names
                    .iter()
                    .position(|n| kind(n) == Kind::Isdoc && !n.contains('/'))
            })
            .or_else(|| names.iter().position(|n| kind(n) == Kind::Isdoc));
        let Some(main) = main else {
            return Ok(Err(INVALID_XML));
        };
        let xml = self.read(&mut zip, main, u64::MAX)?.unwrap_or_default();
        // Only the chosen PDF is decompressed (and counted).
        let preview = super::parse::preview_file_of(&xml);
        let pdf = match choose_pdf(&names, preview.as_deref()) {
            Some(i) => Some(Pdf {
                name: names[i].clone(),
                bytes: self.read(&mut zip, i, self.limits.max_pdf)?,
            }),
            None => None,
        };
        Ok(Ok(Doc { xml, pdf }))
    }
}

/// Name of entry `i` and whether it is a directory.
fn entry_name(zip: &mut Archive, i: usize) -> Option<(String, bool)> {
    let e = zip.by_index_raw(i).ok()?;
    let is_dir = e.is_dir();
    Some((e.name().ok()?.into_owned(), is_dir))
}

/// `manifest/maindocument/@filename`.
fn main_document(xml: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(xml).ok()?;
    let doc = roxmltree::Document::parse(s.trim_start_matches('\u{feff}')).ok()?;
    doc.root_element()
        .children()
        .find(|c| {
            c.tag_name().name() == "maindocument" && c.tag_name().namespace() == Some(MANIFEST_NS)
        })
        .and_then(|m| m.attribute("filename"))
        .map(str::to_string)
}

/// Every ISDOC document of the upload, in upload order (depth first). A
/// top-level file is a zip / isdocx by extension, anything else is read as
/// ISDOC XML; inside a zip, only `.zip`, `.isdocx` and `.isdoc` count.
pub fn unpack(files: Vec<File>, limits: Limits) -> Result<Vec<Found>, Exceeded> {
    let mut u = Unpacker {
        limits,
        unpacked: 0,
        found: Vec::new(),
        keys: HashSet::new(),
    };
    for f in files {
        let k = match kind(&f.name) {
            Kind::Zip => Kind::Zip,
            Kind::Isdocx => Kind::Isdocx,
            _ => Kind::Isdoc,
        };
        u.file(&f.name, k, &f.bytes, 1)?;
    }
    Ok(u.found)
}

/// Index of the PDF to keep as the original: the one the ISDOC marks as its
/// preview, else the only PDF of the archive.
pub fn choose_pdf(names: &[String], preview: Option<&str>) -> Option<usize> {
    if let Some(i) = preview.and_then(|p| names.iter().position(|n| n == p)) {
        return Some(i);
    }
    let mut pdfs = names
        .iter()
        .enumerate()
        .filter(|(_, n)| kind(n) == Kind::Pdf);
    match (pdfs.next(), pdfs.next()) {
        (Some((i, _)), None) => Some(i),
        _ => None,
    }
}

/// The original's bytes: none without a PDF (or when it is no PDF at all),
/// [`PDF_SKIPPED`] when it was too large.
pub fn pdf_bytes(pdf: Option<&Pdf>) -> Result<Option<bytes::Bytes>, Code> {
    match pdf {
        None => Ok(None),
        Some(Pdf { bytes: None, .. }) => Err(PDF_SKIPPED),
        Some(Pdf { bytes: Some(b), .. }) => {
            Ok(crate::document::original::is_pdf(b).then(|| bytes::Bytes::copy_from_slice(b)))
        }
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
