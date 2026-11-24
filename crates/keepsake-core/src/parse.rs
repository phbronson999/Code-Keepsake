//! Parsers for git's text output and the rules for classifying ignored files.
//! std-only so they can be tested without a git binary.

/// Ignored directories that are rebuilt by a tool and safe to delete.
pub const ARTIFACT_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "bin",
    "obj",
    ".next",
    ".nuxt",
    ".turbo",
    ".vite",
    ".angular",
    ".svelte-kit",
    ".parcel-cache",
    ".gradle",
    ".cache",
    "coverage",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".venv",
    "venv",
];

/// Local git config keys that every clone has, so there is nothing to carry over.
const BORING_CONFIG: &[&str] = &[
    "core.repositoryformatversion",
    "core.filemode",
    "core.bare",
    "core.logallrefupdates",
    "core.symlinks",
    "core.ignorecase",
    "core.precomposeunicode",
];

/// `git remote -v` → `(name, fetch_url, push_url if it differs)`.
/// Splits on the tab and the last space, so local paths with spaces survive.
pub fn parse_remotes(output: &str) -> Vec<(String, String, Option<String>)> {
    let mut remotes: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for line in output.lines() {
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let Some((url, kind)) = rest.rsplit_once(' ') else {
            continue;
        };
        let index = match remotes.iter().position(|r| r.0 == name) {
            Some(i) => i,
            None => {
                remotes.push((name.to_string(), None, None));
                remotes.len() - 1
            }
        };
        match kind {
            "(fetch)" => remotes[index].1 = Some(url.to_string()),
            "(push)" => remotes[index].2 = Some(url.to_string()),
            _ => {}
        }
    }
    remotes
        .into_iter()
        .filter_map(|(name, fetch, push)| {
            let fetch = fetch.or_else(|| push.clone())?;
            let push = push.filter(|p| *p != fetch);
            Some((name, fetch, push))
        })
        .collect()
}

/// `git for-each-ref --format=%(refname:short)%09%(upstream:short) refs/heads`
/// → `(branch, upstream)`.
pub fn parse_branch_upstreams(output: &str) -> Vec<(String, Option<String>)> {
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let (branch, upstream) = line.split_once('\t').unwrap_or((line, ""));
            let upstream = Some(upstream.trim().to_string()).filter(|u| !u.is_empty());
            (branch.trim().to_string(), upstream)
        })
        .collect()
}

/// Number of worktrees in `git worktree list --porcelain` (the main one included).
pub fn count_worktrees(output: &str) -> usize {
    output.lines().filter(|l| l.starts_with("worktree ")).count()
}

/// Keys from `git config --local --list` that a fresh clone would not have.
pub fn interesting_config_keys(output: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for line in output.lines() {
        let key = line.split_once('=').map_or(line, |(k, _)| k).trim().to_lowercase();
        if key.is_empty()
            || BORING_CONFIG.contains(&key.as_str())
            || key.starts_with("remote.")
            || key.starts_with("branch.")
        {
            continue;
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// Splits NUL-separated output (`-z`) into entries.
pub fn split_nul(output: &str) -> Vec<String> {
    output
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn last_component(entry: &str) -> &str {
    entry.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// An ignored directory entry (ends in `/`) whose name is a known build folder.
pub fn is_artifact_entry(entry: &str) -> bool {
    entry.ends_with('/') && ARTIFACT_DIRS.contains(&last_component(entry))
}

/// An ignored entry that probably matters and can't be rebuilt: secrets, local
/// settings, keys, local databases, editor configs.
pub fn is_important_ignored(entry: &str) -> bool {
    let name = last_component(entry).to_ascii_lowercase();
    if entry.ends_with('/') {
        return matches!(name.as_str(), ".vscode" | ".idea" | "secrets" | ".secrets" | "certs");
    }
    const KEY_EXTENSIONS: &[&str] = &[
        "pfx", "p12", "pem", "key", "snk", "jks", "keystore", "db", "sqlite", "sqlite3",
    ];
    let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    name.starts_with(".env")
        || KEY_EXTENSIONS.contains(&extension)
        || name.ends_with(".local")
        || name.contains(".local.")
        || name == "local.settings.json"
        || name == "secrets.json"
        || name == "credentials.json"
        || name == ".npmrc"
        || (name.starts_with("appsettings.") && name.ends_with(".json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remotes() {
        let out = "origin\tgit@github.com:me/app.git (fetch)\n\
                   origin\tgit@github.com:me/app.git (push)\n\
                   backup\tD:\\My Backups\\app.git (fetch)\n\
                   backup\thttps://example.com/push.git (push)\n";
        let r = parse_remotes(out);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("origin".into(), "git@github.com:me/app.git".into(), None));
        assert_eq!(r[1].1, "D:\\My Backups\\app.git");
        assert_eq!(r[1].2.as_deref(), Some("https://example.com/push.git"));
    }

    #[test]
    fn parses_upstreams_and_worktrees() {
        let b = parse_branch_upstreams("main\torigin/main\nspike\t\n");
        assert_eq!(b[0].1.as_deref(), Some("origin/main"));
        assert_eq!(b[1], ("spike".into(), None));
        let wt = "worktree /a\nHEAD 1\nbranch refs/heads/main\n\nworktree /b\nHEAD 2\n";
        assert_eq!(count_worktrees(wt), 2);
    }

    #[test]
    fn filters_config() {
        let out = "core.repositoryformatversion=0\ncore.bare=false\nremote.origin.url=x\n\
                   branch.main.remote=origin\nuser.email=me@work.com\ncore.hooksPath=.githooks\n";
        assert_eq!(interesting_config_keys(out), vec!["user.email", "core.hookspath"]);
    }

    #[test]
    fn classifies_ignored_entries() {
        assert!(is_artifact_entry("node_modules/"));
        assert!(is_artifact_entry("src-tauri/target/"));
        assert!(!is_artifact_entry("target"));
        assert!(!is_artifact_entry("docs/"));

        assert!(is_important_ignored(".env"));
        assert!(is_important_ignored("api/.env.production"));
        assert!(is_important_ignored("certs/signing.pfx"));
        assert!(is_important_ignored("appsettings.Development.json"));
        assert!(is_important_ignored("src-tauri/tauri.local.conf.json"));
        assert!(is_important_ignored(".vscode/"));
        assert!(!is_important_ignored("node_modules/"));
        assert!(!is_important_ignored("debug.log"));
    }
}
