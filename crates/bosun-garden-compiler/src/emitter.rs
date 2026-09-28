//! Multi-tenant static emitter.
//!
//! Notes are grouped by the frontmatter domain key (`blog: "tenant-slug"`).
//! Each tenant is compiled on its own, then written under `dist/{tenant}/`:
//!
//! - `{slug}/index.html` for ordinary notes (Hugo pretty URLs and MkDocs directory URLs)
//! - `index.html` for `index.md` / `_index.md`, or a generated home page when neither exists
//! - `404.html` and `sitemap.xml` at the tenant root
//! - `assets/stylesheets/main.css` and `assets/javascripts/search.js` (MkDocs Material assets)
//! - `search/search_index.json` (MkDocs Material) and `index.json` (Hugo)
//!
//! The public region stays above the cut. Backlink markdown stays in a collapsed footer.
//! Search documents are built from that public region only.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use bosun_garden_core::public_slug_path;

use crate::emit::{bundle_document, emit_page, BASELINE_CSS};
use crate::{public_html_region, CompileError, CompiledPage, CompiledVault};

/// One note written into a tenant site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedPage {
    /// Vault-relative markdown path.
    pub source: String,
    /// Absolute path of the emitted HTML file.
    pub output: PathBuf,
}

/// One isolated tenant directory under `dist/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedTenant {
    pub tenant: String,
    pub output_dir: PathBuf,
    pub pages: Vec<EmittedPage>,
}

/// Structured progress event for Bosun Studio and the `bosun-garden` CLI.
///
/// The CLI prints each event as one JSON object on stdout. In-process callers
/// receive the same sequence through the `report` callback on [`emit_tenant_sites`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildEvent {
    /// Compilation of `tenant` is starting.
    Started { tenant: String },
    /// In-memory HTML for `tenant` is ready. `pages` counts notes, not generated files.
    Compiled { tenant: String, pages: usize },
    /// One note has been written to disk.
    Page {
        tenant: String,
        source: String,
        output: PathBuf,
    },
    /// The tenant directory, including assets and indexes, is complete.
    Finished {
        tenant: String,
        output_dir: PathBuf,
        pages: usize,
    },
}

struct SearchDoc {
    title: String,
    location: String,
    text: String,
    permalink: String,
}

const SEARCH_JS: &str = "\
(function () {\n\
  var node = document.getElementById(\"search-index\");\n\
  var input = document.getElementById(\"garden-search\");\n\
  var out = document.getElementById(\"garden-search-results\");\n\
  if (!node || !input || !out) return;\n\
  var payload;\n\
  try { payload = JSON.parse(node.textContent || \"{}\"); } catch (err) { return; }\n\
  var docs = Array.isArray(payload) ? payload : (payload.docs || []);\n\
  var prefix = document.body.getAttribute(\"data-site-root\") || \"\";\n\
  input.addEventListener(\"input\", function () {\n\
    var query = input.value.trim().toLowerCase();\n\
    out.replaceChildren();\n\
    if (!query) return;\n\
    docs.filter(function (doc) {\n\
      var title = doc.title || \"\";\n\
      var text = doc.text || doc.content || \"\";\n\
      return (title + \" \" + text).toLowerCase().indexOf(query) !== -1;\n\
    }).slice(0, 20).forEach(function (doc) {\n\
      var item = document.createElement(\"li\");\n\
      var link = document.createElement(\"a\");\n\
      var location = doc.location || \"\";\n\
      link.href = location ? prefix + location : (prefix || \"./\");\n\
      link.textContent = doc.title || \"Untitled\";\n\
      item.appendChild(link);\n\
      out.appendChild(item);\n\
    });\n\
  });\n\
})();\n\
";

/// Walk `root`, group notes by `domain`, and write one static site per tenant under `dist`.
///
/// `only` limits the publish to one frontmatter slug. `None` publishes every tenant.
/// `report` receives [`BuildEvent`]s as each tenant is compiled and written.
pub fn emit_tenant_sites<R>(
    root: &Path,
    domain: &str,
    origin: &str,
    dist: &Path,
    only: Option<&str>,
    mut report: R,
) -> Result<Vec<EmittedTenant>, CompileError>
where
    R: FnMut(BuildEvent),
{
    let origin = crate::normalize_origin(origin)?;
    let mut tenants = crate::list_tenant_slugs(root, domain)?;
    if let Some(tenant) = only {
        if !is_safe_tenant(tenant) {
            return Err(CompileError::UnsafeTenant {
                tenant: tenant.to_string(),
            });
        }
        if !tenants.iter().any(|slug| slug.as_str() == tenant) {
            return Err(CompileError::UnknownTenant {
                tenant: tenant.to_string(),
            });
        }
        tenants.retain(|slug| slug.as_str() == tenant);
    }
    for tenant in &tenants {
        if !is_safe_tenant(tenant) {
            return Err(CompileError::UnsafeTenant {
                tenant: tenant.clone(),
            });
        }
    }
    let mut compiled = Vec::with_capacity(tenants.len());
    for tenant in &tenants {
        report(BuildEvent::Started {
            tenant: tenant.clone(),
        });
        let vault = crate::compile_vault(root, domain, tenant, &origin)?;
        report(BuildEvent::Compiled {
            tenant: tenant.clone(),
            pages: vault.pages.len(),
        });
        compiled.push((tenant.clone(), vault));
    }
    let mut emitted = Vec::with_capacity(compiled.len());
    for (tenant, vault) in &compiled {
        emitted.push(write_site(dist, tenant, &origin, vault, &mut report)?);
    }
    Ok(emitted)
}

fn is_safe_tenant(tenant: &str) -> bool {
    !tenant.is_empty()
        && tenant.len() <= 128
        && tenant
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn write_site<R>(
    dist: &Path,
    tenant: &str,
    origin: &str,
    vault: &CompiledVault,
    report: &mut R,
) -> Result<EmittedTenant, CompileError>
where
    R: FnMut(BuildEvent),
{
    let output_dir = dist.join(tenant);
    let mut planned: Vec<(&CompiledPage, String)> = Vec::new();
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    for page in &vault.pages {
        let rel = page_output_rel(&page.relative_path);
        if let Some(previous) = taken.insert(rel.clone(), page.relative_path.clone()) {
            return Err(CompileError::OutputCollision {
                left: previous,
                right: page.relative_path.clone(),
                path: format!("{tenant}/{rel}"),
            });
        }
        planned.push((page, rel));
    }

    let mut docs: Vec<SearchDoc> = planned
        .iter()
        .map(|(page, rel)| SearchDoc {
            title: html_title(&page.html),
            location: location_of(rel),
            text: visible_text(public_html_region(&page.html).unwrap_or("")),
            permalink: String::new(),
        })
        .collect();
    for doc in &mut docs {
        doc.permalink = permalink(origin, tenant, &doc.location);
    }
    let has_home = planned.iter().any(|(_, rel)| rel == "index.html");
    if !has_home {
        let listing = docs
            .iter()
            .map(|doc| doc.title.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        docs.push(SearchDoc {
            title: tenant.to_string(),
            location: String::new(),
            text: listing,
            permalink: permalink(origin, tenant, ""),
        });
    }
    docs.sort_by(|left, right| {
        left.location
            .cmp(&right.location)
            .then(left.title.cmp(&right.title))
    });

    let search_json = mkdocs_search_index(&docs);
    let hugo_json = hugo_index(&docs);

    let mut pages = Vec::with_capacity(planned.len());
    for (page, rel) in &planned {
        let bundled = bundle_page(&page.html, rel, &search_json, &hugo_json)?;
        let path = push_rel(&output_dir, rel);
        write_file(&path, bundled.as_bytes())?;
        report(BuildEvent::Page {
            tenant: tenant.to_string(),
            source: page.relative_path.clone(),
            output: path.clone(),
        });
        pages.push(EmittedPage {
            source: page.relative_path.clone(),
            output: path,
        });
    }
    pages.sort_by(|left, right| left.source.cmp(&right.source));

    if !has_home {
        let home = emit_page("index.html", tenant, &home_markdown(tenant, &docs), "")?;
        let bundled = bundle_page(&home, "index.html", &search_json, &hugo_json)?;
        write_file(&output_dir.join("index.html"), bundled.as_bytes())?;
    }
    let not_found = emit_page(
        "404.html",
        "Not found",
        "That page is not in this site.\n",
        "",
    )?;
    let bundled = bundle_page(&not_found, "404.html", &search_json, &hugo_json)?;
    write_file(&output_dir.join("404.html"), bundled.as_bytes())?;
    write_file(
        &output_dir
            .join("assets")
            .join("stylesheets")
            .join("main.css"),
        BASELINE_CSS.as_bytes(),
    )?;
    write_file(
        &output_dir
            .join("assets")
            .join("javascripts")
            .join("search.js"),
        SEARCH_JS.as_bytes(),
    )?;
    write_file(
        &output_dir.join("search").join("search_index.json"),
        search_json.as_bytes(),
    )?;
    write_file(&output_dir.join("index.json"), hugo_json.as_bytes())?;
    write_file(&output_dir.join("sitemap.xml"), sitemap(&docs).as_bytes())?;

    report(BuildEvent::Finished {
        tenant: tenant.to_string(),
        output_dir: output_dir.clone(),
        pages: pages.len(),
    });

    Ok(EmittedTenant {
        tenant: tenant.to_string(),
        output_dir,
        pages,
    })
}

fn page_output_rel(relative_md: &str) -> String {
    let slug = public_slug_path(relative_md);
    if slug.is_empty() {
        "index.html".to_string()
    } else {
        format!("{slug}/index.html")
    }
}

fn location_of(output_rel: &str) -> String {
    match output_rel.strip_suffix("index.html") {
        Some("") | None => String::new(),
        Some(dir) => dir.to_string(),
    }
}

fn permalink(origin: &str, tenant: &str, location: &str) -> String {
    let loc = location.trim_end_matches('/');
    if loc.is_empty() {
        format!("{origin}/{tenant}")
    } else {
        format!("{origin}/{tenant}/{loc}")
    }
}

fn home_markdown(tenant: &str, docs: &[SearchDoc]) -> String {
    let mut markdown = format!("# {tenant}\n\n");
    for doc in docs {
        if doc.location.is_empty() {
            continue;
        }
        let label = markdown_label(&doc.title);
        markdown.push_str(&format!("[{label}]({})\n\n", doc.permalink));
    }
    markdown
}

fn markdown_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|ch| !matches!(ch, '[' | ']' | '(' | ')'))
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "note".to_string()
    } else {
        trimmed.to_string()
    }
}

fn bundle_page(
    html: &str,
    output_rel: &str,
    search_json: &str,
    hugo_json: &str,
) -> Result<String, CompileError> {
    let depth = output_rel.matches('/').count();
    let prefix = "../".repeat(depth);
    let site_root = if depth == 0 { "" } else { prefix.as_str() };
    bundle_document(
        html,
        &format!("{prefix}assets/stylesheets/main.css"),
        &format!("{prefix}assets/javascripts/search.js"),
        &site_root,
        search_json,
        hugo_json,
    )
}

fn push_rel(base: &Path, rel: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for segment in rel.split('/') {
        if !segment.is_empty() && segment != "." && segment != ".." {
            path.push(segment);
        }
    }
    path
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CompileError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| CompileError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, bytes).map_err(|source| CompileError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn html_title(html: &str) -> String {
    let Some((_, rest)) = html.split_once("<title>") else {
        return "note".to_string();
    };
    let raw = rest
        .split_once("</title>")
        .map(|(title, _)| title)
        .unwrap_or("note");
    let decoded = decode_entities(raw);
    if decoded.is_empty() {
        "note".to_string()
    } else {
        decoded
    }
}

fn visible_text(html: &str) -> String {
    let mut raw = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            raw.push(' ');
            continue;
        }
        if ch == '>' {
            in_tag = false;
            continue;
        }
        if !in_tag {
            raw.push(ch);
        }
    }
    let decoded = decode_entities(&raw).replace("[[", "").replace("]]", "");
    let mut collapsed = String::new();
    let mut spaced = false;
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            if !spaced && !collapsed.is_empty() {
                collapsed.push(' ');
                spaced = true;
            }
        } else {
            collapsed.push(ch);
            spaced = false;
        }
    }
    collapsed
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#91;", "[")
        .replace("&#93;", "]")
        .replace("&amp;", "&")
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn mkdocs_search_index(docs: &[SearchDoc]) -> String {
    let mut out = String::from("{\n  \"config\": {\n    \"lang\": [\"en\"],\n    \"separator\": ");
    out.push_str(&json_string(r"[\s\-]+"));
    out.push_str(",\n    \"pipeline\": [\"stemmer\"]\n  },\n  \"docs\": [");
    if docs.is_empty() {
        out.push_str("]\n}\n");
        return out;
    }
    for (index, doc) in docs.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("\n    {\n      \"location\": ");
        out.push_str(&json_string(&doc.location));
        out.push_str(",\n      \"title\": ");
        out.push_str(&json_string(&doc.title));
        out.push_str(",\n      \"text\": ");
        out.push_str(&json_string(&doc.text));
        out.push_str("\n    }");
    }
    out.push_str("\n  ]\n}\n");
    out
}

fn hugo_index(docs: &[SearchDoc]) -> String {
    let mut out = String::from("[");
    if docs.is_empty() {
        out.push_str("]\n");
        return out;
    }
    for (index, doc) in docs.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("\n  {\n    \"title\": ");
        out.push_str(&json_string(&doc.title));
        out.push_str(",\n    \"permalink\": ");
        out.push_str(&json_string(&doc.permalink));
        out.push_str(",\n    \"content\": ");
        out.push_str(&json_string(&doc.text));
        out.push_str("\n  }");
    }
    out.push_str("\n]\n");
    out
}

fn sitemap(docs: &[SearchDoc]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for doc in docs {
        out.push_str("  <url><loc>");
        out.push_str(&xml_escape(&doc.permalink));
        out.push_str("</loc></url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_paths_follow_directory_urls() {
        assert_eq!(page_output_rel("Harbor Log.md"), "harbor-log/index.html");
        assert_eq!(page_output_rel("sub/deep.md"), "sub/deep/index.html");
        assert_eq!(page_output_rel("index.md"), "index.html");
        assert_eq!(page_output_rel("_index.md"), "index.html");
        assert_eq!(page_output_rel("guides/index.md"), "guides/index.html");
        assert_eq!(location_of("harbor-log/index.html"), "harbor-log/");
        assert_eq!(location_of("index.html"), "");
    }

    #[test]
    fn search_text_drops_wikilink_brackets() {
        let text = visible_text("<p>See &#91;&#91;Secret&#93;&#93; and Embedded fact</p>");
        assert!(!text.contains("[["));
        assert!(text.contains("Secret"));
        assert!(text.contains("Embedded fact"));
    }
}
