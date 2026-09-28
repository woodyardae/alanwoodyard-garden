//! Global in-memory link graph built from links captured at index time.

use crate::note::IndexedNote;
use crate::LinkKind;

/// Wikilink or transclusion edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    Wikilink,
    Transclusion,
}

/// One directed edge in the vault graph.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub heading: Option<String>,
    pub resolved: bool,
}

/// Adjacency for one tenant compile. Nodes are vault-relative paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkGraph {
    pub nodes: Vec<String>,
    pub edges: Vec<GraphEdge>,
}

/// Build the graph from already-indexed notes. This function does not parse.
pub fn build_link_graph(notes: &[IndexedNote]) -> LinkGraph {
    let mut nodes: Vec<String> = notes
        .iter()
        .map(|note| note.relative_path.clone())
        .collect();
    nodes.sort();
    nodes.dedup();

    let mut edges = Vec::new();
    for note in notes {
        for link in &note.links {
            let (to, resolved) = if link.note.is_empty() {
                (note.relative_path.clone(), true)
            } else if let Some(index) = resolve_note_index(notes, &link.note) {
                (notes[index].relative_path.clone(), true)
            } else {
                (link.note.clone(), false)
            };
            let kind = match link.kind {
                LinkKind::Wikilink => EdgeKind::Wikilink,
                LinkKind::Embed => EdgeKind::Transclusion,
            };
            edges.push(GraphEdge {
                from: note.relative_path.clone(),
                to,
                kind,
                heading: link.heading.clone(),
                resolved,
            });
        }
    }
    edges.sort();
    edges.dedup();
    LinkGraph { nodes, edges }
}

/// Resolve a wikilink or embed target to a note index.
///
/// Order: vault-relative path, unique stem, unique title, then a unique
/// alphanumeric loose key (`Harbor Log` matches `harbor-log`).
pub fn resolve_note_index(notes: &[IndexedNote], name: &str) -> Option<usize> {
    let name = name.trim().trim_start_matches("./");
    if name.is_empty() {
        return None;
    }
    let want_rel = norm_rel(name);
    if let Some(index) = decide(hits(notes, |note| {
        norm_rel(&note.relative_path) == want_rel
    })) {
        return index;
    }

    if let Some(index) = decide(hits(notes, |note| note.stem.eq_ignore_ascii_case(name))) {
        return index;
    }

    if let Some(index) = decide(hits(notes, |note| {
        note.title
            .as_ref()
            .is_some_and(|title| title.eq_ignore_ascii_case(name))
    })) {
        return index;
    }

    let loose = loose_key(name);
    if loose.is_empty() {
        return None;
    }
    decide(hits(notes, |note| {
        loose_key(&note.stem) == loose
            || note
                .title
                .as_ref()
                .is_some_and(|title| loose_key(title) == loose)
            || loose_key(&norm_rel(&note.relative_path)) == loose
    }))
    .flatten()
}

fn hits(notes: &[IndexedNote], pred: impl Fn(&IndexedNote) -> bool) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, note)| pred(note))
        .map(|(index, _)| index)
        .collect()
}

/// `Some(None)` means the step was ambiguous and lookup must stop.
fn decide(hits: Vec<usize>) -> Option<Option<usize>> {
    match hits.len() {
        0 => None,
        1 => Some(Some(hits[0])),
        _ => Some(None),
    }
}

pub(crate) fn norm_rel(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .to_ascii_lowercase()
        .trim_end_matches(".md")
        .to_string()
}

pub(crate) fn loose_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_note;

    #[test]
    fn loose_name_matches_hyphenated_stem() {
        let note = index_note(
            "harbor-log.md",
            "---\ntitle: Harbor\n---\nHi\n".to_string(),
            Some("Harbor".into()),
        )
        .unwrap();
        let notes = [note];
        assert_eq!(resolve_note_index(&notes, "Harbor Log"), Some(0));
    }
}
