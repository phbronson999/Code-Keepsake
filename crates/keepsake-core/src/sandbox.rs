//! A throwaway set of real git repos, remotes and workspace files for trying
//! Code Keepsake (and its removal) without touching anything that matters.
//!
//! Everything lives in one folder that carries a marker file. `destroy` and
//! `create` refuse to delete a folder without that marker.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Present at the root of every sandbox; the only folder we'll ever delete.
pub const MARKER: &str = ".keepsake-demo";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemoRepo {
    pub name: String,
    pub path: PathBuf,
    /// What state this repo is in, and what to expect from it.
    pub blurb: String,
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    pub root: PathBuf,
    /// Point Code Keepsake at this folder to find the demo workspaces.
    pub workspaces_dir: PathBuf,
    pub repos: Vec<DemoRepo>,
}

pub fn default_dir() -> PathBuf {
    std::env::temp_dir().join("code-keepsake-demo")
}

/// True when `dir` is a sandbox created by [`create`].
pub fn is_sandbox(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

/// True when `path` is inside a sandbox, however deep.
pub fn contains(sandbox_root: &Path, path: &Path) -> bool {
    is_sandbox(sandbox_root) && path.starts_with(sandbox_root)
}

/// Deletes a sandbox. Refuses anything without the marker file.
pub fn destroy(dir: &Path) -> Result<()> {
    if !is_sandbox(dir) {
        return Err(not_sandbox(dir));
    }
    std::fs::remove_dir_all(dir).map_err(|e| Error::io(dir, e))
}

fn not_sandbox(dir: &Path) -> Error {
    Error::io(
        dir,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "not a Code Keepsake demo folder, so it was left alone",
        ),
    )
}

/// Builds a fresh sandbox at `dir`, replacing a previous one. A folder that
/// exists but isn't a sandbox is never touched.
pub fn create(dir: &Path) -> Result<Sandbox> {
    if dir.exists() {
        if is_sandbox(dir) {
            destroy(dir)?;
        } else if std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?.next().is_some() {
            return Err(not_sandbox(dir));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let root = dir.canonicalize().map_err(|e| Error::io(dir, e))?;
    // `canonicalize` gives \\?\ paths on Windows; git and workspaces prefer plain ones.
    let root = strip_verbatim(root);
    std::fs::write(root.join(MARKER), "Safe to delete. Made by `code-keepsake demo`.\n")
        .map_err(|e| Error::io(&root, e))?;

    let b = Builder { root: root.clone() };
    let mut repos = Vec::new();
    let mut add = |name: &str, path: PathBuf, blurb: &str| {
        repos.push(DemoRepo {
            name: name.to_string(),
            path,
            blurb: blurb.to_string(),
        })
    };

    let p = b.repo("clean-app", "", 3, Some(3_000))?;
    add("clean-app", p, "Pushed and clean, with big build folders. Safe at every level.");

    let p = b.repo("sleepy-api", "", 120, Some(1_500))?;
    add("sleepy-api", p, "Pushed and untouched for months. A good one to put away.");

    let p = b.repo("dirty-notes", "", 1, Some(200))?;
    b.write(&p, "README.md", "# dirty-notes\n\nEdited but not committed.\n")?;
    b.write(&p, "scratch.txt", "an untracked file\n")?;
    add("dirty-notes", p, "Uncommitted and untracked changes. Hollow and Full are blocked.");

    let p = b.repo("unpushed-work", "", 2, Some(200))?;
    for i in 1..=2 {
        b.write(&p, &format!("feature{i}.txt"), "work in progress\n")?;
        b.git(&p, &["add", "-A"], 0)?;
        b.git(&p, &["commit", "-q", "-m", &format!("Local feature {i}")], 0)?;
    }
    add("unpushed-work", p, "Two commits that exist only here. Hollow and Full bundle them into the stub.");

    let p = b.repo("stash-keeper", "", 5, Some(200))?;
    b.write(&p, "README.md", "# stash-keeper\n\nStashed for later.\n")?;
    b.git(&p, &["stash", "push", "-q", "-m", "half-finished idea"], 0)?;
    add("stash-keeper", p, "Has a stash. Hollow and Full save it into the stub.");

    let p = b.repo_without_remote("no-remote", "", 30, Some(200))?;
    add("no-remote", p, "No remote at all. Hollow and Full bundle the whole history into the stub.");

    let p = b.repo("behind-remote", "", 14, Some(200))?;
    b.push_from_another_clone("behind-remote")?;
    add("behind-remote", p, "The remote has newer commits. Press f to fetch and see it.");

    let p = b.repo("worktree-repo", "", 7, Some(200))?;
    let wt = root.join("worktrees").join("worktree-repo-feature");
    std::fs::create_dir_all(wt.parent().unwrap()).map_err(|e| Error::io(&root, e))?;
    b.git(&p, &["worktree", "add", "-q", "-b", "feature", &wt.to_string_lossy()], 0)?;
    add("worktree-repo", p, "Has a second worktree checked out elsewhere. Hollow and Full are blocked.");

    for (name, days) in [("api", 10), ("web", 20), ("worker", 45)] {
        let p = b.repo(name, "services", days, Some(800))?;
        add(
            match name {
                "api" => "services/api",
                "web" => "services/web",
                _ => "services/worker",
            },
            p,
            "One of three repos inside the 'services' folder. Raise depth to find it.",
        );
    }

    let ws_dir = root.join("workspaces");
    std::fs::create_dir_all(&ws_dir).map_err(|e| Error::io(&ws_dir, e))?;
    let workspaces = [
        (
            "Demo Daily",
            vec!["../repos/clean-app", "../repos/dirty-notes", "../repos/unpushed-work", "../repos/stash-keeper"],
        ),
        // One folder holding three repos: shows nothing until its depth is raised.
        ("Demo Services", vec!["../repos/services"]),
        (
            "Demo Archive",
            vec!["../repos/sleepy-api", "../repos/no-remote", "../repos/behind-remote", "../repos/worktree-repo"],
        ),
    ];
    for (name, folders) in workspaces {
        let entries: Vec<String> = folders.iter().map(|f| format!("    {{ \"path\": \"{f}\" }}")).collect();
        let text = format!("{{\n  \"folders\": [\n{}\n  ]\n}}\n", entries.join(",\n"));
        let path = ws_dir.join(format!("{name}.code-workspace"));
        std::fs::write(&path, text).map_err(|e| Error::io(&path, e))?;
    }

    // The marker doubles as the manifest, so `open` can bring the list back.
    let manifest = serde_json::to_string_pretty(&repos).unwrap_or_default();
    std::fs::write(root.join(MARKER), manifest).map_err(|e| Error::io(&root, e))?;

    Ok(Sandbox {
        root,
        workspaces_dir: ws_dir,
        repos,
    })
}

/// Reuses the sandbox at `dir` so a half-way session can be picked up again,
/// building one if there is none or it was never finished.
pub fn open_or_create(dir: &Path) -> Result<Sandbox> {
    match open(dir) {
        Ok(sandbox) if !sandbox.repos.is_empty() => Ok(sandbox),
        _ => create(dir),
    }
}

/// Reopens a sandbox made earlier by [`create`].
pub fn open(dir: &Path) -> Result<Sandbox> {
    if !is_sandbox(dir) {
        return Err(not_sandbox(dir));
    }
    let root = strip_verbatim(dir.canonicalize().map_err(|e| Error::io(dir, e))?);
    let text = std::fs::read_to_string(root.join(MARKER)).map_err(|e| Error::io(&root, e))?;
    let repos: Vec<DemoRepo> = serde_json::from_str(&text).unwrap_or_default();
    Ok(Sandbox {
        workspaces_dir: root.join("workspaces"),
        root,
        repos,
    })
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    match p.to_string_lossy().strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

struct Builder {
    root: PathBuf,
}

impl Builder {
    fn remote_path(&self, name: &str) -> PathBuf {
        self.root.join("remotes").join(format!("{name}.git"))
    }

    /// Runs git with a fixed identity. `days_ago` back-dates any commit it makes.
    fn git(&self, dir: &Path, args: &[&str], days_ago: u64) -> Result<String> {
        let when = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
            .saturating_sub(days_ago * 86_400);
        let date = format!("@{when} +0000");
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Demo Dev", "-c", "user.email=demo@example.invalid"])
            .args(["-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"])
            .args(args)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Error::GitMissing
            } else {
                Error::io(dir, e)
            }
        })?;
        if !out.status.success() {
            return Err(Error::Git {
                repo: dir.to_path_buf(),
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn write(&self, repo: &Path, rel: &str, text: &str) -> Result<()> {
        let path = repo.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::write(&path, text).map_err(|e| Error::io(&path, e))
    }

    /// A repo with one commit, ignored build folders of about `build_kb` in total,
    /// and a bare remote that has the commit.
    fn repo(&self, name: &str, group: &str, days_ago: u64, build_kb: Option<usize>) -> Result<PathBuf> {
        let path = self.repo_without_remote(name, group, days_ago, build_kb)?;
        let remote = self.remote_path(name);
        std::fs::create_dir_all(&remote).map_err(|e| Error::io(&remote, e))?;
        self.git(&remote, &["init", "-q", "--bare", "-b", "main"], 0)?;
        self.git(&path, &["remote", "add", "origin", &remote.to_string_lossy()], 0)?;
        self.git(&path, &["push", "-q", "-u", "origin", "main"], 0)?;
        Ok(path)
    }

    fn repo_without_remote(
        &self,
        name: &str,
        group: &str,
        days_ago: u64,
        build_kb: Option<usize>,
    ) -> Result<PathBuf> {
        let mut path = self.root.join("repos");
        if !group.is_empty() {
            path.push(group);
        }
        path.push(name);
        std::fs::create_dir_all(&path).map_err(|e| Error::io(&path, e))?;
        self.git(&path, &["init", "-q", "-b", "main"], 0)?;
        self.write(&path, ".gitignore", "node_modules/\ntarget/\ndist/\n")?;
        self.write(&path, "README.md", &format!("# {name}\n\nA demo repo.\n"))?;
        self.write(&path, "src/main.txt", "hello\n")?;
        self.git(&path, &["add", "-A"], 0)?;
        self.git(&path, &["commit", "-q", "-m", "Initial commit"], days_ago)?;

        if let Some(kb) = build_kb {
            // Ignored build output, split roughly 60/30/10.
            for (rel, share) in [
                ("node_modules/left-pad/index.js", 60),
                ("target/debug/app.bin", 30),
                ("dist/bundle.js", 10),
            ] {
                self.write_bytes(&path, rel, kb * 1024 * share / 100)?;
            }
        }
        Ok(path)
    }

    fn write_bytes(&self, repo: &Path, rel: &str, len: usize) -> Result<()> {
        let path = repo.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::write(&path, vec![b'k'; len]).map_err(|e| Error::io(&path, e))
    }

    /// Pushes a new commit to `name`'s remote from a scratch clone, leaving the
    /// repo itself one commit behind until it fetches.
    fn push_from_another_clone(&self, name: &str) -> Result<()> {
        let scratch = self.root.join(".scratch");
        std::fs::create_dir_all(&scratch).map_err(|e| Error::io(&scratch, e))?;
        let remote = self.remote_path(name);
        self.git(&scratch, &["clone", "-q", &remote.to_string_lossy(), "other"], 0)?;
        let other = scratch.join("other");
        self.write(&other, "from-teammate.txt", "pushed elsewhere\n")?;
        self.git(&other, &["add", "-A"], 0)?;
        self.git(&other, &["commit", "-q", "-m", "A teammate's commit"], 0)?;
        self.git(&other, &["push", "-q"], 0)?;
        std::fs::remove_dir_all(&scratch).map_err(|e| Error::io(&scratch, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::analyze,
        scan::{collect_targets, target_for_path},
        workspace::{self, WorkspaceFile},
    };

    fn temp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("keepsake-sbx-{tag}-{}", std::process::id()))
    }

    #[test]
    fn creates_repos_workspaces_and_refuses_strangers() {
        let dir = temp("create");
        let _ = std::fs::remove_dir_all(&dir);
        let sb = create(&dir).expect("sandbox builds");

        assert!(is_sandbox(&sb.root));
        assert_eq!(sb.repos.len(), 11);
        for r in &sb.repos {
            assert!(r.path.join(".git").exists(), "{} is a repo", r.name);
        }

        // The workspace files resolve, and depth finds the three services.
        let files = workspace::discover(&[sb.workspaces_dir.clone()], 0);
        assert_eq!(files.len(), 3);
        let loaded: Vec<WorkspaceFile> = files.iter().map(|f| WorkspaceFile::load(f).unwrap()).collect();
        assert_eq!(collect_targets(&loaded, |_| 0).len(), 9);
        assert_eq!(collect_targets(&loaded, |_| 1).len(), 11);

        // A folder that isn't ours is never replaced or deleted.
        let stranger = temp("stranger");
        let _ = std::fs::remove_dir_all(&stranger);
        std::fs::create_dir_all(&stranger).unwrap();
        std::fs::write(stranger.join("precious.txt"), "keep").unwrap();
        assert!(create(&stranger).is_err());
        assert!(destroy(&stranger).is_err());
        assert!(stranger.join("precious.txt").exists());
        let _ = std::fs::remove_dir_all(&stranger);

        // Rebuilding replaces the old one, then destroy removes everything.
        create(&dir).expect("rebuild");
        let reopened = open(&dir).expect("reopen");
        assert_eq!(reopened.repos.len(), 11);
        assert_eq!(reopened.workspaces_dir, sb.workspaces_dir);
        destroy(&dir).expect("destroy");
        assert!(!dir.exists());
    }

    #[test]
    fn repos_are_in_the_states_they_claim() {
        use crate::safety::Severity;
        let dir = temp("states");
        let _ = std::fs::remove_dir_all(&dir);
        let sb = create(&dir).unwrap();
        let count = |name: &str, severity: Severity| -> usize {
            let repo = sb.repos.iter().find(|r| r.name == name).unwrap();
            let a = analyze(&target_for_path(&repo.path), false);
            a.safety
                .map(|s| s.findings().iter().filter(|f| f.severity == severity).count())
                .unwrap_or(0)
        };
        for ready in ["clean-app", "sleepy-api", "behind-remote"] {
            assert_eq!(count(ready, Severity::Blocker) + count(ready, Severity::Preserve), 0, "{ready}");
        }
        for blocked in ["dirty-notes", "worktree-repo"] {
            assert!(count(blocked, Severity::Blocker) > 0, "{blocked}");
        }
        for preserved in ["unpushed-work", "stash-keeper", "no-remote"] {
            assert_eq!(count(preserved, Severity::Blocker), 0, "{preserved}");
            assert!(count(preserved, Severity::Preserve) > 0, "{preserved}");
        }
        destroy(&dir).unwrap();
    }
}
