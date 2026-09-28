//! In-memory publishing core for Bosun Garden.
//!
//! Each note is parsed once with Marlinspike's Rowan CST. Headings, wikilinks,
//! and `![[note#sec]]` embeds are taken from that parse and reused for the link
//! graph and for recursive transclusion. Nothing in this crate calls `parse`
//! a second time.

#![forbid(unsafe_code)]

mod graph;
mod links;
mod note;
mod publish;

pub use graph::{build_link_graph, resolve_note_index, EdgeKind, GraphEdge, LinkGraph};
pub use links::{LinkKind, OutboundLink};
pub use note::{index_note, CoreError, HeadingSpan, IndexedNote};
pub use publish::{public_slug_path, publish_vault, PublishedNote};
