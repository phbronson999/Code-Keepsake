//! The three ways to put a repo away, and what each one would do.
//! Nothing here touches the disk: plans are always shown before anything runs.

use serde::{Deserialize, Serialize};

use crate::{
    safety::{SafetyReport, Severity},
    size::SizeBreakdown,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Delete ignored build output only. The repo stays fully usable.
    Clean,
    /// Keep `.git`, empty the working tree. Restores offline in seconds.
    Hollow,
    /// Keep only the stub. Restoring clones from the remote (or the bundle).
    Full,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Clean, Level::Hollow, Level::Full];

    pub fn label(self) -> &'static str {
        match self {
            Level::Clean => "Clean",
            Level::Hollow => "Hollow",
            Level::Full => "Full",
        }
    }

    pub fn tagline(self) -> &'static str {
        match self {
            Level::Clean => "delete build output, repo stays usable",
            Level::Hollow => "keep history, empty the folder",
            Level::Full => "keep only a stub, clone again on restore",
        }
    }

    pub fn key(self) -> char {
        match self {
            Level::Clean => '1',
            Level::Hollow => '2',
            Level::Full => '3',
        }
    }

    /// Bytes this level frees. Hollow and Full leave a few KB of stub behind.
    pub fn bytes_freed(self, sizes: &SizeBreakdown) -> u64 {
        match self {
            Level::Clean => sizes.build,
            Level::Hollow => sizes.build + sizes.source + sizes.ignored_other,
            Level::Full => sizes.total(),
        }
    }

    /// Clean only deletes ignored build folders, so nothing can block it.
    pub fn can_be_blocked(self) -> bool {
        self != Level::Clean
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffloadPlan {
    pub level: Level,
    pub bytes_freed: u64,
    pub steps: Vec<String>,
    pub blocked_by: Vec<String>,
}

pub fn plan(level: Level, sizes: &SizeBreakdown, safety: Option<&SafetyReport>) -> OffloadPlan {
    let findings = safety.map(|s| s.findings()).unwrap_or_default();
    let preserve: Vec<&str> = findings
        .iter()
        .filter(|f| f.severity == Severity::Preserve)
        .map(|f| f.title.as_str())
        .collect();
    let blocked_by = if level.can_be_blocked() {
        findings
            .iter()
            .filter(|f| f.severity == Severity::Blocker)
            .map(|f| f.title.clone())
            .collect()
    } else {
        Vec::new()
    };

    let mut steps = Vec::new();
    match level {
        Level::Clean => {
            steps.push("Delete ignored build folders".to_string());
            steps.push("Leave source, history and settings as they are".to_string());
        }
        Level::Hollow | Level::Full => {
            if level == Level::Full {
                steps.push("Fetch and confirm the remote has every commit".to_string());
            }
            if !preserve.is_empty() {
                steps.push(format!("Save to the stub: {}", preserve.join(", ").to_lowercase()));
            }
            steps.push("Write .keepsake.json with remotes, branch and commit".to_string());
            steps.push(if level == Level::Hollow {
                "Empty the folder but keep .git".to_string()
            } else {
                "Delete everything except the stub".to_string()
            });
        }
    }

    OffloadPlan {
        level,
        bytes_freed: level.bytes_freed(sizes),
        steps,
        blocked_by,
    }
}
