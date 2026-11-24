//! `.keepsake.json`: the file left behind in a folder that was put away.
//! It is the record of truth for that repo; the central index is only a cache.
//! (Writing stubs arrives with the `store` command; reading them works now.)

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    git::{Head, Remote},
    plan::Level,
    size::SizeBreakdown,
};

pub const STUB_FILE: &str = ".keepsake.json";
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
    /// Zip of important ignored files (`.env`, keys, local settings).
    pub ignored_archive: Option<String>,
    pub hooks: Vec<String>,
    pub local_config: BTreeMap<String, String>,
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
