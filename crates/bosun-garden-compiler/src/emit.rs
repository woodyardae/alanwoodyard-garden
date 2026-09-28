//! HTML emitter. The public region is the web stream above the cut-line.

use crate::CompileError;

const PUBLIC_OPEN: &str = "<div class=\"public\">";
const PUBLIC_CLOSE: &str = "</div>";
const BACKLINK_OPEN: &str = "<section class=\"backlinks\" data-cut=\"below\">";
const BACKLINK_CLOSE: &str = "</section>";
const SEARCH_SLOT_OPEN: &str = "<script type=\"application/json\" id=\"search-index\">";

/// Baseline rules inlined in every page and copied to `assets/stylesheets/main.css`.
pub(crate) const BASELINE_CSS: &str = "\
body { font-family: system-ui, sans-serif; line-height: 1.55; max-width: 42rem; margin: 2rem auto; padding: 0 1rem; color: #1b1b1b; background: #fff; }\n\
a { color: #0b57d0; }\n\
code, pre { font-family: ui-monospace, monospace; }\n\
pre { overflow: auto; padding: 0.75rem; background: #f4f4f4; }\n\
.search { margin: 0 0 1.5rem; }\n\
.backlinks { margin-top: 2rem; border-top: 1px solid #ccc; padding-top: 0.5rem; }\n\
.backlinks summary { cursor: pointer; }\n\
";

/// Byte slice of the public HTML region, inside the cut-line divider.
pub fn public_html_region(html: &str) -> Option<&str> {
    let start = html.find(PUBLIC_OPEN)? + PUBLIC_OPEN.len();
    let end = html[start..].find(PUBLIC_CLOSE)? + start;
    Some(&html[start..end])
}

/// Byte slice of the preserved backlink region below the cut.
pub fn backlink_html_region(html: &str) -> Option<&str> {
    let start = html.find(BACKLINK_OPEN)? + BACKLINK_OPEN.len();
    let end = html[start..].find(BACKLINK_CLOSE)? + start;
    Some(&html[start..end])
}

/// Reject raw Obsidian `[[wikilinks]]` in the public web stream.
pub fn enforce_public_stream(path: &str, public_html: &str) -> Result<(), CompileError> {
    if public_html.contains("[[") {
        return Err(CompileError::CutLine {
            path: path.to_string(),
        });
    }
    Ok(())
}

/// Render one note. Backlink metadata below the cut keeps its `[[wikilink]]` text.
pub fn emit_page(
    relative_path: &str,
    title: &str,
    public_markdown: &str,
    backlink_markdown: &str,
) -> Result<String, CompileError> {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>");
    html.push_str(&escape_public(title));
    html.push_str("</title>\n<style>\n");
    html.push_str(BASELINE_CSS);
    html.push_str("</style>\n");
    html.push_str(SEARCH_SLOT_OPEN);
    html.push_str("[]</script>\n</head>\n<body>\n<form class=\"search\" role=\"search\">\n<label for=\"garden-search\">Search</label>\n<input id=\"garden-search\" type=\"search\" name=\"q\" autocomplete=\"off\">\n<ul id=\"garden-search-results\"></ul>\n</form>\n<article data-path=\"");
    html.push_str(&escape_attr(relative_path));
    html.push_str("\">\n");
    html.push_str(PUBLIC_OPEN);
    html.push('\n');
    html.push_str(&render_public(public_markdown));
    html.push_str(PUBLIC_CLOSE);
    html.push('\n');
    let backlink = backlink_markdown.trim();
    if !backlink.is_empty() {
        html.push_str(BACKLINK_OPEN);
        html.push_str("\n<details>\n<summary>Backlinks</summary>\n");
        html.push_str(&escape_backlink(backlink));
        html.push_str("\n</details>\n");
        html.push_str(BACKLINK_CLOSE);
        html.push('\n');
    }
    html.push_str("</article>\n</body>\n</html>\n");

    let public_html = public_html_region(&html).ok_or_else(|| CompileError::CutLine {
        path: relative_path.to_string(),
    })?;
    enforce_public_stream(relative_path, public_html)?;
    Ok(html)
}

/// Attach tenant asset links and replace the head search payload.
///
/// `search_index_json` is the MkDocs Material `search/search_index.json` document.
/// `hugo_index_json` is the Hugo `index.json` document. Both are injected into `<head>`.
pub(crate) fn bundle_document(
    html: &str,
    stylesheet_href: &str,
    search_script_href: &str,
    site_root: &str,
    search_index_json: &str,
    hugo_index_json: &str,
) -> Result<String, CompileError> {
    let Some(html) = fill_json_slot(html, "search-index", search_index_json) else {
        return Err(CompileError::HeadSlot);
    };
    let hugo =
        format!("<script type=\"application/json\" id=\"hugo-index\">{hugo_index_json}</script>\n");
    let Some(html) = insert_after_slot(&html, "search-index", &hugo) else {
        return Err(CompileError::HeadSlot);
    };
    let mut extra = String::new();
    if !stylesheet_href.is_empty() {
        extra.push_str("<link rel=\"stylesheet\" href=\"");
        extra.push_str(&escape_attr(stylesheet_href));
        extra.push_str("\">\n");
    }
    if !search_script_href.is_empty() {
        extra.push_str("<script src=\"");
        extra.push_str(&escape_attr(search_script_href));
        extra.push_str("\" defer></script>\n");
    }
    let mut html = if extra.is_empty() {
        html
    } else {
        html.replacen("<head>\n", &format!("<head>\n{extra}"), 1)
    };
    let body = format!("<body data-site-root=\"{}\">\n", escape_attr(site_root));
    html = html.replacen("<body>\n", &body, 1);
    Ok(html)
}

fn fill_json_slot(html: &str, id: &str, json: &str) -> Option<String> {
    let start_tag = format!("<script type=\"application/json\" id=\"{id}\">");
    let start = html.find(&start_tag)? + start_tag.len();
    let end = html[start..].find("</script>")? + start;
    let mut out = String::with_capacity(html.len() + json.len());
    out.push_str(&html[..start]);
    out.push_str(json);
    out.push_str(&html[end..]);
    Some(out)
}

fn insert_after_slot(html: &str, id: &str, insertion: &str) -> Option<String> {
    let start_tag = format!("<script type=\"application/json\" id=\"{id}\">");
    let start = html.find(&start_tag)?;
    let close = html[start..].find("</script>")? + start + "</script>".len();
    let mut out = String::with_capacity(html.len() + insertion.len());
    out.push_str(&html[..close]);
    out.push('\n');
    out.push_str(insertion);
    out.push_str(&html[close..]);
    Some(out)
}

fn render_public(markdown: &str) -> String {
    let mut html = String::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut code: Vec<String> = Vec::new();
    for line in markdown.lines() {
        if in_fence {
            if line.trim_start().starts_with("```") {
                in_fence = false;
                html.push_str("<pre><code>");
                html.push_str(&escape_public(&code.join("\n")));
                html.push_str("</code></pre>\n");
                code.clear();
            } else {
                code.push(line.to_string());
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            flush_paragraph(&mut paragraph, &mut html);
            in_fence = true;
            continue;
        }
        if line.trim().is_empty() {
            flush_paragraph(&mut paragraph, &mut html);
            continue;
        }
        if line.trim() == "---" {
            flush_paragraph(&mut paragraph, &mut html);
            html.push_str("<hr>\n");
            continue;
        }
        if let Some((level, text)) = heading_rest(line.trim_start()) {
            flush_paragraph(&mut paragraph, &mut html);
            html.push_str(&format!("<h{level}>{}</h{level}>\n", render_inlines(text)));
            continue;
        }
        paragraph.push(line.trim().to_string());
    }
    flush_paragraph(&mut paragraph, &mut html);
    if in_fence {
        html.push_str("<pre><code>");
        html.push_str(&escape_public(&code.join("\n")));
        html.push_str("</code></pre>\n");
    }
    html
}

fn flush_paragraph(paragraph: &mut Vec<String>, html: &mut String) {
    if paragraph.is_empty() {
        return;
    }
    let text = paragraph.join(" ");
    paragraph.clear();
    html.push_str("<p>");
    html.push_str(&render_inlines(&text));
    html.push_str("</p>\n");
}

fn heading_rest(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        Some((hashes, line[hashes + 1..].trim()))
    } else {
        None
    }
}

fn render_inlines(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some(end) = inline_code_end(input, i) {
                out.push_str("<code>");
                out.push_str(&escape_public(&input[i + 1..end]));
                out.push_str("</code>");
                i = end + 1;
                continue;
            }
        }
        if let Some((label, url, next)) = take_markdown_link(input, i) {
            out.push_str("<a href=\"");
            out.push_str(&escape_attr(&url));
            out.push_str("\">");
            out.push_str(&escape_public(&label));
            out.push_str("</a>");
            i = next;
            continue;
        }
        let ch = input[i..].chars().next().unwrap();
        push_public_char(&mut out, ch);
        i += ch.len_utf8();
    }
    out
}

fn inline_code_end(input: &str, open: usize) -> Option<usize> {
    let rest = input.get(open + 1..)?;
    let rel = rest.find('`')?;
    let between = &rest[..rel];
    if between.contains('\n') || between.contains('\r') {
        return None;
    }
    Some(open + 1 + rel)
}

fn take_markdown_link(input: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = input.get(start..)?;
    if !rest.starts_with('[') || rest.starts_with("[[") {
        return None;
    }
    let close_rel = rest[1..].find("](")?;
    let label = &rest[1..1 + close_rel];
    if label.contains('[') || label.contains('\n') {
        return None;
    }
    let url_start = start + 1 + close_rel + 2;
    let url_rel = input.get(url_start..)?.find(')')?;
    let url = &input[url_start..url_start + url_rel];
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some((label.to_string(), url.to_string(), url_start + url_rel + 1))
}

fn escape_public(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        push_public_char(&mut out, ch);
    }
    out
}

fn push_public_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '[' => out.push_str("&#91;"),
        ']' => out.push_str("&#93;"),
        _ => out.push(ch),
    }
}

fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

fn escape_backlink(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_stream_has_anchors_and_encoded_code_fences() {
        let html = emit_page(
            "a.md",
            "A",
            "See [Note](https://garden.example/acme/note).\n\n```\n[[x]]\n```\n",
            "Backlinks: [[Secret]]",
        )
        .unwrap();
        let public = public_html_region(&html).unwrap();
        assert!(!public.contains("[["));
        assert!(public.contains("href=\"https://garden.example/acme/note\""));
        assert!(public.contains(">Note</a>"));
        assert!(public.contains("&#91;&#91;x&#93;&#93;"));
        let backlink = backlink_html_region(&html).unwrap();
        assert!(backlink.contains("[[Secret]]"));
        assert!(backlink.contains("<details>"));
        assert!(backlink.contains("<summary>Backlinks</summary>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("id=\"search-index\""));
        assert!(html.contains("id=\"garden-search\""));
    }

    #[test]
    fn enforcer_rejects_a_raw_wikilink() {
        let err = enforce_public_stream("a.md", "<p>[[Nope]]</p>").unwrap_err();
        assert!(matches!(err, CompileError::CutLine { .. }));
    }
}
