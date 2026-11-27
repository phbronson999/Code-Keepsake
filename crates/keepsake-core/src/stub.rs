//! `.keepsake.json`: the file left behind in a folder that was put away.
//! It is the record of truth for that repo; the central index is only a cache.
//! Extra files restore needs sit next to it in `.keepsake/`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    git::{Head, Remote},
    plan::Level,
    size::SizeBreakdown,
    Error, Result,
};

pub const STUB_FILE: &str = ".keepsake.json";
pub const STUB_DIR: &str = ".keepsake";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepsakeStub {
    pub schema_version: u32,
    /// e.g. `code-keepsake/0.1.0`
    pub tool: String,
    pub stored_at_unix: i64,
    pub level: Level,
    pub head: Option<Head>,
    pub upstream: Option<String>,
    pub remotes: Vec<Remote>,
    #[serde(default)]
    pub preserved: Preserved,
    pub sizes_before: SizeBreakdown,
    #[serde(default)]
    pub workspaces: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preserved {
    /// Bundle file next to the stub, holding only what no remote has.
    pub bundle: Option<String>,
    pub bundle_refs: Vec<String>,
    /// One ref per stash (`refs/keepsake/stash/N`), restored oldest first.
    pub stashes: Vec<StashRecord>,
    /// Every local branch and tag, so restore can recreate them.
    pub refs: Vec<SavedRef>,
    pub hooks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRef {
    pub name: String,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashRecord {
    pub reference: String,
    pub message: String,
}

pub fn read(dir: &Path) -> Option<KeepsakeStub> {
    let text = std::fs::read_to_string(dir.join(STUB_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write(dir: &Path, stub: &KeepsakeStub) -> Result<()> {
    let path = dir.join(STUB_FILE);
    let text = serde_json::to_string_pretty(stub).map_err(|e| Error::io(&path, e.into()))?;
    std::fs::write(&path, text).map_err(|e| Error::io(&path, e))
}
