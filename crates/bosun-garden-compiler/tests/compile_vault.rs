use std::fs;
use std::path::PathBuf;
use std::process;

use bosun_garden_compiler::{
    backlink_html_region, compile_vault, emit_tenant_sites, public_html_region, walk_vault,
    BuildEvent, CompileError, EdgeKind,
};

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

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "bosun-garden-{name}-{}-{}",
            process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn compiles_a_tenant_vault_without_public_wikilinks() {
    let scratch = Scratch::new("vault");
    scratch.write("hello.md", HELLO);
    scratch.write("detail.md", DETAIL);
    scratch.write("nested.md", NESTED);
    scratch.write("Harbor Log.md", HARBOR);
    scratch.write(
        "sub/deep.md",
        "---\nblog: \"acme\"\ntitle: Deep\n---\n\nDeep page.\n",
    );
    scratch.write(
        "other.md",
        "---\nblog: \"other\"\ntitle: Other\n---\n\nOther tenant.\n",
    );
    scratch.write("skip.txt", "[[ShouldNotAppear]]\n");
    scratch.write(
        ".git/hidden.md",
        "---\nblog: \"acme\"\n---\n\nhidden secret phrase\n",
    );

    let walked = walk_vault(&scratch.0).unwrap();
    let rel: Vec<String> = walked
        .iter()
        .map(|path| {
            path.strip_prefix(&scratch.0)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    assert!(rel.contains(&"hello.md".to_string()));
    assert!(rel.contains(&"sub/deep.md".to_string()));
    assert!(rel.contains(&"other.md".to_string()));
    assert!(!rel.iter().any(|path| path.contains("hidden.md")));
    assert!(!rel.iter().any(|path| path.ends_with("skip.txt")));

    let vault = compile_vault(&scratch.0, "blog", "acme", "https://garden.example").unwrap();
    let paths: Vec<&str> = vault
        .pages
        .iter()
        .map(|page| page.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "Harbor Log.md",
            "detail.md",
            "hello.md",
            "nested.md",
            "sub/deep.md",
        ]
    );

    let hello = vault
        .pages
        .iter()
        .find(|page| page.relative_path == "hello.md")
        .unwrap();
    let public = public_html_region(&hello.html).unwrap();
    assert!(!public.contains("[["), "{public}");
    assert!(public.contains("href=\"https://garden.example/acme/harbor-log\""));
    assert!(public.contains(">the log</a>"));
    assert!(public.contains("Embedded fact"));
    assert!(public.contains("Nested fact"));
    assert!(!public.contains("Nope"));
    assert!(!public.contains("Secret"));
    let backlink = backlink_html_region(&hello.html).unwrap();
    assert!(backlink.contains("[[Secret]]"));
    assert!(!hello.html.contains("Other tenant."));
    assert!(!hello.html.contains("hidden secret phrase"));
    assert!(!hello.html.contains("ShouldNotAppear"));

    assert!(vault.graph.edges.iter().any(|edge| {
        edge.kind == EdgeKind::Wikilink
            && edge.from == "hello.md"
            && edge.to == "Harbor Log.md"
            && edge.resolved
    }));
    assert!(vault.graph.edges.iter().any(|edge| {
        edge.kind == EdgeKind::Transclusion
            && edge.from == "hello.md"
            && edge.to == "detail.md"
            && edge.resolved
    }));
    assert!(vault.graph.edges.iter().any(|edge| {
        edge.kind == EdgeKind::Transclusion
            && edge.from == "detail.md"
            && edge.to == "nested.md"
            && edge.resolved
    }));
}

#[test]
fn emits_isolated_tenant_directories() {
    let scratch = Scratch::new("emit");
    scratch.write("hello.md", HELLO);
    scratch.write("detail.md", DETAIL);
    scratch.write("nested.md", NESTED);
    scratch.write("Harbor Log.md", HARBOR);
    scratch.write(
        "sub/deep.md",
        "---\nblog: \"acme\"\ntitle: Deep\n---\n\nDeep page.\n",
    );
    scratch.write(
        "other.md",
        "---\nblog: \"other\"\ntitle: Other\n---\n\nOther tenant.\n",
    );
    scratch.write(
        "guides/index.md",
        "---\nblog: \"acme\"\ntitle: Guides\n---\n\nGuide home.\n",
    );

    let dist = scratch.0.join("dist");
    let sites = emit_tenant_sites(
        &scratch.0,
        "blog",
        "https://garden.example/",
        &dist,
        None,
        |_| {},
    )
    .unwrap();
    let names: Vec<&str> = sites.iter().map(|site| site.tenant.as_str()).collect();
    assert_eq!(names, vec!["acme", "other"]);

    let hello_path = dist.join("acme").join("hello").join("index.html");
    let hello = fs::read_to_string(&hello_path).unwrap();
    let public = public_html_region(&hello).unwrap();
    assert!(!public.contains("[["), "{public}");
    assert!(public.contains("href=\"https://garden.example/acme/harbor-log\""));
    assert!(public.contains(">the log</a>"));
    assert!(public.contains("Embedded fact"));
    assert!(!public.contains("Nope"));
    assert!(!public.contains("Secret"));
    assert!(!hello.contains("Other tenant."));
    let backlink = backlink_html_region(&hello).unwrap();
    assert!(backlink.contains("<details>"));
    assert!(backlink.contains("<summary>Backlinks</summary>"));
    assert!(backlink.contains("[[Secret]]"));
    assert!(hello.contains("<style>"));
    assert!(hello.contains("id=\"search-index\""));
    assert!(hello.contains("id=\"hugo-index\""));
    assert!(hello.contains("href=\"../assets/stylesheets/main.css\""));
    assert!(hello.contains("src=\"../assets/javascripts/search.js\""));
    assert!(hello.contains("data-site-root=\"../\""));

    let harbor = dist.join("acme").join("harbor-log").join("index.html");
    assert!(harbor.is_file());
    assert!(dist
        .join("acme")
        .join("sub")
        .join("deep")
        .join("index.html")
        .is_file());
    assert!(dist
        .join("acme")
        .join("guides")
        .join("index.html")
        .is_file());
    assert!(!dist.join("acme").join("guides").join("index").exists());

    let home = fs::read_to_string(dist.join("acme").join("index.html")).unwrap();
    assert!(home.contains("href=\"assets/stylesheets/main.css\""));
    assert!(home.contains("https://garden.example/acme/hello"));
    assert!(dist.join("acme").join("404.html").is_file());
    assert!(dist.join("acme").join("sitemap.xml").is_file());
    let sitemap = fs::read_to_string(dist.join("acme").join("sitemap.xml")).unwrap();
    assert!(sitemap.contains("https://garden.example/acme/harbor-log"));
    assert!(!sitemap.contains("other"));

    let css = fs::read_to_string(
        dist.join("acme")
            .join("assets")
            .join("stylesheets")
            .join("main.css"),
    )
    .unwrap();
    assert!(css.contains(".backlinks summary"));
    assert!(dist
        .join("acme")
        .join("assets")
        .join("javascripts")
        .join("search.js")
        .is_file());

    let search =
        fs::read_to_string(dist.join("acme").join("search").join("search_index.json")).unwrap();
    assert!(search.contains("\"location\": \"hello/\""));
    assert!(search.contains("Embedded fact"));
    assert!(search.contains("Guide home"));
    assert!(!search.contains("Secret"));
    assert!(!search.contains("[["));
    assert!(!search.contains("Other tenant"));
    assert!(hello.contains("Embedded fact"));
    let hugo = fs::read_to_string(dist.join("acme").join("index.json")).unwrap();
    assert!(hugo.contains("\"permalink\": \"https://garden.example/acme/harbor-log\""));
    assert!(!hugo.contains("Secret"));

    let other = fs::read_to_string(dist.join("other").join("other").join("index.html")).unwrap();
    assert!(other.contains("Other tenant."));
    assert!(!other.contains("Embedded fact"));
    assert!(!other.contains("[[Secret]]"));
    let other_search =
        fs::read_to_string(dist.join("other").join("search").join("search_index.json")).unwrap();
    assert!(other_search.contains("Other tenant"));
    assert!(!other_search.contains("Embedded fact"));
    assert!(!other_search.contains("Secret"));
}

#[test]
fn rejects_a_tenant_slug_that_escapes_dist() {
    let scratch = Scratch::new("unsafe-tenant");
    scratch.write("bad.md", "---\nblog: \"../x\"\ntitle: Bad\n---\n\nNope\n");
    let dist = scratch.0.join("dist");
    let err = emit_tenant_sites(
        &scratch.0,
        "blog",
        "https://garden.example",
        &dist,
        None,
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::UnsafeTenant { .. }));
    assert_eq!(err.code(), "unsafe_tenant");
    assert!(!dist.exists());
}

#[test]
fn emits_only_the_requested_tenant_and_reports_progress() {
    let scratch = Scratch::new("one-tenant");
    scratch.write("hello.md", HELLO);
    scratch.write(
        "other.md",
        "---\nblog: \"other\"\ntitle: Other\n---\n\nOther tenant.\n",
    );
    let dist = scratch.0.join("dist");
    let mut events = Vec::new();
    let sites = emit_tenant_sites(
        &scratch.0,
        "blog",
        "https://garden.example",
        &dist,
        Some("acme"),
        |event| events.push(event),
    )
    .unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].tenant, "acme");
    assert!(dist.join("acme").join("hello").join("index.html").is_file());
    assert!(!dist.join("other").exists());
    assert!(matches!(
        events.first(),
        Some(BuildEvent::Started { tenant }) if tenant == "acme"
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        BuildEvent::Page { source, .. } if source == "hello.md"
    )));
    assert!(matches!(events.last(), Some(BuildEvent::Finished { pages, .. }) if *pages >= 1));

    let missing = emit_tenant_sites(
        &scratch.0,
        "blog",
        "https://garden.example",
        &dist,
        Some("nope"),
        |_| {},
    )
    .unwrap_err();
    assert!(matches!(missing, CompileError::UnknownTenant { .. }));
    assert_eq!(missing.code(), "unknown_tenant");
    assert_eq!(missing.diagnostic_path().as_deref(), Some("nope"));
}

#[test]
fn rejects_non_utf8_markdown() {
    let scratch = Scratch::new("utf8");
    fs::write(scratch.0.join("bad.md"), [0xff, 0xfe, b'\n']).unwrap();
    let err = compile_vault(&scratch.0, "blog", "acme", "https://garden.example").unwrap_err();
    assert!(matches!(err, CompileError::Utf8 { .. }));
}
