//! rex-tabs: the terminal UI of the Rex `cmd+e` tab picker.
//!
//! Like Axel's launcher, the popup is two Rex panes (a prompt on top, a list below), so Rex draws
//! the rounded borders and title bars itself. This one program runs in both panes:
//!
//!   rex-tabs prompt LIST_JSON RESULT_FILE STATE_FILE [--start N] [--title TEXT]
//!   rex-tabs list   LIST_JSON STATE_FILE
//!
//! `prompt` has the keyboard focus: it filters, moves the highlight, writes the highlighted tab to
//! RESULT_FILE (three lines: tab number, session id, window id) and its view of the world to
//! STATE_FILE. `list` only draws STATE_FILE. Neither switches tabs: Rex key bindings (Enter / Esc /
//! cmd+e in the `tabpick` mode) read RESULT_FILE and do that, which is why no Remote Control is
//! needed.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{env, fs, process};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use serde::{Deserialize, Serialize};

// ---- palette (follows Axel's launcher: dark, warm headings, muted greys) -----------------------
const ACCENT: Color = Color::Rgb(0xe0, 0x6c, 0x75);
const HEADING: Color = Color::Rgb(0xe0, 0xaf, 0x68);
const TEXT: Color = Color::Rgb(0xc0, 0xca, 0xf5);
const MUTED: Color = Color::Rgb(0x56, 0x5f, 0x89);
const SELECTED_BG: Color = Color::Rgb(0x29, 0x2e, 0x42);
const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
const SESSION_COLORS: [Color; 5] = [
    Color::Rgb(0xbb, 0x9a, 0xf7),
    Color::Rgb(0xe0, 0xaf, 0x68),
    Color::Rgb(0x7a, 0xa2, 0xf7),
    Color::Rgb(0x7d, 0xcf, 0xff),
    Color::Rgb(0xf7, 0x76, 0x8e),
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

#[derive(Serialize, Deserialize, Clone, Default)]
struct HitS {
    row: usize,
    /// char positions inside the tab name that matched the query
    hl: Vec<u32>,
}

#[derive(Serialize, Deserialize, Default)]
struct State {
    query: String,
    /// position in `hits` of the highlighted tab
    selected: usize,
    hits: Vec<HitS>,
}

// ---- model -------------------------------------------------------------------------------------

struct Model {
    rows: Vec<Row>,
    query: String,
    hits: Vec<(HitS, u32)>, // grouped by session; the u32 is the match score
    selected: usize,
    matcher: Matcher,
}

impl Model {
    fn new(rows: Vec<Row>, start: usize) -> Self {
        let mut m = Model {
            rows,
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        m.refilter();
        m.selected = m.hits.iter().position(|(h, _)| h.row == start).unwrap_or(0);
        m
    }

    fn refilter(&mut self) {
        let mut hits: Vec<(HitS, u32)> = Vec::new();
        if self.query.trim().is_empty() {
            hits = (0..self.rows.len()).map(|row| (HitS { row, hl: vec![] }, 0)).collect();
        } else {
            let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
            let multi_word = self.query.trim().contains(char::is_whitespace);
            let mut buf = Vec::new();
            for (i, r) in self.rows.iter().enumerate() {
                let mut best: Option<(u32, Vec<u32>)> = None;
                if multi_word {
                    // several words may match different fields: search them joined
                    let hay = format!("{} {} {}", r.name, r.dir, r.session);
                    let mut idx = Vec::new();
                    if let Some(sc) =
                        pattern.indices(Utf32Str::new(&hay, &mut buf), &mut self.matcher, &mut idx)
                    {
                        let name_len = r.name.chars().count() as u32;
                        idx.retain(|&p| p < name_len);
                        best = Some((sc, idx));
                    }
                } else {
                    // one word: it must match inside a single field, not be scattered over all
                    for (field, bonus, is_name) in
                        [(&r.name, 20u32, true), (&r.dir, 0, false), (&r.session, 0, false)]
                    {
                        let mut idx = Vec::new();
                        if let Some(sc) = pattern.indices(
                            Utf32Str::new(field, &mut buf),
                            &mut self.matcher,
                            &mut idx,
                        ) {
                            let sc = sc + bonus;
                            if best.as_ref().map_or(true, |(b, _)| sc > *b) {
                                best = Some((sc, if is_name { idx } else { vec![] }));
                            }
                        }
                    }
                }
                if let Some((score, hl)) = best {
                    hits.push((HitS { row: i, hl }, score));
                }
            }
            hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.row.cmp(&b.0.row)));
        }
        // group by session: sessions in order of their best hit, tabs inside keep their order
        let mut order: Vec<&str> = Vec::new();
        for (h, _) in &hits {
            let sid = self.rows[h.row].sid.as_str();
            if !order.contains(&sid) {
                order.push(sid);
            }
        }
        let mut grouped = Vec::with_capacity(hits.len());
        for sid in order {
            for (h, sc) in &hits {
                if self.rows[h.row].sid == sid {
                    grouped.push((h.clone(), *sc));
                }
            }
        }
        self.hits = grouped;
        self.selected = 0;
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.hits.len();
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        }
    }

    fn jump(&mut self, to: usize) {
        if !self.hits.is_empty() {
            self.selected = to.min(self.hits.len() - 1);
        }
    }

    fn state(&self) -> State {
        State {
            query: self.query.clone(),
            selected: self.selected,
            hits: self.hits.iter().map(|(h, _)| h.clone()).collect(),
        }
    }
}

fn write_atomic(path: &Path, body: &str) {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// the prompt's side effects: the highlighted tab for Rex, and the view for the list pane
fn publish(m: &Model, result: &Path, state: &Path) {
    let body = match m.hits.get(m.selected) {
        Some((h, _)) => {
            let r = &m.rows[h.row];
            format!("{}\n{}\n{}\n", r.n, r.sid, r.wid)
        }
        None => String::new(), // nothing highlighted: Enter must do nothing
    };
    write_atomic(result, &body);
    write_atomic(state, &serde_json::to_string(&m.state()).unwrap_or_default());
}

// ---- helpers -----------------------------------------------------------------------------------

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

fn put(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) -> u16 {
    let (nx, _) = buf.set_stringn(x, y, text, usize::MAX, style);
    nx
}

fn set_title(title: &str) {
    // Rex draws the pane's title bar from the terminal title
    let clean: String = title.chars().filter(|c| !c.is_control()).collect();
    print!("\x1b]0;{clean}\x07");
    let _ = io::stdout().flush();
}

// ---- prompt pane -------------------------------------------------------------------------------

fn draw_prompt(f: &mut Frame, m: &Model) {
    let area = f.area();
    let buf = f.buffer_mut();
    if area.width < 12 || area.height < 1 {
        return;
    }
    // vertically centred, like Axel's prompt
    let y = area.y + area.height.saturating_sub(1) / 2;
    let left = area.x + 2;
    let right = area.x + area.width - 2;
    let mut x = put(buf, left, y, "› ", Style::new().fg(MUTED));
    if m.query.is_empty() {
        x = put(buf, x, y, "▏", Style::new().fg(TEXT));
        put(buf, x, y, "type to filter tabs", Style::new().fg(MUTED));
    } else {
        x = put(buf, x, y, &m.query, Style::new().fg(TEXT));
        put(buf, x, y, "▏", Style::new().fg(TEXT));
    }
    if let Some(cur) = m.rows.iter().find(|r| r.current) {
        let name = truncate(&cur.name, 30);
        let w = width(&cur.session) + 3 + width(&name);
        let rx = right.saturating_sub(w as u16);
        let mut x = put(buf, rx, y, &cur.session, Style::new().fg(session_color(&cur.session)));
        x = put(buf, x, y, " · ", Style::new().fg(MUTED));
        put(buf, x, y, &name, Style::new().fg(TEXT));
    }
}

fn run_prompt(rows: Vec<Row>, result: PathBuf, state: PathBuf, start: usize, title: String) {
    let mut m = Model::new(rows, start);
    publish(&m, &result, &state);
    if !title.is_empty() {
        set_title(&title); // keeps the tab's own name unchanged while the popup is open
    }
    let mut terminal = ratatui::init();
    loop {
        if terminal.draw(|f| draw_prompt(f, &m)).is_err() {
            break;
        }
        let Ok(ev) = event::read() else { break };
        let Event::Key(KeyEvent { code, modifiers, kind, .. }) = ev else { continue };
        if kind == KeyEventKind::Release {
            continue;
        }
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let mut changed = true;
        match (code, ctrl) {
            // Enter / Esc are normally handled by Rex; if they reach us, just stop
            (KeyCode::Esc, _) | (KeyCode::Enter, _) => break,
            (KeyCode::Char('c'), true) | (KeyCode::Char('g'), true) => break,
            (KeyCode::Down, _) | (KeyCode::Char('n'), true) | (KeyCode::Char('j'), true) => {
                m.move_by(1)
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::Char('k'), true) => {
                m.move_by(-1)
            }
            (KeyCode::PageDown, _) => m.move_by(8),
            (KeyCode::PageUp, _) => m.move_by(-8),
            (KeyCode::Home, _) => m.jump(0),
            (KeyCode::End, _) => m.jump(usize::MAX),
            (KeyCode::Backspace, _) => {
                m.query.pop();
                m.refilter();
            }
            (KeyCode::Char('u'), true) => {
                m.query.clear();
                m.refilter();
            }
            (KeyCode::Char('w'), true) => {
                let t = m.query.trim_end().len();
                let cut = m.query[..t].rfind(' ').map(|i| i + 1).unwrap_or(0);
                m.query.truncate(cut);
                m.refilter();
            }
            (KeyCode::Char(c), false) => {
                m.query.push(c);
                m.refilter();
            }
            _ => changed = false,
        }
        if changed {
            publish(&m, &result, &state);
        }
    }
    ratatui::restore();
}

// ---- list pane ---------------------------------------------------------------------------------

enum Item {
    Header(usize), // index into hits of the group's first tab
    Tab(usize),    // index into hits
}

fn draw_list(f: &mut Frame, rows: &[Row], st: &State, scroll: &mut usize) {
    let area = f.area();
    let buf = f.buffer_mut();
    if area.width < 24 || area.height < 3 {
        return;
    }
    let left = area.x + 2;
    let right = area.x + area.width - 2; // exclusive
    let w = (right - left) as usize;

    // heading: "tabs  recent ........ N"
    let hy = area.y + 1;
    let hs = Style::new();
    let x = put(buf, left, hy, "tabs", hs.fg(HEADING).add_modifier(Modifier::BOLD));
    put(buf, x + 2, hy, "recent", hs.fg(MUTED));
    let count = st.hits.len().to_string();
    put(buf, right.saturating_sub(width(&count) as u16), hy, &count, hs.fg(MUTED));

    // outline: a session header, then its tabs
    let mut items: Vec<Item> = Vec::new();
    let mut sel_item = 0usize;
    let mut last: Option<&str> = None;
    for (i, h) in st.hits.iter().enumerate() {
        let Some(r) = rows.get(h.row) else { continue };
        if last != Some(r.sid.as_str()) {
            items.push(Item::Header(i));
            last = Some(r.sid.as_str());
        }
        if i == st.selected {
            sel_item = items.len();
        }
        items.push(Item::Tab(i));
    }

    let rows_y = hy + 2;
    let visible = (area.y + area.height).saturating_sub(rows_y + 1) as usize;
    if visible == 0 {
        return;
    }
    if st.hits.is_empty() {
        put(buf, left, rows_y, "no matching tabs", hs.fg(MUTED));
        return;
    }
    if sel_item < *scroll {
        *scroll = sel_item;
        if *scroll > 0 && matches!(items[*scroll - 1], Item::Header(_)) {
            *scroll -= 1; // keep a group's header visible with its first tab
        }
    } else if sel_item >= *scroll + visible {
        *scroll = sel_item + 1 - visible;
    }

    let right_w = 8usize;
    for (line, item) in items.iter().enumerate().skip(*scroll).take(visible) {
        let y = rows_y + (line - *scroll) as u16;
        match item {
            Item::Header(hi) => {
                let r = &rows[st.hits[*hi].row];
                let n = st
                    .hits
                    .iter()
                    .filter(|h| rows.get(h.row).map_or(false, |x| x.sid == r.sid))
                    .count();
                let label = format!("{} {}", n, if n == 1 { "tab" } else { "tabs" });
                let name = truncate(&r.session, w.saturating_sub(label.len() + 4).max(8));
                put(
                    buf,
                    left,
                    y,
                    &name,
                    hs.fg(session_color(&r.session)).add_modifier(Modifier::BOLD),
                );
                put(buf, right.saturating_sub(width(&label) as u16), y, &label, hs.fg(MUTED));
            }
            Item::Tab(hi) => {
                let hit = &st.hits[*hi];
                let r = &rows[hit.row];
                let selected = *hi == st.selected;
                let base = if selected { Style::new().bg(SELECTED_BG) } else { Style::new() };
                if selected {
                    put(buf, left - 1, y, "▎", Style::new().fg(ACCENT).bg(SELECTED_BG));
                    for xx in left..right + 1 {
                        put(buf, xx, y, " ", base);
                    }
                }
                let mut x = left + 1;
                if r.current {
                    put(buf, x, y, "●", base.fg(GREEN));
                }
                x += 3;
                let right_start = right.saturating_sub(right_w as u16);
                let avail = right_start.saturating_sub(x + 2) as usize;
                let title = truncate(&r.name, avail);
                let hl: HashSet<u32> = hit.hl.iter().copied().collect();
                for (i, ch) in title.chars().enumerate() {
                    let style = if hl.contains(&(i as u32)) {
                        base.fg(HEADING).add_modifier(Modifier::BOLD)
                    } else {
                        base.fg(TEXT).add_modifier(Modifier::BOLD)
                    };
                    x = put(buf, x, y, &ch.to_string(), style);
                }
                if !r.dir.is_empty() && r.dir != r.name {
                    let room = right_start.saturating_sub(x + 3) as usize;
                    if room >= 6 {
                        put(buf, x + 2, y, &truncate(&r.dir, room), base.fg(MUTED));
                    }
                }
                let (text, col) = if r.current {
                    ("Current".to_string(), GREEN)
                } else {
                    (ago_text(r.ago), MUTED)
                };
                put(buf, right_start, y, &format!("{:>w$}", text, w = right_w), base.fg(col));
            }
        }
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn run_list(rows: Vec<Row>, state_path: PathBuf) {
    set_title("tabs");
    let mut terminal = ratatui::init();
    let mut st = State::default();
    let mut seen: Option<SystemTime> = None;
    let mut scroll = 0usize;
    let mut dirty = true;
    loop {
        let t = mtime(&state_path);
        if t != seen {
            if let Some(s) = fs::read_to_string(&state_path)
                .ok()
                .and_then(|s| serde_json::from_str::<State>(&s).ok())
            {
                st = s;
                seen = t;
                dirty = true;
            }
        }
        if dirty {
            if terminal.draw(|f| draw_list(f, &rows, &st, &mut scroll)).is_err() {
                break;
            }
            dirty = false;
        }
        // wait a little for a key / resize, then look at the state file again
        match event::poll(Duration::from_millis(25)) {
            Ok(true) => match event::read() {
                Ok(Event::Resize(..)) => dirty = true,
                Ok(Event::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, .. }))
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    break
                }
                Ok(_) => {}
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
    }
    ratatui::restore();
}

// ---- main --------------------------------------------------------------------------------------

fn usage() -> ! {
    eprintln!("usage: rex-tabs prompt LIST RESULT STATE [--start N] [--title TEXT]");
    eprintln!("       rex-tabs list   LIST STATE");
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| usage());
    let list = args.next().unwrap_or_else(|| usage());
    let rows: Vec<Row> = match fs::read_to_string(&list)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("rex-tabs: cannot read {list}: {e}");
            process::exit(1);
        }
    };
    match mode.as_str() {
        "prompt" => {
            let result = PathBuf::from(args.next().unwrap_or_else(|| usage()));
            let state = PathBuf::from(args.next().unwrap_or_else(|| usage()));
            let mut start = 0usize;
            let mut title = String::new();
            while let Some(a) = args.next() {
                match a.as_str() {
                    "--start" => start = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
                    "--title" => title = args.next().unwrap_or_default(),
                    _ => usage(),
                }
            }
            run_prompt(rows, result, state, start, title);
        }
        "list" => {
            let state = PathBuf::from(args.next().unwrap_or_else(|| usage()));
            run_list(rows, state);
        }
        _ => usage(),
    }
}
