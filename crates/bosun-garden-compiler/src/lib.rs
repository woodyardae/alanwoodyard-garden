//! Parallel ingest of a tenant vault and HTML emission with the cut-line enforcer.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use bosun_garden_core::{index_note, publish_vault, CoreError, IndexedNote};
use rayon::prelude::*;
use thiserror::Error;

use crate::emit::emit_page;
use crate::walk::{relative_key, walk_markdown};

mod emit;
mod emitter;
mod frontmatter;
mod walk;

pub use bosun_garden_core::{EdgeKind, GraphEdge, LinkGraph};
pub use emit::{backlink_html_region, enforce_public_stream, public_html_region};
pub use emitter::{emit_tenant_sites, BuildEvent, EmittedPage, EmittedTenant};
pub use frontmatter::{parse_note_frontmatter, NoteFrontmatter, Scalar};
pub use walk::walk_markdown as walk_vault;

/// One rendered page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPage {
    pub relative_path: String,
    pub html: String,
}

/// Tenant publish result: HTML pages plus the in-memory link graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledVault {
    pub pages: Vec<CompiledPage>,
    pub graph: LinkGraph,
}

/// Walker, frontmatter, CST, or cut-line failure.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("failed to read {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("markdown file is not valid UTF-8: {path}")]
    Utf8 { path: PathBuf },
    #[error("frontmatter in {path} is not YAML 1.2: {message}")]
    Frontmatter { path: PathBuf, message: String },
    #[error("marlinspike CST did not round-trip {path}")]
    LossyCst { path: PathBuf },
    #[error("public HTML for {path} contains a raw Obsidian wikilink above the cut-line")]
    CutLine { path: String },
    #[error("origin must be an https URL")]
    Origin,
    #[error("domain and tenant must be non-empty")]
    Tenant,
    #[error("tenant slug {tenant} is not a single safe directory name")]
    UnsafeTenant { tenant: String },
    #[error("tenant {tenant} is not in the vault")]
    UnknownTenant { tenant: String },
    #[error("notes {left} and {right} both emit to {path}")]
    OutputCollision {
        left: String,
        right: String,
        path: String,
    },
    #[error("compiled HTML is missing the search index slot")]
    HeadSlot,
}

impl CompileError {
    /// Stable diagnostic code for JSON telemetry and Bosun Studio.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "io",
            Self::Utf8 { .. } => "utf8",
            Self::Frontmatter { .. } => "frontmatter",
            Self::LossyCst { .. } => "lossy_cst",
            Self::CutLine { .. } => "cut_line",
            Self::Origin => "origin",
            Self::Tenant => "tenant",
            Self::UnsafeTenant { .. } => "unsafe_tenant",
            Self::UnknownTenant { .. } => "unknown_tenant",
            Self::OutputCollision { .. } => "output_collision",
            Self::HeadSlot => "head_slot",
        }
    }

    /// Path or tenant slug associated with this failure, when there is one.
    pub fn diagnostic_path(&self) -> Option<String> {
        match self {
            Self::Io { path, .. }
            | Self::Utf8 { path }
            | Self::Frontmatter { path, .. }
            | Self::LossyCst { path } => Some(path.display().to_string()),
            Self::CutLine { path } | Self::OutputCollision { path, .. } => Some(path.clone()),
            Self::UnsafeTenant { tenant } | Self::UnknownTenant { tenant } => Some(tenant.clone()),
            Self::Origin | Self::Tenant | Self::HeadSlot => None,
        }
    }
}

struct Loaded {
    relative_path: String,
    source: String,
    title: Option<String>,
}

/// Walk `root`, keep notes whose `domain` frontmatter key equals `tenant`, and emit HTML.
///
/// `blog: "tenant-slug"` is the domain-scoped tenant key. Each selected note is
/// parsed once by Marlinspike inside [`index_note`](bosun_garden_core::index_note).
pub fn compile_vault(
    root: &Path,
    domain: &str,
    tenant: &str,
    origin: &str,
) -> Result<CompiledVault, CompileError> {
    if domain.is_empty() || tenant.is_empty() {
        return Err(CompileError::Tenant);
    }
    let origin = normalize_origin(origin)?;
    let files = walk_markdown(root)?;
    let indexed: Vec<IndexedNote> = files
        .par_iter()
        .map(|path| load_and_index(root, path, domain, tenant))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let published = publish_vault(&indexed, &origin, tenant);
    let graph = bosun_garden_core::build_link_graph(&indexed);
    let mut pages = Vec::with_capacity(indexed.len());
    for (note, published_note) in indexed.iter().zip(published.iter()) {
        let title = note.title.as_deref().unwrap_or(note.stem.as_str());
        let html = emit_page(
            &note.relative_path,
            title,
            &published_note.public_markdown,
            &published_note.backlink_markdown,
        )?;
        pages.push(CompiledPage {
            relative_path: note.relative_path.clone(),
            html,
        });
    }
    Ok(CompiledVault { pages, graph })
}

fn load_and_index(
    root: &Path,
    path: &Path,
    domain: &str,
    tenant: &str,
) -> Result<Option<IndexedNote>, CompileError> {
    let Some(loaded) = load_note(root, path, domain, tenant)? else {
        return Ok(None);
    };
    match index_note(loaded.relative_path, loaded.source, loaded.title) {
        Ok(note) => Ok(Some(note)),
        Err(CoreError::LossyRoundTrip) => Err(CompileError::LossyCst {
            path: path.to_path_buf(),
        }),
    }
}

fn load_note(
    root: &Path,
    path: &Path,
    domain: &str,
    tenant: &str,
) -> Result<Option<Loaded>, CompileError> {
    let source = read_markdown(path)?;
    let Some(frontmatter) = parsed_frontmatter(path, &source)? else {
        return Ok(None);
    };
    let Some(slug) = frontmatter.get_str(domain) else {
        return Ok(None);
    };
    if slug != tenant {
        return Ok(None);
    }
    Ok(Some(Loaded {
        relative_path: relative_key(root, path),
        source,
        title: frontmatter.get_str("title").map(str::to_string),
    }))
}

pub(crate) fn list_tenant_slugs(root: &Path, domain: &str) -> Result<Vec<String>, CompileError> {
    if domain.is_empty() {
        return Err(CompileError::Tenant);
    }
    let files = walk_markdown(root)?;
    let found: Vec<Result<Option<String>, CompileError>> = files
        .par_iter()
        .map(|path| tenant_slug(path, domain))
        .collect();
    let mut slugs = BTreeSet::new();
    for slug in found {
        if let Some(slug) = slug? {
            slugs.insert(slug);
        }
    }
    Ok(slugs.into_iter().collect())
}

fn tenant_slug(path: &Path, domain: &str) -> Result<Option<String>, CompileError> {
    let source = read_markdown(path)?;
    let Some(frontmatter) = parsed_frontmatter(path, &source)? else {
        return Ok(None);
    };
    Ok(frontmatter
        .get_str(domain)
        .map(str::to_string)
        .filter(|slug| !slug.is_empty()))
}

fn read_markdown(path: &Path) -> Result<String, CompileError> {
    let bytes = fs::read(path).map_err(|source| CompileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut source = String::from_utf8(bytes).map_err(|_| CompileError::Utf8 {
        path: path.to_path_buf(),
    })?;
    if source.starts_with('\u{feff}') {
        source.remove(0);
    }
    Ok(source)
}

fn parsed_frontmatter(
    path: &Path,
    source: &str,
) -> Result<Option<crate::frontmatter::NoteFrontmatter>, CompileError> {
    match parse_note_frontmatter(source) {
        Ok(frontmatter) => Ok(frontmatter),
        Err(message) => Err(CompileError::Frontmatter {
            path: path.to_path_buf(),
            message,
        }),
    }
}

pub(crate) fn normalize_origin(origin: &str) -> Result<String, CompileError> {
    let trimmed = origin.trim().trim_end_matches('/');
    if !trimmed.starts_with("https://") || trimmed.len() <= "https://".len() {
        return Err(CompileError::Origin);
    }
    Ok(trimmed.to_string())
}
