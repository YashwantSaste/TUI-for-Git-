use crate::git::{self, Branch, FileEntry, GraphRow, StatusReport};
use crate::worker::{DiffSource, GitJob, GitJobResult};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// Number of ticks a modal takes to grow to full size (see `on_tick`).
pub const MODAL_ANIM_MAX: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Status,
    Graph,
    Branches,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSection {
    Staged,
    Unstaged,
    Untracked,
}

pub enum Mode {
    Normal,
    CommandPalette,
    CommitMessage,
    NewBranch,
    CloneRepo,
    ConfirmDiscard(String),
    Help,
}

impl Mode {
    fn is_modal(&self) -> bool {
        !matches!(self, Mode::Normal)
    }
}

pub enum MessageLevel {
    Info,
    Error,
}

#[derive(Clone)]
struct PaletteItem {
    label: String,
    job: PaletteJob,
}

#[derive(Clone)]
enum PaletteJob {
    Refresh,
    StageAll,
    Commit,
    Push,
    Pull,
    Fetch,
    Stash,
    StashPop,
    NewBranch,
    Clone,
    Checkout(String),
    Quit,
}

pub struct App {
    pub repo: PathBuf,
    pub branch: String,
    pub ahead_behind: Option<(u32, u32)>,
    pub status: StatusReport,
    pub graph: Vec<GraphRow>,
    pub branches: Vec<Branch>,

    pub focus: Panel,
    pub status_section: StatusSection,
    pub status_selected: usize,
    pub graph_selected: usize,
    pub graph_scroll: usize,
    pub branch_selected: usize,

    pub mode: Mode,
    pub input_buffer: String,
    pub palette_query: String,
    palette_items: Vec<PaletteItem>,
    pub palette_filtered: Vec<usize>,
    pub palette_selected: usize,

    pub diff_title: String,
    pub diff_text: String,
    pub diff_scroll: u16,

    pub message: Option<(String, MessageLevel)>,
    pub should_quit: bool,
    pub loading: bool,

    /// Free-running frame counter, incremented on every tick. Drives the
    /// header spinner and anything else that wants a cheap animation
    /// clock without tracking its own timer.
    pub tick: u64,
    /// 0 the instant a modal opens, growing to `MODAL_ANIM_MAX` a few
    /// ticks later, so popups grow in instead of just appearing.
    pub modal_anim: u8,

    job_tx: Sender<GitJob>,
    matcher: SkimMatcherV2,
}

impl App {
    pub fn new(repo: PathBuf, job_tx: Sender<GitJob>) -> Self {
        Self {
            repo,
            branch: String::new(),
            ahead_behind: None,
            status: StatusReport::default(),
            graph: Vec::new(),
            branches: Vec::new(),
            focus: Panel::Status,
            status_section: StatusSection::Unstaged,
            status_selected: 0,
            graph_selected: 0,
            graph_scroll: 0,
            branch_selected: 0,
            mode: Mode::Normal,
            input_buffer: String::new(),
            palette_query: String::new(),
            palette_items: Vec::new(),
            palette_filtered: Vec::new(),
            palette_selected: 0,
            diff_title: String::new(),
            diff_text: String::new(),
            diff_scroll: 0,
            message: None,
            should_quit: false,
            loading: true,
            tick: 0,
            modal_anim: 0,
            job_tx,
            matcher: SkimMatcherV2::default(),
        }
    }

    /// Advances the animation clock. Called once per tick event.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.mode.is_modal() {
            self.modal_anim = (self.modal_anim + 1).min(MODAL_ANIM_MAX);
        } else {
            self.modal_anim = 0;
        }
    }

    /// Label for the item at `filtered_idx` within `palette_filtered`.
    pub fn palette_label(&self, filtered_idx: usize) -> String {
        self.palette_filtered
            .get(filtered_idx)
            .and_then(|&i| self.palette_items.get(i))
            .map(|item| item.label.clone())
            .unwrap_or_default()
    }

    pub fn dispatch(&self, job: GitJob) {
        let _ = self.job_tx.send(job);
    }

    pub fn refresh(&mut self) {
        self.loading = true;
        self.dispatch(GitJob::Refresh);
    }

    /// Switches into a modal mode and resets its grow-in animation.
    fn open_modal(&mut self, mode: Mode) {
        self.modal_anim = 0;
        self.input_buffer.clear();
        self.mode = mode;
    }

    /// Leaves whatever modal is open and clears its scratch input.
    fn close_modal(&mut self) {
        self.mode = Mode::Normal;
        self.input_buffer.clear();
    }

    pub fn apply_result(&mut self, result: GitJobResult) {
        match result {
            GitJobResult::Refreshed(data) => {
                self.branch = data.branch;
                self.ahead_behind = data.ahead_behind;
                self.status = data.status;
                self.graph = data.graph;
                self.branches = data.branches;
                self.loading = false;
                self.clamp_selections();
            }
            GitJobResult::Diff { title, text } => {
                self.diff_title = title;
                self.diff_text = if text.is_empty() {
                    "(no changes / binary file)".to_string()
                } else {
                    text
                };
                self.diff_scroll = 0;
            }
            GitJobResult::ActionOk(msg) => {
                self.message = Some((msg, MessageLevel::Info));
            }
            GitJobResult::ActionErr(msg) => {
                self.message = Some((msg, MessageLevel::Error));
            }
            GitJobResult::Cloned(path) => {
                self.repo = path;
            }
        }
    }

    fn clamp_selections(&mut self) {
        let len = self.current_section_files().len();
        if len == 0 {
            self.status_selected = 0;
        } else if self.status_selected >= len {
            self.status_selected = len - 1;
        }
        if self.graph.is_empty() {
            self.graph_selected = 0;
        } else if self.graph_selected >= self.graph.len() {
            self.graph_selected = self.graph.len() - 1;
        }
        if self.branches.is_empty() {
            self.branch_selected = 0;
        } else if self.branch_selected >= self.branches.len() {
            self.branch_selected = self.branches.len() - 1;
        }
    }

    pub fn current_section_files(&self) -> &[FileEntry] {
        match self.status_section {
            StatusSection::Staged => &self.status.staged,
            StatusSection::Unstaged => &self.status.unstaged,
            StatusSection::Untracked => &self.status.untracked,
        }
    }

    fn selected_file(&self) -> Option<&FileEntry> {
        self.current_section_files().get(self.status_selected)
    }

    fn load_diff_for_selection(&mut self) {
        if let Some(entry) = self.selected_file() {
            let path = entry.path.clone();
            let source = match self.status_section {
                StatusSection::Staged => DiffSource::Staged(path),
                StatusSection::Unstaged => DiffSource::Unstaged(path),
                StatusSection::Untracked => DiffSource::Untracked(path),
            };
            self.dispatch(GitJob::LoadDiff(source));
        }
    }

    fn load_diff_for_commit(&mut self) {
        if let Some(row) = self.graph.get(self.graph_selected) {
            self.dispatch(GitJob::LoadDiff(DiffSource::Commit(row.commit.hash.clone())));
        }
    }

    fn open_palette(&mut self) {
        let mut items = vec![
            PaletteItem { label: "Push".into(), job: PaletteJob::Push },
            PaletteItem { label: "Pull".into(), job: PaletteJob::Pull },
            PaletteItem { label: "Commit staged changes".into(), job: PaletteJob::Commit },
            PaletteItem { label: "Clone repository...".into(), job: PaletteJob::Clone },
            PaletteItem { label: "Fetch --all".into(), job: PaletteJob::Fetch },
            PaletteItem { label: "Stage all changes".into(), job: PaletteJob::StageAll },
            PaletteItem { label: "Stash changes".into(), job: PaletteJob::Stash },
            PaletteItem { label: "Pop stash".into(), job: PaletteJob::StashPop },
            PaletteItem { label: "New branch...".into(), job: PaletteJob::NewBranch },
            PaletteItem { label: "Refresh".into(), job: PaletteJob::Refresh },
            PaletteItem { label: "Quit".into(), job: PaletteJob::Quit },
        ];
        for b in &self.branches {
            items.push(PaletteItem {
                label: format!("Checkout branch: {}", b.name),
                job: PaletteJob::Checkout(b.name.clone()),
            });
        }
        self.palette_items = items;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.refilter_palette();
        self.modal_anim = 0;
        self.mode = Mode::CommandPalette;
    }

    fn refilter_palette(&mut self) {
        if self.palette_query.is_empty() {
            self.palette_filtered = (0..self.palette_items.len()).collect();
        } else {
            let mut scored: Vec<(i64, usize)> = self
                .palette_items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    self.matcher
                        .fuzzy_match(&item.label, &self.palette_query)
                        .map(|score| (score, i))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            self.palette_filtered = scored.into_iter().map(|(_, i)| i).collect();
        }
        self.palette_selected = 0;
    }

    fn run_palette_selection(&mut self) {
        if let Some(&idx) = self.palette_filtered.get(self.palette_selected) {
            let job = self.palette_items[idx].job.clone();
            match job {
                PaletteJob::Refresh => self.refresh(),
                PaletteJob::StageAll => self.dispatch(GitJob::StageAll),
                PaletteJob::Commit => {
                    self.open_modal(Mode::CommitMessage);
                    return;
                }
                PaletteJob::Push => self.dispatch(GitJob::Push),
                PaletteJob::Pull => self.dispatch(GitJob::Pull),
                PaletteJob::Fetch => self.dispatch(GitJob::Fetch),
                PaletteJob::Stash => self.dispatch(GitJob::Stash),
                PaletteJob::StashPop => self.dispatch(GitJob::StashPop),
                PaletteJob::NewBranch => {
                    self.open_modal(Mode::NewBranch);
                    return;
                }
                PaletteJob::Clone => {
                    self.open_modal(Mode::CloneRepo);
                    return;
                }
                PaletteJob::Checkout(name) => self.dispatch(GitJob::CheckoutBranch(name)),
                PaletteJob::Quit => self.should_quit = true,
            }
        }
        self.mode = Mode::Normal;
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        match &self.mode {
            Mode::Normal => self.on_key_normal(key),
            Mode::CommandPalette => self.on_key_palette(key),
            Mode::CommitMessage => self.on_key_text_input(key, TextInputTarget::Commit),
            Mode::NewBranch => self.on_key_text_input(key, TextInputTarget::NewBranch),
            Mode::CloneRepo => self.on_key_text_input(key, TextInputTarget::Clone),
            Mode::ConfirmDiscard(path) => {
                let path = path.clone();
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        self.dispatch(GitJob::Discard(path));
                        self.close_modal();
                    }
                    _ => self.close_modal(),
                }
            }
            Mode::Help => {
                self.close_modal();
            }
        }
    }

    fn on_key_normal(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
            self.open_palette();
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char(':') => self.open_palette(),
            KeyCode::Char('?') => self.open_modal(Mode::Help),
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Left | KeyCode::Char('h') => {
                if self.focus == Panel::Status {
                    self.cycle_status_section(-1);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.focus == Panel::Status {
                    self.cycle_status_section(1);
                }
            }
            KeyCode::Enter => self.on_enter(),
            KeyCode::Char(' ') | KeyCode::Char('s') => self.on_toggle_stage(),
            KeyCode::Char('c') => {
                if !self.status.staged.is_empty() {
                    self.open_modal(Mode::CommitMessage);
                } else {
                    self.message = Some(("nothing staged to commit".into(), MessageLevel::Error));
                }
            }
            KeyCode::Char('g') => self.open_modal(Mode::CloneRepo),
            KeyCode::Char('n') => self.open_modal(Mode::NewBranch),
            KeyCode::Char('d') => {
                if self.focus == Panel::Status {
                    if let Some(entry) = self.selected_file() {
                        let path = entry.path.clone();
                        self.open_modal(Mode::ConfirmDiscard(path));
                    }
                }
            }
            KeyCode::Char('P') => self.dispatch(GitJob::Push),
            KeyCode::Char('F') => self.dispatch(GitJob::Fetch),
            KeyCode::Char('U') => self.dispatch(GitJob::Pull),
            _ => {}
        }
    }

    fn on_key_palette(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_modal(),
            KeyCode::Enter => self.run_palette_selection(),
            KeyCode::Down => {
                if !self.palette_filtered.is_empty() {
                    self.palette_selected =
                        (self.palette_selected + 1).min(self.palette_filtered.len() - 1);
                }
            }
            KeyCode::Up => {
                self.palette_selected = self.palette_selected.saturating_sub(1);
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.refilter_palette();
            }
            KeyCode::Char(c) => {
                self.palette_query.push(c);
                self.refilter_palette();
            }
            _ => {}
        }
    }

    fn on_key_text_input(&mut self, key: KeyEvent, target: TextInputTarget) {
        match key.code {
            KeyCode::Esc => self.close_modal(),
            KeyCode::Enter => {
                let text = self.input_buffer.trim().to_string();
                if !text.is_empty() {
                    match target {
                        TextInputTarget::Commit => self.dispatch(GitJob::Commit(text)),
                        TextInputTarget::NewBranch => self.dispatch(GitJob::NewBranch(text)),
                        TextInputTarget::Clone => self.start_clone(&text),
                    }
                }
                self.close_modal();
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    /// Parses "`<url>` [dest]" from the clone modal and dispatches the job.
    /// A relative/omitted destination is resolved next to the currently
    /// open repo, so cloning doesn't require typing a full path.
    fn start_clone(&mut self, text: &str) {
        let mut parts = text.splitn(2, char::is_whitespace);
        let url = match parts.next() {
            Some(u) if !u.is_empty() => u.to_string(),
            _ => return,
        };
        let dest_arg = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

        let base = self.repo.parent().unwrap_or(&self.repo).to_path_buf();
        let dest = match dest_arg {
            Some(d) => {
                let p = PathBuf::from(d);
                if p.is_absolute() {
                    p
                } else {
                    base.join(p)
                }
            }
            None => base.join(git::guess_clone_dir_name(&url)),
        };

        self.loading = true;
        self.dispatch(GitJob::Clone { url, dest });
    }

    fn cycle_focus(&mut self, dir: i32) {
        let order = [Panel::Status, Panel::Graph, Panel::Branches, Panel::Diff];
        let idx = order.iter().position(|p| *p == self.focus).unwrap_or(0) as i32;
        let len = order.len() as i32;

        // Skip panels that currently have nothing to show/select, so Tab
        // doesn't park focus somewhere that looks like nothing happened.
        // The diff panel is always a valid landing spot (it shows a
        // placeholder even when empty), so this can never loop forever.
        for step in 1..=len {
            let next = ((idx + dir * step) % len + len) % len;
            let candidate = order[next as usize];
            if self.panel_has_content(candidate) {
                self.focus = candidate;
                return;
            }
        }
    }

    fn panel_has_content(&self, panel: Panel) -> bool {
        match panel {
            Panel::Status => {
                !self.status.staged.is_empty()
                    || !self.status.unstaged.is_empty()
                    || !self.status.untracked.is_empty()
            }
            Panel::Graph => !self.graph.is_empty(),
            Panel::Branches => !self.branches.is_empty(),
            Panel::Diff => true,
        }
    }

    fn cycle_status_section(&mut self, dir: i32) {
        let order = [
            StatusSection::Unstaged,
            StatusSection::Staged,
            StatusSection::Untracked,
        ];
        let idx = order
            .iter()
            .position(|s| *s == self.status_section)
            .unwrap_or(0) as i32;
        let len = order.len() as i32;
        let next = ((idx + dir) % len + len) % len;
        self.status_section = order[next as usize];
        self.status_selected = 0;
    }

    fn move_selection(&mut self, dir: i32) {
        match self.focus {
            Panel::Status => {
                let len = self.current_section_files().len();
                if len == 0 {
                    return;
                }
                let idx = self.status_selected as i32 + dir;
                self.status_selected = idx.clamp(0, len as i32 - 1) as usize;
                self.load_diff_for_selection();
            }
            Panel::Graph => {
                if self.graph.is_empty() {
                    return;
                }
                let idx = self.graph_selected as i32 + dir;
                self.graph_selected = idx.clamp(0, self.graph.len() as i32 - 1) as usize;
                self.load_diff_for_commit();
            }
            Panel::Branches => {
                if self.branches.is_empty() {
                    return;
                }
                let idx = self.branch_selected as i32 + dir;
                self.branch_selected = idx.clamp(0, self.branches.len() as i32 - 1) as usize;
            }
            Panel::Diff => {
                self.diff_scroll = if dir > 0 {
                    self.diff_scroll.saturating_add(1)
                } else {
                    self.diff_scroll.saturating_sub(1)
                };
            }
        }
    }

    fn on_enter(&mut self) {
        match self.focus {
            Panel::Status => self.load_diff_for_selection(),
            Panel::Graph => self.load_diff_for_commit(),
            Panel::Branches => {
                if let Some(b) = self.branches.get(self.branch_selected) {
                    self.dispatch(GitJob::CheckoutBranch(b.name.clone()));
                }
            }
            Panel::Diff => {}
        }
    }

    fn on_toggle_stage(&mut self) {
        if self.focus != Panel::Status {
            return;
        }
        if let Some(entry) = self.selected_file() {
            let path = entry.path.clone();
            match self.status_section {
                StatusSection::Staged => self.dispatch(GitJob::Unstage(path)),
                StatusSection::Unstaged | StatusSection::Untracked => {
                    self.dispatch(GitJob::Stage(path))
                }
            }
        }
    }
}

enum TextInputTarget {
    Commit,
    NewBranch,
    Clone,
}
