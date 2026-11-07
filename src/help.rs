//! F1: Far's help viewer (help.cpp of Far 3.0.6446) over Far's own help
//! files (`assets/help/FarEng.hlf`, `FarRus.hlf`, in Options > Languages'
//! help language). A port, cell for cell: ReadHelp lays a topic out to the
//! window's width (`$` lines are the fixed header, a line starting with a
//! space starts a paragraph, `^<wrap>` where wrapped lines go on, `@-`/`@+`
//! formatting off and on, `@=` a line across); OutString draws `#bold#`,
//! `~links~@Topic@`, `\hh` and `\(T…:T…)` colors; MoveToReference walks the
//! links as Far does. Keys: Tab/Shift+Tab, Enter, Alt+F1/BS back, F1 help on
//! help, Shift+F1 the contents, Shift+F2 the plugins' help, F5 zoom, F7
//! search, Alt+F9 video mode, Esc/F10 close; the mouse as Far's.

use std::sync::{Mutex, OnceLock};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::colors::{bg, fg};
use crate::dialog::{self, Action, Dialog, Item};
use crate::grid::Grid;
use crate::palette;

const ENG: &str = include_str!("../assets/help/FarEng.hlf");
const RUS: &str = include_str!("../assets/help/FarRus.hlf");

const CONTENTS: &str = "Contents";
const FOUND_CONTENTS: &str = "__FoundContents__";
const PLUGIN_CONTENTS: &str = "__PluginContents__";
const HELP_ON_HELP: &str = ":Help";

/// A help file: its lines and the options of its header.
struct HelpFile {
    lines: Vec<&'static str>,
    color_char: Option<char>,
    start_pos_char: String,
    tab_size: usize,
    plugin_contents: String,
}

impl HelpFile {
    fn parse(text: &'static str) -> Self {
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let lines: Vec<&'static str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        let mut file = Self { lines, color_char: None, start_pos_char: String::new(), tab_size: 8, plugin_contents: String::new() };
        // the header: everything up to the first topic
        for line in file.lines.iter().take_while(|l| !l.starts_with('@')) {
            if let Some(value) = line.strip_prefix(".Options CtrlColorChar=") {
                file.color_char = value.chars().next();
            } else if let Some(value) = line.strip_prefix(".Options CtrlStartPosChar=") {
                file.start_pos_char = value.to_string();
            } else if let Some(value) = line.strip_prefix(".Options TabSize=") {
                if let Ok(n) = value.trim().parse::<usize>()
                    && n <= 16
                {
                    file.tab_size = n;
                }
            } else if let Some(value) = line.strip_prefix(".PluginContents=") {
                file.plugin_contents = value.to_string();
            }
        }
        file
    }
}

/// The help file of the help language (Options > Languages).
fn help_file() -> &'static HelpFile {
    static ENGLISH: OnceLock<HelpFile> = OnceLock::new();
    static RUSSIAN: OnceLock<HelpFile> = OnceLock::new();
    if crate::settings::string("Language.Help") == "Russian" {
        RUSSIAN.get_or_init(|| HelpFile::parse(RUS))
    } else {
        ENGLISH.get_or_init(|| HelpFile::parse(ENG))
    }
}

/// The search dialog's choices, kept between searches (Far's shared help group).
#[derive(Clone, Default)]
struct SearchParams {
    text: String,
    case_sensitive: bool,
    whole_words: bool,
}

static SEARCH: Mutex<SearchParams> = Mutex::new(SearchParams { text: String::new(), case_sensitive: false, whole_words: false });

/// Far's Opt.FullScreenHelp (F5), for the session.
static FULL_SCREEN: Mutex<bool> = Mutex::new(false);

fn full_screen() -> bool {
    *FULL_SCREEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// A text in the interface language.
pub(crate) fn msg(en: &'static str, ru: &'static str) -> &'static str {
    if crate::lang::russian() { ru } else { en }
}

/// Far's StackHelpData.
#[derive(Clone, Default)]
struct StackData {
    topic: String,
    sel_topic: String,
    top: i32,
    cur_x: i32,
    cur_y: i32,
}

pub enum Outcome {
    Open,
    Closed,
    /// A box over the help: the search dialog or a message.
    Dialog(Dialog),
    /// Alt+F9: Far's video mode.
    Video,
}

pub struct Help {
    data: StackData,
    stack: Vec<StackData>,
    list: Vec<Vec<char>>,
    fix_count: i32,
    topic_found: bool,
    error: bool,
    is_new_topic: bool,
    /// The window: left, top, right, bottom (inclusive).
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    screen: (u16, u16),
    color_char: Option<char>,
    plugin_contents: String,
    /// The topic Far could not find, for its message.
    not_found: Option<String>,
    mouse_down: Option<(i32, i32, i32, i32)>,
    click_outside: bool,
    last_mouse: (i32, i32),
    /// Where the key bar's keys end, as last painted.
    key_ends: Vec<u16>,
}

impl Help {
    /// Far's help::show(topic) on a screen of `cols` by `rows`.
    pub fn new(topic: &str, cols: u16, rows: u16) -> Self {
        let mut help = Self {
            data: StackData { topic: topic.to_string(), ..Default::default() },
            stack: vec![],
            list: vec![],
            fix_count: 0,
            topic_found: false,
            error: true,
            is_new_topic: true,
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
            screen: (cols, rows),
            color_char: None,
            plugin_contents: String::new(),
            not_found: None,
            mouse_down: None,
            click_outside: false,
            last_mouse: (-1, -1),
            key_ends: vec![],
        };
        help.set_position();
        if !help.read_help() {
            help.data.topic = CONTENTS.to_string();
            help.read_help();
        }
        help.move_to_reference(true, true);
        help
    }

    /// help::show with FHELP_NOSHOWERROR: the help at `topic` if the help
    /// file has it, else nothing.
    pub fn found(topic: &str, cols: u16, rows: u16) -> Option<Self> {
        let mut help = Self::new(CONTENTS, cols, rows);
        help.data.topic = topic.to_string();
        help.list.clear();
        if !help.read_help() {
            return None;
        }
        help.move_to_reference(true, true);
        Some(help)
    }

    fn set_position(&mut self) {
        let (cols, rows) = (self.screen.0 as i32, self.screen.1 as i32);
        let (scr_x, scr_y) = (cols - 1, rows - 1);
        if full_screen() {
            (self.left, self.top, self.right, self.bottom) = (0, 0, scr_x, scr_y);
        } else {
            (self.left, self.top, self.right, self.bottom) = (4, 2, scr_x - 4, scr_y - 2);
        }
    }

    fn obj_width(&self) -> i32 {
        self.right - self.left + 1
    }
    fn obj_height(&self) -> i32 {
        self.bottom - self.top + 1
    }
    fn canvas_width(&self) -> i32 {
        self.obj_width() - 2
    }
    fn canvas_height(&self) -> i32 {
        self.obj_height() - 2
    }
    fn header_height(&self) -> i32 {
        if self.fix_count > 0 { self.fix_count + 1 } else { 0 }
    }
    fn body_height(&self) -> i32 {
        self.canvas_height() - self.header_height()
    }

    /// Whether the key bar shows under the help (not in full screen).
    pub fn key_bar(&self) -> bool {
        !full_screen()
    }

    // ------------------------------------------------------------ reading

    /// Far's ReadHelp: the topic `data.topic` laid out to the window.
    fn read_help(&mut self) -> bool {
        if self.data.topic == PLUGIN_CONTENTS {
            self.read_documents_help();
            return true;
        }
        let file = help_file();
        self.color_char = file.color_char;
        self.plugin_contents = file.plugin_contents.clone();
        self.list.clear();
        if self.data.topic == FOUND_CONTENTS {
            self.search(file);
            return true;
        }
        let (list, fix_count, found, topic) = layout(file, &self.data.topic, self.canvas_width(), self.color_char);
        self.list = list;
        self.fix_count = fix_count;
        self.topic_found = found;
        self.data.topic = topic;
        self.error = false;
        if self.is_new_topic {
            self.data.cur_x = 0;
            self.data.cur_y = 0;
            self.data.top = 0;
        }
        self.topic_found
    }

    /// Shift+F2 (ReadDocumentsHelp): the plugins' help files. farsh has no
    /// plugins with help yet: the title alone.
    fn read_documents_help(&mut self) {
        self.list.clear();
        self.plugin_contents.clear();
        self.fix_count = 1;
        self.data.top = 0;
        self.topic_found = true;
        self.data.cur_x = 0;
        self.data.cur_y = 0;
        self.color_char = None;
        self.add_title(msg("Plugins help", "Плагины"));
        self.error = false;
    }

    fn add_title(&mut self, title: &str) {
        self.list.push(format!("^ #{title}#").chars().collect());
    }

    /// F7's results (Help::Search): a link to every topic with the text.
    fn search(&mut self, file: &HelpFile) {
        self.fix_count = 1;
        self.data.top = 0;
        self.topic_found = true;
        self.data.cur_x = 0;
        self.data.cur_y = 0;
        self.color_char = None;
        let params = SEARCH.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.add_title(&params.text);
        let mut topic_found = false;
        let (mut cur_topic, mut entry) = (String::new(), String::new());
        for raw in &file.lines {
            let line = raw.trim_end();
            if line.starts_with('@') && !(line.len() > 1 && matches!(line.as_bytes()[1], b'+' | b'-')) && !line.contains('=') {
                entry.clear();
                cur_topic.clear();
                let line = line.trim();
                if !line[1..].eq_ignore_ascii_case(CONTENTS) {
                    cur_topic = line.to_string();
                    topic_found = true;
                }
            } else if topic_found && line.chars().count() > 1 && line.starts_with('$') && !cur_topic.is_empty() {
                entry.extend(line[1..].chars().filter(|&c| c != '#'));
            }
            if topic_found && !entry.is_empty() && search_string(line, &params) {
                self.list.push(format!("   ~{entry}~{cur_topic}@").chars().collect());
                cur_topic.clear();
                entry.clear();
                topic_found = false;
            }
        }
        self.error = false;
    }

    // ------------------------------------------------------------ drawing

    pub fn paint(&mut self, grid: &mut Grid) {
        if self.screen != (grid.cols, grid.rows) {
            self.resize(grid.cols, grid.rows);
        }
        if !self.topic_found {
            return;
        }
        if self.data.sel_topic.is_empty() {
            self.move_to_reference(true, true);
        }
        self.fast_show(Some(grid));
        let (w, h) = (self.obj_width().max(0) as u16, self.obj_height().max(0) as u16);
        palette::drop_shadow(grid, self.left as u16, self.top as u16, w, h);
        if self.key_bar() && crate::settings::bool("Screen.KeyBar") {
            let held = crate::modkeys::held();
            self.key_ends = crate::panels::paint_labels(grid, &key_bar_labels(held));
        }
    }

    /// Help::ResizeConsole.
    fn resize(&mut self, cols: u16, rows: u16) {
        self.screen = (cols, rows);
        let old_new = self.is_new_topic;
        self.is_new_topic = false;
        self.set_position();
        self.read_help();
        self.error = false;
        self.data.cur_x -= 1;
        self.move_to_reference(true, true);
        self.is_new_topic = old_new;
    }

    fn draw_window_frame(&self, grid: &mut Grid) {
        let (l, t) = (self.left as u16, self.top as u16);
        let (w, h) = (self.obj_width() as u16, self.obj_height() as u16);
        grid.fill_with(l, t, w, h, fg("Help.Text"), bg("Help.Text"));
        let (bf, bb) = (fg("Help.Box"), bg("Help.Box"));
        let line = "\u{2550}".repeat((w as usize).saturating_sub(2));
        grid.put(l, t, &format!("\u{2554}{line}\u{2557}"), bf, bb);
        grid.put(l, t + h - 1, &format!("\u{255A}{line}\u{255D}"), bf, bb);
        for row in t + 1..t + h - 1 {
            grid.put(l, row, "\u{2551}", bf, bb);
            grid.put(l + w - 1, row, "\u{2551}", bf, bb);
        }
        let contents = if self.plugin_contents.is_empty() { "Far".to_string() } else { self.plugin_contents.clone() };
        let title = format!("{} - {contents}", msg("Help", "Помощь"));
        let title: String = title.chars().take((self.canvas_width() - 2).max(0) as usize).collect();
        let x = self.left + (self.obj_width() - title.chars().count() as i32 - 2) / 2;
        grid.put(x as u16, t, &format!(" {title} "), fg("Help.Box.Title"), bg("Help.Box.Title"));
    }

    /// Far's FastShow; without a grid it only finds the reference under the
    /// cursor (MoveToReference's dry run).
    fn fast_show(&mut self, mut grid: Option<&mut Grid>) {
        if let Some(grid) = grid.as_deref_mut() {
            self.draw_window_frame(grid);
        }
        self.correct_position();
        self.data.sel_topic.clear();
        let mut cur = (fg("Help.Text"), bg("Help.Text"));
        let render = grid.is_some();
        for i in 0..self.canvas_height() {
            let str_pos;
            if i < self.fix_count {
                str_pos = i;
            } else if i == self.fix_count && self.fix_count > 0 {
                if let Some(grid) = grid.as_deref_mut() {
                    let w = self.obj_width() as usize;
                    let line = format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(w.saturating_sub(2)));
                    grid.put(self.left as u16, (self.top + i + 1) as u16, &line, fg("Help.Box"), bg("Help.Box"));
                }
                continue;
            } else {
                str_pos = i + self.data.top - if self.fix_count > 0 { 1 } else { 0 };
                // the dry run needs the cursor's row only
                if !render && i != self.data.cur_y + self.header_height() {
                    continue;
                }
            }
            if str_pos < 0 || str_pos as usize >= self.list.len() {
                continue;
            }
            let line = self.list[str_pos as usize].clone();
            let y = self.top + i + 1;
            let (text, x) = if line.first() == Some(&'^') {
                let rest = &line[1..];
                (rest.to_vec(), self.left + 1 + ((self.canvas_width() - self.string_len(rest)) / 2).max(0))
            } else {
                (line, self.left + 1)
            };
            self.out_string(&text, x, y, grid.as_deref_mut(), &mut cur);
        }
        if let Some(grid) = grid {
            let length = self.body_height().max(0) as usize;
            let size = (self.list.len() as i32 - self.fix_count).max(0) as usize;
            if length >= 2 && size > length {
                let start = self.data.top.max(0) as usize;
                let bar = crate::vmenu::scroll_bar(length, start, start + length, size);
                let y0 = self.top + self.header_height() + 1;
                for (k, c) in bar.chars().enumerate() {
                    grid.put(self.right as u16, (y0 + k as i32) as u16, &c.to_string(), fg("Help.Scrollbar"), bg("Help.Scrollbar"));
                }
            }
        }
    }

    fn string_len(&self, text: &[char]) -> i32 {
        fast_parse_line(text, 0, -1, self.color_char).0
    }

    /// Far's OutString: one line of the help at (`x`, `y`).
    fn out_string(&mut self, s: &[char], x0: i32, y: i32, mut grid: Option<&mut Grid>, cur: &mut (Color, Color)) {
        let max_width = self.obj_width() - 4;
        let cc = self.color_char;
        let mut out = String::new();
        let mut out_len = 0i32;
        let (mut i, mut start_topic) = (0usize, 0usize);
        let (mut highlight, mut topic) = (false, false);
        let mut x = x0;
        let n = s.len();
        loop {
            if i + 1 < n {
                let (a, b) = (s[i], s[i + 1]);
                if (a == '~' && b == '~') || (a == '#' && b == '#') || (a == '@' && b == '@') || (cc == Some(a) && cc == Some(b)) {
                    out.push(a);
                    out_len += 1;
                    i += 2;
                    continue;
                }
            }
            let flush = i >= n || s[i] == '~' || ((s[i] == '#' || Some(s[i]) == cc) && !topic);
            if flush {
                let color = if topic {
                    let real_cur_x = self.left + self.data.cur_x + 1;
                    let real_cur_y = self.top + self.data.cur_y + self.header_height() + 1;
                    let found = y == real_cur_y && real_cur_x >= x && real_cur_x < x + (i - start_topic) as i32 - 1;
                    if i + 1 < n && s[i + 1] == '@' {
                        let (next, name) = skip_link(s, i + 2);
                        if found {
                            self.data.sel_topic = name;
                        }
                        i = next;
                        topic = false;
                    }
                    if found { (fg("Help.Topic.Selected"), bg("Help.Topic.Selected")) } else { (fg("Help.Topic"), bg("Help.Topic")) }
                } else if highlight {
                    (fg("Help.Text.Highlight"), bg("Help.Text.Highlight"))
                } else {
                    *cur
                };
                let max_size = (max_width - (x - self.left - 2)).max(0);
                let shown = out_len.min(max_size);
                if let Some(grid) = grid.as_deref_mut() {
                    let text: String = out.chars().take(shown as usize).collect();
                    grid.put(x as u16, y as u16, &text, color.0, color.1);
                }
                x += shown;
                out.clear();
                out_len = 0;
            }
            if i >= n {
                break;
            }
            match s[i] {
                '~' => {
                    if !topic {
                        start_topic = i;
                    }
                    topic = !topic;
                    i += 1;
                }
                '@' => i = skip_link(s, i + 1).0,
                '#' => {
                    highlight = !highlight;
                    i += 1;
                }
                c => {
                    if let Some(next) = help_color(s, i, cc, cur) {
                        i = next;
                    } else {
                        out.push(c);
                        out_len += 1;
                        i += 1;
                    }
                }
            }
        }
        if x < self.right
            && let Some(grid) = grid
        {
            grid.put(x as u16, y as u16, &" ".repeat((self.right - x) as usize), cur.0, cur.1);
        }
    }

    fn correct_position(&mut self) {
        self.data.cur_x = self.data.cur_x.clamp(0, (self.canvas_width() - 1).max(0));
        let body = self.body_height();
        if self.data.cur_y > body - 1 {
            self.data.top += self.data.cur_y - (body - 1);
            self.data.cur_y = body - 1;
        }
        if self.data.cur_y < 0 {
            self.data.top += self.data.cur_y;
            self.data.cur_y = 0;
        }
        self.data.top = self.data.top.min(self.list.len() as i32 - body).max(0);
    }

    fn is_reference_present(&mut self) -> bool {
        self.correct_position();
        let pos = self.fix_count + self.data.top + self.data.cur_y;
        if pos < 0 || pos as usize >= self.list.len() {
            return false;
        }
        let line = &self.list[pos as usize];
        line.windows(2).any(|w| w == ['~', '@'])
    }

    /// Far's MoveToReference: to the next (or previous) link, on this screen
    /// only if `cur_screen`.
    fn move_to_reference(&mut self, forward: bool, cur_screen: bool) {
        let mut start_selection = !self.data.sel_topic.is_empty();
        let saved = (self.data.cur_x, self.data.cur_y, self.data.top);
        self.data.sel_topic.clear();
        if !self.error {
            let width = self.canvas_width();
            while self.data.sel_topic.is_empty() {
                let present = self.is_reference_present();
                if forward {
                    if self.data.cur_x == 0 && !present {
                        self.data.cur_x = width - 1;
                    }
                    self.data.cur_x += 1;
                    if self.data.cur_x >= width - 1 {
                        start_selection = false;
                        self.data.cur_x = 0;
                        self.data.cur_y += 1;
                        if self.data.top + self.data.cur_y >= self.list.len() as i32 - self.fix_count || (cur_screen && self.data.cur_y > self.body_height() - 1) {
                            break;
                        }
                    }
                } else {
                    if self.data.cur_x == width - 1 && !present {
                        self.data.cur_x = 0;
                    }
                    self.data.cur_x -= 1;
                    if self.data.cur_x < 0 {
                        start_selection = false;
                        self.data.cur_x = width - 1;
                        self.data.cur_y -= 1;
                        if self.data.top + self.data.cur_y < 0 || (cur_screen && self.data.cur_y < 0) {
                            break;
                        }
                    }
                }
                self.fast_show(None);
                if self.data.sel_topic.is_empty() {
                    start_selection = false;
                } else {
                    if present && cur_screen {
                        start_selection = false;
                    }
                    if start_selection {
                        self.data.sel_topic.clear();
                    }
                }
            }
        }
        if self.data.sel_topic.is_empty() {
            (self.data.cur_x, self.data.cur_y, self.data.top) = saved;
        }
        self.fast_show(None);
    }

    // ------------------------------------------------------------ keys

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.data.sel_topic.is_empty() {
            self.data.cur_x = 0;
            self.data.cur_y = 0;
        }
        let m = key.modifiers;
        let (none, shift, alt, ctrl) = (m.is_empty(), m == KeyModifiers::SHIFT, m == KeyModifiers::ALT, m == KeyModifiers::CONTROL);
        match key.code {
            KeyCode::F(5) if none => {
                {
                    let mut full = FULL_SCREEN.lock().unwrap_or_else(|e| e.into_inner());
                    *full = !*full;
                }
                let (cols, rows) = self.screen;
                self.resize(cols, rows);
            }
            KeyCode::Esc | KeyCode::F(10) if none => return Outcome::Closed,
            KeyCode::Home if none || ctrl => self.home(),
            KeyCode::PageUp if ctrl => self.home(),
            KeyCode::End if none || ctrl => self.end(),
            KeyCode::PageDown if ctrl => self.end(),
            KeyCode::Up if none => self.up(),
            KeyCode::Down if none => self.down(),
            KeyCode::PageUp if none => {
                self.data.cur_x = 0;
                self.data.cur_y = 0;
                self.data.top -= self.body_height() - 1;
                self.fast_show(None);
                if self.data.sel_topic.is_empty() {
                    self.data.cur_x = 0;
                    self.data.cur_y = 0;
                    self.move_to_reference(true, true);
                }
            }
            KeyCode::PageDown if none => {
                let prev = self.data.top;
                self.data.top += self.body_height() - 1;
                self.fast_show(None);
                if self.data.top == prev {
                    self.end();
                } else {
                    self.data.cur_x = 0;
                    self.data.cur_y = 0;
                    self.move_to_reference(true, true);
                }
            }
            KeyCode::Right | KeyCode::Tab if none => self.move_to_reference(true, false),
            KeyCode::Left if none => self.move_to_reference(false, false),
            KeyCode::BackTab => self.move_to_reference(false, false),
            KeyCode::Tab if shift => self.move_to_reference(false, false),
            KeyCode::F(1) if none => {
                if self.data.topic != HELP_ON_HELP {
                    self.push_and_jump(HELP_ON_HELP);
                }
            }
            KeyCode::F(1) if shift => {
                if self.data.topic != CONTENTS {
                    self.push_and_jump(CONTENTS);
                }
            }
            KeyCode::F(2) if shift => {
                if self.data.topic != PLUGIN_CONTENTS {
                    self.push_and_jump(PLUGIN_CONTENTS);
                }
            }
            KeyCode::F(7) if none => {
                if self.data.topic != FOUND_CONTENTS {
                    return Outcome::Dialog(search_dialog());
                }
            }
            KeyCode::F(9) if alt => return Outcome::Video,
            KeyCode::F(1) if alt => return self.back(),
            KeyCode::Backspace if none => return self.back(),
            KeyCode::Enter if none => return self.enter(),
            _ => {}
        }
        self.take_not_found()
    }

    fn home(&mut self) {
        self.data.cur_x = 0;
        self.data.cur_y = 0;
        self.data.top = 0;
        self.fast_show(None);
        if self.data.sel_topic.is_empty() {
            self.move_to_reference(true, true);
        }
    }

    fn end(&mut self) {
        self.data.cur_x = 0;
        self.data.cur_y = 0;
        self.data.top = self.list.len() as i32;
        self.fast_show(None);
        if self.data.sel_topic.is_empty() {
            self.data.cur_x = 0;
            self.data.cur_y = self.body_height() - 1;
            self.move_to_reference(false, true);
        }
    }

    fn up(&mut self) {
        if self.data.top > 0 {
            self.data.top -= 1;
            if self.data.cur_y < self.body_height() - 1 {
                self.data.cur_x = self.canvas_width() - 2 - 1;
                self.data.cur_y += 1;
            }
            self.fast_show(None);
            if self.data.sel_topic.is_empty() {
                self.move_to_reference(false, true);
            }
        } else {
            self.move_to_reference(false, false);
        }
    }

    fn down(&mut self) {
        if self.data.top < self.list.len() as i32 - self.fix_count - self.body_height() {
            self.data.top += 1;
            if self.data.cur_y > 0 {
                self.data.cur_y -= 1;
            }
            self.data.cur_x = 0;
            self.fast_show(None);
            if self.data.sel_topic.is_empty() {
                self.move_to_reference(true, true);
            }
        } else {
            self.move_to_reference(true, false);
        }
    }

    fn push_and_jump(&mut self, topic: &str) {
        self.stack.push(self.data.clone());
        self.is_new_topic = true;
        self.jump_topic_to(topic);
        self.is_new_topic = false;
        self.error = false;
    }

    /// Alt+F1, BS: the topic before, or out of the help.
    fn back(&mut self) -> Outcome {
        let Some(data) = self.stack.pop() else { return Outcome::Closed };
        self.data = data;
        let topic = self.data.topic.clone();
        self.jump_topic_to(&topic);
        self.error = false;
        self.take_not_found()
    }

    fn enter(&mut self) -> Outcome {
        if !self.data.sel_topic.is_empty() && !self.data.sel_topic.eq_ignore_ascii_case(&self.data.topic) {
            self.stack.push(self.data.clone());
            self.is_new_topic = true;
            if !self.jump_topic() {
                if let Some(data) = self.stack.pop() {
                    self.data = data;
                }
                self.read_help();
            }
            self.error = false;
            self.is_new_topic = false;
        }
        self.take_not_found()
    }

    fn take_not_found(&mut self) -> Outcome {
        match self.not_found.take() {
            Some(topic) => {
                let lines = [msg("Requested help topic not found:", "Не найден запрошенный раздел помощи:").to_string(), topic];
                Outcome::Dialog(Dialog::message(msg("Help", "Помощь"), &lines, &[("OK", "ok")], true, Action::Info))
            }
            None => Outcome::Open,
        }
    }

    fn jump_topic_to(&mut self, topic: &str) -> bool {
        self.data.sel_topic = topic.to_string();
        self.jump_topic()
    }

    /// Far's JumpTopic for the links of Far's own help: a topic, `:Help`,
    /// the generated ones, or a URL (opened outside).
    fn jump_topic(&mut self) -> bool {
        let sel = self.data.sel_topic.clone();
        if let Some(colon) = sel.find(':')
            && colon != 0
            && !sel.starts_with('<')
        {
            open_url(&sel);
            return false;
        }
        let topic = if sel == HELP_ON_HELP { sel[1..].to_string() } else { sel.clone() };
        self.data.topic = topic.clone();
        if !self.read_help() {
            self.data.topic = topic.clone();
            self.read_help();
        }
        if self.list.is_empty() || !self.topic_found {
            self.error = true;
            self.not_found = Some(topic);
            return false;
        }
        if self.is_new_topic || sel == FOUND_CONTENTS || sel == PLUGIN_CONTENTS {
            self.move_to_reference(true, true);
        }
        true
    }

    /// The search dialog's Find: the results topic.
    pub fn search_for(&mut self, values: &dialog::Values) -> Outcome {
        {
            let mut params = SEARCH.lock().unwrap_or_else(|e| e.into_inner());
            params.text = values.text("text");
            params.case_sensitive = values.checked("case");
            params.whole_words = values.checked("words");
        }
        self.push_and_jump(FOUND_CONTENTS);
        self.take_not_found()
    }

    // ------------------------------------------------------------ mouse

    /// Far's ProcessMouse.
    pub fn mouse(&mut self, event: MouseEvent) -> Outcome {
        let (mx, my) = (event.column as i32, event.row as i32);
        // the key bar under the help
        if self.key_bar() && my == self.screen.1 as i32 - 1 {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind
                && let Some(n) = self.key_ends.iter().position(|&end| (mx as u16) < end)
            {
                return self.key(KeyEvent::new(KeyCode::F(n as u8 + 1), KeyModifiers::NONE));
            }
            return Outcome::Open;
        }
        let inside = mx >= self.left && mx <= self.right && my >= self.top && my <= self.bottom;
        match event.kind {
            MouseEventKind::ScrollUp => {
                for _ in 0..3 {
                    self.up();
                }
                return Outcome::Open;
            }
            MouseEventKind::ScrollDown => {
                for _ in 0..3 {
                    self.down();
                }
                return Outcome::Open;
            }
            MouseEventKind::Down(MouseButton::Middle) => return self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            _ => {}
        }
        let moved = matches!(event.kind, MouseEventKind::Moved);
        if !moved && !inside {
            if self.click_outside {
                return Outcome::Closed;
            }
            if matches!(event.kind, MouseEventKind::Down(_)) {
                self.click_outside = true;
            }
            return Outcome::Open;
        }
        let pressed = matches!(event.kind, MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right));
        // the scroll bar: its arrows, or a jump to where it was clicked
        if pressed && mx == self.right {
            let scroll_y = self.top + self.header_height() + 1;
            if my == scroll_y {
                self.up();
                return Outcome::Open;
            }
            if my == scroll_y + self.body_height() - 1 {
                self.down();
                return Outcome::Open;
            }
            if (self.list.len() as i32) > self.body_height() && my > scroll_y && my < scroll_y + self.body_height() + 1 {
                self.data.cur_x = 0;
                self.data.cur_y = 0;
                self.data.top = (my - scroll_y - 1) * (self.list.len() as i32 - self.fix_count - self.body_height() + 1) / (self.body_height() - 2).max(1);
                self.fast_show(None);
                return Outcome::Open;
            }
        }
        if pressed {
            let before = (self.data.cur_x, self.data.cur_y);
            self.data.cur_x = mx - self.left - 1;
            self.data.cur_y = my - self.top - 1 - self.header_height();
            self.mouse_down = Some((self.data.cur_x, self.data.cur_y, before.0, before.1));
        }
        if let MouseEventKind::Up(_) = event.kind
            && let Some((down_x, down_y, before_x, before_y)) = self.mouse_down.take()
        {
            self.fast_show(None);
            let same = self.data.cur_x == down_x && self.data.cur_y == down_y;
            if !self.data.sel_topic.is_empty() {
                if same {
                    return self.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            } else if same {
                self.data.cur_x = before_x;
                self.data.cur_y = before_y;
            }
        }
        if moved && (mx, my) != self.last_mouse && self.topic_at(mx, my).is_some() {
            self.data.cur_x = mx - self.left - 1;
            self.data.cur_y = my - self.top - 1 - self.header_height();
        }
        self.last_mouse = (mx, my);
        self.fast_show(None);
        Outcome::Open
    }

    /// Far's GetTopic: the link at a screen cell, if any.
    fn topic_at(&self, x: i32, y: i32) -> Option<String> {
        if y <= self.top || y >= self.bottom || x <= self.left || x >= self.right {
            return None;
        }
        let row = if y - self.top <= self.header_height() {
            y - self.top - 1
        } else {
            y - self.top - 1 - self.header_height() + self.fix_count + self.data.top
        };
        let line = self.list.get(usize::try_from(row).ok()?)?;
        if line.is_empty() {
            return None;
        }
        let (text, x0) = if line[0] == '^' {
            let rest = &line[1..];
            (rest, self.left + 1 + ((self.canvas_width() - self.string_len(rest)) / 2).max(0))
        } else {
            (&line[..], self.left + 1)
        };
        let (_, found, topic) = fast_parse_line(text, x0, x, self.color_char);
        found.then_some(topic)
    }
}

/// The help's key bar (MHelpF1.., MHelpShiftF1.., MHelpAltF1..), in the
/// interface language; one modifier at most, as farsh's keys.
fn key_bar_labels(held: u8) -> [&'static str; 12] {
    use crate::modkeys::{ALT, CTRL, SHIFT};
    let ru = crate::lang::russian();
    match held & (SHIFT | CTRL | ALT) {
        SHIFT => {
            if ru {
                ["Содерж", "Плагин", "", "", "", "", "", "", "", "", "", ""]
            } else {
                ["Index", "Plugin", "", "", "", "", "", "", "", "", "", ""]
            }
        }
        ALT => {
            if ru {
                ["Пред.", "", "", "", "", "", "", "", "Видео", "", "", ""]
            } else {
                ["Prev", "", "", "", "", "", "", "", "Video", "", "", ""]
            }
        }
        CTRL => [""; 12],
        _ => {
            if ru {
                ["Помощь", "", "", "", "Размер", "", "Поиск", "", "", "Выход", "", ""]
            } else {
                ["Help", "", "", "", "Zoom", "", "Search", "", "", "Quit", "", ""]
            }
        }
    }
}

/// F7's dialog (GetSearchReplaceString for the help): Search for, Case
/// sensitive, Whole words, Fuzzy search, Regular expressions; Find, Cancel.
/// Fuzzy search and regular expressions are not there yet: grey.
fn search_dialog() -> Dialog {
    let params = SEARCH.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let text: Vec<char> = params.text.chars().collect();
    let cursor = text.len();
    let items: Vec<(Item, dialog::Place)> = vec![
        (Item::Label("&Search for".into()), dialog::at(5, 2)),
        (Item::Edit { id: "text", label: None, x: 0, text, cursor, unchanged: !params.text.is_empty(), enabled: true, history: true, secret: false }, dialog::span(5, 3, 70)),
        (Item::Separator(None), dialog::at(-1, 4)),
        (Item::Check { id: "case", label: "&Case sensitive", checked: params.case_sensitive, enabled: true }, dialog::at(5, 5)),
        (Item::Check { id: "words", label: "&Whole words", checked: params.whole_words, enabled: true }, dialog::at(5, 6)),
        (Item::Check { id: "fuzzy", label: "Fu&zzy search", checked: false, enabled: false }, dialog::at(5, 7)),
        (Item::Check { id: "regex", label: "R&egular expressions", checked: false, enabled: false }, dialog::at(39, 5)),
        (Item::Separator(None), dialog::at(-1, 8)),
        (Item::Buttons(vec![dialog::button("Find", "ok", true), dialog::button("Cancel", "cancel", false)]), dialog::at(-1, 9)),
    ];
    let (items, places): (Vec<Item>, Vec<dialog::Place>) = items.into_iter().unzip();
    let mut dialog = Dialog::placed(crate::lang::tr("Search").into(), (76, 12), items, places, 1, Some(search_hook), Action::HelpSearch);
    // Find is there for some text only
    dialog.enable_button("ok", !params.text.is_empty());
    dialog
}

fn search_hook(dialog: &mut Dialog, event: dialog::Event) {
    if event == dialog::Event::Edited("text") {
        let empty = dialog.values().text("text").is_empty();
        dialog.enable_button("ok", !empty);
    }
}

/// Far's SearchString for the help: the text in `line`, in any case unless
/// asked, as a whole word if asked (Editor.WordDiv's dividers around it).
fn search_string(line: &str, params: &SearchParams) -> bool {
    if params.text.is_empty() {
        return false;
    }
    let fold = |s: &str| if params.case_sensitive { s.to_string() } else { s.to_lowercase() };
    let (hay, needle) = (fold(line), fold(&params.text));
    let hay: Vec<char> = hay.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    if needle.len() > hay.len() {
        return false;
    }
    let divider = |c: char| c.is_whitespace() || "~!%^&*()+|{}:\"<>?`-=\\[];',./".contains(c);
    (0..=hay.len() - needle.len()).any(|at| {
        hay[at..at + needle.len()] == needle[..]
            && (!params.whole_words || ((at == 0 || divider(hay[at - 1])) && (at + needle.len() == hay.len() || divider(hay[at + needle.len()]))))
    })
}

/// A URL of the help, opened by the system.
fn open_url(url: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, target) = (wide("open"), wide(url));
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

// ---------------------------------------------------------------- parsing

/// Far's SkipLink: past `Topic@` (a doubled `@@` is a literal one); the
/// index after it and the name.
fn skip_link(s: &[char], mut i: usize) -> (usize, String) {
    let mut name = String::new();
    loop {
        while i < s.len() && s[i] != '@' {
            name.push(s[i]);
            i += 1;
        }
        if i < s.len() {
            i += 1;
        }
        if i >= s.len() || s[i] != '@' {
            break;
        }
        name.push('@');
        i += 1;
    }
    (i, name)
}

/// A color index of the help (`\hh`, `(Fxx…)`) as the terminal gets it.
fn index_color(n: u32, foreground: bool) -> Color {
    if n < 16 {
        if foreground { palette::fg(n as u8) } else { palette::bg(n as u8) }
    } else {
        Color::AnsiValue(n.min(255) as u8)
    }
}

/// One side of Far's new color format: `T` and RRGGBB true color, or an index.
fn parse_color(part: &str, foreground: bool) -> Option<Color> {
    if let Some(hex) = part.strip_prefix('T') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return Some(Color::Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 });
    }
    let v = u32::from_str_radix(part, 16).ok()?;
    Some(index_color(v, foreground))
}

/// Far's ExtractColorInNewFormat at `s[i]` (`(fg:bg)`): the index after it
/// and the colors (None where a side is not given).
fn new_color(s: &[char], i: usize) -> Option<(usize, Option<Color>, Option<Color>)> {
    if s.get(i) != Some(&'(') {
        return None;
    }
    let close = (i + 1..s.len()).find(|&k| s[k] == ')')?;
    let inner: String = s[i + 1..close].iter().collect();
    let (f, b) = inner.split_once(':').unwrap_or((&inner, ""));
    let f = if f.is_empty() { None } else { Some(parse_color(f, true)?) };
    let b = if b.is_empty() { None } else { Some(parse_color(b, false)?) };
    Some((close + 1, f, b))
}

/// Far's GetHelpColor at `s[i]`: a color change, and the index after it.
fn help_color(s: &[char], i: usize, cc: Option<char>, cur: &mut (Color, Color)) -> Option<usize> {
    let cc = cc?;
    if s.len() < i + 2 || s[i] != cc || s[i + 1] == cc {
        return None;
    }
    if s[i + 1] == '-' {
        *cur = (fg("Help.Text"), bg("Help.Text"));
        return Some(i + 2);
    }
    if s.len() > i + 2 && s[i + 1].is_ascii_hexdigit() && s[i + 2].is_ascii_hexdigit() {
        let v = u32::from_str_radix(&format!("{}{}", s[i + 1], s[i + 2]), 16).ok()?;
        *cur = (index_color(v & 15, true), index_color(v >> 4, false));
        return Some(i + 3);
    }
    let (next, f, b) = new_color(s, i + 1)?;
    if let Some(f) = f {
        cur.0 = f;
    }
    if let Some(b) = b {
        cur.1 = b;
    }
    Some(next)
}

/// Far's FastParseLine: the shown length of `s` from `x0`, and whether the
/// link under `real_x` was found (with its topic).
fn fast_parse_line(s: &[char], x0: i32, real_x: i32, cc: Option<char>) -> (i32, bool, String) {
    let (mut x, mut start_topic) = (x0, -1);
    let mut found = false;
    let mut topic = String::new();
    let mut i = 0;
    let n = s.len();
    while i < n {
        let wc = s[i];
        i += 1;
        if i < n && s[i] == wc && (matches!(wc, '~' | '@' | '#') || Some(wc) == cc) {
            i += 1;
        } else if wc == '#' {
            continue;
        } else if cc == Some(wc) {
            if i >= n {
                break;
            }
            if s[i] == '-' {
                i += 1;
                continue;
            }
            if i + 1 < n && s[i].is_ascii_hexdigit() && s[i + 1].is_ascii_hexdigit() {
                i += 2;
                continue;
            }
            if let Some((next, _, _)) = new_color(s, i) {
                i = next;
                continue;
            }
        } else if wc == '@' {
            i = skip_link(s, i).0;
            continue;
        } else if wc == '~' {
            if start_topic < 0 {
                start_topic = x;
            } else {
                found = real_x >= start_topic && real_x < x;
                if i < n && s[i] == '@' {
                    let (next, name) = skip_link(s, i + 1);
                    i = next;
                    if found {
                        topic = name;
                    }
                }
                if found {
                    break;
                }
                start_topic = -1;
            }
            continue;
        }
        x += 1;
        if real_x >= 0 && x > real_x && start_topic < 0 {
            break;
        }
    }
    (x - x0, found, topic)
}

fn string_len(s: &[char], cc: Option<char>) -> i32 {
    fast_parse_line(s, 0, -1, cc).0
}

/// Far's HighlightsCorrection: an odd count of `#` opens it at the start.
fn highlights_correction(s: &mut Vec<char>) {
    if s.iter().filter(|&&c| c == '#').count() % 2 == 1 && s.first() != Some(&'$') {
        s.insert(0, '#');
    }
}

fn find_sub(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == needle[..])
}

/// The layout's state (ReadHelp's locals).
struct Layout {
    list: Vec<Vec<char>>,
    start_pos: i32,
    last_start_pos: i32,
}

impl Layout {
    /// Far's AddLine: indented to the wrap position.
    fn add_line(&mut self, line: &[char]) {
        let width = if self.start_pos != 0 && line.first() == Some(&' ') { self.start_pos - 1 } else { self.start_pos };
        let mut out = vec![' '; width.max(0) as usize];
        out.extend_from_slice(line);
        self.list.push(out);
    }
}

/// What a pass of ReadHelp's body asks the loop to do next.
enum Next {
    Continue,
    Break,
    /// On to the end of the loop's turn (PrevSymbol).
    Fall,
}

/// Far's ReadHelp for one topic of `file`, laid out to `width` columns:
/// the lines, the fixed header's count, whether the topic was found, and
/// the topic (after `@Name=Other` redirections).
fn layout(file: &HelpFile, topic: &str, width: i32, cc: Option<char>) -> (Vec<Vec<char>>, i32, bool, String) {
    let mut topic = topic.to_string();
    let mut l = Layout { list: vec![], start_pos: 0, last_start_pos: 0 };
    let mut fix_count = 0;
    let mut topic_found = false;
    let mut repeat_last_line = false;
    let mut break_process = false;
    let mut formatting = true;
    let mut near_topic_found = false;
    let mut prev_symbol: Option<char> = None;
    let mut drawline = false;
    let mut draw_line_char: Option<char> = None;
    let max_length = width;
    let start_pos_char: Vec<char> = file.start_pos_char.chars().collect();
    let mut lines = file.lines.iter();
    let mut in_header = true;
    let mut split: Vec<char> = vec![];
    let mut read: Vec<char> = vec![];

    loop {
        let real_max = max_length - l.start_pos;
        if !repeat_last_line && !break_process {
            match lines.next() {
                Some(line) => {
                    read = line.chars().collect();
                    if in_header {
                        in_header = read.first() != Some(&'@');
                        if in_header {
                            continue;
                        }
                    }
                }
                None => {
                    if string_len(&split, cc) < max_length {
                        if !split.is_empty() {
                            l.add_line(&split);
                        }
                    } else {
                        read.clear();
                        repeat_last_line = true;
                        continue;
                    }
                    break;
                }
            }
        }
        repeat_last_line = false;

        // tabs to spaces, as Far does
        while let Some(pos) = read.iter().position(|&c| c == '\t') {
            read[pos] = ' ';
            if file.tab_size > 1 {
                let pad = file.tab_size - (pos % file.tab_size);
                for _ in 0..pad {
                    read.insert(pos, ' ');
                }
            }
        }
        while read.last().is_some_and(|c| c.is_whitespace()) {
            read.pop();
        }
        if !start_pos_char.is_empty()
            && let Some(pos) = find_sub(&read, &start_pos_char)
        {
            let p1 = read.iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
            let p1 = if p1 > pos { 0 } else { p1 };
            l.last_start_pos = string_len(&read[p1..pos], cc);
            read.drain(pos..pos + start_pos_char.len());
        }
        if topic_found {
            highlights_correction(&mut read);
        }

        let mut goto_m1 = false;
        if read.first() == Some(&'@') && !break_process {
            if topic_found {
                if read == ['@', '+'] {
                    formatting = true;
                    prev_symbol = None;
                    continue;
                }
                if read == ['@', '-'] {
                    formatting = false;
                    prev_symbol = None;
                    continue;
                }
                if read.get(1) == Some(&'=') {
                    draw_line_char = read.get(2).copied();
                    drawline = true;
                    prev_symbol = None;
                    goto_m1 = true;
                } else if !split.is_empty() {
                    break_process = true;
                    read.clear();
                    prev_symbol = None;
                    goto_m1 = true;
                } else {
                    break;
                }
            } else {
                let name: String = read[1..].iter().collect();
                if name.eq_ignore_ascii_case(&topic) {
                    topic_found = true;
                    near_topic_found = true;
                } else {
                    // a redirection: @SearchTopic=RealTopic
                    let n1 = topic.chars().count();
                    let n2 = read.len();
                    let head: String = read[1..(1 + n1).min(n2)].iter().collect();
                    if 1 + n1 + 1 < n2 && head.eq_ignore_ascii_case(&topic) && read[1 + n1] == '=' {
                        topic = read[1 + n1 + 1..].iter().collect();
                        continue;
                    }
                }
            }
            if !goto_m1 {
                prev_symbol = read.first().copied();
                continue;
            }
        }

        // m1: the topic's text
        let next = loop {
            if read.is_empty() && break_process && split.is_empty() {
                break Next::Break;
            }
            let mut again = false;
            if topic_found {
                // macro lists: farsh has no macros
                if starts_with(&read, "<!Macro:") {
                    break Next::Continue;
                }
                let dollar = read.first() == Some(&'$');
                let after_title = matches!(prev_symbol, Some('$') | Some('@'));
                if !(dollar && near_topic_found && after_title) {
                    near_topic_found = false;
                }
                if near_topic_found {
                    l.start_pos = 0;
                    l.last_start_pos = 0;
                }
                if dollar && near_topic_found && after_title {
                    l.add_line(&read[1..]);
                    fix_count += 1;
                } else {
                    near_topic_found = false;
                    if read.is_empty() || !formatting {
                        if !split.is_empty() {
                            if string_len(&split, cc) < real_max {
                                l.add_line(&split);
                                split.clear();
                                if string_len(&read, cc) < real_max {
                                    l.add_line(&read);
                                    l.last_start_pos = 0;
                                    l.start_pos = 0;
                                    break Next::Continue;
                                }
                            } else {
                                repeat_last_line = true;
                            }
                        } else if string_len(&read, cc) < real_max {
                            l.add_line(&read);
                            break Next::Continue;
                        }
                    }
                    if read.first().is_some_and(|c| *c == ' ' || *c == '\t') && formatting {
                        if string_len(&split, cc) < real_max {
                            if !split.is_empty() {
                                l.add_line(&split);
                                l.start_pos = 0;
                            }
                            split = std::mem::take(&mut read);
                            break Next::Continue;
                        } else {
                            repeat_last_line = true;
                        }
                    }
                    if drawline {
                        drawline = false;
                        if !split.is_empty() {
                            l.add_line(&split);
                            l.start_pos = 0;
                        }
                        let c = draw_line_char.unwrap_or('\u{2500}');
                        let mul = if matches!(c, '@' | '~' | '#') { 2 } else { 1 };
                        let len = (width * mul - (mul >> 1)).max(2) as usize;
                        let mut line = vec![' '];
                        line.extend(std::iter::repeat_n(c, len - 2));
                        line.push(' ');
                        l.add_line(&line);
                        read.clear();
                        split.clear();
                        break Next::Continue;
                    }
                    if !repeat_last_line {
                        if !split.is_empty() {
                            split.push(' ');
                        }
                        split.extend_from_slice(&read);
                    }
                    if string_len(&split, cc) < real_max {
                        if read.is_empty() && break_process {
                            continue;
                        }
                        break Next::Continue;
                    }
                    let mut splitted = false;
                    let mut k = split.len() as i32 - 1;
                    while k > 0 {
                        let i = k as usize;
                        if split[i] == '~' && split[i - 1] == '~' {
                            k -= 2;
                            continue;
                        }
                        if split[i] == '~' && split[i - 1] != '~' {
                            loop {
                                k -= 1;
                                if !(k > 0 && split[k as usize] != '~') {
                                    break;
                                }
                            }
                            k -= 1;
                            continue;
                        }
                        if split[i] == ' ' && string_len(&split[..i], cc) < real_max {
                            let first: Vec<char> = split[..i].to_vec();
                            l.add_line(&first);
                            split.drain(1..=i);
                            split[0] = ' ';
                            highlights_correction(&mut split);
                            splitted = true;
                            break;
                        }
                        k -= 1;
                    }
                    if !splitted {
                        l.add_line(&split);
                        split.clear();
                    } else {
                        l.start_pos = l.last_start_pos;
                    }
                }
            }
            if break_process {
                if !split.is_empty() {
                    again = true;
                } else {
                    break Next::Break;
                }
            }
            if !again {
                break Next::Fall;
            }
        };
        match next {
            Next::Continue => continue,
            Next::Break => break,
            Next::Fall => {}
        }
        prev_symbol = read.first().copied();
    }
    (l.list, fix_count, topic_found, topic)
}

fn starts_with(s: &[char], prefix: &str) -> bool {
    let p: Vec<char> = prefix.chars().collect();
    s.len() >= p.len() && s[..p.len()] == p[..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &[char]) -> String {
        line.iter().collect()
    }

    #[test]
    fn the_contents_have_the_fixed_header_and_links() {
        let file = HelpFile::parse(ENG);
        assert_eq!(file.color_char, Some('\\'));
        assert_eq!(file.start_pos_char, "^<wrap>");
        let (list, fix, found, _) = layout(&file, "Contents", 90, file.color_char);
        assert!(found);
        assert_eq!(fix, 4);
        assert_eq!(text(&list[0]), "^#File and archive manager#");
        assert_eq!(text(&list[4]), " ~Help file index~@Index@");
    }

    #[test]
    fn paragraphs_wrap_to_the_width() {
        let file = HelpFile::parse(ENG);
        let (list, _, found, _) = layout(&file, "Help", 90, file.color_char);
        assert!(found);
        let widths: Vec<i32> = list.iter().map(|l| string_len(l, file.color_char)).collect();
        assert!(widths.iter().all(|&w| w < 90), "{widths:?}");
        assert!(list.iter().any(|l| text(l).starts_with(" Help screens can have reference items")));
    }

    #[test]
    fn links_and_markup_are_measured_without_their_marks() {
        let s: Vec<char> = " ~Help file index~@Index@ #bold# \\1Fx\\-".chars().collect();
        assert_eq!(string_len(&s, Some('\\')), " Help file index bold x".chars().count() as i32);
        let (_, found, topic) = fast_parse_line(&s, 0, 3, Some('\\'));
        assert!(found);
        assert_eq!(topic, "Index");
    }

    #[test]
    fn tab_and_enter_go_to_a_topic_and_back() {
        let mut help = Help::new("Contents", 100, 30);
        assert_eq!(help.data.sel_topic, "Index", "the first link is picked");
        help.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(help.data.sel_topic, "Help");
        help.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(help.data.topic, "Help");
        assert!(matches!(help.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)), Outcome::Open));
        assert_eq!(help.data.topic, "Contents");
        assert!(matches!(help.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)), Outcome::Closed));
    }

    #[test]
    fn the_help_paints_its_frame_title_and_text() {
        let mut help = Help::new("Contents", 100, 30);
        let mut grid = Grid::new(100, 30);
        help.paint(&mut grid);
        let row = |r: u16| grid.row(r).iter().map(|c| c.text.clone()).collect::<String>();
        assert!(row(2).contains(" Help - Far "), "{}", row(2));
        assert!(row(3).contains("File and archive manager"));
        assert!(row(8).contains("Help file index"));
    }

    #[test]
    fn the_russian_help_lays_out_too() {
        let file = HelpFile::parse(RUS);
        let (list, fix, found, _) = layout(&file, "Contents", 90, file.color_char);
        assert!(found);
        assert_eq!(fix, 4);
        assert!(list.iter().any(|l| text(l).contains("Индекс файла помощи")));
    }

    #[test]
    fn a_setting_has_its_topic_or_its_group_s() {
        assert!(Help::found("Panel.ShowHidden", 100, 30).is_none() || Help::found("PanelSettings", 100, 30).is_some());
        assert!(Help::found("PanelSettings", 100, 30).is_some());
        assert!(Help::found("No.Such.Topic", 100, 30).is_none());
    }

    #[test]
    fn search_finds_topics_with_the_text() {
        {
            let mut params = SEARCH.lock().unwrap();
            params.text = "panel".into();
        }
        let mut help = Help::new("Contents", 100, 30);
        help.push_and_jump(FOUND_CONTENTS);
        assert!(help.list.len() > 5);
        assert!(text(&help.list[1]).starts_with("   ~"));
    }
}
