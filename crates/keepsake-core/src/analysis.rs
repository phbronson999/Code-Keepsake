//! Everything known about one repo after a scan.

use std::path::PathBuf;

use serde::Serialize;

use crate::{
    git::{self, RepoInfo},
    plan::Level,
    safety::{self, SafetyReport, Verdict},
    scan::RepoTarget,
    size::{self, SizeBreakdown},
    stub::{self, KeepsakeStub},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoStatus {
    /// A normal git repo.
    Active,
    /// Put away: the folder holds a `.keepsake.json` stub instead of a repo.
    Kept,
    /// Listed in a workspace, but the folder doesn't exist.
    Missing,
    /// The folder exists but isn't inside a git repo.
    NotGit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoAnalysis {
    pub path: PathBuf,
    pub name: String,
    pub workspaces: Vec<String>,
    pub status: RepoStatus,
    pub info: Option<RepoInfo>,
    pub sizes: Option<SizeBreakdown>,
    pub safety: Option<SafetyReport>,
    pub stub: Option<KeepsakeStub>,
    pub error: Option<String>,
}

impl RepoAnalysis {
    /// Time since the last commit, or since it was put away.
    pub fn idle_secs(&self, now: i64) -> Option<i64> {
        match self.status {
            RepoStatus::Kept => self.stub.as_ref().map(|s| (now - s.stored_at_unix).max(0)),
            _ => self
                .info
                .as_ref()?
                .last_commit_unix
                .map(|t| (now - t).max(0)),
        }
    }

    pub fn verdict(&self) -> Option<Verdict> {
        self.safety.as_ref().map(SafetyReport::verdict)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self.verdict(), Some(Verdict::Blocked(_)))
    }

    /// Bytes freed at `level`, or `None` if that level isn't possible right now.
    pub fn freed_at(&self, level: Level) -> Option<u64> {
        if self.status != RepoStatus::Active || (level.can_be_blocked() && self.is_blocked()) {
            return None;
        }
        self.sizes.map(|s| level.bytes_freed(&s))
    }

    /// Ranks repos worth putting away: lots of space and a long time idle.
    pub fn score(&self, now: i64) -> f64 {
        let Some(freed) = self.freed_at(Level::Full).or_else(|| self.freed_at(Level::Clean)) else {
            return -1.0;
        };
        let idle_days = self.idle_secs(now).unwrap_or(0) as f64 / 86_400.0;
        let idle_factor = (idle_days / 30.0).clamp(0.1, 36.0);
        freed as f64 / 1_048_576.0 * idle_factor
    }
}

/// Inspects one repo. Never fails: problems land in `error` so one broken repo
/// doesn't stop a scan.
pub fn analyze(target: &RepoTarget, fetch: bool) -> RepoAnalysis {
    let mut analysis = RepoAnalysis {
        path: target.path.clone(),
        name: target.name.clone(),
        workspaces: target.workspaces.clone(),
        status: RepoStatus::Missing,
        info: None,
        sizes: None,
        safety: None,
        stub: None,
        error: None,
    };

    if !target.path.exists() {
        return analysis;
    }

    analysis.stub = stub::read(&target.path);
    if !git::has_git_dir(&target.path) {
        analysis.status = if analysis.stub.is_some() {
            RepoStatus::Kept
        } else {
            RepoStatus::NotGit
        };
        return analysis;
    }

    analysis.status = RepoStatus::Active;
    let info = match git::inspect(&target.path) {
        Ok(info) => info,
        Err(e) => {
            analysis.error = Some(e.to_string());
            return analysis;
        }
    };

    let ignored = git::ignored_entries(&target.path).unwrap_or_default();
    analysis.sizes = Some(size::measure(&target.path, &ignored));
    analysis.safety = Some(safety::check(&target.path, fetch, &ignored, Some(&info)));
    analysis.info = Some(info);
    analysis
}
