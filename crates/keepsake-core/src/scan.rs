//! Turns workspace files into a de-duplicated list of repos and analyzes them
//! on a small thread pool, reporting each one as soon as it's done.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    thread,
};

use serde::Serialize;

use crate::{
    analysis::{analyze, RepoAnalysis},
    git,
    workspace::{self, WorkspaceFile},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoTarget {
    /// The repo root (a workspace may point at a subfolder of it).
    pub path: PathBuf,
    pub name: String,
    /// Names of the workspace files that include this repo.
    pub workspaces: Vec<String>,
    /// Workspace folders inside this repo that aren't the root itself.
    pub subfolders: Vec<PathBuf>,
}

pub enum ScanEvent {
    Started { total: usize },
    Repo(Box<RepoAnalysis>),
    Finished,
}

/// Workspace folders often point inside a repo (`app/src-tauri`); this finds the root.
fn resolve_root(path: &Path) -> PathBuf {
    if !path.exists() || git::has_git_dir(path) {
        return path.to_path_buf();
    }
    git::toplevel(path)
        .map(|p| workspace::normalize(&p))
        .unwrap_or_else(|| path.to_path_buf())
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// `depth_for` gives, per workspace name, how many levels below each folder to look
/// for repos. 0 treats a folder as the one repo it is (or sits inside).
pub fn collect_targets(
    workspaces: &[WorkspaceFile],
    depth_for: impl Fn(&str) -> usize,
) -> Vec<RepoTarget> {
    let mut targets: Vec<RepoTarget> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for ws in workspaces {
        let depth = depth_for(&ws.name);
        for folder in &ws.folders {
            // (repo root, whether the workspace folder is that root, name override)
            let mut roots: Vec<(PathBuf, bool, Option<String>)> = Vec::new();
            if depth > 0 && !git::has_git_dir(&folder.path) {
                for repo in workspace::find_repos(&folder.path, depth) {
                    roots.push((repo, true, None));
                }
            }
            if roots.is_empty() {
                let root = resolve_root(&folder.path);
                let is_root = workspace::path_key(&root) == workspace::path_key(&folder.path);
                roots.push((root, is_root, folder.name.clone()));
            }

            for (root, is_root, name_override) in roots {
                let i = *index.entry(workspace::path_key(&root)).or_insert_with(|| {
                    let name = name_override
                        .filter(|_| is_root)
                        .unwrap_or_else(|| folder_name(&root));
                    targets.push(RepoTarget {
                        path: root.clone(),
                        name,
                        workspaces: Vec::new(),
                        subfolders: Vec::new(),
                    });
                    targets.len() - 1
                });

                let target = &mut targets[i];
                if !target.workspaces.contains(&ws.name) {
                    target.workspaces.push(ws.name.clone());
                }
                if !is_root && !target.subfolders.contains(&folder.path) {
                    target.subfolders.push(folder.path.clone());
                }
            }
        }
    }

    targets.sort_by_key(|t| t.name.to_lowercase());
    targets
}

/// A target for a single path given on the command line.
pub fn target_for_path(path: &Path) -> RepoTarget {
    let root = resolve_root(&workspace::absolute(path));
    RepoTarget {
        name: folder_name(&root),
        path: root,
        workspaces: Vec::new(),
        subfolders: Vec::new(),
    }
}

/// Analyzes every target on a background thread. Results arrive in completion order.
pub fn spawn_scan(
    targets: Vec<RepoTarget>,
    fetch: bool,
    tx: mpsc::Sender<ScanEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let _ = tx.send(ScanEvent::Started {
            total: targets.len(),
        });
        let queue = Mutex::new(VecDeque::from(targets));
        // Disk- and process-bound, so a few workers is plenty.
        let workers = thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(2, 8);

        thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let next = queue.lock().unwrap().pop_front();
                    let Some(target) = next else { break };
                    if tx.send(ScanEvent::Repo(Box::new(analyze(&target, fetch)))).is_err() {
                        break;
                    }
                });
            }
        });
        let _ = tx.send(ScanEvent::Finished);
    })
}

/// Scans and waits. `progress` is called with (done, total) after each repo.
pub fn scan_blocking(
    targets: Vec<RepoTarget>,
    fetch: bool,
    mut progress: impl FnMut(usize, usize),
) -> Vec<RepoAnalysis> {
    let (tx, rx) = mpsc::channel();
    let handle = spawn_scan(targets, fetch, tx);
    let mut results = Vec::new();
    let mut total = 0;
    for event in rx {
        match event {
            ScanEvent::Started { total: t } => total = t,
            ScanEvent::Repo(analysis) => {
                results.push(*analysis);
                progress(results.len(), total);
            }
            ScanEvent::Finished => break,
        }
    }
    let _ = handle.join();
    results
}
