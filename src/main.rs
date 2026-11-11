//! rex-tabs: the terminal UI of the Rex `cmd+e` tab picker.
//!
//! It only draws, filters and moves the highlight. It never switches tabs itself: the
//! highlighted tab is written to RESULT_FILE (three lines: tab number, session id, window id)
//! after every change, and Rex key bindings (Enter / Esc / cmd+e in the `tabpick` mode) read it
//! and do the switching. That is what lets the picker work without Remote Control.
//!
//! usage: rex-tabs LIST_JSON RESULT_FILE [--start N] [--title TEXT]

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::{env, fs, process};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use serde::Deserialize;

// ---- palette (Tokyo Night-ish, close to Axel's launcher) -------------------------------------
const ACCENT: Color = Color::Rgb(0xe0, 0x6c, 0x75); // prompt border
const HEADING: Color = Color::Rgb(0xe0, 0xaf, 0x68); // "tabs" heading, matched letters
const TEXT: Color = Color::Rgb(0xc0, 0xca, 0xf5);
const MUTED: Color = Color::Rgb(0x56, 0x5f, 0x89);
const DIM_BORDER: Color = Color::Rgb(0x3b, 0x42, 0x61);
const SELECTED_BG: Color = Color::Rgb(0x29, 0x2e, 0x42);
const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
const SESSION_COLORS: [Color; 5] = [
    Color::Rgb(0xbb, 0x9a, 0xf7), // purple
    Color::Rgb(0xe0, 0xaf, 0x68), // orange
    Color::Rgb(0x7a, 0xa2, 0xf7), // blue
    Color::Rgb(0x7d, 0xcf, 0xff), // cyan
    Color::Rgb(0xf7, 0x76, 0x8e), // pink
];

#[derive(Deserialize, Clone)]
struct Row {
    wid: String,
    sid: String,
    n: u32,
    session: String,
    name: String,
    #[serde(default)]
    dir: String,
    #[serde(default)]
    current: bool,
    /// seconds since this tab was last used through the picker
    #[serde(default)]
    ago: Option<i64>,
}

struct Hit {
    row: usize,
    /// char positions inside `name` that matched the query
    hl: Vec<u32>,
    score: u32,
}

struct App {
    rows: Vec<Row>,
    query: String,
    hits: Vec<Hit>,
    selected: usize,
    scroll: usize,
    result: PathBuf,
    matcher: Matcher,
    list_area: Rect,
    list_first_row_y: u16,
}

impl App {
    fn new(rows: Vec<Row>, result: PathBuf, start: usize) -> Self {
        let mut app = App {
            rows,
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
            scroll: 0,
            result,
            matcher: Matcher::new(Config::DEFAULT),
            list_area: Rect::default(),
            list_first_row_y: 0,
        };
        app.refilter();
        app.selected = start.min(app.hits.len().saturating_sub(1));
        app.write_result();
        app
    }

    fn refilter(&mut self) {
        self.hits.clear();
        if self.query.trim().is_empty() {
            self.hits = (0..self.rows.len())
                .map(|row| Hit { row, hl: vec![], score: 0 })
                .collect();
        } else {
            let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
            let mut buf = Vec::new();
            for (i, r) in self.rows.iter().enumerate() {
                // search name, folder and session together; highlight only the name part
                let hay = format!("{} {} {}", r.name, r.dir, r.session);
                let mut idx = Vec::new();
                if let Some(score) =
                    pattern.indices(Utf32Str::new(&hay, &mut buf), &mut self.matcher, &mut idx)
                {
                    let name_len = r.name.chars().count() as u32;
                    idx.retain(|&p| p < name_len);
                    self.hits.push(Hit { row: i, hl: idx, score });
                }
            }
            self.hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.row.cmp(&b.row)));
        }
        self.selected = 0;
        self.scroll = 0;
    }

    fn write_result(&self) {
        let body = match self.hits.get(self.selected) {
            Some(h) => {
                let r = &self.rows[h.row];
                format!("{}\n{}\n{}\n", r.n, r.sid, r.wid)
            }
            None => String::new(), // nothing highlighted: Enter must do nothing
        };
        let tmp = self.result.with_extension("tmp");
        if fs::write(&tmp, body).is_ok() {
            let _ = fs::rename(&tmp, &self.result);
        }
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.hits.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        self.write_result();
    }

    fn jump(&mut self, to: usize) {
        if self.hits.is_empty() {
            return;
        }
        self.selected = to.min(self.hits.len() - 1);
        self.write_result();
    }
}

// ---- helpers ---------------------------------------------------------------------------------

fn session_color(label: &str) -> Color {
    // FNV-1a, so a session keeps its colour between runs
    let mut h: u32 = 0x811c9dc5;
    for b in label.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    SESSION_COLORS[(h as usize) % SESSION_COLORS.len()]
}

fn ago_text(ago: Option<i64>) -> String {
    match ago {
        None => String::new(),
        Some(s) if s < 45 => "now".into(),
        Some(s) if s < 3600 => format!("{}m ago", (s / 60).max(1)),
        Some(s) if s < 86400 => format!("{}h ago", s / 3600),
        Some(s) => format!("{}d ago", s / 86400),
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn pad(s: &str, w: usize) -> String {
    let n = width(s);
    if n >= w {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(w - n))
    }
}

// ---- drawing ---------------------------------------------------------------------------------

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [prompt_area, list_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).areas(area);

    // prompt panel
    let prompt_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));
    let prompt_inner = prompt_block.inner(prompt_area);
    f.render_widget(prompt_block, prompt_area);

    let left = if app.query.is_empty() {
        Line::from(vec![
            Span::styled(" › ", Style::new().fg(MUTED)),
            Span::styled("▏", Style::new().fg(TEXT)),
            Span::styled("type to filter tabs", Style::new().fg(MUTED)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" › ", Style::new().fg(MUTED)),
            Span::styled(app.query.clone(), Style::new().fg(TEXT)),
            Span::styled("▏", Style::new().fg(TEXT)),
        ])
    };
    f.render_widget(Paragraph::new(left), prompt_inner);

    if let Some(cur) = app.rows.iter().find(|r| r.current) {
        let right = Line::from(vec![
            Span::styled(cur.session.clone(), Style::new().fg(session_color(&cur.session))),
            Span::styled(" · ", Style::new().fg(MUTED)),
            Span::styled(truncate(&cur.name, 28), Style::new().fg(TEXT)),
            Span::raw(" "),
        ]);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right), prompt_inner);
    }

    // list panel
    let list_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM_BORDER));
    let inner = list_block.inner(list_area);
    f.render_widget(list_block, list_area);
    app.list_area = inner;
    if inner.height < 2 || inner.width < 20 {
        return;
    }

    // heading row: "tabs  recent ........ N"
    let count = format!("{} ", app.hits.len());
    let heading = Line::from(vec![
        Span::styled(" tabs", Style::new().fg(HEADING).add_modifier(Modifier::BOLD)),
        Span::styled("  recent", Style::new().fg(MUTED)),
    ]);
    f.render_widget(Paragraph::new(heading), Rect { height: 1, ..inner });
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(count, Style::new().fg(MUTED))))
            .alignment(Alignment::Right),
        Rect { height: 1, ..inner },
    );

    let rows_area = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
    app.list_first_row_y = rows_area.y;
    let visible = rows_area.height as usize;

    // keep the selection on screen
    if app.selected < app.scroll {
        app.scroll = app.selected;
    } else if app.selected >= app.scroll + visible {
        app.scroll = app.selected + 1 - visible;
    }

    if app.hits.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(" no matching tabs", Style::new().fg(MUTED)))),
            rows_area,
        );
        return;
    }

    // columns
    let total = rows_area.width as usize;
    let sess_w = app.rows.iter().map(|r| width(&r.session)).max().unwrap_or(0).min(16);
    let dir_w = app.rows.iter().map(|r| width(&r.dir)).max().unwrap_or(0).min(18);
    let num_w = 3;
    let ago_w = 8;
    let wide = total >= 78;
    let leader_w = if wide { 10 } else { 0 };
    let fixed = 3 /*marker*/ + sess_w + 1 + leader_w + (if leader_w > 0 { 1 } else { 0 })
        + dir_w + 1 + num_w + 1 + ago_w + 1;
    let name_w = total.saturating_sub(fixed).max(8);

    for (line_no, hit) in app.hits.iter().enumerate().skip(app.scroll).take(visible) {
        let r = &app.rows[hit.row];
        let selected = line_no == app.selected;
        let bg = if selected { SELECTED_BG } else { Color::Reset };
        let base = Style::new().bg(bg);

        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(
            if selected { "▎" } else { " " },
            base.fg(ACCENT),
        ));
        spans.push(if r.current {
            Span::styled("● ", base.fg(GREEN))
        } else {
            Span::styled("  ", base)
        });

        // name, with matched letters highlighted
        let name = truncate(&r.name, name_w);
        let hl: std::collections::HashSet<u32> = hit.hl.iter().copied().collect();
        let name_style = base.fg(TEXT).add_modifier(Modifier::BOLD);
        for (i, ch) in name.chars().enumerate() {
            let st = if hl.contains(&(i as u32)) {
                base.fg(HEADING).add_modifier(Modifier::BOLD)
            } else {
                name_style
            };
            spans.push(Span::styled(ch.to_string(), st));
        }
        spans.push(Span::styled(" ".repeat(name_w.saturating_sub(width(&name)) + 1), base));

        spans.push(Span::styled(
            pad(&truncate(&r.session, sess_w), sess_w),
            base.fg(session_color(&r.session)),
        ));
        spans.push(Span::styled(" ", base));
        if leader_w > 0 {
            spans.push(Span::styled("·".repeat(leader_w), base.fg(DIM_BORDER)));
            spans.push(Span::styled(" ", base));
        }
        spans.push(Span::styled(pad(&truncate(&r.dir, dir_w), dir_w), base.fg(MUTED)));
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled(format!("{:>w$}", r.n, w = num_w), base.fg(MUTED)));
        spans.push(Span::styled(" ", base));
        spans.push(Span::styled(
            format!("{:>w$}", ago_text(r.ago), w = ago_w),
            base.fg(MUTED),
        ));
        spans.push(Span::styled(" ", base));

        let y = rows_area.y + (line_no - app.scroll) as u16;
        let area = Rect { x: rows_area.x, y, width: rows_area.width, height: 1 };
        f.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
    }
}

// ---- main ------------------------------------------------------------------------------------

fn usage() -> ! {
    eprintln!("usage: rex-tabs LIST_JSON RESULT_FILE [--start N] [--title TEXT]");
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let list = args.next().unwrap_or_else(|| usage());
    let result = args.next().unwrap_or_else(|| usage());
    let mut start = 0usize;
    let mut title = String::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--title" => title = args.next().unwrap_or_default(),
            _ => usage(),
        }
    }

    let rows: Vec<Row> = match fs::read_to_string(Path::new(&list))
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("rex-tabs: cannot read {list}: {e}");
            process::exit(1);
        }
    };

    let mut app = App::new(rows, PathBuf::from(&result), start);

    // keep the tab's name unchanged while the popup is open (Rex names tabs after the title)
    if !title.is_empty() {
        let clean: String = title.chars().filter(|c| !c.is_control()).collect();
        print!("\x1b]0;{clean}\x07");
        let _ = io::stdout().flush();
    }

    let mut terminal = ratatui::init(); // raw mode, alternate screen, panic hook
    let _ = execute!(io::stdout(), EnableMouseCapture);

    loop {
        if terminal.draw(|f| draw(f, &mut app)).is_err() {
            break;
        }
        let ev = match event::read() {
            Ok(e) => e,
            Err(_) => break,
        };
        match ev {
            Event::Key(KeyEvent { code, modifiers, kind, .. }) if kind != KeyEventKind::Release => {
                let ctrl = modifiers.contains(KeyModifiers::CONTROL);
                match (code, ctrl) {
                    (KeyCode::Esc, _) | (KeyCode::Enter, _) => break, // normally handled by Rex
                    (KeyCode::Char('c'), true) | (KeyCode::Char('g'), true) => break,
                    (KeyCode::Down, _) | (KeyCode::Char('n'), true) | (KeyCode::Char('j'), true) => {
                        app.move_by(1)
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::Char('k'), true) => {
                        app.move_by(-1)
                    }
                    (KeyCode::PageDown, _) => app.move_by(8),
                    (KeyCode::PageUp, _) => app.move_by(-8),
                    (KeyCode::Home, _) => app.jump(0),
                    (KeyCode::End, _) => app.jump(usize::MAX),
                    (KeyCode::Backspace, _) => {
                        app.query.pop();
                        app.refilter();
                        app.write_result();
                    }
                    (KeyCode::Char('u'), true) => {
                        app.query.clear();
                        app.refilter();
                        app.write_result();
                    }
                    (KeyCode::Char('w'), true) => {
                        let t = app.query.trim_end().len();
                        let cut = app.query[..t].rfind(' ').map(|i| i + 1).unwrap_or(0);
                        app.query.truncate(cut);
                        app.refilter();
                        app.write_result();
                    }
                    (KeyCode::Char(c), false) => {
                        app.query.push(c);
                        app.refilter();
                        app.write_result();
                    }
                    _ => {}
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollDown => app.move_by(1),
                MouseEventKind::ScrollUp => app.move_by(-1),
                MouseEventKind::Down(MouseButton::Left) => {
                    // a click only highlights; Rex's Enter binding does the switching
                    let a = app.list_area;
                    if m.row >= app.list_first_row_y
                        && m.column >= a.x
                        && m.column < a.x + a.width
                    {
                        let idx = app.scroll + (m.row - app.list_first_row_y) as usize;
                        if idx < app.hits.len() {
                            app.jump(idx);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
}
