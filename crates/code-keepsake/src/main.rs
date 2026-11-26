//! `code-keepsake` opens the interactive view. That's the whole interface:
//! everything else is picked from the launch page.
//!
//! `code-keepsake demo` builds a throwaway set of repos and opens the view on
//! them, so putting repos away can be tried safely. `demo --reset` rebuilds it
//! and `demo --clean` deletes it.

mod tui;

use std::path::PathBuf;

use keepsake_core::sandbox;

fn main() -> anyhow::Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.iter().any(|a| a.as_os_str() == "--version" || a.as_os_str() == "-V") {
        println!("code-keepsake {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|a| a.as_os_str() == "demo") {
        return demo(args.get(1).and_then(|a| a.to_str()));
    }
    // Optional: any paths given are searched as well as the saved sources.
    // Nothing needs to be passed; sources are managed from the launch page.
    tui::run(args, None)
}

fn demo(flag: Option<&str>) -> anyhow::Result<()> {
    let dir = sandbox::default_dir();
    match flag {
        Some("--clean") => {
            if sandbox::is_sandbox(&dir) {
                sandbox::destroy(&dir)?;
                println!("Deleted the demo at {}", dir.display());
            } else {
                println!("No demo to delete.");
            }
            Ok(())
        }
        None | Some("--reset") => {
            // Reuse an existing demo unless asked to rebuild it, so a half-way
            // session can be picked up again.
            let sb = if flag.is_none() && sandbox::is_sandbox(&dir) {
                sandbox::open(&dir)?
            } else {
                sandbox::create(&dir)?
            };
            tui::run(Vec::new(), Some(sb))
        }
        Some(other) => anyhow::bail!("unknown option `{other}`. Use `demo`, `demo --reset` or `demo --clean`."),
    }
}
