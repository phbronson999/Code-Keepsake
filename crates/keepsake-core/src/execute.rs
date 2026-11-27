//! Puts a repo away, and brings it back.
//!
//! The rule that keeps this safe: only delete what can be recreated. That means
//! tracked files (git has them), ignored build folders (a tool rebuilds them),
//! and for Full, `.git` itself once everything local-only is in a bundle.
//! Anything else, such as `.env` files or local databases, is left where it is.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    fmt, git,
    parse,
    plan::Level,
    safety::{self, Severity},
    size,
    stub::{self, KeepsakeStub, Preserved, SavedRef, StashRecord},
    workspace, Error, Result,
};

#[derive(Debug, Clone)]
pub struct ExecReport {
    pub level: Level,
    pub dry_run: bool,
    /// What was done, or would be done.
    pub actions: Vec<String>,
    pub bytes_freed: u64,
    /// Why nothing was done. Empty means it went ahead (or would).
    pub blocked: Vec<String>,
    /// Things that failed part way, e.g. a file held open by another program.
    pub errors: Vec<String>,
}

impl ExecReport {
    pub fn succeeded(&self) -> bool {
        self.blocked.is_empty() && self.errors.is_empty()
    }
}

fn refused(path: &Path, message: impl Into<String>) -> Error {
    Error::Refused {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// Refuses folders that must never be emptied, whatever they contain.
fn guard_location(repo: &Path) -> Result<()> {
    if repo.parent().is_none() || repo.parent().is_some_and(|p| p.as_os_str().is_empty()) {
        return Err(refused(repo, "this is a drive or filesystem root"));
    }
    let key = workspace::path_key(repo);
    for var in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(var).map(PathBuf::from) {
            let home = workspace::path_key(&home);
            if home == key || home.starts_with(&format!("{key}/")) {
                return Err(refused(repo, "this is, or contains, your home folder"));
            }
        }
    }
    if !repo.join(".git").exists() {
        return Err(refused(repo, "not the root of a git repo"));
    }
    Ok(())
}

/// True when any folder between `root` and `path` is a symlink or junction, so
/// deleting `path` would reach outside the repo.
fn through_link(root: &Path, path: &Path) -> bool {
    path.ancestors()
        .skip(1)
        .take_while(|a| *a != root)
        .any(|a| fs::symlink_metadata(a).is_ok_and(|m| m.file_type().is_symlink()))
}

/// The first nested repo below `dir`, not following links.
fn nested_repo(dir: &Path, skip: &[PathBuf]) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name() == ".git" {
                if current != dir {
                    return Some(current);
                }
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) && !skip.contains(&path) {
                stack.push(path);
            }
        }
    }
    None
}

fn size_of(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    fs::read_dir(path)
        .map(|entries| entries.flatten().map(|e| size_of(&e.path())).sum())
        .unwrap_or(0)
}

/// Ignored build folders that are safe to delete, as absolute paths.
fn build_dirs(repo: &Path, ignored: &[String]) -> Vec<PathBuf> {
    ignored
        .iter()
        .filter(|e| parse::is_artifact_entry(e))
        .map(|e| workspace::normalize(&repo.join(e.trim_end_matches('/'))))
        .filter(|dir| dir.starts_with(repo) && dir != repo && !through_link(repo, dir))
        // A repo hiding in `build/` or `out/` is somebody's work, not output.
        // Package managers put repos in these two, and they are re-downloaded.
        .filter(|dir| {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned());
            matches!(name.as_deref(), Some("node_modules" | "target")) || nested_repo(dir, &[]).is_none()
        })
        .collect()
}

fn nul_list(repo: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(parse::split_nul(&git::run(repo, args)?))
}

/// Reasons Hollow and Full must not run. Unlike the scan, any git failure here
/// is an error rather than "nothing found".
fn blockers(repo: &Path, level: Level, build: &[PathBuf], ignored: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !repo.join(".git").is_dir() {
        out.push("This is a worktree or submodule checkout (.git is a file)".to_string());
        return Ok(out);
    }
    let info = git::inspect(repo)?;
    if info.head.is_none() {
        out.push("There are no commits yet".to_string());
    }

    let status = git::run(
        repo,
        &["status", "--porcelain", "--untracked-files=normal", "--ignore-submodules=none"],
    )?;
    let dirty = status.lines().filter(|l| !l.is_empty()).count();
    if dirty > 0 {
        out.push(format!("{dirty} uncommitted or untracked change(s)"));
    }

    // Hidden edits: `status` doesn't report these, but deleting would lose them.
    let flagged = git::run(repo, &["ls-files", "-v"])?
        .lines()
        .any(|l| l.starts_with(|c: char| c.is_ascii_lowercase() || c == 'S'));
    if flagged {
        out.push("Some files are marked assume-unchanged or skip-worktree".to_string());
    }

    let report = safety::check(repo, false, ignored, Some(&info));
    for finding in report.findings() {
        // `dirty` is already covered, strictly, above.
        if finding.severity == Severity::Blocker && report.dirty.is_empty() {
            out.push(finding.title);
        }
    }

    if let Some(nested) = nested_repo(repo, build) {
        let rel = nested.strip_prefix(repo).unwrap_or(&nested);
        out.push(format!("Another repo is nested inside: {}", rel.display()));
    }

    let tracked = nul_list(repo, &["ls-files", "-z"])?;
    if tracked
        .iter()
        .any(|f| f == stub::STUB_FILE || f.starts_with(&format!("{}/", stub::STUB_DIR)))
    {
        out.push("The repo tracks files with Code Keepsake's own names".to_string());
    }

    if level == Level::Full {
        if repo.join(".git/lfs/objects").exists() {
            out.push("Has Git LFS objects, which may not all be on the remote".to_string());
        }
        if !info.remotes.is_empty() {
            if let Err(e) = git::run(repo, &["fetch", "--all", "--prune", "--quiet"]) {
                out.push(format!("Couldn't fetch, so the remote can't be confirmed: {e}"));
            } else {
                out.extend(local_only_tags(repo, &info.remotes)?);
            }
        }
    }
    Ok(out)
}

/// Annotated tags no remote has. A bundle that excludes remote commits could
/// miss the tag object, so these must be pushed first.
fn local_only_tags(repo: &Path, remotes: &[git::Remote]) -> Result<Vec<String>> {
    let tags = git::run(repo, &["for-each-ref", "--format=%(objecttype) %(objectname) %(refname:short)", "refs/tags"])?;
    let annotated: Vec<(&str, &str)> = tags
        .lines()
        .filter_map(|l| l.strip_prefix("tag ")?.split_once(' '))
        .collect();
    if annotated.is_empty() {
        return Ok(Vec::new());
    }
    let mut remote_objects = String::new();
    for remote in remotes {
        remote_objects.push_str(&git::run(repo, &["ls-remote", "--tags", &remote.name])?);
    }
    Ok(annotated
        .into_iter()
        .filter(|(sha, _)| !remote_objects.contains(sha))
        .map(|(_, name)| format!("Tag {name} is on no remote. Push it first"))
        .collect())
}

/// Carries out `level` on one repo. With `dry_run`, reports without changing
/// any file (Full still fetches, so the answer is current).
pub fn execute(repo: &Path, level: Level, dry_run: bool, workspaces: &[String]) -> Result<ExecReport> {
    let repo = workspace::absolute(repo);
    guard_location(&repo)?;
    if repo.join(stub::STUB_FILE).exists() {
        return Err(refused(&repo, "already put away. Restore it first"));
    }

    let mut report = ExecReport {
        level,
        dry_run,
        actions: Vec::new(),
        bytes_freed: 0,
        blocked: Vec::new(),
        errors: Vec::new(),
    };
    let ignored = git::ignored_entries(&repo)?;
    let build = build_dirs(&repo, &ignored);

    let mut files = Vec::new();
    if level != Level::Clean {
        report.blocked = blockers(&repo, level, &build, &ignored)?;
        if !report.blocked.is_empty() {
            return Ok(report);
        }
        files = nul_list(&repo, &["ls-files", "-z"])?
            .iter()
            .map(|f| repo.join(f))
            .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir()) && !through_link(&repo, p))
            .collect();
    }

    for dir in &build {
        let bytes = size_of(dir);
        report.bytes_freed += bytes;
        let rel = dir.strip_prefix(&repo).unwrap_or(dir);
        report
            .actions
            .push(format!("Delete build folder {} ({})", rel.display(), fmt::bytes(bytes)));
    }
    if level != Level::Clean {
        let bytes: u64 = files.iter().map(|f| size_of(f)).sum();
        report.bytes_freed += bytes;
        report
            .actions
            .push(format!("Delete {} tracked file(s) ({})", files.len(), fmt::bytes(bytes)));
    }
    if level == Level::Full {
        let bytes = size_of(&repo.join(".git"));
        report.bytes_freed += bytes;
        report.actions.push(format!("Delete .git ({})", fmt::bytes(bytes)));
    }

    if dry_run {
        return Ok(report);
    }

    // The record is written, and checked, before anything is removed.
    if level != Level::Clean {
        if let Err(e) = write_record(&repo, level, &ignored, workspaces) {
            // Nothing was deleted; drop the half-made record too.
            let _ = fs::remove_file(repo.join(stub::STUB_FILE));
            let _ = fs::remove_dir_all(repo.join(stub::STUB_DIR));
            return Err(e);
        }
    }

    let mut parents: BTreeSet<PathBuf> = BTreeSet::new();
    for file in &files {
        match remove_entry(file) {
            Ok(()) => {
                parents.extend(
                    file.ancestors()
                        .skip(1)
                        .take_while(|a| *a != repo)
                        .map(Path::to_path_buf),
                );
            }
            Err(e) => report.errors.push(format!("{}: {e}", file.display())),
        }
    }
    for dir in &build {
        // `remove_dir_all` removes links themselves and never follows them.
        if let Err(e) = fs::remove_dir_all(dir) {
            report.errors.push(format!("{}: {e}", dir.display()));
        }
    }
    // Deepest first. `remove_dir` only succeeds on empty folders.
    for dir in parents.iter().rev() {
        let _ = fs::remove_dir(dir);
    }

    if level == Level::Full {
        if report.errors.is_empty() {
            if let Err(e) = fs::remove_dir_all(repo.join(".git")) {
                report.errors.push(format!(".git: {e}"));
            }
        } else {
            report
                .errors
                .push("Kept .git because some files couldn't be deleted".to_string());
        }
    }
    Ok(report)
}

fn remove_entry(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut permissions = fs::symlink_metadata(path)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
            fs::remove_file(path)
        }
        other => other,
    }
}

/// Writes `.keepsake.json`, and for Full everything restore needs that no
/// remote has.
fn write_record(repo: &Path, level: Level, ignored: &[String], workspaces: &[String]) -> Result<()> {
    let info = git::inspect(repo)?;
    let sizes_before = size::measure(repo, ignored);
    let mut preserved = Preserved::default();

    // Hollow keeps `.git`, so there is nothing else to carry.
    if level == Level::Full {
        let store = repo.join(stub::STUB_DIR);
        fs::create_dir_all(&store).map_err(|e| Error::io(&store, e))?;

        // Only the newest stash has a ref; give each one its own so it's bundled.
        let stashes = git::run(repo, &["stash", "list", "--format=%H%x09%gs"])?;
        for (i, line) in stashes.lines().enumerate() {
            let (sha, message) = line.split_once('\t').unwrap_or((line, ""));
            let reference = format!("refs/keepsake/stash/{i}");
            git::run(repo, &["update-ref", &reference, sha])?;
            preserved.stashes.push(StashRecord {
                reference,
                message: message.to_string(),
            });
        }

        // Remote-tracking refs come back with a fetch, and stashes are handled above.
        let refs = git::run(repo, &["for-each-ref", "--format=%(objectname) %(refname)"])?;
        preserved.refs = refs
            .lines()
            .filter_map(|l| l.split_once(' '))
            .filter(|(_, name)| !name.starts_with("refs/remotes/") && *name != "refs/stash")
            .map(|(sha, name)| SavedRef {
                name: name.to_string(),
                sha: sha.to_string(),
            })
            .collect();

        let count_args: &[&str] = if info.remotes.is_empty() {
            &["rev-list", "--count", "--all"]
        } else {
            &["rev-list", "--count", "--all", "--not", "--remotes"]
        };
        if git::run(repo, count_args)?.trim() != "0" {
            let name = format!("{}/local.bundle", stub::STUB_DIR);
            let path = repo.join(&name).to_string_lossy().into_owned();
            let mut args = vec!["bundle", "create", "--quiet", &path, "--all"];
            if !info.remotes.is_empty() {
                args.extend(["--not", "--remotes"]);
            }
            git::run(repo, &args)?;
            git::run(repo, &["bundle", "verify", "--quiet", &path])?;
            preserved.bundle = Some(name);
        }

        copy(&repo.join(".git/config"), &store.join("config"))?;
        let exclude = repo.join(".git/info/exclude");
        if exclude.is_file() {
            copy(&exclude, &store.join("exclude"))?;
        }
        if let Ok(entries) = fs::read_dir(repo.join(".git/hooks")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_file() && !name.ends_with(".sample") {
                    let hooks = store.join("hooks");
                    fs::create_dir_all(&hooks).map_err(|e| Error::io(&hooks, e))?;
                    copy(&entry.path(), &hooks.join(&name))?;
                    preserved.hooks.push(name);
                }
            }
        }
    }

    stub::write(
        repo,
        &KeepsakeStub {
            schema_version: stub::SCHEMA_VERSION,
            tool: format!("code-keepsake/{}", env!("CARGO_PKG_VERSION")),
            stored_at_unix: fmt::now_unix(),
            level,
            head: info.head,
            upstream: info.upstream,
            remotes: info.remotes,
            preserved,
            sizes_before,
            workspaces: workspaces.to_vec(),
        },
    )
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to).map(drop).map_err(|e| Error::io(from, e))
}

/// Brings a Hollow or Full repo back. Existing files are never overwritten.
/// Build folders aren't recreated; the project's own tools do that.
pub fn restore(repo: &Path) -> Result<()> {
    let repo = workspace::absolute(repo);
    let record = stub::read(&repo).ok_or_else(|| refused(&repo, "there is no readable .keepsake.json here"))?;

    if !repo.join(".git").exists() {
        if let Err(e) = rebuild_git(&repo, &record) {
            // Only ever removes the `.git` this attempt just made.
            let _ = fs::remove_dir_all(repo.join(".git"));
            return Err(e);
        }
    }

    git::run(&repo, &["reset", "--quiet"])?;
    git::run(&repo, &["checkout-index", "--all", "--quiet", "--index"])?;

    let stub_file = repo.join(stub::STUB_FILE);
    fs::remove_file(&stub_file).map_err(|e| Error::io(&stub_file, e))?;
    let store = repo.join(stub::STUB_DIR);
    if store.exists() {
        fs::remove_dir_all(&store).map_err(|e| Error::io(&store, e))?;
    }
    Ok(())
}

fn rebuild_git(repo: &Path, record: &KeepsakeStub) -> Result<()> {
    let store = repo.join(stub::STUB_DIR);
    git::run(repo, &["init", "--quiet"])?;
    copy(&store.join("config"), &repo.join(".git/config"))?;
    if store.join("exclude").is_file() {
        fs::create_dir_all(repo.join(".git/info")).map_err(|e| Error::io(repo, e))?;
        copy(&store.join("exclude"), &repo.join(".git/info/exclude"))?;
    }
    for hook in &record.preserved.hooks {
        copy(&store.join("hooks").join(hook), &repo.join(".git/hooks").join(hook))?;
    }

    if !record.remotes.is_empty() {
        git::run(repo, &["fetch", "--all", "--tags", "--quiet"])?;
    }
    if let Some(bundle) = &record.preserved.bundle {
        let path = repo.join(bundle).to_string_lossy().into_owned();
        git::run(repo, &["bundle", "unbundle", &path])?;
    }
    for saved in &record.preserved.refs {
        if !saved.name.starts_with("refs/keepsake/") {
            git::run(repo, &["update-ref", &saved.name, &saved.sha])?;
        }
    }
    // Oldest first, so the newest ends up as stash@{0} again.
    for stash in record.preserved.stashes.iter().rev() {
        if let Some(saved) = record.preserved.refs.iter().find(|r| r.name == stash.reference) {
            git::run(repo, &["stash", "store", "-m", &stash.message, &saved.sha])?;
        }
    }

    match &record.head {
        Some(head) => match &head.branch {
            Some(branch) => {
                git::run(repo, &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])?;
            }
            None => {
                git::run(repo, &["update-ref", "--no-deref", "HEAD", &head.commit])?;
            }
        },
        None => return Err(refused(repo, "the record has no commit to return to")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::{analyze, RepoStatus},
        sandbox::{self, Sandbox},
        scan::target_for_path,
    };

    struct Demo(Sandbox);

    impl Demo {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("keepsake-exec-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Demo(sandbox::create(&dir).expect("sandbox"))
        }

        fn repo(&self, name: &str) -> PathBuf {
            self.0.repos.iter().find(|r| r.name == name).unwrap().path.clone()
        }
    }

    impl Drop for Demo {
        fn drop(&mut self) {
            let _ = sandbox::destroy(&self.0.root);
        }
    }

    fn status(repo: &Path) -> RepoStatus {
        analyze(&target_for_path(repo), false).status
    }

    fn refs(repo: &Path) -> String {
        git::run(repo, &["for-each-ref", "--format=%(objectname) %(refname)", "refs/heads", "refs/stash"]).unwrap()
    }

    #[test]
    fn dry_run_changes_nothing_and_clean_keeps_the_repo() {
        let demo = Demo::new("clean");
        let repo = demo.repo("clean-app");
        fs::write(repo.join(".env"), "SECRET=1").unwrap();
        fs::write(repo.join(".git/info/exclude"), ".env\n").unwrap();

        for level in Level::ALL {
            let report = execute(&repo, level, true, &[]).unwrap();
            assert!(report.blocked.is_empty(), "{level:?}: {:?}", report.blocked);
            assert!(report.bytes_freed > 0);
        }
        assert!(repo.join("node_modules").exists());
        assert!(repo.join("README.md").exists());
        assert!(!repo.join(stub::STUB_FILE).exists());

        let report = execute(&repo, Level::Clean, false, &[]).unwrap();
        assert!(report.succeeded(), "{:?}", report.errors);
        for gone in ["node_modules", "target", "dist"] {
            assert!(!repo.join(gone).exists(), "{gone}");
        }
        assert!(repo.join("README.md").exists());
        assert!(repo.join(".env").exists());
        assert_eq!(status(&repo), RepoStatus::Active);
    }

    #[test]
    fn hollow_round_trips() {
        let demo = Demo::new("hollow");
        let repo = demo.repo("stash-keeper");
        let before = refs(&repo);

        let report = execute(&repo, Level::Hollow, false, &["Demo".into()]).unwrap();
        assert!(report.succeeded(), "{report:?}");
        assert!(repo.join(".git").is_dir());
        assert!(!repo.join("README.md").exists());
        assert!(!repo.join("src").exists());
        assert_eq!(status(&repo), RepoStatus::Kept);
        assert!(execute(&repo, Level::Full, false, &[]).is_err(), "can't put away twice");

        restore(&repo).unwrap();
        assert!(repo.join("src/main.txt").exists());
        assert!(!repo.join(stub::STUB_FILE).exists());
        assert_eq!(refs(&repo), before);
        assert_eq!(git::run(&repo, &["status", "--porcelain"]).unwrap(), "");
    }

    #[test]
    fn full_round_trips_local_only_work() {
        let demo = Demo::new("full");
        for name in ["clean-app", "unpushed-work", "stash-keeper", "no-remote"] {
            let repo = demo.repo(name);
            fs::write(repo.join(".env"), "SECRET=1").unwrap();
            fs::write(repo.join(".git/info/exclude"), ".env\n").unwrap();
            let before = refs(&repo);
            let stashes = git::run(&repo, &["stash", "list"]).unwrap();

            let report = execute(&repo, Level::Full, false, &[]).unwrap();
            assert!(report.succeeded(), "{name}: {report:?}");
            assert!(!repo.join(".git").exists(), "{name}");
            assert!(!repo.join("README.md").exists(), "{name}");
            assert!(repo.join(".env").exists(), "{name}: ignored files stay");
            assert_eq!(status(&repo), RepoStatus::Kept, "{name}");

            restore(&repo).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(refs(&repo), before, "{name}");
            assert_eq!(git::run(&repo, &["stash", "list"]).unwrap(), stashes, "{name}");
            assert!(repo.join("README.md").exists(), "{name}");
            assert!(!repo.join(stub::STUB_DIR).exists(), "{name}");
            assert!(git::run(&repo, &["status", "--porcelain"]).unwrap().trim().is_empty(), "{name}");
        }
    }

    #[test]
    fn blocked_repos_are_left_alone() {
        let demo = Demo::new("blocked");
        for name in ["dirty-notes", "worktree-repo"] {
            let repo = demo.repo(name);
            for level in [Level::Hollow, Level::Full] {
                let report = execute(&repo, level, false, &[]).unwrap();
                assert!(!report.blocked.is_empty(), "{name} {level:?}");
            }
            assert!(repo.join("README.md").exists());
            assert!(!repo.join(stub::STUB_FILE).exists());
        }

        // Hidden from `status`, but still local work.
        let repo = demo.repo("sleepy-api");
        git::run(&repo, &["update-index", "--assume-unchanged", "README.md"]).unwrap();
        assert!(!execute(&repo, Level::Hollow, false, &[]).unwrap().blocked.is_empty());

        // A repo tucked inside an ignored folder.
        let repo = demo.repo("behind-remote");
        fs::create_dir_all(repo.join("vendor/lib/.git")).unwrap();
        fs::write(repo.join(".git/info/exclude"), "vendor/\n").unwrap();
        assert!(!execute(&repo, Level::Full, false, &[]).unwrap().blocked.is_empty());
        assert!(repo.join("README.md").exists());
    }

    #[test]
    fn refuses_places_that_are_not_repos() {
        let demo = Demo::new("guard");
        assert!(execute(&demo.0.root, Level::Clean, false, &[]).is_err());
        assert!(execute(&demo.0.root.join("repos/services"), Level::Full, false, &[]).is_err());
        assert!(restore(&demo.repo("clean-app")).is_err());
    }
}
