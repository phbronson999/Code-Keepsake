//! Putting repos away and bringing them back from the Repos screen.
//!
//! Nothing runs without the confirm box, which first does a dry run and shows
//! what would happen. The work itself runs on a background thread.

use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::Sender,
    thread,
};

use keepsake_core::{
    analysis::{analyze, RepoAnalysis, RepoStatus},
    execute::{self, ExecReport},
    fmt,
    plan::Level,
    scan::RepoTarget,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::app::{App, ToastKind};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    PutAway(Level),
    Restore,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::PutAway(level) => level.label(),
            Action::Restore => "Restore",
        }
    }

    /// Anything that deletes source or history asks for the repo's name.
    pub fn needs_name(self) -> bool {
        matches!(self, Action::PutAway(Level::Hollow | Level::Full))
    }
}

pub struct Confirm {
    pub action: Action,
    pub target: RepoTarget,
    /// The dry run; `None` while it's still checking. Unused for restore.
    pub preview: Option<Result<ExecReport, String>>,
    pub typed: String,
}

impl Confirm {
    /// True once the dry run says there is something to do and nothing against it.
    pub fn ready(&self) -> bool {
        match (&self.action, &self.preview) {
            (Action::Restore, _) => true,
            (_, Some(Ok(report))) => report.blocked.is_empty() && !report.actions.is_empty(),
            _ => false,
        }
    }
}

pub enum JobEvent {
    Preview(PathBuf, Action, Result<ExecReport, String>),
    Done(Action, Result<Option<ExecReport>, String>, Box<RepoAnalysis>),
}

fn preview(target: RepoTarget, level: Level, tx: Sender<JobEvent>) {
    thread::spawn(move || {
        let result = execute::execute(&target.path, level, true, &target.workspaces).map_err(|e| e.to_string());
        let _ = tx.send(JobEvent::Preview(target.path, Action::PutAway(level), result));
    });
}

impl App {
    pub fn open_confirm(&mut self) {
        let Some(entry) = self.selected() else { return };
        let target = entry.target.clone();
        let status = entry.analysis.as_ref().map(|a| a.status);
        if self.scan.running || entry.busy {
            self.notify("Wait for the scan to finish first", ToastKind::Warn);
            return;
        }
        if self.job_running {
            self.notify("Still working on the last one", ToastKind::Warn);
            return;
        }
        let action = match status {
            Some(RepoStatus::Active) => Action::PutAway(self.level),
            Some(RepoStatus::Kept) => Action::Restore,
            _ => {
                self.notify("There's no repo here to put away", ToastKind::Warn);
                return;
            }
        };
        if let Action::PutAway(level) = action {
            preview(target.clone(), level, self.job_tx.clone());
        }
        self.confirm = Some(Confirm {
            action,
            target,
            preview: None,
            typed: String::new(),
        });
    }

    /// Shows the selected repo's folder in the system file manager.
    pub fn open_folder(&mut self) {
        let Some(path) = self.selected().map(|e| e.target.path.clone()) else { return };
        if !path.is_dir() {
            self.notify("That folder doesn't exist", ToastKind::Warn);
            return;
        }
        let program = if cfg!(windows) {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        // Explorer wants backslashes, and reports failure even when it works,
        // so only a failure to start it is worth mentioning.
        let shown: PathBuf = path.components().collect();
        let spawned = Command::new(program)
            .arg(shown)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                thread::spawn(move || child.wait());
                self.notify("Opened the folder", ToastKind::Info);
            }
            Err(e) => self.notify(format!("Couldn't open the folder: {e}"), ToastKind::Warn),
        }
    }

    pub fn on_confirm_key(&mut self, key: KeyEvent) {
        let Some(confirm) = &mut self.confirm else { return };
        let ready = confirm.ready();
        let go = match key.code {
            KeyCode::Esc => {
                self.confirm = None;
                return;
            }
            KeyCode::Backspace => {
                confirm.typed.pop();
                false
            }
            KeyCode::Enter => ready && confirm.action.needs_name() && confirm.typed == confirm.target.name,
            KeyCode::Char('y') if !confirm.action.needs_name() => ready,
            KeyCode::Char(c) if ready && confirm.action.needs_name() => {
                confirm.typed.push(c);
                false
            }
            _ => false,
        };
        if go {
            self.run_confirmed();
        }
    }

    fn run_confirmed(&mut self) {
        let Some(Confirm { action, target, .. }) = self.confirm.take() else { return };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.target.path == target.path) {
            entry.busy = true;
        }
        self.job_running = true;
        self.notify(format!("{} {}…", action.label(), target.name), ToastKind::Info);
        let tx = self.job_tx.clone();
        thread::spawn(move || {
            let result = match action {
                Action::PutAway(level) => execute::execute(&target.path, level, false, &target.workspaces).map(Some),
                Action::Restore => execute::restore(&target.path).map(|()| None),
            }
            .map_err(|e| e.to_string());
            let analysis = analyze(&target, false);
            let _ = tx.send(JobEvent::Done(action, result, Box::new(analysis)));
        });
    }

    /// Takes in finished previews and jobs. Called once per frame.
    pub fn pump_jobs(&mut self) {
        while let Ok(event) = self.job_rx.try_recv() {
            match event {
                JobEvent::Preview(path, action, result) => {
                    if let Some(confirm) = &mut self.confirm {
                        if confirm.target.path == path && confirm.action == action {
                            confirm.preview = Some(result);
                        }
                    }
                }
                JobEvent::Done(action, result, analysis) => self.finish_job(action, result, *analysis),
            }
        }
    }

    fn finish_job(&mut self, action: Action, result: Result<Option<ExecReport>, String>, analysis: RepoAnalysis) {
        self.job_running = false;
        let name = analysis.name.clone();
        let (text, kind) = match result {
            Err(e) => (format!("{name}: {e}"), ToastKind::Warn),
            Ok(None) => (format!("Restored {name}"), ToastKind::Success),
            Ok(Some(report)) => {
                if let Some(reason) = report.blocked.first() {
                    (format!("{name} was left alone: {reason}"), ToastKind::Warn)
                } else if let Some(error) = report.errors.first() {
                    (format!("{name}, partly done: {error}"), ToastKind::Warn)
                } else {
                    (
                        format!("{} {name} · freed {}", action.label(), fmt::bytes(report.bytes_freed)),
                        ToastKind::Success,
                    )
                }
            }
        };
        if let Some(entry) = self.entries.iter_mut().find(|e| e.target.path == analysis.path) {
            entry.busy = false;
            entry.analysis = Some(analysis);
        }
        self.rebuild_view();
        self.notify(text, kind);
    }
}

/// Drives the real key handling and drawing against a sandbox, with no terminal.
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use keepsake_core::sandbox;
    use ratatui::{backend::TestBackend, crossterm::event::KeyModifiers, Terminal};

    use super::*;
    use crate::tui::{app::Screen, ui};

    fn wait(app: &mut App, term: &mut Terminal<TestBackend>, done: impl Fn(&App) -> bool) -> String {
        let start = Instant::now();
        loop {
            app.pump();
            app.pump_jobs();
            term.draw(|f| ui::draw(f, app)).unwrap();
            if done(app) {
                return term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
            }
            assert!(start.elapsed() < Duration::from_secs(60), "timed out");
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn select(app: &mut App, name: &str) {
        let row = app.view.iter().position(|&i| app.entries[i].target.name == name).unwrap();
        app.table.select(Some(row));
    }

    #[test]
    fn put_away_and_restore_through_the_confirm_box() {
        let dir = std::env::temp_dir().join(format!("keepsake-tui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sb = sandbox::create(&dir).unwrap();
        let root = sb.root.clone();
        let repo = root.join("repos").join("clean-app");

        let mut app = App::new(Vec::new(), Some(sb));
        let mut term = Terminal::new(TestBackend::new(140, 45)).unwrap();
        app.start_scan(false);
        wait(&mut app, &mut term, |a| !a.scan.running);
        app.screen = Screen::Repos;
        app.level = Level::Full;
        select(&mut app, "clean-app");

        press(&mut app, KeyCode::Char('s'));
        let shown = wait(&mut app, &mut term, |a| a.confirm.as_ref().is_some_and(|c| c.preview.is_some()));
        assert!(shown.contains("and press enter"), "{shown}");

        // A wrong name does nothing.
        press(&mut app, KeyCode::Enter);
        assert!(app.confirm.is_some() && !app.job_running);
        "clean-app".chars().for_each(|c| press(&mut app, KeyCode::Char(c)));
        press(&mut app, KeyCode::Enter);
        assert!(app.job_running);
        wait(&mut app, &mut term, |a| !a.job_running);
        assert!(!repo.join(".git").exists());

        select(&mut app, "clean-app");
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('y'));
        wait(&mut app, &mut term, |a| !a.job_running);
        assert!(repo.join("README.md").exists() && repo.join(".git").exists());

        // A blocked repo never offers to go ahead.
        select(&mut app, "dirty-notes");
        press(&mut app, KeyCode::Char('s'));
        let shown = wait(&mut app, &mut term, |a| a.confirm.as_ref().is_some_and(|c| c.preview.is_some()));
        assert!(shown.contains("Blocked"), "{shown}");
        assert!(!app.confirm.as_ref().unwrap().ready());
        press(&mut app, KeyCode::Esc);

        sandbox::destroy(&root).unwrap();
    }
}
