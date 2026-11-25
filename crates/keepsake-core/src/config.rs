//! Saved settings, so nothing has to be typed on the command line.
//! Windows: %APPDATA%\code-keepsake\config.json
//! Elsewhere: $XDG_CONFIG_HOME/code-keepsake/config.json (or ~/.config/…)

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Folders to search for `.code-workspace` files, or individual workspace files.
    pub sources: Vec<PathBuf>,
    /// How many folder levels deep to search each source.
    pub search_depth: usize,
    /// How many levels below each workspace folder to look for `.git`, per workspace
    /// file name. Missing entries use `repo_depth_default`.
    pub repo_depth: BTreeMap<String, usize>,
    /// 0 means a workspace folder is just the one repo it points at (or inside).
    pub repo_depth_default: usize,
}

/// Deepest nested-repo search the UI allows.
pub const MAX_REPO_DEPTH: usize = 6;

impl Default for Config {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            search_depth: 3,
            repo_depth: BTreeMap::new(),
            repo_depth_default: 0,
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("code-keepsake"))
}

impl Config {
    pub fn depth_for(&self, workspace: &str) -> usize {
        self.repo_depth
            .get(workspace)
            .copied()
            .unwrap_or(self.repo_depth_default)
    }

    /// Sets a workspace's depth, dropping the entry when it matches the default.
    pub fn set_depth(&mut self, workspace: &str, depth: usize) {
        let depth = depth.min(MAX_REPO_DEPTH);
        if depth == self.repo_depth_default {
            self.repo_depth.remove(workspace);
        } else {
            self.repo_depth.insert(workspace.to_string(), depth);
        }
    }

    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("config.json"))
    }

    /// Never fails: a missing or unreadable config just means defaults.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().ok_or_else(|| {
            Error::io(
                "config",
                std::io::Error::new(std::io::ErrorKind::NotFound, "no config folder on this system"),
            )
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(&path, text).map_err(|e| Error::io(&path, e))
    }
}
