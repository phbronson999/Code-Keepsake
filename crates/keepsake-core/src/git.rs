//! Runs the `git` command. Shelling out (instead of libgit2) means fetches use
//! the user's existing credential helper and SSH setup with no extra work.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

use crate::{parse, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub branch: Option<String>,
    pub commit: String,
    pub detached: bool,
}

impl Head {
    pub fn short_commit(&self) -> &str {
        &self.commit[..self.commit.len().min(7)]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub head: Option<Head>,
    pub upstream: Option<String>,
    pub remotes: Vec<Remote>,
    pub last_commit_unix: Option<i64>,
}

/// Runs `git -C <repo> <args>` and returns stdout.
pub fn run(repo: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        // Never sit waiting for a password prompt behind the TUI.
        .env("GIT_TERMINAL_PROMPT", "0");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::GitMissing
        } else {
            Error::io(repo, e)
        }
    })?;

    if !output.status.success() {
        return Err(Error::Git {
            repo: repo.to_path_buf(),
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Like [`run`], but returns trimmed non-empty output or `None`.
pub fn run_opt(repo: &Path, args: &[&str]) -> Option<String> {
    run(repo, args)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// True when the folder itself is a repo root (`.git` can be a folder or a file).
pub fn has_git_dir(path: &Path) -> bool {
    path.join(".git").exists()
}

/// The repo root containing `path`, if any.
pub fn toplevel(path: &Path) -> Option<PathBuf> {
    run_opt(path, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

pub fn inspect(repo: &Path) -> Result<RepoInfo> {
    let remotes = parse::parse_remotes(&run(repo, &["remote", "-v"])?)
        .into_iter()
        .map(|(name, fetch_url, push_url)| Remote {
            name,
            fetch_url,
            push_url,
        })
        .collect();

    let branch = run_opt(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let head = run_opt(repo, &["rev-parse", "--verify", "--quiet", "HEAD"]).map(|commit| Head {
        detached: branch.is_none(),
        branch,
        commit,
    });

    Ok(RepoInfo {
        head,
        upstream: run_opt(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]),
        remotes,
        last_commit_unix: run_opt(repo, &["log", "-1", "--format=%ct"]).and_then(|s| s.parse().ok()),
    })
}

/// Ignored files and top-most ignored folders, repo-relative, folders ending in `/`.
pub fn ignored_entries(repo: &Path) -> Result<Vec<String>> {
    let out = run(
        repo,
        &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"],
    )?;
    Ok(parse::split_nul(&out))
}
