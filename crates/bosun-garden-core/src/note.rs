//! One lossless Marlinspike parse per note, plus the spans later stages reuse.

use std::ops::Range;
use std::path::Path;

use marlinspike_ast::{parse, Parse, SyntaxKind};
use thiserror::Error;

use crate::links::{extract_links, OutboundLink};

/// Failure while indexing a note. Marlinspike reports a lossy tree by round-trip.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("marlinspike CST did not round-trip losslessly")]
    LossyRoundTrip,
}

/// A heading and the byte span of its section, clipped to the public cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingSpan {
    pub depth: usize,
    pub text: String,
    /// From the heading line through the byte before the next sibling heading or the cut.
    pub section: Range<usize>,
}

/// A vault note after its single CST parse.
#[derive(Debug, Clone)]
pub struct IndexedNote {
    pub relative_path: String,
    pub stem: String,
    pub title: Option<String>,
    pub source: String,
    pub body: Range<usize>,
    /// Absolute offset of the trailing `---` cut line, when the body has one.
    pub cut: Option<usize>,
    pub headings: Vec<HeadingSpan>,
    pub links: Vec<OutboundLink>,
    parsed: Parse,
}

impl IndexedNote {
    /// Source reconstructed from the Rowan tree. Equal to [`Self::source`] when indexing succeeded.
    pub fn cst_round_trip(&self) -> String {
        self.parsed.to_source()
    }
}

/// Parse `source` once and record headings, the trailing cut, and outbound links.
pub fn index_note(
    relative_path: impl Into<String>,
    source: String,
    title: Option<String>,
) -> Result<IndexedNote, CoreError> {
    let relative_path = relative_path.into();
    let parsed = parse(&source);
    if parsed.to_source() != source {
        return Err(CoreError::LossyRoundTrip);
    }
    let body = body_range(&parsed, source.len());
    let cut = find_trailing_cut(&source, body.clone());
    let headings = headings_from(&parsed, &source, body.clone(), cut);
    let links = extract_links(&source, body.clone());
    let stem = Path::new(&relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("note")
        .to_string();
    Ok(IndexedNote {
        relative_path,
        stem,
        title,
        source,
        body,
        cut,
        headings,
        links,
        parsed,
    })
}

fn body_range(parsed: &Parse, len: usize) -> Range<usize> {
    let root = parsed.syntax();
    if let Some(body) = root.children().find(|node| node.kind() == SyntaxKind::Body) {
        let start = u32::from(body.text_range().start()) as usize;
        let end = u32::from(body.text_range().end()) as usize;
        return start.min(len)..end.min(len);
    }
    0..len
}

fn headings_from(
    parsed: &Parse,
    source: &str,
    body: Range<usize>,
    cut: Option<usize>,
) -> Vec<HeadingSpan> {
    let mut found = Vec::new();
    for node in parsed.syntax().descendants() {
        if node.kind() != SyntaxKind::Heading {
            continue;
        }
        let start = u32::from(node.text_range().start()) as usize;
        let end = u32::from(node.text_range().end()) as usize;
        if start < body.start || start >= body.end {
            continue;
        }
        let raw = source.get(start..end.min(source.len())).unwrap_or("");
        let line = raw.lines().next().unwrap_or("");
        let depth = line.chars().take_while(|c| *c == '#').count().max(1);
        let text = line.trim_start_matches('#').trim().to_string();
        if text.is_empty() {
            continue;
        }
        found.push((depth, text, start));
    }

    let mut headings = Vec::with_capacity(found.len());
    for (index, (depth, text, start)) in found.iter().enumerate() {
        let mut section_end = body.end;
        for (next_depth, _, next_start) in found.iter().skip(index + 1) {
            if next_depth <= depth {
                section_end = *next_start;
                break;
            }
        }
        if let Some(cut_at) = cut {
            if *start >= cut_at {
                section_end = *start;
            } else if section_end > cut_at {
                section_end = cut_at;
            }
        }
        headings.push(HeadingSpan {
            depth: *depth,
            text: text.clone(),
            section: *start..section_end,
        });
    }
    headings
}

/// Last `---` line in the body that is outside a fenced code block.
///
/// The divider trails the public section: everything after it is backlink metadata.
/// Frontmatter fences sit outside `body` and are ignored.
pub(crate) fn find_trailing_cut(source: &str, body: Range<usize>) -> Option<usize> {
    let body_text = source.get(body.clone())?;
    let mut offset = 0usize;
    let mut fence = false;
    let mut last = None;
    while offset < body_text.len() {
        let rest = &body_text[offset..];
        let (line, next) = split_line(rest);
        if is_fence_line(line) {
            fence = !fence;
        } else if !fence && line.trim() == "---" {
            let rel = line.find("---").unwrap_or(0);
            last = Some(body.start + offset + rel);
        }
        if next == 0 {
            break;
        }
        offset += next;
    }
    last
}

fn is_fence_line(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

fn split_line(text: &str) -> (&str, usize) {
    match text.find('\n') {
        Some(newline) => {
            let end = if newline > 0 && text.as_bytes()[newline - 1] == b'\r' {
                newline - 1
            } else {
                newline
            };
            (&text[..end], newline + 1)
        }
        None => (text, text.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = "\
---\n\
blog: \"acme\"\n\
title: \"Hello\"\n\
---\n\
\n\
See [[Harbor Log|the log]].\n\
\n\
![[detail#excerpt]]\n\
\n\
---\n\
\n\
Backlinks: [[Secret]]\n\
";

    #[test]
    fn cst_round_trip_keeps_the_cut_line() {
        let note = index_note("hello.md", HELLO.to_string(), Some("Hello".into())).unwrap();
        assert_eq!(note.cst_round_trip(), HELLO);
        assert!(note.cut.is_some());
        assert!(note.links.iter().any(|link| link.note == "Harbor Log"));
        assert!(note
            .links
            .iter()
            .any(|link| link.kind == crate::LinkKind::Embed));
        let secret = note
            .links
            .iter()
            .find(|link| link.note == "Secret")
            .expect("backlink");
        assert!(secret.range.start > note.cut.unwrap());
    }
}
