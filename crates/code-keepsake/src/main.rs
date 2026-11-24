//! `code-keepsake` opens the interactive view. That's the whole interface:
//! everything else is picked from the launch page.

mod tui;

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.iter().any(|a| a.as_os_str() == "--version" || a.as_os_str() == "-V") {
        println!("code-keepsake {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Optional: any paths given are searched as well as the saved sources.
    // Nothing needs to be passed; sources are managed from the launch page.
    tui::run(args)
}
