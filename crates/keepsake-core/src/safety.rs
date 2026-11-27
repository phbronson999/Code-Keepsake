//! Works out what exists only on this machine, and sorts it into: things that
//! block putting the repo away, things Code Keepsake will preserve, and notes.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    git::{self, RepoInfo},
    parse,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyReport {
    pub fetched: bool,
    pub fetch_error: Option<String>,
    pub has_remote: bool,
    /// `git status --porcelain` lines.
    pub dirty: Vec<String>,
    /// Commits on local branches that no remote has (all commits if there's no remote).
    pub unpushed_commits: usize,
    pub branches_without_upstream: Vec<String>,
    pub stashes: usize,
    pub extra_worktrees: usize,
    pub has_submodules: bool,
    pub important_ignored: Vec<String>,
    pub custom_hooks: Vec<String>,
    pub local_config_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Must be dealt with before Hollow or Full.
    Blocker,
    /// Exists only here, and Code Keepsake will save it into the stub.
    Preserve,
    /// Worth knowing, nothing to do.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "count", rename_all = "lowercase")]
pub enum Verdict {
    /// Everything here is also on a remote.
    Ready,
    /// Safe, but this many things will be saved into the stub.
    Preserve(usize),
    /// This many things must be resolved first.
    Blocked(usize),
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn sample(items: &[String], max: usize) -> String {
    let mut text = items.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > max {
        text.push_str(&format!(" and {} more", items.len() - max));
    }
    text
}

impl SafetyReport {
    pub fn findings(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        let mut push = |severity, title: String, detail: Option<String>| {
            out.push(Finding {
                severity,
                title,
                detail,
            })
        };

        if !self.dirty.is_empty() {
            push(
                Severity::Blocker,
                format!("{} not committed", plural(self.dirty.len(), "change")),
                Some("Commit, stash or discard them first.".into()),
            );
        }
        if self.extra_worktrees > 0 {
            push(
                Severity::Blocker,
                format!("{} use this repo", plural(self.extra_worktrees, "other worktree")),
                Some("Remove them with `git worktree remove` first.".into()),
            );
        }
        if !self.has_remote {
            push(
                Severity::Preserve,
                "No remote".into(),
                Some("Hollow keeps .git; Full bundles the whole history.".into()),
            );
        } else if self.unpushed_commits > 0 {
            push(
                Severity::Preserve,
                format!("{} on no remote", plural(self.unpushed_commits, "commit")),
                Some("Hollow keeps them in .git; Full bundles them.".into()),
            );
        }
        if self.stashes > 0 {
            push(
                Severity::Preserve,
                plural(self.stashes, "stash"),
                Some("Every stash is kept, not just the newest.".into()),
            );
        }
        if !self.important_ignored.is_empty() {
            push(
                Severity::Preserve,
                format!("{} worth keeping", plural(self.important_ignored.len(), "ignored file")),
                Some(format!("Left in place: {}", sample(&self.important_ignored, 3))),
            );
        }
        if !self.custom_hooks.is_empty() {
            push(
                Severity::Preserve,
                format!("Custom hooks: {}", sample(&self.custom_hooks, 3)),
                Some("Copied into the stub.".into()),
            );
        }
        if !self.branches_without_upstream.is_empty() {
            push(
                Severity::Note,
                format!("{} without upstream", plural(self.branches_without_upstream.len(), "branch")),
                Some(sample(&self.branches_without_upstream, 3)),
            );
        }
        if self.has_submodules {
            push(
                Severity::Blocker,
                "Has submodules".into(),
                Some("Not supported yet: they can hold work of their own.".into()),
            );
        }
        if !self.local_config_keys.is_empty() {
            push(
                Severity::Note,
                format!("Local config: {}", sample(&self.local_config_keys, 2)),
                Some("Copied into the stub.".into()),
            );
        }
        if let Some(error) = &self.fetch_error {
            push(Severity::Note, "Fetch failed".into(), Some(error.clone()));
        } else if self.has_remote && !self.fetched {
            push(
                Severity::Note,
                "Not fetched yet".into(),
                Some("Based on the last fetch. Press f to fetch now.".into()),
            );
        }
        out
    }

    pub fn verdict(&self) -> Verdict {
        let findings = self.findings();
        let count = |s| findings.iter().filter(|f| f.severity == s).count();
        match (count(Severity::Blocker), count(Severity::Preserve)) {
            (0, 0) => Verdict::Ready,
            (0, n) => Verdict::Preserve(n),
            (n, _) => Verdict::Blocked(n),
        }
    }
}

fn count_lines(output: Option<String>) -> usize {
    output.map_or(0, |o| o.lines().filter(|l| !l.trim().is_empty()).count())
}

/// Runs every check. With `fetch`, runs `git fetch --all --prune` first so the
/// unpushed-commit count is compared against what the remote has now.
pub fn check(repo: &Path, fetch: bool, ignored: &[String], info: Option<&RepoInfo>) -> SafetyReport {
    let mut report = SafetyReport {
        has_remote: info.is_some_and(|i| !i.remotes.is_empty()),
        ..Default::default()
    };

    if fetch && report.has_remote {
        match git::run(repo, &["fetch", "--all", "--prune", "--quiet"]) {
            Ok(_) => report.fetched = true,
            Err(e) => {
                report.fetch_error = Some(e.to_string().lines().last().unwrap_or("").to_string())
            }
        }
    }

    report.dirty = git::run(repo, &["status", "--porcelain", "--untracked-files=normal"])
        .map(|o| o.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();

    let unpushed: &[&str] = if report.has_remote {
        &["rev-list", "--count", "--branches", "--not", "--remotes"]
    } else {
        &["rev-list", "--count", "--branches"]
    };
    report.unpushed_commits = git::run_opt(repo, unpushed)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    report.branches_without_upstream = git::run(
        repo,
        &["for-each-ref", "--format=%(refname:short)%09%(upstream:short)", "refs/heads"],
    )
    .map(|o| {
        parse::parse_branch_upstreams(&o)
            .into_iter()
            .filter(|(_, upstream)| upstream.is_none())
            .map(|(branch, _)| branch)
            .collect()
    })
    .unwrap_or_default();

    report.stashes = count_lines(git::run_opt(repo, &["stash", "list"]));
    report.extra_worktrees = git::run(repo, &["worktree", "list", "--porcelain"])
        .map(|o| parse::count_worktrees(&o).saturating_sub(1))
        .unwrap_or(0);
    report.has_submodules = repo.join(".gitmodules").exists();

    report.important_ignored = ignored
        .iter()
        .filter(|e| parse::is_important_ignored(e))
        .cloned()
        .collect();

    let hooks = repo.join(".git").join("hooks");
    if let Ok(entries) = std::fs::read_dir(hooks) {
        report.custom_hooks = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with(".sample"))
            .collect();
        report.custom_hooks.sort();
    }

    report.local_config_keys = git::run(repo, &["config", "--local", "--list"])
        .map(|o| parse::interesting_config_keys(&o))
        .unwrap_or_default();

    report
}
