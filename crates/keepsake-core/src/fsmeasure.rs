//! Measures a repo folder in one pass and splits the bytes into categories.
//! std-only. Symlinks and junctions are never followed or counted.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Measured {
    /// Ignored build folders (`target/`, `node_modules/`, …).
    pub build: u64,
    /// The `.git` folder.
    pub git: u64,
    /// Everything not ignored: tracked files and untracked-but-not-ignored files.
    pub source: u64,
    /// Ignored files that aren't build output (`.env`, logs, local DBs, …).
    pub ignored_other: u64,
}

#[derive(Clone, Copy)]
enum Category {
    Build,
    Git,
    Ignored,
}

/// `ignored` is the output of `git ls-files --others --ignored --exclude-standard
/// --directory -z`: repo-relative paths with `/` separators, directories ending in `/`.
pub fn measure(repo: &Path, ignored: &[String]) -> Measured {
    let lookup: HashMap<&str, Category> = ignored
        .iter()
        .map(|entry| {
            let category = if crate::parse::is_artifact_entry(entry) {
                Category::Build
            } else {
                Category::Ignored
            };
            (entry.trim_end_matches('/'), category)
        })
        .collect();

    let mut measured = Measured::default();
    let mut stack: Vec<(PathBuf, String, Option<Category>)> =
        vec![(repo.to_path_buf(), String::new(), None)];

    while let Some((dir, rel, category)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let child_rel = if rel.is_empty() {
                name.to_string()
            } else {
                format!("{rel}/{name}")
            };
            // git can list both `src-tauri/` and `src-tauri/target/` when a folder
            // holds only ignored files, so a nested build folder wins over its parent.
            let child_category = if rel.is_empty() && name == ".git" {
                Some(Category::Git)
            } else {
                match (category, lookup.get(child_rel.as_str()).copied()) {
                    (Some(Category::Git), _) => Some(Category::Git),
                    (_, Some(Category::Build)) => Some(Category::Build),
                    (None, found) => found,
                    (current, _) => current,
                }
            };

            if file_type.is_dir() {
                stack.push((entry.path(), child_rel, child_category));
            } else if file_type.is_file() {
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                match child_category {
                    None => measured.source += len,
                    Some(Category::Build) => measured.build += len,
                    Some(Category::Git) => measured.git += len,
                    Some(Category::Ignored) => measured.ignored_other += len,
                }
            }
        }
    }
    measured
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_into_categories() {
        let root = std::env::temp_dir().join(format!("keepsake-measure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |rel: &str, len: usize| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![b'x'; len]).unwrap();
        };
        write("src/main.rs", 100);
        write(".git/objects/pack/p.pack", 1000);
        write("node_modules/a/index.js", 5000);
        write("src-tauri/target/debug/app.exe", 7000);
        write(".env", 10);
        write("docs/notes.md", 50);

        // Real `git ls-files` output for this layout: `src-tauri/` holds only
        // ignored files, so git lists it as well as `src-tauri/target/`.
        let ignored = vec![
            ".env".to_string(),
            "node_modules/".to_string(),
            "src-tauri/".to_string(),
            "src-tauri/target/".to_string(),
        ];
        let m = measure(&root, &ignored);
        assert_eq!(m.build, 12_000);
        assert_eq!(m.git, 1000);
        assert_eq!(m.source, 150);
        assert_eq!(m.ignored_other, 10);
        let _ = fs::remove_dir_all(&root);
    }
}
