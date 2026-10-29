use crate::event::AppEvent;
use crate::git::{self, Branch, GraphRow, StatusReport};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

#[derive(Clone, Debug)]
pub enum DiffSource {
    Staged(String),
    Unstaged(String),
    Untracked(String),
    Commit(String),
}

#[derive(Debug)]
pub enum GitJob {
    Refresh,
    Stage(String),
    Unstage(String),
    StageAll,
    Discard(String),
    Commit(String),
    Push,
    Pull,
    Fetch,
    CheckoutBranch(String),
    NewBranch(String),
    Stash,
    StashPop,
    LoadDiff(DiffSource),
    Clone { url: String, dest: PathBuf },
}

pub struct RefreshData {
    pub repo: PathBuf,
    pub branch: String,
    pub ahead_behind: Option<(u32, u32)>,
    pub status: StatusReport,
    pub graph: Vec<GraphRow>,
    pub branches: Vec<Branch>,
}

pub enum GitJobResult {
    Refreshed(RefreshData),
    Diff { title: String, text: String },
    ActionOk(String),
    ActionErr(String),
    Cloned(PathBuf),
}

fn build_refresh(repo: &PathBuf) -> RefreshData {
    let branch = git::current_branch(repo).unwrap_or_else(|_| "?".into());
    let ahead_behind = git::ahead_behind(repo);
    let status = git::status(repo).unwrap_or_default();
    let commits = git::log(repo, 800).unwrap_or_default();
    let graph = git::build_graph(&commits);
    let branches = git::branches(repo).unwrap_or_default();
    RefreshData {
        repo: repo.clone(),
        branch,
        ahead_behind,
        status,
        graph,
        branches,
    }
}

pub fn spawn_worker(repo: PathBuf, jobs_rx: Receiver<GitJob>, events_tx: Sender<AppEvent>) {
    thread::spawn(move || {
        let mut repo = repo;
        for job in jobs_rx {
            match job {
                GitJob::Refresh => {
                    let data = build_refresh(&repo);
                    let _ = events_tx.send(AppEvent::Git(GitJobResult::Refreshed(data)));
                }
                GitJob::Stage(path) => {
                    report(&events_tx, git::stage(&repo, &path), &format!("staged {}", path));
                    refresh(&repo, &events_tx);
                }
                GitJob::Unstage(path) => {
                    report(&events_tx, git::unstage(&repo, &path), &format!("unstaged {}", path));
                    refresh(&repo, &events_tx);
                }
                GitJob::StageAll => {
                    report(&events_tx, git::stage_all(&repo), "staged all changes");
                    refresh(&repo, &events_tx);
                }
                GitJob::Discard(path) => {
                    report(&events_tx, git::discard(&repo, &path), &format!("discarded {}", path));
                    refresh(&repo, &events_tx);
                }
                GitJob::Commit(msg) => {
                    report(&events_tx, git::commit(&repo, &msg), "committed");
                    refresh(&repo, &events_tx);
                }
                GitJob::Push => {
                    match git::push(&repo) {
                        Ok(_) => {
                            let _ = events_tx.send(AppEvent::Git(GitJobResult::ActionOk("pushed".into())));
                        }
                        Err(e) => {
                            let _ = events_tx
                                .send(AppEvent::Git(GitJobResult::ActionErr(format!("push failed: {}", e))));
                        }
                    }
                    refresh(&repo, &events_tx);
                }
                GitJob::Pull => {
                    match git::pull(&repo) {
                        Ok(_) => {
                            let _ = events_tx.send(AppEvent::Git(GitJobResult::ActionOk("pulled".into())));
                        }
                        Err(e) => {
                            let _ = events_tx
                                .send(AppEvent::Git(GitJobResult::ActionErr(format!("pull failed: {}", e))));
                        }
                    }
                    refresh(&repo, &events_tx);
                }
                GitJob::Fetch => {
                    match git::fetch(&repo) {
                        Ok(_) => {
                            let _ = events_tx.send(AppEvent::Git(GitJobResult::ActionOk("fetched".into())));
                        }
                        Err(e) => {
                            let _ = events_tx
                                .send(AppEvent::Git(GitJobResult::ActionErr(format!("fetch failed: {}", e))));
                        }
                    }
                    refresh(&repo, &events_tx);
                }
                GitJob::CheckoutBranch(name) => {
                    report(&events_tx, git::checkout_branch(&repo, &name), &format!("switched to {}", name));
                    refresh(&repo, &events_tx);
                }
                GitJob::NewBranch(name) => {
                    report(&events_tx, git::create_branch(&repo, &name), &format!("created {}", name));
                    refresh(&repo, &events_tx);
                }
                GitJob::Stash => {
                    report(&events_tx, git::stash(&repo), "stashed changes");
                    refresh(&repo, &events_tx);
                }
                GitJob::StashPop => {
                    report(&events_tx, git::stash_pop(&repo), "popped stash");
                    refresh(&repo, &events_tx);
                }
                GitJob::LoadDiff(source) => {
                    let (title, text) = match &source {
                        DiffSource::Staged(p) => (
                            format!("staged: {}", p),
                            git::diff_staged(&repo, p).unwrap_or_default(),
                        ),
                        DiffSource::Unstaged(p) => (
                            format!("unstaged: {}", p),
                            git::diff_unstaged(&repo, p).unwrap_or_default(),
                        ),
                        DiffSource::Untracked(p) => (
                            format!("untracked: {}", p),
                            git::diff_untracked(&repo, p).unwrap_or_default(),
                        ),
                        DiffSource::Commit(h) => (
                            format!("commit: {}", &h[..h.len().min(10)]),
                            git::show_commit(&repo, h).unwrap_or_default(),
                        ),
                    };
                    let _ = events_tx.send(AppEvent::Git(GitJobResult::Diff { title, text }));
                }
                GitJob::Clone { url, dest } => {
                    match git::clone_repo(&url, &dest) {
                        Ok(()) => {
                            let _ = events_tx.send(AppEvent::Git(GitJobResult::ActionOk(format!(
                                "cloned into {}",
                                dest.display()
                            ))));
                            repo = dest.clone();
                            let _ = events_tx.send(AppEvent::Git(GitJobResult::Cloned(dest)));
                            refresh(&repo, &events_tx);
                        }
                        Err(e) => {
                            let _ = events_tx.send(AppEvent::Git(GitJobResult::ActionErr(format!(
                                "clone failed: {}",
                                e
                            ))));
                        }
                    }
                }
            }
        }
    });
}

fn report(tx: &Sender<AppEvent>, result: anyhow::Result<()>, ok_msg: &str) {
    match result {
        Ok(()) => {
            let _ = tx.send(AppEvent::Git(GitJobResult::ActionOk(ok_msg.to_string())));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::Git(GitJobResult::ActionErr(e.to_string())));
        }
    }
}

fn refresh(repo: &PathBuf, tx: &Sender<AppEvent>) {
    let data = build_refresh(repo);
    let _ = tx.send(AppEvent::Git(GitJobResult::Refreshed(data)));
}
