//! TUI state and key handling. Drawing lives in `ui.rs`.
//!
//! Screens: Home (the launch page and its menu) → Repos (table + detail)
//! and Sources (where to look for workspace files). A text prompt and the
//! help sheet can sit on top of any screen.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use keepsake_core::{
    analysis::{analyze, RepoAnalysis, RepoStatus},
    config::{Config, MAX_REPO_DEPTH},
    fmt,
    plan::Level,
    sandbox::Sandbox,
    scan::{collect_targets, spawn_scan, target_for_path, RepoTarget, ScanEvent},
    workspace::{self, WorkspaceFile},
};
use super::actions::{Confirm, JobEvent};
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::{ListState, TableState},
};

// ── entries ─────────────────────────────────────────────────────────────────

pub struct Entry {
    pub target: RepoTarget,
    pub analysis: Option<RepoAnalysis>,
    /// Being scanned or fetched right now.
    pub busy: bool,
}

impl Entry {
    fn score(&self, now: i64) -> f64 {
        self.analysis.as_ref().map_or(-2.0, |a| a.score(now))
    }
}

// ── screens, menu, prompt ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Repos,
    Sources,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Browse,
    Check,
    Rescan,
    RescanFetch,
    Sources,
    Export,
    Quit,
}

pub struct MenuItem {
    pub key: char,
    pub title: &'static str,
    pub action: MenuAction,
}

pub const MENU: [MenuItem; 7] = [
    MenuItem { key: 'b', title: "Browse repos", action: MenuAction::Browse },
    MenuItem { key: 'c', title: "Check a repo by path", action: MenuAction::Check },
    MenuItem { key: 'r', title: "Rescan", action: MenuAction::Rescan },
    MenuItem { key: 'f', title: "Rescan and fetch every remote", action: MenuAction::RescanFetch },
    MenuItem { key: 'w', title: "Workspace sources", action: MenuAction::Sources },
    MenuItem { key: 'e', title: "Export report", action: MenuAction::Export },
    MenuItem { key: 'q', title: "Quit", action: MenuAction::Quit },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromptPurpose {
    CheckRepo,
    AddSource,
}

impl PromptPurpose {
    pub fn title(self) -> &'static str {
        match self {
            PromptPurpose::CheckRepo => "Check a repo",
            PromptPurpose::AddSource => "Add a workspace source",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            PromptPurpose::CheckRepo => "Paste or type a folder path. It's fetched and fully checked.",
            PromptPurpose::AddSource => {
                "A folder to search for .code-workspace files, or one workspace file."
            }
        }
    }
}

pub struct Prompt {
    pub purpose: PromptPurpose,
    pub input: String,
}

// ── sources ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Saved in the config file.
    Saved,
    /// Nothing saved yet, so the current folder is used.
    Default,
    /// Passed on the command line this time only.
    Launch,
}

pub struct SourceInfo {
    pub path: PathBuf,
    pub kind: SourceKind,
    pub files: usize,
    pub exists: bool,
}

// ── misc state ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Score,
    Size,
    Idle,
    Name,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            SortMode::Score => SortMode::Size,
            SortMode::Size => SortMode::Idle,
            SortMode::Idle => SortMode::Name,
            SortMode::Name => SortMode::Score,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Score => "best to put away",
            SortMode::Size => "size",
            SortMode::Idle => "idle time",
            SortMode::Name => "name",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    at: Instant,
}

pub struct ScanState {
    pub running: bool,
    pub total: usize,
    pub done: usize,
    started: Instant,
    pub took: Option<Duration>,
}

/// Totals for the summary cards, over the current workspace tab.
pub struct Totals {
    pub repos: usize,
    pub on_disk: u64,
    pub build: u64,
    pub freeable: u64,
    pub freeable_repos: usize,
    pub kept: usize,
    pub kept_saved: u64,
    pub needs_attention: usize,
}

pub struct App {
    pub screen: Screen,
    pub menu: ListState,
    pub prompt: Option<Prompt>,
    pub confirm: Option<Confirm>,
    pub job_running: bool,
    pub(super) job_tx: Sender<JobEvent>,
    pub(super) job_rx: Receiver<JobEvent>,
    pub show_help: bool,

    pub config: Config,
    /// Set when running against the throwaway sandbox. Settings stay in memory so
    /// the real config file is never touched.
    pub demo: Option<Sandbox>,
    launch_sources: Vec<PathBuf>,
    pub sources: Vec<SourceInfo>,
    pub source_list: ListState,
    pub load_warnings: Vec<String>,
    reload_after_scan: bool,

    pub workspaces: Vec<WorkspaceFile>,
    pub entries: Vec<Entry>,
    /// Indices into `entries`, filtered and sorted.
    pub view: Vec<usize>,
    pub table: TableState,
    pub tab: usize,
    pub sort: SortMode,
    pub filter: String,
    pub filtering: bool,
    pub level: Level,

    pub scan: ScanState,
    pub toast: Option<Toast>,
    pub tick: u64,
    pub now: i64,
    pub quit: bool,
    tx: Sender<ScanEvent>,
    rx: Receiver<ScanEvent>,
}

/// Windows "Copy as path" wraps paths in quotes; people paste those.
fn clean_path_input(input: &str) -> PathBuf {
    PathBuf::from(input.trim().trim_matches('"').trim_matches('\'').trim())
}

impl App {
    pub fn new(launch_sources: Vec<PathBuf>, demo: Option<Sandbox>) -> Self {
        let (tx, rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel();
        let config = match &demo {
            Some(sb) => Config {
                sources: vec![sb.workspaces_dir.clone()],
                ..Config::default()
            },
            None => Config::load(),
        };
        let mut app = Self {
            screen: Screen::Home,
            menu: ListState::default().with_selected(Some(0)),
            prompt: None,
            confirm: None,
            job_running: false,
            job_tx,
            job_rx,
            show_help: false,
            config,
            demo,
            launch_sources,
            sources: Vec::new(),
            source_list: ListState::default().with_selected(Some(0)),
            load_warnings: Vec::new(),
            reload_after_scan: false,
            workspaces: Vec::new(),
            entries: Vec::new(),
            view: Vec::new(),
            table: TableState::default().with_selected(Some(0)),
            tab: 0,
            sort: SortMode::Score,
            filter: String::new(),
            filtering: false,
            level: Level::Full,
            scan: ScanState {
                running: false,
                total: 0,
                done: 0,
                started: Instant::now(),
                took: None,
            },
            toast: None,
            tick: 0,
            now: fmt::now_unix(),
            quit: false,
            tx,
            rx,
        };
        app.load_workspaces();
        app
    }

    // ── loading ─────────────────────────────────────────────────────────────

    fn source_list_with_kinds(&self) -> Vec<(PathBuf, SourceKind)> {
        let mut list: Vec<(PathBuf, SourceKind)> = if self.config.sources.is_empty() {
            std::env::current_dir()
                .into_iter()
                .map(|p| (p, SourceKind::Default))
                .collect()
        } else {
            self.config
                .sources
                .iter()
                .map(|p| (p.clone(), SourceKind::Saved))
                .collect()
        };
        for extra in &self.launch_sources {
            let extra = workspace::absolute(extra);
            if !list.iter().any(|(p, _)| workspace::path_key(p) == workspace::path_key(&extra)) {
                list.push((extra, SourceKind::Launch));
            }
        }
        list
    }

    /// Finds and reads workspace files from every source, and rebuilds the repo list.
    fn load_workspaces(&mut self) {
        let depth = self.config.search_depth;
        let mut files: Vec<PathBuf> = Vec::new();
        self.sources = self
            .source_list_with_kinds()
            .into_iter()
            .map(|(path, kind)| {
                let found = workspace::discover(std::slice::from_ref(&path), depth);
                let info = SourceInfo {
                    exists: path.exists(),
                    files: found.len(),
                    path,
                    kind,
                };
                for f in found {
                    if !files.iter().any(|x| workspace::path_key(x) == workspace::path_key(&f)) {
                        files.push(f);
                    }
                }
                info
            })
            .collect();

        self.load_warnings.clear();
        self.workspaces = files
            .iter()
            .filter_map(|f| match WorkspaceFile::load(f) {
                Ok(ws) => Some(ws),
                Err(e) => {
                    self.load_warnings.push(e.to_string());
                    None
                }
            })
            .collect();
        self.workspaces.sort_by_key(|w| w.name.to_lowercase());

        let config = &self.config;
        self.entries = collect_targets(&self.workspaces, |name| config.depth_for(name))
            .into_iter()
            .map(|target| Entry {
                target,
                analysis: None,
                busy: false,
            })
            .collect();
        self.tab = 0;
        let sources_len = self.sources.len();
        if self.source_list.selected().is_none_or(|i| i >= sources_len) {
            self.source_list.select(if sources_len == 0 { None } else { Some(0) });
        }
        self.rebuild_view();
    }

    /// Saves settings, except in the demo, which never writes the real config.
    fn persist_config(&mut self) {
        if self.demo.is_some() {
            return;
        }
        if let Err(e) = self.config.save() {
            self.notify(format!("Couldn't save settings: {e}"), ToastKind::Warn);
        }
    }

    fn sources_changed(&mut self) {
        self.persist_config();
        if self.scan.running {
            self.reload_after_scan = true;
            self.notify("Saved. The list updates when this scan finishes.", ToastKind::Info);
        } else {
            self.load_workspaces();
            self.start_scan(false);
        }
    }

    /// Changes how deep the current workspace tab looks for nested repos.
    fn change_depth(&mut self, delta: isize) {
        let Some(ws) = self.tab.checked_sub(1).and_then(|i| self.workspaces.get(i)) else {
            self.notify("Pick a workspace tab to set its repo depth", ToastKind::Info);
            return;
        };
        let name = ws.name.clone();
        let current = self.config.depth_for(&name);
        let next = current.saturating_add_signed(delta).min(MAX_REPO_DEPTH);
        if next == current {
            return;
        }
        if self.scan.running {
            self.notify("Wait for the scan to finish, then change depth", ToastKind::Warn);
            return;
        }
        self.config.set_depth(&name, next);
        self.persist_config();
        self.load_workspaces();
        // load_workspaces resets to the "All" tab; stay on this workspace.
        if let Some(i) = self.workspaces.iter().position(|w| w.name == name) {
            self.tab = i + 1;
            self.rebuild_view();
        }
        self.start_scan(false);
        let found = self.tab_count(self.tab);
        self.notify(
            format!("{name}: depth {next} · {found} repos"),
            ToastKind::Info,
        );
    }

    pub fn start_scan(&mut self, fetch: bool) {
        if self.job_running {
            self.notify("Wait for the current job to finish", ToastKind::Warn);
            return;
        }
        if self.scan.running {
            self.notify("A scan is already running", ToastKind::Warn);
            return;
        }
        for e in &mut self.entries {
            e.busy = true;
        }
        let targets: Vec<RepoTarget> = self.entries.iter().map(|e| e.target.clone()).collect();
        self.scan = ScanState {
            running: !targets.is_empty(),
            total: targets.len(),
            done: 0,
            started: Instant::now(),
            took: None,
        };
        if !targets.is_empty() {
            spawn_scan(targets, fetch, self.tx.clone());
        }
    }

    /// Takes in finished work from background threads. Called once per frame.
    pub fn pump(&mut self) {
        self.now = fmt::now_unix();
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                ScanEvent::Started { total } => self.scan.total = total,
                ScanEvent::Repo(analysis) => {
                    if self.scan.running {
                        self.scan.done += 1;
                    }
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.target.path == analysis.path) {
                        entry.busy = false;
                        entry.analysis = Some(*analysis);
                    }
                    changed = true;
                }
                ScanEvent::Finished => {
                    self.scan.running = false;
                    self.scan.took = Some(self.scan.started.elapsed());
                    if self.reload_after_scan {
                        self.reload_after_scan = false;
                        self.load_workspaces();
                        self.start_scan(false);
                        continue;
                    }
                    let t = self.totals_for_tab(0);
                    self.notify(
                        format!(
                            "Scanned {} repos · {} could be freed",
                            self.entries.len(),
                            fmt::bytes(t.freeable)
                        ),
                        ToastKind::Success,
                    );
                }
            }
        }
        if changed {
            self.rebuild_view();
        }
        if self.toast.as_ref().is_some_and(|t| t.at.elapsed() > Duration::from_secs(4)) {
            self.toast = None;
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toast = Some(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
    }

    // ── repo list ───────────────────────────────────────────────────────────

    pub fn tab_names(&self) -> Vec<String> {
        std::iter::once("All".to_string())
            .chain(self.workspaces.iter().map(|w| w.name.clone()))
            .collect()
    }

    fn in_tab(&self, entry: &Entry, tab: usize) -> bool {
        tab == 0
            || self
                .workspaces
                .get(tab - 1)
                .is_some_and(|ws| entry.target.workspaces.contains(&ws.name))
    }

    pub fn tab_count(&self, tab: usize) -> usize {
        self.entries.iter().filter(|e| self.in_tab(e, tab)).count()
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.table
            .selected()
            .and_then(|i| self.view.get(i))
            .map(|&i| &self.entries[i])
    }

    fn select_path(&mut self, path: &Path) {
        if let Some(pos) = self.view.iter().position(|&i| self.entries[i].target.path == path) {
            self.table.select(Some(pos));
        }
    }

    pub(super) fn rebuild_view(&mut self) {
        let keep = self.selected().map(|e| e.target.path.clone());
        let needle = self.filter.to_lowercase();
        let now = self.now;

        let mut view: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                self.in_tab(e, self.tab)
                    && (needle.is_empty()
                        || e.target.name.to_lowercase().contains(&needle)
                        || e.target.path.to_string_lossy().to_lowercase().contains(&needle))
            })
            .collect();

        let entries = &self.entries;
        let size = |i: usize| {
            entries[i]
                .analysis
                .as_ref()
                .and_then(|a| a.sizes)
                .map_or(0, |s| s.total())
        };
        let idle = |i: usize| {
            entries[i]
                .analysis
                .as_ref()
                .and_then(|a| a.idle_secs(now))
                .unwrap_or(-1)
        };
        match self.sort {
            SortMode::Score => view.sort_by(|&a, &b| entries[b].score(now).total_cmp(&entries[a].score(now))),
            SortMode::Size => view.sort_by_key(|&i| std::cmp::Reverse(size(i))),
            SortMode::Idle => view.sort_by_key(|&i| std::cmp::Reverse(idle(i))),
            SortMode::Name => view.sort_by_key(|&i| entries[i].target.name.to_lowercase()),
        }

        self.view = view;
        let position = keep
            .and_then(|path| self.view.iter().position(|&i| self.entries[i].target.path == path))
            .or(if self.view.is_empty() { None } else { Some(0) });
        self.table.select(position);
    }

    pub fn totals(&self) -> Totals {
        self.totals_for_tab(self.tab)
    }

    pub fn totals_for_tab(&self, tab: usize) -> Totals {
        let mut t = Totals {
            repos: 0,
            on_disk: 0,
            build: 0,
            freeable: 0,
            freeable_repos: 0,
            kept: 0,
            kept_saved: 0,
            needs_attention: 0,
        };
        for e in self.entries.iter().filter(|e| self.in_tab(e, tab)) {
            t.repos += 1;
            let Some(a) = &e.analysis else { continue };
            match a.status {
                RepoStatus::Active => {
                    if let Some(s) = a.sizes {
                        t.on_disk += s.total();
                        t.build += s.build;
                    }
                    if let Some(freed) = a.freed_at(self.level).filter(|&b| b > 0) {
                        t.freeable += freed;
                        t.freeable_repos += 1;
                    }
                    if a.is_blocked() || a.error.is_some() {
                        t.needs_attention += 1;
                    }
                }
                RepoStatus::Kept => {
                    t.kept += 1;
                    t.kept_saved += a.stub.as_ref().map_or(0, |s| s.level.bytes_freed(&s.sizes_before));
                }
                RepoStatus::Missing => t.needs_attention += 1,
                RepoStatus::NotGit => {}
            }
        }
        t
    }

    /// Moves one row, going round from either end to the other.
    fn step(&mut self, forward: bool) {
        let len = self.view.len();
        if len == 0 {
            return;
        }
        let current = self.table.selected().unwrap_or(0);
        let next = if forward { current + 1 } else { current + len - 1 };
        self.table.select(Some(next % len));
    }

    /// Jumps several rows, stopping at the ends.
    fn move_by(&mut self, delta: isize) {
        if self.view.is_empty() {
            return;
        }
        let current = self.table.selected().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.view.len() as isize - 1);
        self.table.select(Some(next as usize));
    }

    fn switch_tab(&mut self, forward: bool) {
        let count = self.workspaces.len() + 1;
        self.tab = if forward {
            (self.tab + 1) % count
        } else {
            (self.tab + count - 1) % count
        };
        self.rebuild_view();
    }

    fn fetch_entry(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        if entry.busy {
            return;
        }
        if entry.analysis.as_ref().is_some_and(|a| a.status != RepoStatus::Active) {
            self.notify("Only active repos can be fetched", ToastKind::Warn);
            return;
        }
        entry.busy = true;
        let target = entry.target.clone();
        let name = target.name.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(ScanEvent::Repo(Box::new(analyze(&target, true))));
        });
        self.notify(format!("Fetching {name} and checking again…"), ToastKind::Info);
    }

    fn fetch_selected(&mut self) {
        if let Some(&index) = self.table.selected().and_then(|i| self.view.get(i)) {
            self.fetch_entry(index);
        }
    }

    // ── actions ─────────────────────────────────────────────────────────────

    pub fn menu_description(&self, action: MenuAction) -> String {
        match action {
            MenuAction::Browse => {
                if self.entries.is_empty() {
                    "No repos found yet. Add a workspace source first.".into()
                } else if self.scan.running {
                    format!(
                        "Scanning {}/{}. You can start browsing now.",
                        self.scan.done, self.scan.total
                    )
                } else {
                    let t = self.totals_for_tab(0);
                    format!(
                        "{} repos · {} could be freed ({})",
                        t.repos,
                        fmt::bytes(t.freeable),
                        self.level.label().to_lowercase()
                    )
                }
            }
            MenuAction::Check => "Paste any folder path. It's fetched and fully checked.".into(),
            MenuAction::Rescan => "Measure every repo again. Quick, uses the last fetch.".into(),
            MenuAction::RescanFetch => {
                "Fetch first so unpushed counts are current. Slower.".into()
            }
            MenuAction::Sources => format!(
                "{} workspace files from {} {}. Add or remove folders.",
                self.workspaces.len(),
                self.sources.len(),
                if self.sources.len() == 1 { "place" } else { "places" }
            ),
            MenuAction::Export => "Save everything to code-keepsake-report.json here.".into(),
            MenuAction::Quit => "You won't remember this. The stubs will.".into(),
        }
    }

    fn activate(&mut self, action: MenuAction) {
        match action {
            MenuAction::Browse => {
                self.screen = Screen::Repos;
            }
            MenuAction::Check => {
                self.prompt = Some(Prompt {
                    purpose: PromptPurpose::CheckRepo,
                    input: String::new(),
                })
            }
            MenuAction::Rescan => {
                if !self.scan.running {
                    self.load_workspaces();
                }
                self.start_scan(false);
            }
            MenuAction::RescanFetch => self.start_scan(true),
            MenuAction::Sources => self.screen = Screen::Sources,
            MenuAction::Export => self.export_report(),
            MenuAction::Quit => self.quit = true,
        }
    }

    fn export_report(&mut self) {
        if self.scan.running {
            self.notify("Wait for the scan to finish first", ToastKind::Warn);
            return;
        }
        let analyses: Vec<&RepoAnalysis> = self.entries.iter().filter_map(|e| e.analysis.as_ref()).collect();
        if analyses.is_empty() {
            self.notify("Nothing to export yet", ToastKind::Warn);
            return;
        }
        let path = std::env::current_dir()
            .unwrap_or_default()
            .join("code-keepsake-report.json");
        let result = serde_json::to_string_pretty(&analyses)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
        match result {
            Ok(()) => self.notify(format!("Saved {}", path.display()), ToastKind::Success),
            Err(e) => self.notify(format!("Couldn't save the report: {e}"), ToastKind::Warn),
        }
    }

    fn submit_prompt(&mut self, prompt: Prompt) {
        let path = clean_path_input(&prompt.input);
        if path.as_os_str().is_empty() {
            return;
        }
        match prompt.purpose {
            PromptPurpose::CheckRepo => {
                if !path.exists() {
                    self.notify(format!("No folder at {}", path.display()), ToastKind::Warn);
                    return;
                }
                let target = target_for_path(&path);
                let root = target.path.clone();
                let index = match self.entries.iter().position(|e| e.target.path == root) {
                    Some(i) => i,
                    None => {
                        self.entries.push(Entry {
                            target,
                            analysis: None,
                            busy: false,
                        });
                        self.entries.len() - 1
                    }
                };
                self.screen = Screen::Repos;
                self.tab = 0;
                self.filter.clear();
                self.rebuild_view();
                self.select_path(&root);
                self.fetch_entry(index);
            }
            PromptPurpose::AddSource => {
                let path = workspace::absolute(&path);
                if !path.exists() {
                    self.notify(format!("Nothing at {}", path.display()), ToastKind::Warn);
                    return;
                }
                let key = workspace::path_key(&path);
                if self.config.sources.iter().any(|p| workspace::path_key(p) == key) {
                    self.notify("That's already a source", ToastKind::Info);
                    return;
                }
                self.config.sources.push(path);
                self.sources_changed();
            }
        }
    }

    fn remove_selected_source(&mut self) {
        let Some(info) = self.source_list.selected().and_then(|i| self.sources.get(i)) else {
            return;
        };
        let (kind, key) = (info.kind, workspace::path_key(&info.path));
        match kind {
            SourceKind::Default => self.notify(
                "This is the default (the current folder). Add a folder and it's replaced.",
                ToastKind::Info,
            ),
            SourceKind::Launch => self.notify(
                "This one was passed when launching, so it's only here this time.",
                ToastKind::Info,
            ),
            SourceKind::Saved => {
                self.config.sources.retain(|p| workspace::path_key(p) != key);
                self.sources_changed();
            }
        }
    }

    // ── keys ────────────────────────────────────────────────────────────────

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.confirm.is_some() {
            self.on_confirm_key(key);
            return;
        }
        if let Some(prompt) = &mut self.prompt {
            match key.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => {
                    if let Some(prompt) = self.prompt.take() {
                        self.submit_prompt(prompt);
                    }
                }
                KeyCode::Backspace => {
                    prompt.input.pop();
                }
                KeyCode::Char(c) => prompt.input.push(c),
                _ => {}
            }
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        if key.code == KeyCode::Char('?') {
            self.show_help = true;
            return;
        }
        match self.screen {
            Screen::Home => self.on_home_key(key),
            Screen::Repos => self.on_repos_key(key),
            Screen::Sources => self.on_sources_key(key),
        }
    }

    fn on_home_key(&mut self, key: KeyEvent) {
        let selected = self.menu.selected().unwrap_or(0);
        match key.code {
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.menu.select(Some((selected + 1) % MENU.len()))
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.menu.select(Some((selected + MENU.len() - 1) % MENU.len()))
            }
            KeyCode::Enter | KeyCode::Right => self.activate(MENU[selected].action),
            KeyCode::Esc => self.quit = true,
            KeyCode::Char(c @ ('1' | '2' | '3')) => self.level = Level::ALL[(c as u8 - b'1') as usize],
            KeyCode::Char(c) => {
                if let Some(pos) = MENU.iter().position(|m| m.key == c) {
                    self.menu.select(Some(pos));
                    self.activate(MENU[pos].action);
                }
            }
            _ => {}
        }
    }

    fn on_sources_key(&mut self, key: KeyEvent) {
        let len = self.sources.len();
        let selected = self.source_list.selected().unwrap_or(0);
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => self.screen = Screen::Home,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') if len > 0 => self.source_list.select(Some((selected + 1) % len)),
            KeyCode::Up | KeyCode::Char('k') if len > 0 => self.source_list.select(Some((selected + len - 1) % len)),
            KeyCode::Char('a') | KeyCode::Insert => {
                self.prompt = Some(Prompt {
                    purpose: PromptPurpose::AddSource,
                    input: String::new(),
                })
            }
            KeyCode::Char('d') | KeyCode::Delete => self.remove_selected_source(),
            _ => {}
        }
    }

    fn on_repos_key(&mut self, key: KeyEvent) {
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter | KeyCode::Down | KeyCode::Up => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.rebuild_view();
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc | KeyCode::Backspace => {
                if self.filter.is_empty() {
                    self.screen = Screen::Home;
                } else {
                    self.filter.clear();
                    self.rebuild_view();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.step(true),
            KeyCode::Up | KeyCode::Char('k') => self.step(false),
            KeyCode::PageDown => self.move_by(8),
            KeyCode::PageUp => self.move_by(-8),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_by(isize::MAX / 2),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.switch_tab(true),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => self.switch_tab(false),
            KeyCode::Char('p') => self.change_depth(1),
            KeyCode::Char('m') => self.change_depth(-1),
            KeyCode::Char('o') => {
                self.sort = self.sort.next();
                self.rebuild_view();
                self.notify(format!("Sorted by {}", self.sort.label()), ToastKind::Info);
            }
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char(c @ ('1' | '2' | '3')) => self.level = Level::ALL[(c as u8 - b'1') as usize],
            KeyCode::Char('f') => self.fetch_selected(),
            KeyCode::Char('r') | KeyCode::F(5) => self.start_scan(false),
            KeyCode::Char('s') | KeyCode::Enter => self.open_confirm(),
            KeyCode::Char('e') => self.open_folder(),
            _ => {}
        }
    }
}
