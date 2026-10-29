# gitwave

An interactive Git TUI built in Rust with [ratatui](https://ratatui.rs). It
shells out to the real `git` binary (never a reimplementation of git's
plumbing), so it stays compatible with hooks, credential helpers, LFS,
SSO, and whatever else an enterprise repo has layered on top of vanilla git.

## Highlights

- **Interactive commit graph** — not just `git log --graph` text; commits
  are laid out on lanes as structured data, colored per-branch, navigable
  with `j`/`k`, and each commit's full diff loads on selection.
- **Quick-actions toolbar** — pull, push, commit, clone, stage, and fetch
  are always visible in a keycap-styled bar on the main screen, so the
  actions you reach for constantly don't require memorizing anything or
  digging into a menu.
- **Command palette** (`Ctrl+K` or `:`) — fuzzy-searchable list of every
  repo action (stage all, commit, push, pull, fetch, stash, checkout any
  branch, new branch, clone, quit) for everything else.
- **Clone from inside the app** (`g`) — clone a repo by URL without
  leaving the TUI; gitwave switches straight into the new clone.
- **Non-blocking on large repos** — all git calls run on a background
  worker thread over an mpsc channel; the UI thread never blocks on a
  slow `status`/`log`/`push` in a big monorepo.
- **Inline diff with staging** — stage/unstage per file with `space`,
  view staged/unstaged/untracked/commit diffs side by side, colored by
  hunk.
- **Branch panel** with one-key checkout, plus stash/pop from the palette.
- A warm single-accent theme, rounded panels, an animated loading spinner,
  and popups that grow in rather than snapping into view.

## Build & run

Requires Rust (edition 2021) and `git` on your `PATH`.

```bash
cargo build --release
./target/release/gitwave        # run from inside any git repo
```

Or during development:

```bash
cargo run
```

By default `gitwave` resolves the repo root (`git rev-parse
--show-toplevel`) from wherever you launch it inside a working tree. To
point it at a different repo, pass the path explicitly — any of these
work, from anywhere on disk:

```bash
gitwave /path/to/repo
gitwave --repo /path/to/repo
gitwave -C /path/to/repo
gitwave ~/code/some-repo       # ~ is expanded
```

The path can be the repo root or any directory inside it. `gitwave
--help` prints the full usage.

## Keybindings

The always-visible **Quick actions** toolbar shows the ones you'll use
constantly; the rest are one key away too:

| Key | Action |
|---|---|
| `U` / `P` / `F` | Pull / Push / Fetch --all |
| `c` | Commit staged changes |
| `g` | Clone a repository (prompts for `<url> [dest]`) |
| `space` or `s` | Stage / unstage selected file |
| `n` | Create a new branch |
| `Tab` / `Shift+Tab` | Cycle focused panel (Files → Graph → Branches → Diff) |
| `j`/`k`, `↑`/`↓` | Move selection / scroll diff |
| `h`/`l` | Switch status section (Unstaged / Staged / Untracked) |
| `Enter` | View diff of selected file or commit, or checkout selected branch |
| `d` | Discard changes to selected file (asks to confirm) |
| `Ctrl+K` or `:` | Open the command palette (fuzzy search over every action) |
| `r` | Refresh |
| `?` | Toggle help |
| `q` | Quit |
| `Esc` | Close whatever popup is open |

## Architecture

```
src/
  main.rs    – terminal setup/teardown, owns the render loop
  event.rs   – input-polling thread; merges keyboard events, ticks, and
               worker results onto one channel so the render loop only
               ever watches a single queue
  worker.rs  – background thread that executes GitJob values (Refresh,
               Stage, Commit, Push, ...) and reports GitJobResult back
  git.rs     – thin synchronous wrapper around the `git` CLI: status
               parsing (porcelain v2), log parsing, and the commit-graph
               lane-assignment algorithm
  app.rs     – application state + key-event handling (pure, no I/O)
  ui.rs      – ratatui rendering: layout, panels, popups
```

The UI thread never calls `git` directly. Every mutating or slow action
(`stage`, `commit`, `push`, `pull`, `fetch`, `checkout`, ...) is dispatched
as a `GitJob` to the worker thread and the UI re-renders as soon as the
result arrives — so a slow network push or a `status` scan on a huge
repo never freezes the interface.

## Extending it

Some natural next steps if you want to keep building on this:

- **Interactive rebase** — drag-select a commit range in the graph and
  emit a `git rebase -i` todo list.
- **Hunk-level staging** — stage/unstage individual diff hunks (`git
  apply --cached` on a synthesized patch) instead of whole files.
- **Conflict resolution view** — parse `git status -z` conflict markers
  and offer per-hunk accept-ours/accept-theirs actions.
- **Config-driven keybindings** — read a `~/.config/gitwave/config.toml`.
- **Search/filter in the graph** — fuzzy-filter commits by author or
  message, following the same `fuzzy-matcher` already wired up for the
  command palette.

`git.rs` and `worker.rs` are deliberately decoupled from `ui.rs`/`app.rs`,
so most of these can be added as a new `GitJob` variant + a new panel
without touching the rest of the app.
