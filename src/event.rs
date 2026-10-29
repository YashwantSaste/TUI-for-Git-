use crate::worker::GitJobResult;
use crossterm::event::{self, Event as CEvent, KeyEvent, KeyEventKind, MouseEvent};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::{Duration, Instant};

pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    Resize,
    Git(GitJobResult),
}

/// Spawns a thread that polls the terminal for input and forwards key /
/// mouse / resize / tick events into a shared channel. The git worker
/// thread (see `worker.rs`) pushes `AppEvent::Git` onto the *same*
/// channel via a cloned sender, so the main loop only ever needs to
/// watch one queue.
pub fn spawn_input_thread(tick_rate: Duration) -> (std::sync::mpsc::Sender<AppEvent>, Receiver<AppEvent>) {
    let (tx, rx) = channel::<AppEvent>();
    let input_tx = tx.clone();

    thread::spawn(move || {
        let mut last_tick = Instant::now();
        loop {
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or(Duration::from_millis(0));

            if event::poll(timeout).unwrap_or(false) {
                match event::read() {
                    Ok(CEvent::Key(key)) => {
                        // Some terminals (Windows console, or Unix terminals
                        // with the Kitty keyboard protocol enabled) report a
                        // Press *and* a Release event per keystroke. If we
                        // handled both, every character would appear twice
                        // and every single-key action would fire twice
                        // (which is what made Tab look like it skipped a
                        // panel). Only act on the initial press.
                        if key.kind == KeyEventKind::Press {
                            if input_tx.send(AppEvent::Key(key)).is_err() {
                                break;
                            }
                        }
                    }
                    Ok(CEvent::Mouse(mouse)) => {
                        if input_tx.send(AppEvent::Mouse(mouse)).is_err() {
                            break;
                        }
                    }
                    Ok(CEvent::Resize(_, _)) => {
                        if input_tx.send(AppEvent::Resize).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if input_tx.send(AppEvent::Tick).is_err() {
                    break;
                }
                last_tick = Instant::now();
            }
        }
    });

    (tx, rx)
}
