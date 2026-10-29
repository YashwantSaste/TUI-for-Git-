mod app;
mod event;
mod git;
mod ui;
mod worker;

use anyhow::{Context, Result};
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::AppEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;
use worker::GitJob;

fn main() -> Result<()> {
    let repo = match resolve_repo_arg()? {
        Some(path) => path,
        None => return Ok(()), // --help was printed
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, repo);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

/// Resolves the repo to open from CLI args, defaulting to the current
/// directory. Accepts a bare path (`gitwave ../other-repo`) or an
/// explicit `--repo <path>` / `-C <path>` flag, either of which may
/// point anywhere on disk — it does not need to be the current
/// directory. Returns `Ok(None)` if `--help`/`-h` was handled instead.
fn resolve_repo_arg() -> Result<Option<PathBuf>> {
    let mut args = std::env::args().skip(1);
    let mut path_arg: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "--repo" | "-C" => {
                path_arg = Some(
                    args.next()
                        .context("--repo/-C requires a path argument")?,
                );
            }
            other if !other.starts_with('-') && path_arg.is_none() => {
                path_arg = Some(other.to_string());
            }
            other => {
                anyhow::bail!("unrecognized argument: {other}\n\nRun `gitwave --help` for usage.");
            }
        }
    }

    let start = match path_arg {
        Some(p) => {
            let expanded = PathBuf::from(shellexpand_home(&p));
            std::fs::canonicalize(&expanded)
                .with_context(|| format!("path does not exist: {}", expanded.display()))?
        }
        None => std::env::current_dir().context("failed to read current directory")?,
    };

    let repo = git::repo_root(&start).with_context(|| {
        format!(
            "{} is not inside a git repository (or git is not on PATH)",
            start.display()
        )
    })?;

    Ok(Some(repo))
}

/// Expands a leading `~` to the user's home directory. We don't want a
/// shell dependency just for this, so it's handled manually.
fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    } else if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    path.to_string()
}

fn print_help() {
    println!(
        "\
gitwave — an interactive git TUI

USAGE:
    gitwave [PATH]
    gitwave --repo <PATH>
    gitwave -C <PATH>

ARGS:
    <PATH>    Path to a git repository, or any directory inside one.
              Defaults to the current directory.

OPTIONS:
    -C, --repo <PATH>   Same as passing PATH positionally.
    -h, --help          Print this help and exit."
    );
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo: PathBuf,
) -> Result<()> {
    let (events_tx, events_rx) = event::spawn_input_thread(Duration::from_millis(80));
    let (jobs_tx, jobs_rx) = channel::<GitJob>();
    worker::spawn_worker(repo.clone(), jobs_rx, events_tx);

    let mut app = App::new(repo, jobs_tx);
    app.refresh();

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        match events_rx.recv() {
            Ok(AppEvent::Key(key)) => app.on_key(key),
            Ok(AppEvent::Git(result)) => app.apply_result(result),
            Ok(AppEvent::Tick) => app.on_tick(),
            Ok(AppEvent::Mouse(_)) | Ok(AppEvent::Resize) => {}
            Err(_) => break,
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
