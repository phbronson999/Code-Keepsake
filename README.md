# Code Keepsake

*For developers with short-term memory loss about their own repos.*

You have a condition. You can't form new memories about which remote that old Tauri experiment was pushed to. You know who you are. You know you had a branch called `spike`. But since the repo left your disk, everything's fuzzy.

Code Keepsake doesn't fix your memory. It does what Leonard Shelby did: it leaves notes everywhere, and it never trusts anyone, least of all you.

> **Status:** scanning, analysis and the interactive view work. Putting repos away (`store`) and getting them back (`restore`) come next. We'll remember. Probably. It's written down here, which is the point.

---

## 5. How it works (told backwards, obviously)

Memento tells its story in reverse, so this README counts down too. You start at the end.

**The end:** the folder is still in your workspace. Open VS Code and it's right there in the sidebar, just much lighter. Inside is a single file, `.keepsake.json`.

**Just before that:** Code Keepsake wrote that file. It's the Polaroid with handwriting on the back. It records the remotes, the branch, the commit, what was saved and when. Unlike a Polaroid, it doesn't fade, and it doesn't come with a note in someone else's handwriting telling you not to trust it.

**Before that:** anything that existed only on your machine got tattooed into the stub. Unpushed commits and every stash went into a bundle; `.env` files, signing keys and local settings went into a zip. Facts, not memories.

**Before that:** it checked everything. Unpushed commits. Stashes. Uncommitted changes. Extra worktrees. Ignored files that matter. It fetched first, because the remote-tracking refs on your disk are just memories, and memories can be changed and distorted.

**The beginning:** you opened Code Keepsake and it read your `.code-workspace` files. That's the wall of photos and string. It's the only map it trusts.

## 4. Run it

There's one command. That's the whole interface. You will not be asked to remember anything else, because you won't.

```powershell
code-keepsake
```

(From the source folder: `cargo run --release`.)

It opens on the launch page and starts scanning right away. Everything you can do is listed there, each with a single-letter shortcut, so even if you forget the letters you can still arrow down and press enter:

| Key | Option | What it does |
|---|---|---|
| `b` | **Browse repos** | The wall of Polaroids: every repo ranked by how much you'd gain from putting it away, with a detail panel for the one you're looking at |
| `c` | **Check a repo by path** | Paste any folder path (quotes from "Copy as path" are fine). It's fetched and fully checked. |
| `r` | **Rescan** | Measure everything again, using the last fetch |
| `f` | **Rescan and fetch every remote** | Same, but check the facts against the remotes first. Slower, more trustworthy. |
| `w` | **Workspace sources** | The folders to search for `.code-workspace` files. Set them once and they're saved, like a tattoo, minus the regret. |
| `e` | **Export report** | Writes `code-keepsake-report.json` to the current folder, for scripts that also can't remember anything |
| `q` | **Quit** | You won't remember this. The stubs will. |

Press `?` on any screen to see its keys, and `esc` to go back.

Until you add a workspace source, Code Keepsake searches the folder you launched it from. Sources are saved in `%APPDATA%\code-keepsake\config.json` (or `~/.config/code-keepsake/` elsewhere).

## 3. Levels

How much do you want to forget?

| Key | Level | What it does | Memento equivalent |
|---|---|---|---|
| `1` | **Clean** | Deletes ignored build output (`target/`, `node_modules/`, `dist/`, …). The repo stays usable. Nothing can block it. | Throwing out the motel receipts. You didn't need them. You were never going to read them. |
| `2` | **Hollow** | Keeps `.git`, empties the working tree. Restores offline in seconds. | The memories are gone but the tattoos stay. You can rebuild everything from what's on your skin. |
| `3` | **Full** | Keeps only the stub. Restoring clones from the remote plus a bundle of anything that was only here. | A single Polaroid with an address on the back. It's enough to find your way back, if you trust the handwriting. (You can. It's yours.) |

## 2. Safety checks

Every repo gets one of three verdicts.

- **✓ ready.** Everything here is also on a remote. Nothing to remember, which suits you.
- **◆ N to save.** Some things exist only on this machine: unpushed commits, stashes, `.env` files, hooks. Code Keepsake saves them into the stub before anything leaves. This is the tattoo stage.
- **✕ blocking.** Uncommitted changes, or other worktrees still using this repo. Code Keepsake won't touch it until you deal with these. Don't make decisions based on a note you haven't finished writing.

Unpushed counts are compared with the last fetch, and the last fetch is a memory. Press `f` on a repo, or pick **Rescan and fetch** on the launch page, to check the facts against the remote right now.

### Remember Sammy Jankis

Sammy was a developer. He had a repo with three stashes, an unpushed branch called `actually-working`, and a `.env` with the only copy of a production API key. He deleted the folder to free up 9 GB because "it's all on GitHub anyway."

It was not all on GitHub.

Sammy now runs `git stash list` every morning in an empty folder, hoping something comes back. Code Keepsake exists so you never become Sammy.

## 1. Layout

```
crates/
  keepsake-core/   no UI: workspace parsing, git, sizes, safety, plans, stub format, scanner, settings
  code-keepsake/   the binary: the ratatui TUI (launch page, repo browser, sources)
tests-std/         the std-only core modules compiled alone, for tests with nothing but rustc
```

`keepsake-core` has no UI code, so a GUI can sit on top of it later, once you remember you wanted one.

## 0. Tests

You can't trust your memory, so test.

```powershell
cargo test --workspace
```

The std-only modules (JSONC parsing, git output parsing, size measurement, formatting) can also be tested with nothing but `rustc`:

```powershell
cd tests-std
rustc --edition 2021 --test harness.rs -o harness; ./harness
```

If a test fails, write it down. On the back of a Polaroid if you have to.
