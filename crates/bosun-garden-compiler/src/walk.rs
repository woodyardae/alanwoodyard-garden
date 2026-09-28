//! Breadth-first Rayon walk of a Markdown vault.
//!
//! Each wave reads its directories on the pool. The next wave runs only after
//! that collect finishes, so workers never block on a nested parallel iterator.

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
/// Skips symlinks and `.git`, `.obsidian`, `.bosun`, `target`, and `node_modules`.
pub fn walk_markdown(root: &Path) -> Result<Vec<PathBuf>, CompileError> {
    if !root.is_dir() {
        return Err(CompileError::Io {
            path: root.to_path_buf(),
            source: io::Error::new(io::ErrorKind::NotFound, "vault root is not a directory"),
        });
    }
    let mut dirs = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while !dirs.is_empty() {
        let batch = std::mem::take(&mut dirs);
        let listed: Vec<Result<DirListing, CompileError>> =
            batch.par_iter().map(|dir| list_dir(dir)).collect();
        for listing in listed {
            let listing = listing?;
            files.extend(listing.files);
            dirs.extend(listing.subdirs);
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
        if matches!(
            name.as_str(),
            ".git" | ".obsidian" | ".bosun" | "target" | "node_modules"
        ) {
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
}
