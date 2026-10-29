use crate::app::{App, MessageLevel, Mode, Panel, StatusSection, MODAL_ANIM_MAX};
use crate::git::FileState;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

// ---- theme ---------------------------------------------------------------
// A warm, single-accent palette (rather than "every panel a new color")
// reads calmer and more deliberate than the crossterm defaults.
const ACCENT: Color = Color::Rgb(217, 119, 87); // warm terracotta
const ACCENT_SOFT: Color = Color::Rgb(196, 154, 108);
const MUTED: Color = Color::Rgb(110, 110, 120);
const SUCCESS: Color = Color::Rgb(120, 190, 120);
const DANGER: Color = Color::Rgb(224, 108, 108);
const TEXT: Color = Color::Rgb(225, 225, 225);

const LANE_COLORS: [Color; 6] = [
    ACCENT,
    Color::Rgb(120, 170, 210),
    Color::Rgb(160, 140, 210),
    SUCCESS,
    Color::Rgb(210, 180, 100),
    Color::Rgb(200, 120, 150),
];

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

fn lane_color(lane: usize) -> Color {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

fn panel_block(title: &str, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED)
    };
    Block::default()
        .title(Span::styled(format!(" {} ", title), style))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style)
}

pub fn draw(f: &mut Frame, app: &App) {
    let size = f.size();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // action toolbar
            Constraint::Min(5),    // panels
            Constraint::Length(1), // status line
        ])
        .split(size);

    draw_header(f, app, root[0]);
    draw_toolbar(f, app, root[1]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(36),
            Constraint::Percentage(36),
        ])
        .split(root[2]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(body[0]);

    draw_status(f, app, left[0]);
    draw_branches(f, app, left[1]);
    draw_graph(f, app, body[1]);
    draw_diff(f, app, body[2]);

    draw_status_line(f, app, root[3]);

    match &app.mode {
        Mode::CommandPalette => draw_palette(f, app),
        Mode::CommitMessage => draw_text_popup(f, app, "Commit message", "type a message"),
        Mode::NewBranch => draw_text_popup(f, app, "New branch name", "e.g. feature/my-change"),
        Mode::CloneRepo => draw_text_popup(
            f,
            app,
            "Clone repository",
            "<url> [optional destination folder]",
        ),
        Mode::ConfirmDiscard(path) => draw_confirm(f, app, path),
        Mode::Help => draw_help(f, app),
        Mode::Normal => {}
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let ab = match app.ahead_behind {
        Some((a, b)) => format!("  ↑{a} ↓{b}"),
        None => String::new(),
    };
    let loading = if app.loading {
        let frame = SPINNER[(app.tick as usize / 2) % SPINNER.len()];
        format!("  {frame} working…")
    } else {
        String::new()
    };
    let title = Line::from(vec![
        Span::styled(
            " gitwave ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            &app.branch,
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        ),
        Span::styled(ab, Style::default().fg(ACCENT_SOFT)),
        Span::styled(loading, Style::default().fg(MUTED)),
    ]);
    let msg = match &app.message {
        Some((text, MessageLevel::Info)) => {
            Line::from(Span::styled(text.clone(), Style::default().fg(SUCCESS)))
        }
        Some((text, MessageLevel::Error)) => {
            Line::from(Span::styled(text.clone(), Style::default().fg(DANGER)))
        }
        None => Line::from(Span::styled(
            app.repo.display().to_string(),
            Style::default().fg(MUTED),
        )),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);
    f.render_widget(Paragraph::new(title), rows[0]);
    f.render_widget(Paragraph::new(msg), rows[1]);
}

/// Keycap-styled shortcut, e.g. `[^K] Palette`.
fn keycap<'a>(key: &'a str, label: &'a str) -> Vec<Span<'a>> {
    vec![
        Span::styled("[", Style::default().fg(MUTED)),
        Span::styled(key, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("] ", Style::default().fg(MUTED)),
        Span::styled(label, Style::default().fg(TEXT)),
    ]
}

/// A persistent, always-visible bar for the actions people reach for
/// constantly (pull/push/commit/clone), so they never have to remember
/// a keybinding or dig through the palette for the common path.
fn draw_toolbar(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(MUTED))
        .title(Span::styled(
            " Quick actions ",
            Style::default().fg(MUTED),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut spans = Vec::new();
    let groups: [(&str, &str); 8] = [
        ("U", "Pull"),
        ("P", "Push"),
        ("c", "Commit"),
        ("g", "Clone"),
        ("s", "Stage"),
        ("F", "Fetch"),
        ("^K", "Palette"),
        ("?", "Help"),
    ];
    for (i, (key, label)) in groups.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.extend(keycap(key, label));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn state_style(state: &FileState) -> Style {
    match state {
        FileState::Modified => Style::default().fg(Color::Rgb(220, 190, 100)),
        FileState::Added => Style::default().fg(SUCCESS),
        FileState::Deleted => Style::default().fg(DANGER),
        FileState::Renamed | FileState::Copied => Style::default().fg(Color::Rgb(120, 170, 210)),
        FileState::Untracked => Style::default().fg(MUTED),
        FileState::Conflicted => Style::default()
            .fg(Color::Rgb(200, 120, 150))
            .add_modifier(Modifier::BOLD),
    }
}

/// Highlight style for a selected row: an accent-tinted background reads
/// as more "designed" than a plain terminal reverse-video block.
fn selected_style(base: Style) -> Style {
    base.bg(Color::Rgb(60, 42, 34)).add_modifier(Modifier::BOLD)
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Panel::Status;
    let section_label = match app.status_section {
        StatusSection::Staged => "Staged",
        StatusSection::Unstaged => "Unstaged",
        StatusSection::Untracked => "Untracked",
    };
    let counts = format!(
        "({} staged / {} unstaged / {} untracked)",
        app.status.staged.len(),
        app.status.unstaged.len(),
        app.status.untracked.len()
    );
    let block = panel_block(&format!("Files · {} {}", section_label, counts), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let files = app.current_section_files();
    let items: Vec<ListItem> = if files.is_empty() {
        vec![ListItem::new(Span::styled(
            "  (nothing here — nice and clean)",
            Style::default().fg(MUTED),
        ))]
    } else {
        files
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let state = entry.staged.as_ref().or(entry.unstaged.as_ref());
                let (glyph, style) = match state {
                    Some(s) => (s.glyph(), state_style(s)),
                    None => ("?", Style::default()),
                };
                let selected = focused && i == app.status_selected;
                let prefix = if selected { "▶ " } else { "  " };
                let line_style = if selected { selected_style(style) } else { style };
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(format!("{} ", glyph), style.add_modifier(Modifier::BOLD)),
                    Span::styled(entry.path.clone(), line_style),
                ]))
            })
            .collect()
    };
    f.render_widget(List::new(items), inner);
}

fn draw_branches(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Panel::Branches;
    let block = panel_block("Branches", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .branches
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let selected = focused && i == app.branch_selected;
            let marker = if b.is_current { "● " } else { "  " };
            let style = if b.is_current {
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
            } else if b.is_remote {
                Style::default().fg(MUTED)
            } else {
                Style::default().fg(TEXT)
            };
            let style = if selected { selected_style(style) } else { style };
            ListItem::new(Line::from(vec![Span::raw(marker), Span::styled(b.name.clone(), style)]))
        })
        .collect();
    f.render_widget(List::new(items), inner);
}

fn draw_graph(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Panel::Graph;
    let block = panel_block("Commit Graph", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let height = inner.height as usize;
    let total = app.graph.len();
    let selected = app.graph_selected;
    let start = if selected >= height { selected + 1 - height.max(1) } else { 0 };
    let end = (start + height).min(total);

    let max_lane_glyphs = 5usize; // cap graph rail width so commit text stays readable

    let lines: Vec<Line> = app.graph[start..end]
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            let idx = start + offset;
            let mut spans = Vec::new();

            for c in 0..max_lane_glyphs.min(row.active_lane_count.max(row.lane + 1)) {
                if c == row.lane {
                    let glyph = if row.is_merge { "◆" } else { "●" };
                    spans.push(Span::styled(glyph, Style::default().fg(lane_color(c)).add_modifier(Modifier::BOLD)));
                } else if row.parent_lanes.contains(&c) || row.active_before.get(c).copied().unwrap_or(false) {
                    spans.push(Span::styled("│", Style::default().fg(lane_color(c))));
                } else {
                    spans.push(Span::raw(" "));
                }
            }
            spans.push(Span::raw(" "));

            let hash_style = Style::default().fg(MUTED);
            spans.push(Span::styled(row.commit.short_hash.clone(), hash_style));
            spans.push(Span::raw(" "));

            if !row.commit.refs.is_empty() {
                let refs = row.commit.refs.join(", ");
                spans.push(Span::styled(format!("({refs}) "), Style::default().fg(ACCENT_SOFT)));
            }

            spans.push(Span::styled(row.commit.subject.clone(), Style::default().fg(TEXT)));

            let line = Line::from(spans);
            if focused && idx == selected {
                Line::from(
                    line.spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, selected_style(s.style)))
                        .collect::<Vec<_>>(),
                )
            } else {
                line
            }
        })
        .collect();

    f.render_widget(Paragraph::new(lines), inner);
}

fn diff_lines(text: &str) -> Vec<Line<'static>> {
    text.lines()
        .map(|l| {
            let style = if l.starts_with('+') && !l.starts_with("+++") {
                Style::default().fg(SUCCESS)
            } else if l.starts_with('-') && !l.starts_with("---") {
                Style::default().fg(DANGER)
            } else if l.starts_with("@@") {
                Style::default().fg(Color::Rgb(120, 170, 210))
            } else if l.starts_with("diff ") || l.starts_with("index ") {
                Style::default().fg(MUTED)
            } else {
                Style::default().fg(TEXT)
            };
            Line::from(Span::styled(l.to_string(), style))
        })
        .collect()
}

fn draw_diff(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Panel::Diff;
    let title = if app.diff_title.is_empty() {
        "Diff".to_string()
    } else {
        format!("Diff · {}", app.diff_title)
    };
    let block = panel_block(&title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = diff_lines(&app.diff_text);
    let paragraph = Paragraph::new(lines).scroll((app.diff_scroll, 0));
    f.render_widget(paragraph, inner);
}

fn draw_status_line(f: &mut Frame, _app: &App, area: Rect) {
    let hint = " Tab focus · j/k move · h/l section · Enter view/checkout · Esc closes any popup · q quit ";
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(MUTED))),
        area,
    );
}

/// Scales a popup's size by the modal grow-in animation so it expands
/// from ~55% up to its target size instead of snapping into view.
fn animated_centered_rect(percent_x: u16, percent_y: u16, anim: u8, r: Rect) -> Rect {
    let t = anim.min(MODAL_ANIM_MAX) as f32 / MODAL_ANIM_MAX as f32;
    let scale = 0.55 + 0.45 * t;
    let x = ((percent_x as f32) * scale).round() as u16;
    let y = ((percent_y as f32) * scale).round() as u16;
    centered_rect(x.clamp(1, 100), y.clamp(1, 100), r)
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_palette(f: &mut Frame, app: &App) {
    let area = animated_centered_rect(60, 60, app.modal_anim, f.size());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" Command palette  (Esc to close) ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 3 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(3)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(ACCENT)),
            Span::raw(app.palette_query.clone()),
            Span::styled("│", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ])),
        chunks[0],
    );

    let items: Vec<ListItem> = app
        .palette_filtered
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let label = app.palette_label(i);
            let selected = i == app.palette_selected;
            let style = if selected {
                selected_style(Style::default().fg(TEXT))
            } else {
                Style::default().fg(TEXT)
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();
    f.render_widget(List::new(items), chunks[2]);
}

fn draw_text_popup(f: &mut Frame, app: &App, title: &str, placeholder: &str) {
    let area = animated_centered_rect(56, 22, app.modal_anim, f.size());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(format!(" {} · Enter to confirm · Esc to cancel ", title))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let text: Line = if app.input_buffer.is_empty() {
        Line::from(Span::styled(placeholder.to_string(), Style::default().fg(MUTED)))
    } else {
        Line::from(vec![
            Span::styled(app.input_buffer.clone(), Style::default().fg(TEXT)),
            Span::styled("│", Style::default().fg(ACCENT).add_modifier(Modifier::SLOW_BLINK)),
        ])
    };
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}

fn draw_confirm(f: &mut Frame, app: &App, path: &str) {
    let area = animated_centered_rect(50, 24, app.modal_anim, f.size());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" Discard changes? ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DANGER));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(format!(
            "Discard all changes to {}?\n\n(y) yes    (Esc or any other key) cancel",
            path
        ))
        .wrap(Wrap { trim: true }),
        inner,
    );
}

fn draw_help(f: &mut Frame, app: &App) {
    let area = animated_centered_rect(62, 72, app.modal_anim, f.size());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" Help · Esc or any key to close ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let text = "\
Quick actions (always visible in the toolbar)
  U  pull            P  push            c  commit
  g  clone           s  stage/unstage   F  fetch --all

Navigation
  Tab / Shift+Tab   cycle focused panel
  j/k, ↑/↓          move selection / scroll diff
  h/l               switch status section (Unstaged/Staged/Untracked)
  Enter             view diff (files/commits) or checkout branch

Files
  d                 discard changes to selected file (asks to confirm)
  n                 create a new branch

Other
  Ctrl+K or :       open the command palette (fuzzy search)
  r                 refresh
  ?                 toggle this help
  q                 quit
";
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}
