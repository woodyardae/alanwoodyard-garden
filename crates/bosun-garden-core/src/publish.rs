//! Resolve `![[note#sec]]` from indexed notes, then rewrite public wikilinks.
//!
//! Expansion reads heading spans and link ranges stored on [`IndexedNote`].
//! It does not call the Marlinspike parser.

use std::ops::Range;

use crate::graph::{loose_key, resolve_note_index};
use crate::links::{extract_links, LinkKind, OutboundLink};
use crate::note::IndexedNote;

const MAX_EMBED_DEPTH: usize = 32;

/// Public markdown above the cut, and untouched backlink markdown below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedNote {
    pub relative_path: String,
    /// Wikilinks outside code fences are `[label](https://...)` anchors.
    pub public_markdown: String,
    /// Original markdown below the trailing `---` divider, including `[[wikilinks]]`.
    pub backlink_markdown: String,
}

/// Publish every indexed note. `notes` must be the same slice used to resolve targets.
pub fn publish_vault(notes: &[IndexedNote], origin: &str, tenant: &str) -> Vec<PublishedNote> {
    let origin = origin.trim().trim_end_matches('/');
    notes
        .iter()
        .map(|note| publish_one(note, notes, origin, tenant))
        .collect()
}

fn publish_one(
    note: &IndexedNote,
    notes: &[IndexedNote],
    origin: &str,
    tenant: &str,
) -> PublishedNote {
    let note_index = notes
        .iter()
        .position(|other| other.relative_path == note.relative_path);
    let mut stack = Vec::new();
    if let Some(index) = note_index {
        stack.push((index, None));
    }
    let expanded = expand_embeds_before_cut(note, notes, origin, tenant, &stack);
    let public_markdown = rewrite_wikilinks(
        &expanded.public_markdown,
        &note.relative_path,
        notes,
        origin,
        tenant,
    );
    PublishedNote {
        relative_path: note.relative_path.clone(),
        public_markdown,
        backlink_markdown: expanded.backlink_markdown,
    }
}

struct Expanded {
    public_markdown: String,
    backlink_markdown: String,
}

fn expand_embeds_before_cut(
    note: &IndexedNote,
    notes: &[IndexedNote],
    origin: &str,
    tenant: &str,
    stack: &[(usize, Option<String>)],
) -> Expanded {
    let limit = note.cut.unwrap_or(note.body.end);
    let mut text = note.source.clone();
    let mut cut = note.cut;
    let mut body_end = note.body.end;
    let mut embeds: Vec<OutboundLink> = note
        .links
        .iter()
        .filter(|link| link.kind == LinkKind::Embed && link.range.end <= limit)
        .cloned()
        .collect();
    embeds.sort_by(|left, right| right.range.start.cmp(&left.range.start));
    for link in embeds {
        let replacement = resolve_embed(&link, &note.relative_path, notes, origin, tenant, stack);
        let delta = replacement.len() as isize - link.range.len() as isize;
        text.replace_range(link.range.clone(), &replacement);
        body_end = shift(body_end, delta);
        if let Some(cut_at) = cut.as_mut() {
            *cut_at = shift(*cut_at, delta);
        }
    }
    let public_markdown = if let Some(cut_at) = cut {
        text[note.body.start..cut_at.min(text.len())].to_string()
    } else {
        let end = body_end.min(text.len());
        text[note.body.start.min(end)..end].to_string()
    };
    let backlink_markdown = if let Some(cut_at) = cut {
        let after = skip_divider(&text, cut_at).min(text.len());
        let end = body_end.min(text.len()).max(after);
        text[after..end]
            .trim_start_matches(['\n', '\r'])
            .to_string()
    } else {
        String::new()
    };
    Expanded {
        public_markdown,
        backlink_markdown,
    }
}

fn resolve_embed(
    link: &OutboundLink,
    current_path: &str,
    notes: &[IndexedNote],
    origin: &str,
    tenant: &str,
    stack: &[(usize, Option<String>)],
) -> String {
    let fallback = markdown_anchor(link, current_path, notes, origin, tenant);
    if stack.len() >= MAX_EMBED_DEPTH {
        return fallback;
    }
    let Some(index) = resolve_note_index(notes, &link.note) else {
        return fallback;
    };
    let key = (index, link.heading.clone());
    if stack.iter().any(|frame| frame == &key) {
        return fallback;
    }
    let Some(range) = section_for(&notes[index], link.heading.as_deref()) else {
        return fallback;
    };
    let mut next_stack = stack.to_vec();
    next_stack.push(key);
    expand_region(
        &notes[index].source,
        range,
        &notes[index].links,
        &notes[index].relative_path,
        notes,
        origin,
        tenant,
        &next_stack,
    )
}

fn expand_region(
    source: &str,
    range: Range<usize>,
    links: &[OutboundLink],
    current_path: &str,
    notes: &[IndexedNote],
    origin: &str,
    tenant: &str,
    stack: &[(usize, Option<String>)],
) -> String {
    let start = range.start.min(source.len());
    let end = range.end.min(source.len()).max(start);
    let mut text = source[start..end].to_string();
    let mut embeds: Vec<OutboundLink> = links
        .iter()
        .filter(|link| {
            link.kind == LinkKind::Embed && link.range.start >= start && link.range.end <= end
        })
        .cloned()
        .map(|mut link| {
            link.range.start -= start;
            link.range.end -= start;
            link
        })
        .collect();
    embeds.sort_by(|left, right| right.range.start.cmp(&left.range.start));
    for link in embeds {
        let replacement = resolve_embed(&link, current_path, notes, origin, tenant, stack);
        if link.range.end <= text.len() {
            text.replace_range(link.range, &replacement);
        }
    }
    text
}

fn section_for(note: &IndexedNote, heading: Option<&str>) -> Option<Range<usize>> {
    match heading {
        None => {
            let end = note.cut.unwrap_or(note.body.end);
            if note.body.start >= end {
                None
            } else {
                Some(note.body.start..end)
            }
        }
        Some(name) => {
            let want = loose_key(name);
            note.headings
                .iter()
                .find(|span| {
                    !span.section.is_empty()
                        && (span.text.eq_ignore_ascii_case(name) || loose_key(&span.text) == want)
                })
                .map(|span| span.section.clone())
        }
    }
}

fn rewrite_wikilinks(
    public: &str,
    current_path: &str,
    notes: &[IndexedNote],
    origin: &str,
    tenant: &str,
) -> String {
    let mut text = public.to_string();
    let mut links = extract_links(&text, 0..text.len());
    links.sort_by(|left, right| right.range.start.cmp(&left.range.start));
    for link in links {
        let replacement = markdown_anchor(&link, current_path, notes, origin, tenant);
        if link.range.end <= text.len() {
            text.replace_range(link.range, &replacement);
        }
    }
    text
}

fn markdown_anchor(
    link: &OutboundLink,
    current_path: &str,
    notes: &[IndexedNote],
    origin: &str,
    tenant: &str,
) -> String {
    let label = markdown_label(&display_label(link));
    let target_path = if link.note.is_empty() {
        current_path.to_string()
    } else if let Some(index) = resolve_note_index(notes, &link.note) {
        notes[index].relative_path.clone()
    } else {
        link.note.clone()
    };
    let href = public_url(origin, tenant, &target_path, link.heading.as_deref());
    format!("[{label}]({href})")
}

fn display_label(link: &OutboundLink) -> String {
    if let Some(alias) = &link.alias {
        return alias.clone();
    }
    if link.note.is_empty() {
        return link.heading.clone().unwrap_or_else(|| "note".to_string());
    }
    link.note
        .split('/')
        .next_back()
        .unwrap_or(link.note.as_str())
        .to_string()
}

fn markdown_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|ch| !matches!(ch, '[' | ']' | '(' | ')'))
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "link".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Vault-relative markdown path as the public directory slug.
///
/// `about.md` becomes `about`. `guides/hello.md` becomes `guides/hello`.
/// `index.md` and `_index.md` collapse to the directory itself (`""` at the
/// vault root, or `guides` for `guides/index.md`) so the URL matches both Hugo
/// section pages and MkDocs `index.md` pages.
pub fn public_slug_path(relative_md: &str) -> String {
    let stem_path = relative_md.trim().trim_end_matches(".md");
    let mut parts: Vec<String> = stem_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let slug = slugify(segment);
            if slug.is_empty() {
                "note".to_string()
            } else {
                slug
            }
        })
        .collect();
    if parts.last().is_some_and(|part| part == "index") {
        parts.pop();
    }
    parts.join("/")
}

pub(crate) fn public_url(
    origin: &str,
    tenant: &str,
    relative_md: &str,
    heading: Option<&str>,
) -> String {
    let path = public_slug_path(relative_md);
    let mut url = if path.is_empty() {
        format!("{origin}/{tenant}")
    } else {
        format!("{origin}/{tenant}/{path}")
    };
    if let Some(heading) = heading {
        let slug = slugify(heading);
        if !slug.is_empty() {
            url.push('#');
            url.push_str(&slug);
        }
    }
    url
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn shift(pos: usize, delta: isize) -> usize {
    if delta >= 0 {
        pos + delta as usize
    } else {
        pos.saturating_sub((-delta) as usize)
    }
}

fn skip_divider(text: &str, cut_at: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = cut_at.min(text.len());
    if text[i..].starts_with("---") {
        i += 3;
    }
    if text[i..].starts_with("\r\n") {
        i += 2;
    } else if i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r') {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_note;
    use crate::{build_link_graph, EdgeKind, LinkKind};

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

    const DETAIL: &str = "\
---\n\
blog: \"acme\"\n\
title: \"detail\"\n\
---\n\
\n\
# excerpt\n\
\n\
Embedded fact\n\
\n\
![[nested#bit]]\n\
\n\
# other\n\
\n\
Nope\n\
";

    const NESTED: &str = "\
---\n\
blog: \"acme\"\n\
title: \"nested\"\n\
---\n\
\n\
# bit\n\
\n\
Nested fact\n\
";

    const HARBOR: &str = "\
---\n\
blog: \"acme\"\n\
title: \"Harbor Log\"\n\
---\n\
\n\
Log body.\n\
";

    fn vault() -> Vec<IndexedNote> {
        vec![
            index_note("hello.md", HELLO.to_string(), Some("Hello".into())).unwrap(),
            index_note("detail.md", DETAIL.to_string(), Some("detail".into())).unwrap(),
            index_note("nested.md", NESTED.to_string(), Some("nested".into())).unwrap(),
            index_note(
                "Harbor Log.md",
                HARBOR.to_string(),
                Some("Harbor Log".into()),
            )
            .unwrap(),
        ]
    }

    #[test]
    fn slug_paths_match_directory_urls() {
        assert_eq!(public_slug_path("Harbor Log.md"), "harbor-log");
        assert_eq!(public_slug_path("sub/deep.md"), "sub/deep");
        assert_eq!(public_slug_path("index.md"), "");
        assert_eq!(public_slug_path("_index.md"), "");
        assert_eq!(public_slug_path("guides/index.md"), "guides");
        assert_eq!(public_slug_path("guides/_index.md"), "guides");
    }

    #[test]
    fn resolves_recursive_transclusion_and_rewrites_above_the_cut() {
        let notes = vault();
        let published = publish_vault(&notes, "https://garden.example", "acme");
        let hello = published
            .iter()
            .find(|note| note.relative_path == "hello.md")
            .unwrap();
        assert!(
            hello
                .public_markdown
                .contains("[the log](https://garden.example/acme/harbor-log)"),
            "{}",
            hello.public_markdown
        );
        assert!(hello.public_markdown.contains("Embedded fact"));
        assert!(hello.public_markdown.contains("Nested fact"));
        assert!(!hello.public_markdown.contains("[["));
        assert!(!hello.public_markdown.contains("Nope"));
        assert!(hello.backlink_markdown.contains("[[Secret]]"));
        assert!(!hello.backlink_markdown.contains("Nested fact"));
    }

    #[test]
    fn transclusion_reuses_indexed_spans_without_second_cst_parse() {
        let notes = vault();
        let hello = notes
            .iter()
            .find(|note| note.relative_path == "hello.md")
            .unwrap();
        let detail = notes
            .iter()
            .find(|note| note.relative_path == "detail.md")
            .unwrap();
        // Indexing already captured the embed range and target heading section.
        assert!(hello.links.iter().any(|link| {
            link.kind == LinkKind::Embed
                && link.note == "detail"
                && link.heading.as_deref() == Some("excerpt")
        }));
        assert!(detail
            .headings
            .iter()
            .any(|span| span.text.eq_ignore_ascii_case("excerpt")));
        let before = notes
            .iter()
            .map(|note| note.cst_round_trip())
            .collect::<Vec<_>>();
        let published = publish_vault(&notes, "https://garden.example", "acme");
        let after = notes
            .iter()
            .map(|note| note.cst_round_trip())
            .collect::<Vec<_>>();
        assert_eq!(
            before, after,
            "publish must not mutate or re-tokenize the CST"
        );
        let hello = published
            .iter()
            .find(|note| note.relative_path == "hello.md")
            .unwrap();
        assert!(hello.public_markdown.contains("Embedded fact"));
        assert!(hello.public_markdown.contains("Nested fact"));
    }

    #[test]
    fn cut_line_keeps_garden_backlinks_out_of_public_markdown() {
        let notes = vault();
        let published = publish_vault(&notes, "https://garden.example", "acme");
        let hello = published
            .iter()
            .find(|note| note.relative_path == "hello.md")
            .unwrap();
        assert!(!hello.public_markdown.contains("Secret"));
        assert!(!hello.public_markdown.contains("Backlinks"));
        assert!(hello.backlink_markdown.contains("[[Secret]]"));
        assert!(hello.backlink_markdown.contains("Backlinks"));
    }

    #[test]
    fn graph_reuses_indexed_links() {
        let notes = vault();
        let graph = build_link_graph(&notes);
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Wikilink
                && edge.from == "hello.md"
                && edge.to == "Harbor Log.md"
                && edge.resolved
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Transclusion
                && edge.from == "hello.md"
                && edge.to == "detail.md"
                && edge.heading.as_deref() == Some("excerpt")
                && edge.resolved
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Transclusion
                && edge.from == "detail.md"
                && edge.to == "nested.md"
                && edge.resolved
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Wikilink
                && edge.from == "hello.md"
                && edge.to == "Secret"
                && !edge.resolved
        }));
    }
}
