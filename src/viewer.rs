//! F3: a plain text viewer over the whole screen; pictures (and what else Som
//! plays) are shown through SRP when farsh runs in Som.

use std::io::Read;
use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::Grid;

/// Bigger files are shown up to this much from the start.
const LIMIT: u64 = 32 << 20;

// Far's viewer: light cyan on blue, the title bar black on cyan
#[allow(non_snake_case)]
fn BG() -> Color {
    crate::colors::bg("Viewer.Text")
}
#[allow(non_snake_case)]
fn FG() -> Color {
    crate::colors::fg("Viewer.Text")
}
#[allow(non_snake_case)]
fn BAR_FG() -> Color {
    crate::colors::fg("Viewer.Status")
}
#[allow(non_snake_case)]
fn BAR_BG() -> Color {
    crate::colors::bg("Viewer.Status")
}

/// Tabs to spaces up to the next multiple of `tab`.
fn expand_tabs(line: &str, tab: usize) -> String {
    let mut out = String::new();
    let mut col = 0;
    for c in line.chars() {
        if c == '\t' {
            let spaces = tab - col % tab;
            out.push_str(&" ".repeat(spaces));
            col += spaces;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

pub struct Viewer {
    name: String,
    lines: Vec<String>,
    truncated: bool,
    top: usize,
    left: usize,
    /// Rows of text on the last screen painted.
    page: usize,
    /// The placement of a file Som shows (SRP), instead of text.
    media: Option<crate::srp::Media>,
    /// The first row of a document's cells that is on the screen (a document is scrolled by its rows).
    media_top: usize,
    /// Scroll to the end of a document when it is next drawn (its rows are known then).
    media_end: bool,
    path: std::path::PathBuf,
    streams: crate::srp::Streams,
    /// `a` or `s` was pressed: the next digit is a stream number (when the
    /// video's streams cannot be listed).
    stream_key: Option<char>,
    /// The video's streams, read on the first `a` or `s` (`None` inside: they
    /// cannot be listed here).
    stream_list: Option<Option<crate::srp::streams::StreamList>>,
    /// What the last `a` or `s` chose, for the status line.
    stream_note: Option<String>,
}

pub enum Outcome {
    Open,
    Closed,
}

impl Viewer {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Self::open_for(path, false)
    }

    /// For the quick view: a file's transfer starts only after the cursor
    /// has stayed on it a moment.
    pub fn open_quick(path: &Path) -> std::io::Result<Self> {
        Self::open_for(path, true)
    }

    fn open_for(path: &Path, quick: bool) -> std::io::Result<Self> {
        if let Some(media) = crate::srp::Media::start(path, quick) {
            let mut viewer = Self::from_text(&path.display().to_string(), "");
            viewer.media = Some(media);
            viewer.path = path.to_path_buf();
            return Ok(viewer);
        }
        let mut bytes = Vec::new();
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        file.take(LIMIT).read_to_end(&mut bytes)?;
        let mut viewer = Self::from_text(&path.display().to_string(), &String::from_utf8_lossy(&bytes));
        viewer.truncated = size > LIMIT;
        Ok(viewer)
    }

    pub fn from_text(name: &str, text: &str) -> Self {
        Self {
            name: name.into(),
            lines: {
                // Viewer settings: Tab size (to the next tab stop)
                let tab = crate::settings::int("Viewer.TabSize").clamp(1, 512) as usize;
                text.lines().map(|line| expand_tabs(&line.replace('\r', ""), tab)).collect()
            },
            truncated: false,
            top: 0,
            left: 0,
            page: 20,
            media: None,
            media_top: 0,
            media_end: false,
            path: std::path::PathBuf::new(),
            streams: crate::srp::Streams::default(),
            stream_key: None,
            stream_list: None,
            stream_note: None,
        }
    }

    /// Paints a placement (or why there is none) into a rectangle; false if
    /// this viewer shows text.
    fn paint_media(&mut self, grid: &mut Grid, x: u16, y: u16, width: u16, height: u16) -> bool {
        let Some(media) = &self.media else { return false };
        match media.status() {
            crate::srp::Status::Failed(message) => {
                let shown: String = message.chars().take(width as usize).collect();
                grid.put(x, y, &shown, FG(), BG());
            }
            _ if media.kind == crate::srp::Kind::Markdown => {
                // the rows of cells Som has asked for so far, seen through a window of the screen's height
                let total = media.rows() as usize;
                if std::mem::take(&mut self.media_end) {
                    self.media_top = total.saturating_sub(height as usize);
                }
                // Som paints a little more than the rows it asked for: the last of them may be scrolled
                // up to the top, so that the end of the text can be seen
                self.media_top = self.media_top.min(total.saturating_sub(1));
                let shown = total.saturating_sub(self.media_top).min(height as usize) as u16;
                grid.place_from(x, y, width, shown, self.media_top as u16, media.ids());
            }
            _ => grid.place(x, y, width, height, media.ids()),
        }
        true
    }

    fn shows_document(&self) -> bool {
        self.media.as_ref().is_some_and(|media| media.kind == crate::srp::Kind::Markdown)
    }

    /// Something of the transfer to Som changed (status, the track of a list) that is not drawn yet.
    pub fn busy(&self) -> bool {
        self.media.as_ref().is_some_and(crate::srp::Media::busy)
    }

    /// The file's name, as the screens list shows it.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn scroll(&mut self, rows: isize) {
        if self.shows_document() {
            // clamped when painted, when the number of rows and the height are known
            self.media_top = (self.media_top as isize + rows).max(0) as usize;
            return;
        }
        let last = self.lines.len().saturating_sub(1) as isize;
        self.top = (self.top as isize + rows).clamp(0, last) as usize;
    }

    /// The wheel scrolls three lines at a time.
    pub fn mouse(&mut self, event: MouseEvent) {
        match event.kind {
            MouseEventKind::ScrollUp => self.scroll(-3),
            MouseEventKind::ScrollDown => self.scroll(3),
            _ => {}
        }
    }

    /// `a` goes to the next audio stream of a video, `s` to the next subtitle
    /// stream and then to none (as `somsrp -a <n> -s <n>`): the video starts
    /// again with them. Where the streams cannot be listed, `a` or `s` and a
    /// digit choose one by its number.
    fn choose_stream(&mut self, key: KeyCode) -> bool {
        let is_video = self.media.as_ref().is_some_and(|media| media.kind == crate::srp::Kind::Video);
        match (key, self.stream_key.take()) {
            (KeyCode::Char(letter @ ('a' | 's')), _) if is_video => {
                let path = self.path.clone();
                let list = self.stream_list.get_or_insert_with(|| crate::srp::streams::list(&path)).clone();
                match list {
                    Some(list) => self.next_stream(letter, &list),
                    None => self.stream_key = Some(letter),
                }
                true
            }
            (KeyCode::Char(digit), Some(letter)) if digit.is_ascii_digit() => {
                let number = Some(digit.to_digit(10).unwrap_or(0));
                match letter {
                    'a' => self.streams.audio = number,
                    _ => self.streams.subtitle = number,
                }
                self.restart();
                true
            }
            _ => false,
        }
    }

    fn next_stream(&mut self, letter: char, list: &crate::srp::streams::StreamList) {
        let (labels, what) = if letter == 'a' { (&list.audio, "audio") } else { (&list.subtitle, "subtitles") };
        let len = labels.len() as u32;
        let current = if letter == 'a' { self.streams.audio } else { self.streams.subtitle };
        let next = match (letter, current) {
            (_, _) if len == 0 => {
                self.stream_note = Some(format!("no {what}"));
                return;
            }
            ('a', _) if len == 1 => {
                self.stream_note = Some(format!("the only audio: {}", labels[0]));
                return;
            }
            // Som starts with the first audio stream FFmpeg deems best, usually the first
            ('a', current) => Some((current.unwrap_or(0) + 1) % len),
            (_, None) => Some(0),
            (_, Some(index)) if index + 1 < len => Some(index + 1),
            _ => None,
        };
        self.stream_note = Some(match next {
            Some(index) => format!("{what} {}/{len}: {}", index + 1, labels[index as usize]),
            None => "subtitles off".to_string(),
        });
        if letter == 'a' {
            self.streams.audio = next;
        } else {
            self.streams.subtitle = next;
        }
        self.restart();
    }

    /// The video again with the streams chosen; the old placement is dropped
    /// (and stopped) when the new one takes its place.
    fn restart(&mut self) {
        self.media = crate::srp::Media::start_with(&self.path, false, self.streams);
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.choose_stream(key.code) {
            return Outcome::Open;
        }
        let page = self.page.max(1) as isize;
        match key.code {
            KeyCode::Esc | KeyCode::F(3) | KeyCode::F(10) | KeyCode::Char('q') => return Outcome::Closed,
            // the previous and next track of a list, as in somsrp
            KeyCode::Char('n' | 'N') if self.media.as_ref().is_some_and(crate::srp::Media::is_list) => {
                self.media.as_ref().inspect(|media| media.step(1));
            }
            KeyCode::Char('p' | 'P') if self.media.as_ref().is_some_and(crate::srp::Media::is_list) => {
                self.media.as_ref().inspect(|media| media.step(-1));
            }
            KeyCode::Up => self.scroll(-1),
            KeyCode::Down | KeyCode::Enter => self.scroll(1),
            KeyCode::PageUp => self.scroll(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll(page),
            KeyCode::Home => {
                self.media_top = 0;
                self.top = 0;
                self.left = 0;
            }
            KeyCode::End => {
                self.media_end = true;
                self.top = self.lines.len().saturating_sub(self.page);
            }
            KeyCode::Left => self.left = self.left.saturating_sub(8),
            KeyCode::Right => self.left += 8,
            _ => {}
        }
        Outcome::Open
    }

    /// The text in a window of the screen, wrapped at its width (the quick
    /// view's viewer: Far's ViewerIsWrap is on by default), no bars.
    pub fn paint_area(&mut self, grid: &mut Grid, x: u16, y: u16, width: u16, height: u16) {
        self.page = height as usize;
        if self.paint_media(grid, x, y, width, height) {
            return;
        }
        let width = width.max(1) as usize;
        let mut row = 0u16;
        'lines: for line in self.lines.iter().skip(self.top) {
            let chars: Vec<char> = line.chars().collect();
            let mut start = 0;
            loop {
                if row >= height {
                    break 'lines;
                }
                let piece: String = chars[start..(start + width).min(chars.len())].iter().collect();
                grid.put(x, y + row, &piece, FG(), BG());
                row += 1;
                start += width;
                if start >= chars.len() {
                    break;
                }
            }
        }
    }

    pub fn paint(&mut self, grid: &mut Grid) {
        grid.fill_with(0, 0, grid.cols, grid.rows, FG(), BG());
        // Som paints a document over everything below its first cell, not only over the
        // cells it has: without bars to cover the whole screen is its place (as in somsrp)
        if self.shows_document() && matches!(self.media.as_ref().map(crate::srp::Media::status), Some(crate::srp::Status::Sent | crate::srp::Status::Sending)) {
            self.page = grid.rows as usize;
            self.paint_media(grid, 0, 0, grid.cols, grid.rows);
            return;
        }
        if grid.rows < 3 {
            return;
        }
        self.page = grid.rows as usize - 2;
        let title: String = self.name.chars().take(grid.cols as usize).collect();
        grid.fill_with(0, 0, grid.cols, 1, BAR_FG(), BAR_BG());
        grid.put(0, 0, &title, BAR_FG(), BAR_BG());
        if self.paint_media(grid, 0, 1, grid.cols, grid.rows - 2) {
            let state = match self.media.as_ref().map(crate::srp::Media::status) {
                Some(crate::srp::Status::Sending) => "sending...",
                Some(crate::srp::Status::Failed(_)) => "failed",
                _ => "shown by Som",
            };
            let track = match self.media.as_ref().and_then(crate::srp::Media::position) {
                Some((index, len)) => format!(" track {}/{len} (n, p) --", index + 1),
                None => String::new(),
            };
            let streams = match (self.media.as_ref(), &self.stream_list, &self.stream_note) {
                (Some(media), _, Some(note)) if media.kind == crate::srp::Kind::Video => format!(" -- {note} (a, s)"),
                (Some(media), Some(None), None) if media.kind == crate::srp::Kind::Video => " -- a/s + digit: audio/subtitle stream".to_string(),
                (Some(media), _, None) if media.kind == crate::srp::Kind::Video => " -- a: audio, s: subtitles".to_string(),
                _ => String::new(),
            };
            let status = format!(" {state}{track}{streams} -- Esc, F3 or F10 closes");
            grid.fill_with(0, grid.rows - 1, grid.cols, 1, BAR_FG(), BAR_BG());
            grid.put(0, grid.rows - 1, &status, BAR_FG(), BAR_BG());
            return;
        }
        for (row, line) in self.lines.iter().skip(self.top).take(self.page).enumerate() {
            let shown: String = line.chars().skip(self.left).take(grid.cols as usize).collect();
            grid.put(0, 1 + row as u16, &shown, FG(), BG());
        }
        let percent = if self.lines.len() <= self.page { 100 } else { (self.top + self.page).min(self.lines.len()) * 100 / self.lines.len() };
        let note = if self.truncated { " (cut)" } else { "" };
        let status = format!(" Line {}/{}  {}%{} -- Esc, F3 or F10 closes", self.top + 1, self.lines.len().max(1), percent, note);
        grid.fill_with(0, grid.rows - 1, grid.cols, 1, BAR_FG(), BAR_BG());
        grid.put(0, grid.rows - 1, &status, BAR_FG(), BAR_BG());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_and_s_go_through_the_streams_the_video_has() {
        let list = crate::srp::streams::StreamList {
            audio: vec!["rus, AC3".into(), "eng, AAC".into()],
            subtitle: vec!["rus, SUBRIP".into()],
        };
        let mut viewer = Viewer::from_text("film.mkv", "");
        viewer.path = "film.mkv".into();
        viewer.next_stream('a', &list);
        assert_eq!(viewer.streams.audio, Some(1));
        assert_eq!(viewer.stream_note.as_deref(), Some("audio 2/2: eng, AAC"));
        viewer.next_stream('a', &list);
        assert_eq!(viewer.streams.audio, Some(0));
        viewer.next_stream('s', &list);
        assert_eq!((viewer.streams.subtitle, viewer.stream_note.as_deref()), (Some(0), Some("subtitles 1/1: rus, SUBRIP")));
        viewer.next_stream('s', &list);
        assert_eq!((viewer.streams.subtitle, viewer.stream_note.as_deref()), (None, Some("subtitles off")));

        let lone = crate::srp::streams::StreamList { audio: vec!["AAC".into()], subtitle: vec![] };
        viewer.next_stream('a', &lone);
        assert_eq!(viewer.stream_note.as_deref(), Some("the only audio: AAC"));
        viewer.next_stream('s', &lone);
        assert_eq!(viewer.stream_note.as_deref(), Some("no subtitles"));
    }

    #[test]
    fn scrolling_stays_inside_the_text_and_esc_closes() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let mut viewer = Viewer::from_text("t.txt", &text);
        let mut grid = Grid::new(40, 12);
        viewer.paint(&mut grid);
        assert_eq!(grid.cell_mut(0, 1).unwrap().text, "l");
        viewer.key(key(KeyCode::PageDown));
        viewer.paint(&mut grid);
        assert_eq!(viewer.top, 10);
        viewer.key(key(KeyCode::Home));
        viewer.key(key(KeyCode::Up));
        assert_eq!(viewer.top, 0);
        viewer.key(key(KeyCode::End));
        viewer.key(key(KeyCode::PageDown));
        assert!(viewer.top < 100);
        assert!(matches!(viewer.key(key(KeyCode::Esc)), Outcome::Closed));
    }

    #[test]
    fn a_file_is_shown_with_tabs_expanded_and_bad_utf8_replaced() {
        let path = std::env::temp_dir().join(format!("farsh-view-{}.txt", std::process::id()));
        std::fs::write(&path, b"a\tb\r\nbad \xff byte\n").unwrap();
        let viewer = Viewer::open(&path).unwrap();
        assert_eq!(viewer.lines[0], "a       b", "Far's default tab size is 8");
        assert!(viewer.lines[1].contains('\u{fffd}'));
        std::fs::remove_file(path).unwrap();
    }
}
