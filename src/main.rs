//! rex-tabs: the terminal UI of the Rex `cmd+e` tab picker.
//!
//!   rex-tabs resident LIST_JSON RESULT_FILE
//!
//! The program is *resident*: it keeps running while the popup is hidden (Rex parks its block as
//! "detached") and is put on screen again in a few milliseconds, instead of starting a new
//! terminal every time. Each time the plugin rewrites LIST_JSON the program reloads it: new rows,
//! empty query, highlight on the preselected tab.
//!
//! LIST_JSON: { "title": "...", "start": N, "rows": [ {wid, sid, n, session, name, dir, current} ] }
//!
//! The highlighted tab is written to RESULT_FILE after every change (three lines: tab number,
//! session id, window id). The program never switches tabs: Rex key bindings (Enter / Esc / cmd+e
//! in the `tabpick` mode) read RESULT_FILE and do that, which is why no Remote Control is needed.

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{env, fs, process};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Position;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use serde::Deserialize;

// ---- palette: quiet, follows the terminal's own text colour ------------------------------------
const MUTED: Color = Color::Rgb(0x6b, 0x72, 0x93);
const SEPARATOR: Color = Color::Rgb(0x2f, 0x34, 0x4d);
const SELECTED_BG: Color = Color::Rgb(0x28, 0x2d, 0x45);
const MATCH: Color = Color::Rgb(0xe0, 0xaf, 0x68);
const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);

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
}

#[derive(Deserialize)]
struct Doc {
    #[serde(default)]
    title: String,
    #[serde(default)]
    start: usize,
    rows: Vec<Row>,
}

#[derive(Clone, Default)]
struct Hit {
    row: usize,
    /// char positions inside the tab name that matched the query
    hl: Vec<u32>,
}

// ---- model -------------------------------------------------------------------------------------

struct Model {
    rows: Vec<Row>,
    query: String,
    hits: Vec<(Hit, u32)>, // grouped by session; the u32 is the match score
    selected: usize,
    scroll: usize,
    /// true after the arrow keys moved the highlight: the search bar then shows that tab's name
    preview: bool,
    matcher: Matcher,
}

impl Model {
    fn new(rows: Vec<Row>, start: usize) -> Self {
        let mut m = Model {
            rows,
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
            scroll: 0,
            preview: false,
            matcher: Matcher::new(Config::DEFAULT),
        };
        m.refilter();
        m.selected = m.hits.iter().position(|(h, _)| h.row == start).unwrap_or(0);
        m
    }

    fn refilter(&mut self) {
        let mut hits: Vec<(Hit, u32)> = Vec::new();
        if self.query.trim().is_empty() {
            hits = (0..self.rows.len()).map(|row| (Hit { row, hl: vec![] }, 0)).collect();
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
                    hits.push((Hit { row: i, hl }, score));
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
        self.scroll = 0;
        self.preview = false; // typing always goes back to showing the query
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.hits.len();
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
            self.preview = true;
        }
    }

    fn jump(&mut self, to: usize) {
        if !self.hits.is_empty() {
            self.selected = to.min(self.hits.len() - 1);
            self.preview = true;
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

/// tell Rex which tab is highlighted
fn publish(m: &Model, result: &Path) {
    let body = match m.hits.get(m.selected) {
        Some((h, _)) => {
            let r = &m.rows[h.row];
            format!("{}\n{}\n{}\n", r.n, r.sid, r.wid)
        }
        None => String::new(), // nothing highlighted: Enter must do nothing
    };
    write_atomic(result, &body);
}

// ---- helpers -----------------------------------------------------------------------------------

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
    // Rex draws the pane's title bar from the terminal title, and names the tab after it
    let clean: String = title.chars().filter(|c| !c.is_control()).collect();
    print!("\x1b]0;{clean}\x07");
    let _ = io::stdout().flush();
}

// ---- drawing: deliberately quiet ---------------------------------------------------------------

enum Item {
    Header(usize), // index into hits of the group's first tab
    Tab(usize),    // index into hits
    Gap,           // one blank row between two sessions
}

fn draw(f: &mut Frame, m: &mut Model) {
    let area = f.area();
    if area.width < 16 || area.height < 4 {
        return;
    }
    // spacing: text sits 3 cells in from the pane edge; the highlight and the separator run 1 cell
    // in from it, so the highlighted row has 2 cells of air around its text
    let left = area.x + 3;
    let right = area.x + area.width - 3; // exclusive
    let (edge_l, edge_r) = (area.x + 1, area.x + area.width - 1);
    let top = area.y + 1;

    // -- filter line: just text, the terminal's own cursor marks the insertion point -------------
    let (cursor_x, cursor_y);
    {
        let buf = f.buffer_mut();
        let shown = if m.preview {
            // arrows moved the highlight: show which tab it is on
            m.hits.get(m.selected).map(|(h, _)| m.rows[h.row].name.clone())
        } else {
            None
        };
        if let Some(name) = shown {
            cursor_x = put(buf, left, top, &truncate(&name, (right - left) as usize), Style::new());
        } else if m.query.is_empty() {
            put(buf, left, top, "Search tabs", Style::new().fg(MUTED));
            cursor_x = left;
        } else {
            cursor_x = put(buf, left, top, &truncate(&m.query, (right - left) as usize), Style::new());
        }
        cursor_y = top;

        // -- thin separator -------------------------------------------------------------------------
        for x in edge_l..edge_r {
            put(buf, x, top + 1, "─", Style::new().fg(SEPARATOR));
        }
    }
    f.set_cursor_position(Position::new(cursor_x, cursor_y));
    let buf = f.buffer_mut();

    // -- outline: a quiet session header, then its tabs ------------------------------------------
    let mut items: Vec<Item> = Vec::new();
    let mut sel_item = 0usize;
    let mut last: Option<&str> = None;
    for (i, (h, _)) in m.hits.iter().enumerate() {
        let Some(r) = m.rows.get(h.row) else { continue };
        if last != Some(r.sid.as_str()) {
            if !items.is_empty() {
                items.push(Item::Gap);
            }
            items.push(Item::Header(i));
            last = Some(r.sid.as_str());
        }
        if i == m.selected {
            sel_item = items.len();
        }
        items.push(Item::Tab(i));
    }

    let rows_y = top + 3;
    let visible = (area.y + area.height).saturating_sub(rows_y + 1) as usize;
    if visible == 0 {
        return;
    }
    if m.hits.is_empty() {
        put(buf, left, rows_y, "No matching tabs", Style::new().fg(MUTED));
        return;
    }
    if sel_item < m.scroll {
        m.scroll = sel_item;
        if m.scroll > 0 && matches!(items[m.scroll - 1], Item::Header(_)) {
            m.scroll -= 1; // keep a group's header visible with its first tab
        }
        if m.scroll > 0 && matches!(items[m.scroll], Item::Header(_)) && matches!(items[m.scroll - 1], Item::Gap) {
            m.scroll -= 1; // and the blank row above it, if it fits
        }
    } else if sel_item >= m.scroll + visible {
        m.scroll = sel_item + 1 - visible;
    }

    for (line, item) in items.iter().enumerate().skip(m.scroll).take(visible) {
        let y = rows_y + (line - m.scroll) as u16;
        match item {
            Item::Gap => {}
            Item::Header(hi) => {
                let r = &m.rows[m.hits[*hi].0.row];
                let name = truncate(&r.session, (right - left) as usize);
                put(buf, left, y, &name, Style::new().fg(MUTED));
            }
            Item::Tab(hi) => {
                let (hit, _) = &m.hits[*hi];
                let r = &m.rows[hit.row];
                let selected = *hi == m.selected;
                let base = if selected { Style::new().bg(SELECTED_BG) } else { Style::new() };
                if selected {
                    for xx in edge_l..edge_r {
                        put(buf, xx, y, " ", base);
                    }
                }
                let mut x = left + 2;
                if r.current {
                    put(buf, left, y, "●", base.fg(GREEN));
                }
                let title = truncate(&r.name, right.saturating_sub(x) as usize);
                let hl: HashSet<u32> = hit.hl.iter().copied().collect();
                for (i, ch) in title.chars().enumerate() {
                    let style = if hl.contains(&(i as u32)) {
                        base.fg(MATCH).add_modifier(Modifier::BOLD)
                    } else if selected {
                        base.add_modifier(Modifier::BOLD)
                    } else {
                        base
                    };
                    x = put(buf, x, y, &ch.to_string(), style);
                }
            }
        }
    }
}

// ---- main loop ---------------------------------------------------------------------------------

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn load(path: &Path) -> Option<Doc> {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn run(list: PathBuf, result: PathBuf) {
    let mut terminal = ratatui::init();
    let mut model = Model::new(Vec::new(), 0);
    let mut seen: Option<SystemTime> = None;
    let mut dirty = true;
    let mut repaint = false;
    loop {
        // the plugin rewrites the list file every time the popup is shown
        let t = mtime(&list);
        if t != seen {
            if let Some(doc) = load(&list) {
                model = Model::new(doc.rows, doc.start);
                if !doc.title.is_empty() {
                    set_title(&doc.title); // keeps the tab's own name unchanged while shown
                }
                publish(&model, &result);
                seen = t;
                dirty = true;
                repaint = true;
            }
        }
        if repaint {
            // the screen may have been wiped while the block was parked or re-placed: draw it all
            let _ = terminal.clear();
            repaint = false;
            dirty = true;
        }
        if dirty {
            if terminal.draw(|f| draw(f, &mut model)).is_err() {
                break;
            }
            dirty = false;
        }
        match event::poll(Duration::from_millis(12)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => break,
        }
        let Ok(ev) = event::read() else { break };
        match ev {
            Event::Resize(..) => {
                dirty = true;
                repaint = true;
            }
            Event::Key(KeyEvent { code, modifiers, kind, .. }) if kind != KeyEventKind::Release => {
                let ctrl = modifiers.contains(KeyModifiers::CONTROL);
                let mut changed = true;
                match (code, ctrl) {
                    // Enter / Esc are handled by Rex's key mode; resident means we never quit on them
                    (KeyCode::Esc, _) | (KeyCode::Enter, _) => changed = false,
                    (KeyCode::Char('c'), true) => break,
                    (KeyCode::Down, _) | (KeyCode::Char('n'), true) | (KeyCode::Char('j'), true) => {
                        model.move_by(1)
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('p'), true) | (KeyCode::Char('k'), true) => {
                        model.move_by(-1)
                    }
                    (KeyCode::PageDown, _) => model.move_by(8),
                    (KeyCode::PageUp, _) => model.move_by(-8),
                    (KeyCode::Home, _) => model.jump(0),
                    (KeyCode::End, _) => model.jump(usize::MAX),
                    (KeyCode::Backspace, _) => {
                        model.query.pop();
                        model.refilter();
                    }
                    (KeyCode::Char('u'), true) => {
                        model.query.clear();
                        model.refilter();
                    }
                    (KeyCode::Char('w'), true) => {
                        let t = model.query.trim_end().len();
                        let cut = model.query[..t].rfind(' ').map(|i| i + 1).unwrap_or(0);
                        model.query.truncate(cut);
                        model.refilter();
                    }
                    (KeyCode::Char(c), false) => {
                        model.query.push(c);
                        model.refilter();
                    }
                    _ => changed = false,
                }
                if changed {
                    publish(&model, &result);
                    dirty = true;
                }
            }
            _ => {}
        }
    }
    ratatui::restore();
}

fn usage() -> ! {
    eprintln!("usage: rex-tabs resident LIST_JSON RESULT_FILE");
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("resident") {
        usage();
    }
    let list = PathBuf::from(args.next().unwrap_or_else(|| usage()));
    let result = PathBuf::from(args.next().unwrap_or_else(|| usage()));
    run(list, result);
}
