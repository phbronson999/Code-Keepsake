//! Reads `.code-workspace` files: the only place Code Keepsake looks for repos.

use std::path::{Component, Path, PathBuf};

use crate::{jsonc, Error, Result};

#[derive(Debug, Clone)]
pub struct WorkspaceFile {
    pub path: PathBuf,
    /// File name without `.code-workspace`.
    pub name: String,
    pub folders: Vec<WorkspaceFolder>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceFolder {
    /// The optional `"name"` from the workspace file.
    pub name: Option<String>,
    /// Absolute, normalized. May not exist (that's how missing repos show up).
    pub path: PathBuf,
}

impl WorkspaceFile {
    pub fn load(path: &Path) -> Result<Self> {
        let path = absolute(path);
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        Self::parse(&path, &text)
    }

    pub fn parse(path: &Path, text: &str) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(&jsonc::to_strict_json(text)).map_err(|e| Error::Workspace {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;

        let base = path.parent().unwrap_or(Path::new("."));
        let folders = value
            .get("folders")
            .and_then(|f| f.as_array())
            .map(|folders| {
                folders
                    .iter()
                    // `"uri"` folders are remote or virtual: nothing on disk to keep.
                    .filter_map(|f| {
                        let rel = f.get("path")?.as_str()?;
                        Some(WorkspaceFolder {
                            name: f.get("name").and_then(|n| n.as_str()).map(str::to_owned),
                            path: normalize(&base.join(rel)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().trim_end_matches(".code-workspace").to_string())
            .unwrap_or_default();

        Ok(Self {
            path: path.to_path_buf(),
            name,
            folders,
        })
    }
}

/// Resolves `.` and `..` without touching the disk (the folder may be gone).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn absolute(path: &Path) -> PathBuf {
    normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// A comparison key for paths: separators unified, case-folded on Windows.
pub fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('\\', "/");
    let key = key.trim_end_matches('/').to_string();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "bin", "obj", "AppData"];

/// Finds `.code-workspace` files. File arguments are used as given; folders are
/// searched up to `max_depth` levels, skipping hidden and build folders.
pub fn discover(inputs: &[PathBuf], max_depth: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if file_type.is_dir() {
                if depth < max_depth && !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                    walk(&path, depth + 1, max_depth, out);
                }
            } else if file_type.is_file() && name.ends_with(".code-workspace") {
                out.push(path);
            }
        }
    }

    let mut found = Vec::new();
    for input in inputs {
        let input = absolute(input);
        if input.is_file() {
            found.push(input);
        } else if input.is_dir() {
            walk(&input, 0, max_depth, &mut found);
        }
    }
    found.sort_by_key(|p| path_key(p));
    found.dedup_by_key(|p| path_key(p));
    found
}
