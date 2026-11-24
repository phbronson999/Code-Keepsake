mod app;
mod theme;
mod ui;
mod widgets;

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

pub fn run(extra_sources: Vec<PathBuf>) -> Result<()> {
    let mut app = app::App::new(extra_sources);
    app.start_scan(false);

    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        while !app.quit {
            app.pump();
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if event::poll(Duration::from_millis(80))? {
                // Windows reports key releases too; act on presses only.
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        app.on_key(key);
                    }
                }
            }
            app.tick = app.tick.wrapping_add(1);
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
