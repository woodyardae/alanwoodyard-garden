//! Breadth-first Rayon walk of a Markdown vault.
//!
//! Each wave reads its directories on the pool. The next wave runs only after
//! that collect finishes, so workers never block on a nested parallel iterator.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use rayon::prelude::*;

use crate::CompileError;

#[derive(Debug)]
struct DirListing {
    files: Vec<PathBuf>,
    subdirs: Vec<PathBuf>,
}

/// Recursively collect `.md` files under `root`.
///
/// Skips symlinks, circular directory traversal, and ignored directories:
/// `.git`, `.obsidian`, `.trash`, `.bosun`, `target`, `node_modules`,
/// temporary/backup folders, raw vault imports, and staging directories.
pub fn walk_markdown(root: &Path) -> Result<Vec<PathBuf>, CompileError> {
    if !root.is_dir() {
        return Err(CompileError::Io {
            path: root.to_path_buf(),
            source: io::Error::new(io::ErrorKind::NotFound, "vault root is not a directory"),
        });
    }
    let mut dirs = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut visited = BTreeSet::new();
    if let Ok(canon) = fs::canonicalize(root) {
        visited.insert(canon);
    } else {
        visited.insert(root.to_path_buf());
    }

    while !dirs.is_empty() {
        let batch = std::mem::take(&mut dirs);
        let listed: Vec<Result<DirListing, CompileError>> =
            batch.par_iter().map(|dir| list_dir(dir)).collect();
        for listing in listed {
            let listing = listing?;
            files.extend(listing.files);
            for subdir in listing.subdirs {
                let canon = fs::canonicalize(&subdir).unwrap_or_else(|_| subdir.clone());
                if visited.insert(canon) {
                    dirs.push(subdir);
                }
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn list_dir(dir: &Path) -> Result<DirListing, CompileError> {
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    let entries = fs::read_dir(dir).map_err(|source| CompileError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| CompileError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if is_ignored_dir(&name) {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| CompileError::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            subdirs.push(path);
        } else if file_type.is_file() && name.ends_with(".md") {
            files.push(path);
        }
    }
    Ok(DirListing { files, subdirs })
}

pub(crate) fn is_ignored_dir(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | ".obsidian"
            | ".trash"
            | ".bosun"
            | "target"
            | "node_modules"
            | "00 inbox"
            | "00_inbox"
            | "imported vaults"
            | "01 imported archive"
            | "quartz-site"
            | "quartz"
    ) || lower.contains("_quarantine")
        || lower.contains("candidate notes")
        || lower.contains("candidate_notes")
        || is_temp_or_backup_dir(&lower)
}

fn is_temp_or_backup_dir(lower: &str) -> bool {
    matches!(
        lower,
        "tmp"
            | ".tmp"
            | "temp"
            | ".temp"
            | "trash"
            | ".trash"
            | "backup"
            | ".backup"
            | "backups"
            | ".backups"
            | "_backup"
            | "_backups"
    ) || lower.starts_with(".trash")
        || lower.starts_with("tmp_")
        || lower.starts_with("temp_")
        || lower.starts_with(".tmp")
        || lower.starts_with(".temp")
        || lower.starts_with("backup_")
        || lower.ends_with("_backup")
        || lower.ends_with(".bak")
        || lower.ends_with(".backup")
        || lower.ends_with('~')
}

pub(crate) fn relative_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_finds_nested_markdown_and_skips_git() {
        let root = std::env::temp_dir().join(format!("bosun-garden-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("a.md"), "a\n").unwrap();
        fs::write(root.join("sub").join("b.md"), "b\n").unwrap();
        fs::write(root.join("skip.txt"), "nope\n").unwrap();
        fs::write(root.join(".git").join("hidden.md"), "hidden\n").unwrap();

        let found = walk_markdown(&root).unwrap();
        let rel: Vec<String> = found.iter().map(|path| relative_key(&root, path)).collect();
        assert_eq!(rel, vec!["a.md".to_string(), "sub/b.md".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_skips_node_modules_and_target() {
        let root = std::env::temp_dir().join(format!(
            "bosun-garden-walk-skip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::create_dir_all(root.join("target").join("debug")).unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("keep.md"), "keep\n").unwrap();
        fs::write(root.join("notes").join("nested.md"), "nested\n").unwrap();
        fs::write(
            root.join("node_modules").join("pkg").join("dep.md"),
            "dep\n",
        )
        .unwrap();
        fs::write(
            root.join("target").join("debug").join("built.md"),
            "built\n",
        )
        .unwrap();

        let found = walk_markdown(&root).unwrap();
        let rel: Vec<String> = found.iter().map(|path| relative_key(&root, path)).collect();
        assert_eq!(
            rel,
            vec!["keep.md".to_string(), "notes/nested.md".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_skips_trash_and_backup_and_temp_folders() {
        let root = std::env::temp_dir().join(format!(
            "bosun-garden-walk-trash-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".trash")).unwrap();
        fs::create_dir_all(root.join(".Trash-1000")).unwrap();
        fs::create_dir_all(root.join("trash")).unwrap();
        fs::create_dir_all(root.join("backup")).unwrap();
        fs::create_dir_all(root.join(".backup")).unwrap();
        fs::create_dir_all(root.join("notes_backup")).unwrap();
        fs::create_dir_all(root.join("notes.bak")).unwrap();
        fs::create_dir_all(root.join("tmp")).unwrap();
        fs::create_dir_all(root.join(".tmp")).unwrap();
        fs::create_dir_all(root.join("temp")).unwrap();
        fs::create_dir_all(root.join(".temp")).unwrap();
        fs::create_dir_all(root.join("drafts~")).unwrap();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::create_dir_all(root.join(".bosun")).unwrap();
        fs::create_dir_all(root.join("valid")).unwrap();

        fs::write(root.join("valid").join("note.md"), "# Valid\n").unwrap();
        fs::write(root.join(".trash").join("t1.md"), "# Trash\n").unwrap();
        fs::write(root.join(".Trash-1000").join("t2.md"), "# Trash\n").unwrap();
        fs::write(root.join("trash").join("t3.md"), "# Trash\n").unwrap();
        fs::write(root.join("backup").join("b1.md"), "# Backup\n").unwrap();
        fs::write(root.join(".backup").join("b2.md"), "# Backup\n").unwrap();
        fs::write(root.join("notes_backup").join("b3.md"), "# Backup\n").unwrap();
        fs::write(root.join("notes.bak").join("b4.md"), "# Backup\n").unwrap();
        fs::write(root.join("tmp").join("tmp.md"), "# Tmp\n").unwrap();
        fs::write(root.join(".tmp").join("tmp2.md"), "# Tmp\n").unwrap();
        fs::write(root.join("temp").join("temp.md"), "# Temp\n").unwrap();
        fs::write(root.join(".temp").join("temp2.md"), "# Temp\n").unwrap();
        fs::write(root.join("drafts~").join("tilde.md"), "# Tilde\n").unwrap();
        fs::write(root.join(".obsidian").join("workspace.md"), "# Obs\n").unwrap();
        fs::write(root.join(".bosun").join("meta.md"), "# Bosun\n").unwrap();

        let found = walk_markdown(&root).unwrap();
        let rel: Vec<String> = found.iter().map(|path| relative_key(&root, path)).collect();
        assert_eq!(rel, vec!["valid/note.md".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_guards_against_circular_traversal() {
        let root = std::env::temp_dir().join(format!(
            "bosun-garden-walk-loop-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("root.md"), "# Root\n").unwrap();
        fs::write(root.join("sub").join("nested.md"), "# Nested\n").unwrap();

        // On Windows or Unix, attempt to create a directory junction / symlink cycle
        #[cfg(windows)]
        {
            let link = root.join("sub").join("cycle");
            let target = &root;
            let _ = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J", link.to_str().unwrap(), target.to_str().unwrap()])
                .output();
        }
        #[cfg(unix)]
        {
            let link = root.join("sub").join("cycle");
            let _ = std::os::unix::fs::symlink(&root, link);
        }

        // walk_markdown should terminate cleanly and not loop endlessly
        let found = walk_markdown(&root).unwrap();
        let rel: Vec<String> = found.iter().map(|path| relative_key(&root, path)).collect();
        assert!(rel.contains(&"root.md".to_string()));
        assert!(rel.contains(&"sub/nested.md".to_string()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_skips_raw_import_and_staging_folders() {
        let root = std::env::temp_dir().join(format!(
            "bosun-garden-walk-staging-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("00 Inbox")).unwrap();
        fs::create_dir_all(root.join("00_Inbox")).unwrap();
        fs::create_dir_all(root.join("Imported Vaults").join("Nested")).unwrap();
        fs::create_dir_all(root.join("01 Imported Archive")).unwrap();
        fs::create_dir_all(root.join("quartz-site")).unwrap();
        fs::create_dir_all(root.join("quartz")).unwrap();
        fs::create_dir_all(root.join("_quarantine")).unwrap();
        fs::create_dir_all(root.join("staging").join("legacy_quarantine_box")).unwrap();
        fs::create_dir_all(root.join("Candidate Notes")).unwrap();
        fs::create_dir_all(root.join("02 Candidate Notes")).unwrap();
        fs::create_dir_all(root.join("03 Published")).unwrap();
        fs::create_dir_all(root.join("woodyardae")).unwrap();

        fs::write(root.join("00 Inbox").join("inbox1.md"), "# Inbox 1\n").unwrap();
        fs::write(root.join("00_Inbox").join("inbox2.md"), "# Inbox 2\n").unwrap();
        fs::write(
            root.join("Imported Vaults").join("Nested").join("imported.md"),
            "# Imported\n",
        )
        .unwrap();
        fs::write(
            root.join("01 Imported Archive").join("archive.md"),
            "# Archive\n",
        )
        .unwrap();
        fs::write(root.join("quartz-site").join("site.md"), "# Quartz Site\n").unwrap();
        fs::write(root.join("quartz").join("q.md"), "# Quartz\n").unwrap();
        fs::write(root.join("_quarantine").join("quarantine.md"), "# Bad\n").unwrap();
        fs::write(
            root.join("staging")
                .join("legacy_quarantine_box")
                .join("bad.md"),
            "# Bad 2\n",
        )
        .unwrap();
        fs::write(
            root.join("Candidate Notes").join("candidate1.md"),
            "# Candidate 1\n",
        )
        .unwrap();
        fs::write(
            root.join("02 Candidate Notes").join("candidate2.md"),
            "# Candidate 2\n",
        )
        .unwrap();
        fs::write(
            root.join("03 Published").join("pub.md"),
            "# Clean Published Note\n",
        )
        .unwrap();
        fs::write(
            root.join("woodyardae").join("tenant.md"),
            "# Clean Tenant Note\n",
        )
        .unwrap();

        let found = walk_markdown(&root).unwrap();
        let rel: Vec<String> = found.iter().map(|path| relative_key(&root, path)).collect();
        assert_eq!(
            rel,
            vec![
                "03 Published/pub.md".to_string(),
                "woodyardae/tenant.md".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn is_ignored_dir_matches_staging_and_import_patterns() {
        assert!(is_ignored_dir("00 Inbox"));
        assert!(is_ignored_dir("00 inbox"));
        assert!(is_ignored_dir("00_Inbox"));
        assert!(is_ignored_dir("00_inbox"));
        assert!(is_ignored_dir("Imported Vaults"));
        assert!(is_ignored_dir("imported vaults"));
        assert!(is_ignored_dir("01 Imported Archive"));
        assert!(is_ignored_dir("01 imported archive"));
        assert!(is_ignored_dir("quartz-site"));
        assert!(is_ignored_dir("QUARTZ-SITE"));
        assert!(is_ignored_dir("quartz"));
        assert!(is_ignored_dir("Quartz"));
        assert!(is_ignored_dir("_quarantine"));
        assert!(is_ignored_dir("vault_quarantine_box"));
        assert!(is_ignored_dir("Candidate Notes"));
        assert!(is_ignored_dir("02 Candidate Notes"));
        assert!(is_ignored_dir("candidate_notes"));

        // Clean content directories should NOT be ignored
        assert!(!is_ignored_dir("03 Published"));
        assert!(!is_ignored_dir("woodyardae"));
        assert!(!is_ignored_dir("notes"));
    }
}
