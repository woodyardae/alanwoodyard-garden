//! Body scan for Obsidian wikilinks and embeds.
//!
//! The scan walks the note source over the CST body span from [`crate::index_note`].
//! It does not parse. Fenced code blocks and inline code are not links.

use std::ops::Range;

/// `[[wikilink]]` or `![[embed]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkKind {
    Wikilink,
    Embed,
}

/// One outbound link captured during the single CST indexing pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundLink {
    pub kind: LinkKind,
    /// Byte range of the full token, including `!` and brackets, in the note source.
    pub range: Range<usize>,
    /// Note name or vault-relative path, without a trailing `.md`. Empty for `[[#sec]]`.
    pub note: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
}

/// Collect wikilinks and embeds inside `source[body]`.
pub(crate) fn extract_links(source: &str, body: Range<usize>) -> Vec<OutboundLink> {
    let bytes = source.as_bytes();
    let end = body.end.min(source.len());
    let mut i = body.start.min(end);
    let mut line_start = true;
    let mut fence = false;
    let mut links = Vec::new();

    while i < end {
        if line_start {
            if let Some(after) = fence_toggle_at(source, i, end) {
                fence = !fence;
                line_start = false;
                i = after;
                continue;
            }
        }
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            line_start = true;
            i += 1;
            continue;
        }
        line_start = false;
        if fence {
            i += 1;
            continue;
        }
        if bytes[i] == b'`' {
            if let Some(after) = skip_inline_code(source, i, end) {
                i = after;
                continue;
            }
        }
        if let Some((kind, inner_start)) = opener_at(bytes, i, end) {
            if let Some(close) = find_close(bytes, inner_start, end) {
                let inner = &source[inner_start..close];
                if let Some(mut link) = parse_inner(kind, inner) {
                    let token_end = close + 2;
                    link.range = i..token_end;
                    links.push(link);
                    i = token_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    links
}

fn fence_toggle_at(source: &str, i: usize, end: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut j = i;
    while j < end && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if source[j..end].starts_with("```") {
        Some(j + 3)
    } else {
        None
    }
}

fn skip_inline_code(source: &str, i: usize, end: usize) -> Option<usize> {
    let rest = &source[i + 1..end];
    let rel = rest.find('`')?;
    let between = &rest[..rel];
    if between.contains('\n') || between.contains('\r') {
        return None;
    }
    Some(i + 1 + rel + 1)
}

fn opener_at(bytes: &[u8], i: usize, end: usize) -> Option<(LinkKind, usize)> {
    if i + 2 < end && bytes[i] == b'!' && bytes[i + 1] == b'[' && bytes[i + 2] == b'[' {
        return Some((LinkKind::Embed, i + 3));
    }
    if i + 1 < end && bytes[i] == b'[' && bytes[i + 1] == b'[' {
        return Some((LinkKind::Wikilink, i + 2));
    }
    None
}

fn find_close(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let mut j = start;
    while j + 1 < end {
        if bytes[j] == b']' && bytes[j + 1] == b']' {
            return Some(j);
        }
        if bytes[j] == b'\n' || bytes[j] == b'\r' {
            return None;
        }
        j += 1;
    }
    None
}

fn parse_inner(kind: LinkKind, inner: &str) -> Option<OutboundLink> {
    let (target, alias) = match inner.split_once('|') {
        Some((target, alias)) => {
            let alias = alias.trim();
            (
                target.trim(),
                if alias.is_empty() {
                    None
                } else {
                    Some(alias.to_string())
                },
            )
        }
        None => (inner.trim(), None),
    };
    let (note, heading) = match target.split_once('#') {
        Some((note, heading)) => {
            let heading = heading.trim();
            (
                note.trim(),
                if heading.is_empty() {
                    None
                } else {
                    Some(heading.to_string())
                },
            )
        }
        None => (target, None),
    };
    let note = note
        .trim_start_matches("./")
        .trim_end_matches(".md")
        .trim_end_matches(".MD")
        .to_string();
    if note.is_empty() && heading.is_none() {
        return None;
    }
    Some(OutboundLink {
        kind,
        range: 0..0,
        note,
        heading,
        alias,
    })
}
