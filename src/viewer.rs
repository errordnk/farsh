//! F3: a viewer over the whole screen, as in Far: a status bar at the top, the
//! key bar at the bottom. Text is fresh, read-only (`fresh_view`); pictures (and
//! what else Som plays) are shown through SRP when farsh runs in Som; a binary
//! file is an error, not a hex view.

use std::path::Path;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use crossterm::style::Color;

use crate::fresh_view::EditorView;
use crate::grid::Grid;

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

pub struct Viewer {
    name: String,
    /// The text, in fresh (read-only).
    editor: Option<EditorView>,
    /// The file's size, for the status bar.
    size: u64,
    /// Rows of the space for a document on the last screen painted (it is scrolled by rows).
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
        let mut viewer = Self::empty(&path.display().to_string());
        viewer.path = path.to_path_buf();
        viewer.size = std::fs::metadata(path)?.len();
        if let Some(media) = crate::srp::Media::start(path, quick) {
            viewer.media = Some(media);
            return Ok(viewer);
        }
        if crate::qview::is_binary(path) {
            return Err(std::io::Error::other("a binary file: nothing to show"));
        }
        let options = crate::fresh_view::viewer_options_in("Viewer.Text", "Viewer.Text.Selected");
        let editor = EditorView::open(path, 80, 24, options).map_err(std::io::Error::other)?;
        viewer.editor = Some(editor);
        Ok(viewer)
    }

    fn empty(name: &str) -> Self {
        Self {
            name: name.into(),
            editor: None,
            size: 0,
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
    /// `bg` is the background of the placement's cells: what Som shows around a
    /// picture, and what it fills a widget with when it takes it from the cells.
    fn paint_media(&mut self, grid: &mut Grid, x: u16, y: u16, width: u16, height: u16, bg: Color) -> bool {
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
                grid.place_from(x, y, width, shown, self.media_top as u16, media.ids(), bg);
            }
            _ => grid.place(x, y, width, height, media.ids(), bg),
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

    /// The editor wants a frame without any input (a file just opened, highlighting).
    pub fn animating(&mut self) -> bool {
        self.editor.as_mut().is_some_and(|editor| editor.next_frame().is_some())
    }

    /// The file's name, as the screens list shows it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Scrolls a document (Som's, drawn by its rows); text is the editor's.
    fn scroll_document(&mut self, rows: isize) {
        // clamped when painted, when the number of rows and the height are known
        self.media_top = (self.media_top as isize + rows).max(0) as usize;
    }

    /// The wheel scrolls three lines at a time (the editor's own step in text).
    pub fn mouse(&mut self, event: MouseEvent) {
        if self.shows_document() {
            match event.kind {
                crossterm::event::MouseEventKind::ScrollUp => self.scroll_document(-3),
                crossterm::event::MouseEventKind::ScrollDown => self.scroll_document(3),
                _ => {}
            }
        } else if let Some(editor) = &mut self.editor {
            let _ = editor.event(Event::Mouse(event), 0, 1);
        }
    }

    /// The editor goes about its own business (a file read, highlighting, an
    /// animation): `true` when what it shows has changed.
    pub fn tick(&mut self) -> bool {
        self.editor.as_mut().is_some_and(|editor| editor.tick().unwrap_or(false))
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
        let plain = key.modifiers.is_empty();
        match key.code {
            KeyCode::Esc | KeyCode::F(3) | KeyCode::F(10) | KeyCode::Char('q') if plain => return Outcome::Closed,
            // the previous and next track of a list, as in somsrp
            KeyCode::Char('n' | 'N') if self.media.as_ref().is_some_and(crate::srp::Media::is_list) => {
                self.media.as_ref().inspect(|media| media.step(1));
            }
            KeyCode::Char('p' | 'P') if self.media.as_ref().is_some_and(crate::srp::Media::is_list) => {
                self.media.as_ref().inspect(|media| media.step(-1));
            }
            _ if self.shows_document() => {
                let page = self.page.max(1) as isize;
                match key.code {
                    KeyCode::Up => self.scroll_document(-1),
                    KeyCode::Down | KeyCode::Enter => self.scroll_document(1),
                    KeyCode::PageUp => self.scroll_document(-page),
                    KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_document(page),
                    KeyCode::Home => self.media_top = 0,
                    KeyCode::End => self.media_end = true,
                    _ => {}
                }
            }
            _ => {
                if let Some(editor) = &mut self.editor {
                    // Far's viewer: Enter goes a line down, Space a page
                    let key = match key.code {
                        KeyCode::Enter => KeyEvent::new(KeyCode::Down, key.modifiers),
                        KeyCode::Char(' ') if plain => KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                        _ => key,
                    };
                    let _ = editor.event(Event::Key(key), 0, 1);
                }
            }
        }
        Outcome::Open
    }

    /// A window of the screen for what Som shows (the quick view's text is
    /// fresh itself, drawn by `qview`), no bars.
    pub fn paint_area(&mut self, grid: &mut Grid, x: u16, y: u16, width: u16, height: u16, bg: Color) {
        self.page = height as usize;
        self.paint_media(grid, x, y, width, height, bg);
    }

    /// What the top bar says on its right: the file's size, or what Som is doing with it.
    fn top_note(&self) -> String {
        let Some(media) = &self.media else { return format!("{}", self.size) };
        let state = match media.status() {
            crate::srp::Status::Sending => "sending...",
            crate::srp::Status::Failed(_) => "failed",
            _ => "shown by Som",
        };
        let track = match media.position() {
            Some((index, len)) => format!(", track {}/{len} (n, p)", index + 1),
            None => String::new(),
        };
        let streams = match (&self.stream_list, &self.stream_note) {
            (_, Some(note)) if media.kind == crate::srp::Kind::Video => format!(", {note} (a, s)"),
            (Some(None), None) if media.kind == crate::srp::Kind::Video => ", a/s + digit: audio/subtitle stream".to_string(),
            (_, None) if media.kind == crate::srp::Kind::Video => ", a: audio, s: subtitles".to_string(),
            _ => String::new(),
        };
        format!("{state}{track}{streams}")
    }

    /// The whole screen as in Far: the status bar on the first row (the name,
    /// then on the right the size or what Som does with the file), the key bar
    /// on the last, the file between. Returns where the cursor goes.
    pub fn paint(&mut self, grid: &mut Grid) -> Option<(u16, u16)> {
        grid.fill_with(0, 0, grid.cols, grid.rows, FG(), BG());
        let keys = crate::settings::bool("Viewer.ShowKeyBar") && crate::settings::bool("Screen.KeyBar");
        let (top, bottom) = (1, u16::from(keys));
        if grid.rows < top + bottom + 1 {
            return None;
        }
        let height = grid.rows - top - bottom;
        let cols = grid.cols as usize;
        // the status bar
        let note = self.top_note();
        let name: String = self.name.chars().take(cols.saturating_sub(note.chars().count() + 2)).collect();
        grid.fill_with(0, 0, grid.cols, 1, BAR_FG(), BAR_BG());
        grid.put(0, 0, &name, BAR_FG(), BAR_BG());
        let note_x = cols.saturating_sub(note.chars().count() + 1);
        if note_x > name.chars().count() {
            grid.put(note_x as u16, 0, &note, BAR_FG(), BAR_BG());
        }
        // the file
        self.page = height as usize;
        let mut cursor = None;
        if !self.paint_media(grid, 0, top, grid.cols, height, BG())
            && let Some(editor) = &mut self.editor
        {
            editor.resize(grid.cols, height);
            cursor = editor.paint(grid, 0, top).ok().flatten();
        }
        // the key bar: only what works here has a label
        if keys {
            let mut labels = [""; 12];
            labels[2] = crate::help::msg("Quit", "Выход");
            labels[9] = crate::help::msg("Quit", "Выход");
            labels[11] = crate::help::msg("Screen", "Экраны");
            crate::panels::paint_labels(grid, &labels);
        }
        cursor
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
        let mut viewer = Viewer::empty("film.mkv");
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

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("farsh-view-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn rows(grid: &mut Grid) -> Vec<String> {
        (0..grid.rows).map(|y| (0..grid.cols).map(|x| grid.cell_mut(x, y).unwrap().text.clone()).collect()).collect()
    }

    #[test]
    fn the_screen_is_a_status_bar_the_file_and_the_key_bar() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let path = temp_file("t.txt", text.as_bytes());
        let mut viewer = Viewer::open(&path).unwrap();
        let mut grid = Grid::new(100, 12);
        viewer.tick();
        viewer.paint(&mut grid);
        let screen = rows(&mut grid);
        assert!(screen[0].contains("t.txt"), "the name on the status bar: {:?}", screen[0]);
        assert!(screen[0].trim_end().ends_with("792"), "the size on its right: {:?}", screen[0]);
        assert!(screen[1].starts_with("line 1 "), "{screen:#?}");
        assert!(screen[10].starts_with("line 10"), "the file down to the row above the key bar: {screen:#?}");
        assert!(screen[11].contains("3Quit") && screen[11].contains("10Quit") && screen[11].contains("12Scree"), "{:?}", screen[11]);
        // the bars are in Far's colors, the text in the viewer's
        assert_eq!(grid.cell_mut(0, 0).unwrap().bg, BAR_BG());
        assert_eq!(grid.cell_mut(30, 5).unwrap().bg, BG());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn keys_scroll_the_text_enter_and_space_as_in_far_and_esc_closes() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let path = temp_file("t.txt", text.as_bytes());
        let mut viewer = Viewer::open(&path).unwrap();
        let mut grid = Grid::new(40, 12);
        viewer.paint(&mut grid);
        viewer.key(key(KeyCode::Enter));
        viewer.tick();
        viewer.paint(&mut grid);
        assert!(rows(&mut grid)[1].starts_with("line 1"), "the cursor moved, the view did not yet");
        viewer.key(key(KeyCode::Char(' ')));
        viewer.key(key(KeyCode::Char(' ')));
        viewer.tick();
        viewer.paint(&mut grid);
        assert!(!rows(&mut grid)[1].starts_with("line 1 "), "a page down scrolls: {:?}", rows(&mut grid)[1]);
        // read-only: typing changes nothing
        viewer.key(key(KeyCode::Char('x')));
        assert!(!viewer.editor.as_mut().unwrap().session().modified());
        assert!(matches!(viewer.key(key(KeyCode::Esc)), Outcome::Closed));
        assert!(matches!(viewer.key(key(KeyCode::F(3))), Outcome::Closed));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_binary_file_is_an_error_not_a_hex_view() {
        let path = temp_file("b.bin", b"MZ\x90\x00\x03\x00\x00\x00");
        let error = Viewer::open(&path).err().expect("an error");
        assert!(error.to_string().contains("binary"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
