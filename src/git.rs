use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

/// Runs a git subcommand in `repo` and returns stdout as a String.
/// Shelling out to the real `git` binary (rather than a library) keeps us
/// compatible with hooks, credential helpers, LFS, SSO, and whatever else
/// an enterprise repo has bolted onto vanilla git.
fn run(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("git {:?} failed: {}", args, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Same as `run` but does not fail the whole call on non-zero exit
/// (some git plumbing, e.g. diff with changes present, can be noisy).
fn run_allow_fail(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn git {:?}", args))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn repo_root(start: &Path) -> Result<std::path::PathBuf> {
    let out = run(start, &["rev-parse", "--show-toplevel"])?;
    Ok(std::path::PathBuf::from(out.trim()))
}

pub fn current_branch(repo: &Path) -> Result<String> {
    let out = run_allow_fail(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = out.trim();
    if branch.is_empty() {
        Ok("(detached)".to_string())
    } else {
        Ok(branch.to_string())
    }
}

pub fn ahead_behind(repo: &Path) -> Option<(u32, u32)> {
    let out = run_allow_fail(
        repo,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    )
    .ok()?;
    let mut parts = out.trim().split_whitespace();
    let ahead = parts.next()?.parse().ok()?;
    let behind = parts.next()?.parse().ok()?;
    Some((ahead, behind))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Conflicted,
}

impl FileState {
    pub fn glyph(&self) -> &'static str {
        match self {
            FileState::Modified => "M",
            FileState::Added => "A",
            FileState::Deleted => "D",
            FileState::Renamed => "R",
            FileState::Copied => "C",
            FileState::Untracked => "?",
            FileState::Conflicted => "!",
        }
    }

    fn from_code(c: char) -> Option<Self> {
        match c {
            'M' => Some(FileState::Modified),
            'A' => Some(FileState::Added),
            'D' => Some(FileState::Deleted),
            'R' => Some(FileState::Renamed),
            'C' => Some(FileState::Copied),
            'U' => Some(FileState::Conflicted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub staged: Option<FileState>,
    pub unstaged: Option<FileState>,
}

#[derive(Debug, Clone, Default)]
pub struct StatusReport {
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub untracked: Vec<FileEntry>,
    pub conflicted: Vec<FileEntry>,
}

/// Parses `git status --porcelain=v2` into staged / unstaged / untracked / conflicted buckets.
pub fn status(repo: &Path) -> Result<StatusReport> {
    let out = run(repo, &["status", "--porcelain=v2", "--untracked-files=all"])?;
    let mut report = StatusReport::default();

    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let tag = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");

        match tag {
            "1" => {
                // "1 XY sub mH mI mW hH hI <path>" — exactly 8 space-separated
                // fields, so splitn(8, ' ') leaves the path untouched as the
                // final element even if it later contains spaces.
                let fields: Vec<&str> = rest.splitn(8, ' ').collect();
                let xy = fields.first().copied().unwrap_or("..");
                let path = fields.last().copied().unwrap_or("").to_string();
                let mut chars = xy.chars();
                let x = chars.next().unwrap_or('.');
                let y = chars.next().unwrap_or('.');
                let staged = FileState::from_code(x);
                let unstaged = FileState::from_code(y);
                if staged.is_some() {
                    report.staged.push(FileEntry {
                        path: path.clone(),
                        staged: staged.clone(),
                        unstaged: None,
                    });
                }
                if unstaged.is_some() {
                    report.unstaged.push(FileEntry {
                        path,
                        staged: None,
                        unstaged,
                    });
                }
            }
            "2" => {
                // "2 XY sub mH mI mW hH hI Xscore <path>\t<origPath>" — 9
                // space-separated fields; the last one holds "path\torig".
                let fields: Vec<&str> = rest.splitn(9, ' ').collect();
                let xy = fields.first().copied().unwrap_or("..");
                let path = fields
                    .last()
                    .and_then(|s| s.split('\t').next())
                    .unwrap_or("")
                    .to_string();
                let mut chars = xy.chars();
                let x = chars.next().unwrap_or('.');
                let y = chars.next().unwrap_or('.');
                let staged = FileState::from_code(x);
                let unstaged = FileState::from_code(y);
                if staged.is_some() {
                    report.staged.push(FileEntry {
                        path: path.clone(),
                        staged: staged.clone(),
                        unstaged: None,
                    });
                }
                if unstaged.is_some() {
                    report.unstaged.push(FileEntry {
                        path,
                        staged: None,
                        unstaged,
                    });
                }
            }
            "u" => {
                // unmerged / conflicted
                let remainder: Vec<&str> = rest.splitn(10, ' ').collect();
                let path = remainder.last().unwrap_or(&"").to_string();
                report.conflicted.push(FileEntry {
                    path,
                    staged: None,
                    unstaged: Some(FileState::Conflicted),
                });
            }
            "?" => {
                let path = rest.trim().to_string();
                report.untracked.push(FileEntry {
                    path,
                    staged: None,
                    unstaged: Some(FileState::Untracked),
                });
            }
            _ => {}
        }
    }

    Ok(report)
}

pub fn stage(repo: &Path, path: &str) -> Result<()> {
    run(repo, &["add", "--", path])?;
    Ok(())
}

pub fn unstage(repo: &Path, path: &str) -> Result<()> {
    run(repo, &["restore", "--staged", "--", path])?;
    Ok(())
}

pub fn stage_all(repo: &Path) -> Result<()> {
    run(repo, &["add", "-A"])?;
    Ok(())
}

pub fn discard(repo: &Path, path: &str) -> Result<()> {
    run(repo, &["checkout", "--", path])?;
    Ok(())
}

pub fn commit(repo: &Path, message: &str) -> Result<()> {
    run(repo, &["commit", "-m", message])?;
    Ok(())
}

pub fn diff_unstaged(repo: &Path, path: &str) -> Result<String> {
    run_allow_fail(repo, &["diff", "--no-color", "--", path])
}

pub fn diff_staged(repo: &Path, path: &str) -> Result<String> {
    run_allow_fail(repo, &["diff", "--cached", "--no-color", "--", path])
}

pub fn diff_untracked(repo: &Path, path: &str) -> Result<String> {
    run_allow_fail(
        repo,
        &["diff", "--no-color", "--no-index", "--", "/dev/null", path],
    )
}

pub fn show_commit(repo: &Path, hash: &str) -> Result<String> {
    run_allow_fail(repo, &["show", "--no-color", hash])
}

pub fn push(repo: &Path) -> Result<String> {
    run(repo, &["push"])
}

pub fn pull(repo: &Path) -> Result<String> {
    run(repo, &["pull"])
}

pub fn fetch(repo: &Path) -> Result<String> {
    run(repo, &["fetch", "--all"])
}

/// Clones `url` into `dest`. Unlike every other function here this does
/// NOT take an existing repo to run `-C` against — a clone doesn't need
/// one, since the destination doesn't exist yet.
pub fn clone_repo(url: &str, dest: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("clone")
        .arg(url)
        .arg(dest)
        .output()
        .context("failed to spawn git clone")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("git clone failed: {}", stderr.trim()));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
}

pub fn branches(repo: &Path) -> Result<Vec<Branch>> {
    let out = run(
        repo,
        &[
            "branch",
            "--all",
            "--format=%(HEAD)|%(refname:short)|%(objecttype)",
        ],
    )?;
    let mut result = Vec::new();
    for line in out.lines() {
        let mut parts = line.splitn(3, '|');
        let head = parts.next().unwrap_or("");
        let name = parts.next().unwrap_or("").to_string();
        if name.contains("HEAD ->") || name.is_empty() {
            continue;
        }
        result.push(Branch {
            is_current: head.trim() == "*",
            is_remote: name.starts_with("remotes/") || name.contains('/'),
            name,
        });
    }
    Ok(result)
}

pub fn checkout_branch(repo: &Path, name: &str) -> Result<()> {
    run(repo, &["checkout", name])?;
    Ok(())
}

/// Best-effort guess at the directory name git will clone into when no
/// explicit destination is given, e.g. "git@host:org/repo.git" -> "repo".
pub fn guess_clone_dir_name(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed);
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

pub fn create_branch(repo: &Path, name: &str) -> Result<()> {
    run(repo, &["checkout", "-b", name])?;
    Ok(())
}

pub fn stash(repo: &Path) -> Result<()> {
    run(repo, &["stash", "push"])?;
    Ok(())
}

pub fn stash_pop(repo: &Path) -> Result<()> {
    run(repo, &["stash", "pop"])?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub refs: Vec<String>,
}

const LOG_SEP: &str = "\u{1f}"; // unit separator, unlikely to appear in messages

pub fn log(repo: &Path, limit: usize) -> Result<Vec<Commit>> {
    let fmt = format!(
        "%H{sep}%h{sep}%P{sep}%an{sep}%ad{sep}%s{sep}%D",
        sep = LOG_SEP
    );
    let out = run(
        repo,
        &[
            "log",
            "--all",
            "--topo-order",
            "--date=short",
            &format!("--pretty=format:{}", fmt),
            &format!("-n{}", limit),
        ],
    )?;

    let mut commits = Vec::new();
    for line in out.lines() {
        let fields: Vec<&str> = line.split(LOG_SEP).collect();
        if fields.len() < 7 {
            continue;
        }
        let refs = fields[6]
            .split(", ")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        commits.push(Commit {
            hash: fields[0].to_string(),
            short_hash: fields[1].to_string(),
            parents: fields[2]
                .split_whitespace()
                .map(|s| s.to_string())
                .collect(),
            author: fields[3].to_string(),
            date: fields[4].to_string(),
            subject: fields[5].to_string(),
            refs,
        });
    }
    Ok(commits)
}

/// One row of the rendered commit graph.
#[derive(Debug, Clone)]
pub struct GraphRow {
    pub commit: Commit,
    pub lane: usize,
    /// Lanes this commit connects down to (first-parent lane first).
    pub parent_lanes: Vec<usize>,
    pub is_merge: bool,
    pub active_lane_count: usize,
    /// Which lanes were occupied (had a pending commit) immediately
    /// before this row was processed - used to draw pass-through lines
    /// for lanes that don't touch this commit.
    pub active_before: Vec<bool>,
}

/// Assigns a lane (column) to every commit so the UI can draw an
/// interactive ASCII/Unicode commit graph, similar in spirit to `git log
/// --graph` but as structured data we can click/select/color per-lane.
///
/// This is a simplified lane-tracking algorithm: it is not a byte-for-byte
/// match of git's own graph renderer, but it produces a correct, readable
/// lane assignment for typical linear + merge histories.
pub fn build_graph(commits: &[Commit]) -> Vec<GraphRow> {
    let mut active: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let active_before: Vec<bool> = active.iter().map(|c| c.is_some()).collect();

        let lane = if let Some(pos) = active.iter().position(|c| c.as_deref() == Some(commit.hash.as_str()))
        {
            pos
        } else if let Some(pos) = active.iter().position(|c| c.is_none()) {
            active[pos] = None; // will be set below
            pos
        } else {
            active.push(None);
            active.len() - 1
        };

        let mut parent_lanes = Vec::new();
        if commit.parents.is_empty() {
            active[lane] = None;
        } else {
            active[lane] = Some(commit.parents[0].clone());
            parent_lanes.push(lane);
            for extra_parent in &commit.parents[1..] {
                if active.iter().any(|c| c.as_deref() == Some(extra_parent.as_str())) {
                    let pos = active
                        .iter()
                        .position(|c| c.as_deref() == Some(extra_parent.as_str()))
                        .unwrap();
                    parent_lanes.push(pos);
                    continue;
                }
                if let Some(pos) = active.iter().position(|c| c.is_none()) {
                    active[pos] = Some(extra_parent.clone());
                    parent_lanes.push(pos);
                } else {
                    active.push(Some(extra_parent.clone()));
                    parent_lanes.push(active.len() - 1);
                }
            }
        }

        rows.push(GraphRow {
            commit: commit.clone(),
            lane,
            parent_lanes,
            is_merge: commit.parents.len() > 1,
            active_lane_count: active.len(),
            active_before,
        });
    }

    rows
}
