//! Far style panels: two directory listings and a key bar, drawn the way
//! Far Manager 3 draws them (filelist.cpp `ShowFileList`/`ShowList`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::style::Color;

use crate::grid::{Cell, Grid};
use crate::listing::{self, Entry};
use crate::sorting::{self, SortMode, Sorting};

// Far's default colors, as Far's console attributes reach the terminal.
/// Blue: the panels' background.
#[allow(non_snake_case)]
fn BG() -> Color {
    crate::colors::bg("Panel.Text")
}
/// Light cyan: panel text, frames, totals.
#[allow(non_snake_case)]
fn TEXT() -> Color {
    crate::colors::fg("Panel.Text")
}
/// Yellow: column titles, the sort letter, marked names.
#[allow(non_snake_case)]
fn TITLE() -> Color {
    crate::colors::fg("Panel.Title.Column")
}
#[allow(non_snake_case)]
fn CURSOR_BG() -> Color {
    crate::colors::bg("Panel.Cursor")
}
#[allow(non_snake_case)]
fn BLACK() -> Color {
    crate::colors::fg("Panel.Cursor")
}

/// Far's panel key bar, plain and with Shift, Ctrl or Alt held
/// (farlang.templ.m4's MF1.., MShiftF1.., MCtrlF1.., MAltF1..): English, Russian.
type KeyBarLabels = [(&'static str, &'static str); 12];
const KEY_BARS: [KeyBarLabels; 4] = {
    const E: (&str, &str) = ("", "");
    [
        [
            ("Help", "Помощь"),
            ("UserMn", "ПользМ"),
            ("View", "Просм"),
            ("Edit", "Редакт"),
            ("Copy", "Копир"),
            ("RenMov", "Перен"),
            ("MkFold", "Папка"),
            ("Delete", "Удален"),
            ("ConfMn", "КонфМн"),
            ("Quit", "Выход"),
            ("Plugin", "Плагины"),
            ("Screen", "Экраны"),
        ],
        // Shift
        [
            ("Add", "Добавл"),
            ("Extrct", "Распак"),
            ("ArcCmd", "АрхКом"),
            ("Edit…", "Редак…"),
            ("Copy", "Копир"),
            ("Rename", "Переим"),
            E,
            ("Delete", "Удален"),
            ("Save", "Сохран"),
            ("Last", "Послдн"),
            ("Group", "Группы"),
            ("SelUp", "Выбран"),
        ],
        // Ctrl
        [
            ("Left", "Левая"),
            ("Right", "Правая"),
            ("Name", "Имя"),
            ("Extens", "Расшир"),
            ("Write", "Запись"),
            ("Size", "Размер"),
            ("Unsort", "Несорт"),
            ("Creatn", "Создан"),
            ("Access", "Доступ"),
            ("Descr", "Описан"),
            ("Owner", "Владел"),
            ("Sort", "Сорт"),
        ],
        // Alt
        [
            ("Left", "Левая"),
            ("Right", "Правая"),
            ("View…", "Смотр…"),
            ("Edit…", "Редак…"),
            ("Print", "Печать"),
            ("MkLink", "Ссылка"),
            ("Find", "Искать"),
            ("Histry", "Истор"),
            ("Video", "Видео"),
            ("Tree", "Дерево"),
            ("ViewHs", "ИстПр"),
            ("FoldHs", "ИстПап"),
        ],
    ]
};

/// The key bar's labels for the modifiers held: one modifier changes them,
/// two or more leave the plain ones (farsh has no keys with two modifiers).
pub(crate) fn key_bar_labels(held: u8) -> impl Iterator<Item = &'static str> {
    use crate::modkeys::{ALT, CTRL, SHIFT};
    let russian = crate::lang::russian();
    let set = match held & (SHIFT | CTRL | ALT) {
        SHIFT => 1,
        CTRL => 2,
        ALT => 3,
        _ => 0,
    };
    KEY_BARS[set].iter().map(move |&(en, ru)| if russian { ru } else { en })
}

/// Far's ten view modes (config.cpp's InitialModes): Ctrl+1..Ctrl+9, Ctrl+0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewMode {
    /// Three columns of names, extensions aligned.
    Brief,
    /// Two columns of names.
    Medium,
    /// Name, Size, Date, Time.
    Full,
    /// Name and Size.
    Wide,
    /// Name, Size, Allocated, Write, Created, Accessed, Attr: the whole screen.
    Detailed,
    /// Name (40%) and Description.
    Descriptions,
    /// Name, Size, Description (70%): the whole screen.
    LongDescriptions,
    /// Name, Size, Owner.
    Owners,
    /// Name, Size, number of hard links.
    Links,
    /// Name with the mark, Size with digit groups, Date.
    AlternativeFull,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Col {
    Name,
    Size,
    /// size_compressed: the space taken on the disk.
    Allocated,
    Date,
    Time,
    Write,
    Created,
    Accessed,
    Attributes,
    Description,
    Owner,
    Links,
    /// DE: the change time.
    Changed,
}

impl Col {
    /// Far's column titles (MColumn*).
    fn title(self) -> &'static str {
        match self {
            Col::Name => "Name",
            Col::Size => "Size",
            Col::Allocated => "Allocated",
            Col::Date => "Date",
            Col::Time => "Time",
            Col::Write => "Write",
            Col::Created => "Created",
            Col::Accessed => "Accessed",
            Col::Attributes => "Attr",
            Col::Description => "Description",
            Col::Owner => "Owner",
            Col::Links => "NmL",
            Col::Changed => "Change",
        }
    }

    /// panelmix.cpp's ColumnInfo default widths (0: shares what is left).
    fn default_width(self) -> usize {
        match self {
            Col::Name | Col::Description | Col::Owner => 0,
            Col::Size | Col::Allocated | Col::Attributes => 6,
            Col::Date => 8,
            Col::Time => 5,
            Col::Write | Col::Created | Col::Accessed | Col::Changed => 14,
            Col::Links => 3,
        }
    }
}

/// A column's width in a mode: its type's default, so many cells, or so
/// many percent of what is left (col_width::percent).
#[derive(Clone, Copy, Debug, PartialEq)]
enum W {
    Default,
    Cells(usize),
    Percent(usize),
}

use W::{Cells, Default as Auto, Percent};

/// A mode as Far keeps it (PanelViewSettings): the column types and
/// widths of the list and of the status line as Far writes them
/// (`N,S,D,T` / `0,6,0,5`, `40%`), the PVS_* flags, a name.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeText {
    pub name: String,
    pub columns: String,
    pub widths: String,
    pub status: String,
    pub status_widths: String,
    pub flags: u32,
}

/// PVS_FULLSCREEN, PVS_ALIGNEXTENSIONS, PVS_FOLDERALIGNEXTENSIONS,
/// PVS_FOLDERUPPERCASE, PVS_FILELOWERCASE, PVS_FILEUPPERTOLOWERCASE.
pub const PVS_FULLSCREEN: u32 = 1;
pub const PVS_ALIGNEXTENSIONS: u32 = 2;
pub const PVS_FOLDERALIGNEXTENSIONS: u32 = 4;
pub const PVS_FOLDERUPPERCASE: u32 = 8;
pub const PVS_FILELOWERCASE: u32 = 16;
pub const PVS_FILEUPPERTOLOWERCASE: u32 = 32;

/// A column with its flags: COLFLAGS_MARK (`NM`), COLFLAGS_GROUPDIGITS (`SC`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Column {
    col: Col,
    width: W,
    mark: bool,
    group: bool,
}

/// Far's column types (panelmix.cpp's ColumnInfo) and their flags.
fn parse_columns(types: &str, widths: &str) -> Vec<Column> {
    let widths: Vec<&str> = widths.split(',').map(str::trim).collect();
    types
        .split(',')
        .map(str::trim)
        .enumerate()
        .filter_map(|(i, t)| {
            let t = t.to_uppercase();
            let (col, flags) = match t.get(..2) {
                Some("DM") => (Col::Write, &t[2..]),
                Some("DC") => (Col::Created, &t[2..]),
                Some("DA") => (Col::Accessed, &t[2..]),
                Some("DE") => (Col::Changed, &t[2..]),
                Some("LN") => (Col::Links, &t[2..]),
                _ => {
                    let col = match t.chars().next()? {
                        'N' => Col::Name,
                        'S' => Col::Size,
                        'P' => Col::Allocated,
                        'D' => Col::Date,
                        'T' => Col::Time,
                        'A' => Col::Attributes,
                        'Z' => Col::Description,
                        'O' => Col::Owner,
                        _ => return None,
                    };
                    (col, &t[1..])
                }
            };
            let w = widths.get(i).copied().unwrap_or("0");
            let width = match w.strip_suffix('%') {
                Some(p) => Percent(p.parse().unwrap_or(0)),
                None => match w.parse::<usize>() {
                    Ok(0) | Err(_) => Auto,
                    Ok(n) => Cells(n),
                },
            };
            Some(Column { col, width, mark: col == Col::Name && flags.contains('M'), group: col == Col::Size && flags.contains('C') })
        })
        .collect()
}

impl ViewMode {
    /// Far's number of the mode (Ctrl+N; 0 is Alternative full).
    fn number(self) -> usize {
        Panels::VIEWS.iter().position(|&m| m == self).unwrap_or(2)
    }

    /// Far's defaults (config.cpp's ResetViewModes).
    pub fn default_text(self) -> ModeText {
        let (columns, widths, status, status_widths, flags) = match self {
            ViewMode::AlternativeFull => ("NM,SC,D", "0,10,0", "N", "0", PVS_ALIGNEXTENSIONS),
            ViewMode::Brief => ("N,N,N", "0,0,0", "N,S,D,T", "0,6,0,5", PVS_ALIGNEXTENSIONS),
            ViewMode::Medium => ("N,N", "0,0", "N,S,D,T", "0,6,0,5", 0),
            ViewMode::Full => ("N,S,D,T", "0,6,0,5", "N", "0", PVS_ALIGNEXTENSIONS),
            ViewMode::Wide => ("N,S", "0,6", "N,S,D,T", "0,6,0,5", 0),
            ViewMode::Detailed => ("N,S,P,DM,DC,DA,A", "0,6,6,14,14,14,0", "N", "0", PVS_ALIGNEXTENSIONS | PVS_FULLSCREEN),
            ViewMode::Descriptions => ("N,Z", "40%,0", "N,S,D,T", "0,6,0,5", PVS_ALIGNEXTENSIONS),
            ViewMode::LongDescriptions => ("N,S,Z", "0,6,70%", "N", "0", PVS_ALIGNEXTENSIONS | PVS_FULLSCREEN),
            ViewMode::Owners => ("N,S,O", "0,6,15", "N,S,D,T", "0,6,0,15", PVS_ALIGNEXTENSIONS),
            ViewMode::Links => ("N,S,LN", "0,6,3", "N,S,D,T", "0,6,0,5", PVS_ALIGNEXTENSIONS),
        };
        ModeText { name: String::new(), columns: columns.into(), widths: widths.into(), status: status.into(), status_widths: status_widths.into(), flags }
    }

    fn key(self) -> String {
        format!("Panel.ViewModes.Mode{}", self.number())
    }

    /// The mode as Options > File panel modes left it (else Far's default).
    pub fn text(self) -> ModeText {
        let default = self.default_text();
        let Some(serde_json::Value::Object(map)) = crate::settings::value(&self.key()) else { return default };
        let get = |k: &str, d: &str| map.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
        ModeText {
            name: get("Name", ""),
            columns: get("Columns", &default.columns),
            widths: get("ColumnWidths", &default.widths),
            status: get("StatusColumns", &default.status),
            status_widths: get("StatusColumnWidths", &default.status_widths),
            flags: map.get("Flags").and_then(|v| v.as_u64()).map_or(default.flags, |f| f as u32),
        }
    }

    pub fn set_text(self, text: Option<&ModeText>) {
        let value = match text {
            None => serde_json::Value::Null,
            Some(t) => serde_json::json!({
                "Name": t.name,
                "Columns": t.columns,
                "ColumnWidths": t.widths,
                "StatusColumns": t.status,
                "StatusColumnWidths": t.status_widths,
                "Flags": t.flags,
            }),
        };
        crate::settings::set(&self.key(), value);
    }

    fn list(self) -> Vec<Column> {
        let t = self.text();
        parse_columns(&t.columns, &t.widths)
    }

    /// The columns and their widths.
    fn columns(self) -> Vec<(Col, W)> {
        self.list().into_iter().map(|c| (c.col, c.width)).collect()
    }

    /// The status line under the list.
    fn status(self) -> Vec<(Col, W)> {
        let t = self.text();
        parse_columns(&t.status, &t.status_widths).into_iter().map(|c| (c.col, c.width)).collect()
    }

    /// PVS_FULLSCREEN: the panel takes the whole screen.
    pub fn full_screen(self) -> bool {
        self.text().flags & PVS_FULLSCREEN != 0
    }

    /// What the mode shows that the listing does not read by itself.
    fn needs(self) -> Vec<crate::sorting::SortMode> {
        use crate::sorting::SortMode;
        let mut cols: Vec<Col> = self.columns().into_iter().map(|c| c.0).collect();
        cols.extend(self.status().into_iter().map(|c| c.0));
        let mut needs = Vec::new();
        for (col, mode) in [(Col::Description, SortMode::Descriptions), (Col::Owner, SortMode::Owner), (Col::Links, SortMode::Links)] {
            if cols.contains(&col) {
                needs.push(mode);
            }
        }
        needs
    }
}

/// Far's `PrepareColumnWidths`: a zero width takes the type's default; the
/// percent columns share their part of what is left, the zero ones the
/// rest; columns that do not fit are dropped from the end, the last one cut.
fn layout(spec: &[(Col, W)], text_width: usize) -> Vec<(Col, usize)> {
    let mut zero = 0isize;
    let (mut total_percent, mut percent_count) = (0isize, 0isize);
    let mut total = spec.len() as isize - 1;
    let mut cols: Vec<(Col, isize, bool)> = spec
        .iter()
        .map(|&(col, w)| match w {
            Auto | Cells(0) => {
                let width = col.default_width() as isize;
                if width == 0 {
                    zero += 1;
                }
                total += width;
                (col, width, false)
            }
            Cells(n) => {
                total += n as isize;
                (col, n as isize, false)
            }
            Percent(p) => {
                total_percent += p as isize;
                percent_count += 1;
                (col, p as isize, true)
            }
        })
        .collect();
    let width = text_width as isize;
    let mut extra = width - total;
    if percent_count > 0 {
        let extra_percent = if total_percent > 100 || zero == 0 { extra } else { extra * total_percent / 100 };
        let mut used = 0;
        for col in cols.iter_mut().filter(|c| c.2) {
            let w = if percent_count > 1 { extra_percent * col.1 / total_percent } else { extra_percent - used };
            let w = w.max(1);
            used += w;
            col.1 = w;
            col.2 = false;
            percent_count -= 1;
        }
        extra -= used;
    }
    for col in cols.iter_mut().filter(|c| c.1 == 0) {
        let auto = (extra / zero).max(1);
        col.1 = auto;
        extra -= auto;
        zero -= 1;
    }
    let mut cols: Vec<(Col, usize)> = cols.into_iter().map(|(c, w, _)| (c, w.max(0) as usize)).collect();
    loop {
        let sum = cols.iter().map(|c| c.1 as isize).sum::<isize>() + cols.len() as isize - 1;
        if sum <= width {
            break;
        }
        let last = cols.len() - 1;
        if cols.len() <= 1 {
            cols[0].1 = width.max(0) as usize;
            break;
        }
        let without = sum - cols[last].1 as isize;
        if width >= without {
            cols[last].1 = (width - without) as usize;
            break;
        }
        cols.pop();
    }
    cols
}

/// Far's `PrepareStripes`: how many columns make one stripe of the list.
fn stripe_len(cols: &[(Col, usize)]) -> usize {
    let n = cols.len();
    (1..=n / 2)
        .find(|&stride| n % stride == 0 && cols.chunks(stride).all(|c| c.iter().map(|x| x.0).eq(cols[..stride].iter().map(|x| x.0))))
        .unwrap_or(n.max(1))
}

/// An archive the panel shows (arclite's plugin panel): the file, what is
/// in it, the folder inside (`""` is the top).
#[derive(Clone, Debug, PartialEq)]
pub struct ArchiveView {
    pub file: PathBuf,
    pub listing: crate::archive::Listing,
    pub dir: String,
    pub password: Option<String>,
}

impl ArchiveView {
    /// arclite's PanelTitle: `ArcLite:zip:a.zip`, then `:\folder\inside`.
    pub fn title(&self) -> String {
        let name = self.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut title = format!("ArcLite:{}:{name}", self.listing.chain_name());
        if !self.dir.is_empty() {
            title.push_str(&format!(":\\{}", self.dir.replace('/', "\\")));
        }
        title
    }

    /// The path inside the archive of `name` in the current folder.
    pub fn inner(&self, name: &str) -> String {
        if self.dir.is_empty() { name.to_string() } else { format!("{}/{name}", self.dir) }
    }
}

/// What F3/F5/F6 asked of an archive the panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivePick {
    pub view: ArchiveView,
    /// Paths inside the archive.
    pub items: Vec<String>,
}

/// What a panel is (Far's panel_type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Files,
    /// Ctrl+L: about the other panel's disk and folder.
    Info,
    /// Ctrl+Q: the other panel's file under the cursor.
    QuickView,
}

pub struct Panel {
    pub dir: PathBuf,
    pub kind: Kind,
    /// Ctrl+F1 / Ctrl+F2 hide and show it.
    pub visible: bool,
    /// Ctrl+N off: the short (8.3) names where there are any.
    pub short_names: bool,
    /// Shown instead of `dir` while an archive is open in the panel.
    pub archive: Option<ArchiveView>,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    /// Names of the marked entries.
    pub marked: HashSet<String>,
    /// Far's PrevSelected: the marks before the last group operation, and
    /// the folder they were in (Ctrl+M brings them back).
    prev_marked: (PathBuf, HashSet<String>),
    pub mode: ViewMode,
    pub sorting: Sorting,
    top: usize,
    /// List rows and stripes at the last paint.
    height: usize,
    stripes: usize,
    /// Where it was last drawn: column, row, width, height.
    area: (u16, u16, u16, u16),
    /// Screen columns of each stripe at the last paint (first, last).
    stripe_cols: Vec<(u16, u16)>,
    /// Which filter marks apply (Far's panel_left / panel_right): the
    /// panel keeps them when the panels swap places.
    pub filter_area: usize,
    /// The first row of the list: 2 under the column titles, 1 without them.
    list_top: u16,
    /// Free space of the folder's volume, read with the folder.
    free: Option<u64>,
    /// The mode's flags, marks and digit groups, read once a paint.
    mode_flags: (u32, bool, bool),
}

impl Panel {
    pub fn new(dir: PathBuf) -> Self {
        let mut panel = Self {
            dir,
            kind: Kind::Files,
            visible: true,
            short_names: false,
            archive: None,
            entries: Vec::new(),
            cursor: 0,
            marked: HashSet::new(),
            prev_marked: (PathBuf::new(), HashSet::new()),
            mode: ViewMode::Medium,
            sorting: Sorting::default(),
            top: 0,
            height: 20,
            stripes: 1,
            area: (0, 0, 0, 0),
            stripe_cols: Vec::new(),
            filter_area: 0,
            list_top: 2,
            free: None,
            mode_flags: (PVS_ALIGNEXTENSIONS, false, false),
        };
        panel.reload(None);
        panel
    }

    /// Reads the directory: `..` first, then folders, then files, in the
    /// panel's sort order; the cursor goes to `select` if it is there, else
    /// to the top. Marks are dropped.
    pub fn reload(&mut self, select: Option<&str>) {
        if let Some(view) = &self.archive {
            // arclite's GetFindData, with OPIF_ADDDOTS: `..` even at the top
            let mut entries: Vec<Entry> = view
                .listing
                .children(&view.dir)
                .iter()
                .enumerate()
                .map(|(position, item)| Entry {
                    name: item.path.rsplit('/').next().unwrap_or(&item.path).to_string(),
                    is_dir: item.is_dir,
                    size: item.size,
                    modified: item.modified,
                    position,
                    ..Entry::default()
                })
                .collect();
            entries.insert(0, Entry::up(None));
            self.marked.clear();
            sorting::sort(&mut entries, &self.sorting, &self.marked, &self.dir);
            self.entries = entries;
            self.cursor = select.and_then(|name| self.entries.iter().position(|e| e.name == name)).unwrap_or(0);
            self.top = 0;
            return;
        }
        let mut entries = listing::read(&self.dir);
        // Panel settings: Show hidden and system files
        if !crate::settings::bool("Panel.ShowHidden") {
            entries.retain(|entry| !entry.hidden);
        }
        if crate::settings::bool("Panel.Layout.FreeInfo") {
            self.free = crate::drives::free_space(&self.dir);
        }
        // Ctrl+I: what the panel's filter marks hide
        let area = self.filter_area;
        crate::filter::with(|filters| {
            if filters.enabled(area) {
                let now = std::time::SystemTime::now();
                entries.retain(|entry| filters.shows(entry, area, now));
            }
        });
        // Panel settings: Show ".." in root folders
        if self.dir.parent().is_some() || crate::settings::bool("Interface.ShowDotsInRoot") {
            let modified = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
            entries.insert(0, Entry::up(modified));
        }
        let needs = self.mode.needs();
        if !needs.is_empty() {
            crate::sorting::fill(&mut entries, &needs, &self.dir);
        }
        self.marked.clear();
        sorting::sort(&mut entries, &self.sorting, &self.marked, &self.dir);
        self.entries = entries;
        self.cursor = select.and_then(|name| self.entries.iter().position(|e| e.name == name)).unwrap_or(0);
        self.top = 0;
    }

    /// Reads what the view mode shows that the listing does not (owners,
    /// links, descriptions), as Far reads them only when shown.
    fn fill_for_view(&mut self) {
        let needs = self.mode.needs();
        if self.archive.is_none() && !needs.is_empty() {
            crate::sorting::fill(&mut self.entries, &needs, &self.dir);
        }
    }

    /// Ctrl+F3..Ctrl+F11: Far's `SetSortMode` (the same mode again flips).
    pub fn set_sort(&mut self, mode: SortMode) {
        self.sorting.choose(mode);
        self.resort();
    }

    /// Sorts again by the panel's settings; the cursor stays on its entry.
    pub fn resort(&mut self) {
        let keep = self.current().map(|e| e.name.clone());
        let mut entries = std::mem::take(&mut self.entries);
        sorting::sort(&mut entries, &self.sorting, &self.marked, &self.dir);
        self.entries = entries;
        if let Some(index) = keep.and_then(|name| self.entries.iter().position(|e| e.name == name)) {
            self.cursor = index;
        }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    fn page(&self) -> usize {
        (self.stripes * self.height).max(1)
    }

    /// Far's `FileList::FindPartName`: the cursor to the next (`direct` 1) or
    /// previous (-1) name that begins with `typed` (from the cursor itself unless
    /// `next`), wrapping round; `..` is never found. Whether there is one.
    pub fn find_part(&mut self, typed: &str, next: bool, direct: isize) -> bool {
        let (mask, folders_only) = crate::fastfind::mask(typed);
        let hit = |entry: &Entry| !entry.is_up() && (!folders_only || entry.is_dir) && crate::masks::name_matches(&mask, &entry.name);
        let n = self.entries.len() as isize;
        let cursor = self.cursor as isize;
        let mut found = None;
        let mut i = cursor + if next { direct } else { 0 };
        while i >= 0 && i < n {
            if hit(&self.entries[i as usize]) {
                found = Some(i);
                break;
            }
            i += direct;
        }
        if found.is_none() {
            // round the end to the start (or the other way) up to the cursor
            let mut i = if direct > 0 { 0 } else { n - 1 };
            while if direct > 0 { i < cursor } else { i > cursor } {
                if hit(&self.entries[i as usize]) {
                    found = Some(i);
                    break;
                }
                i += direct;
            }
        }
        let Some(i) = found else { return false };
        self.cursor = i as usize;
        self.top = self.cursor.saturating_sub(self.height.saturating_sub(1) / 2);
        true
    }

    /// Far's PgUp / PgDn: a page less one, the list moving with the cursor.
    fn page_move(&mut self, down: bool) {
        let n = self.page().saturating_sub(1).max(1);
        if down {
            self.top += n;
            self.move_cursor(n as isize);
        } else {
            self.top = self.top.saturating_sub(n);
            self.move_cursor(-(n as isize));
        }
    }

    /// Far's Left / Right: one stripe over.
    fn stripe_move(&mut self, right: bool) {
        let h = self.height.max(1);
        if right {
            if self.cursor + h < self.entries.len() && self.cursor - self.top.min(self.cursor) >= (self.stripes - 1) * h {
                self.top += h;
            }
            self.move_cursor(h as isize);
        } else {
            if self.top >= h && self.cursor.saturating_sub(self.top) < h {
                self.top -= h;
            }
            self.move_cursor(-(h as isize));
        }
    }

    /// Far's `CorrectPosition`: the cursor inside the list, the list on the cursor.
    fn correct_position(&mut self) {
        let n = self.entries.len();
        if n == 0 {
            self.cursor = 0;
            self.top = 0;
            return;
        }
        let page = self.page();
        if self.top + page > n {
            self.top = n.saturating_sub(page);
        }
        self.cursor = self.cursor.min(n - 1);
        self.top = self.top.min(n - 1);
        if self.cursor < self.top {
            self.top = self.cursor;
        }
        if self.cursor > self.top + page - 1 {
            self.top = self.cursor + 1 - page;
        }
    }

    /// Marks or unmarks the entry under the cursor and moves down (Insert).
    pub fn toggle_mark(&mut self) {
        self.toggle_mark_at(self.cursor);
        self.move_cursor(1);
        if self.sorting.selected_first {
            self.resort();
        }
    }

    fn toggle_mark_at(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index).filter(|e| !e.is_up()) {
            let name = entry.name.clone();
            if !self.marked.remove(&name) {
                self.marked.insert(name);
            }
        }
    }

    /// Far's SaveSelection.
    pub fn save_selection(&mut self) {
        self.prev_marked = (self.dir.clone(), self.marked.clone());
    }

    /// Ctrl+M (Far's RestoreSelection): the marks saved before the last group
    /// operation come back, and the ones replaced are saved in their place.
    pub fn restore_selection(&mut self) {
        let prev = if self.prev_marked.0 == self.dir { std::mem::take(&mut self.prev_marked.1) } else { HashSet::new() };
        let names: HashSet<&str> = self.entries.iter().filter(|e| !e.is_up()).map(|e| e.name.as_str()).collect();
        let prev: HashSet<String> = prev.into_iter().filter(|n| names.contains(n.as_str())).collect();
        let replaced = std::mem::replace(&mut self.marked, prev);
        self.prev_marked = (self.dir.clone(), replaced);
        if self.sorting.selected_first {
            self.resort();
        }
    }

    /// Select group / Deselect group (Far's SelectFiles with SELECT_ADD and
    /// SELECT_REMOVE): the names the masks take (the short ones when shown)
    /// are marked, folders only unmarked (Far's SelectFolders is off).
    pub fn select_mask(&mut self, masks: &crate::masks::Masks, add: bool) {
        self.save_selection();
        for entry in self.entries.iter().filter(|e| !e.is_up()) {
            let shown = if self.short_names && !entry.short_name.is_empty() { &entry.short_name } else { &entry.name };
            if !masks.check(shown) {
                continue;
            }
            // Panel settings: Select folders
            if !add {
                self.marked.remove(&entry.name);
            } else if !entry.is_dir || crate::settings::bool("Panel.SelectFolders") {
                self.marked.insert(entry.name.clone());
            }
        }
        if self.sorting.selected_first {
            self.resort();
        }
    }

    /// The names a command works on: the marked ones, or the one under the cursor (never `..`).
    pub fn selected_names(&self) -> Vec<String> {
        if self.marked.is_empty() {
            return self.current().filter(|e| !e.is_up()).map(|e| vec![e.name.clone()]).unwrap_or_default();
        }
        self.entries.iter().filter(|e| self.marked.contains(&e.name)).map(|e| e.name.clone()).collect()
    }

    fn contains(&self, col: u16, row: u16) -> bool {
        let (x, y, w, h) = self.area;
        col >= x && col < x + w && row >= y && row < y + h
    }

    /// The entry drawn at (`col`, `row`), if one is.
    fn entry_at(&self, col: u16, row: u16) -> Option<usize> {
        let line = (row as usize).checked_sub(self.list_top as usize).filter(|&line| line < self.height)?;
        let stripe = self.stripe_cols.iter().position(|&(first, last)| col >= first && col <= last)?;
        let index = self.top + stripe * self.height + line;
        (index < self.entries.len()).then_some(index)
    }

    /// The marked entries, or the one under the cursor when nothing is marked
    /// (never `..`), as full paths.
    pub fn sources(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            return self
                .current()
                .filter(|e| !e.is_up())
                .map(|e| vec![self.dir.join(&e.name)])
                .unwrap_or_default();
        }
        self.entries.iter().filter(|e| self.marked.contains(&e.name)).map(|e| self.dir.join(&e.name)).collect()
    }

    /// Opens the folder under the cursor (`..` goes up and lands on the
    /// folder it came from); a file does nothing. A link whose own path
    /// cannot be listed (Windows' compatibility junctions, `My Documents`,
    /// `Application Data`..., deny listing themselves) is followed to its
    /// target. A folder that cannot be read at all is an error, and the
    /// panel stays where it was.
    pub fn enter(&mut self) -> Result<(), String> {
        let Some(entry) = self.entries.get(self.cursor).cloned() else { return Ok(()) };
        if let Some(view) = &mut self.archive {
            if entry.is_up() {
                match view.dir.rsplit_once('/') {
                    Some((parent, name)) => {
                        let name = name.to_string();
                        view.dir = parent.to_string();
                        self.reload(Some(&name));
                    }
                    None if !view.dir.is_empty() => {
                        let name = std::mem::take(&mut view.dir);
                        self.reload(Some(&name));
                    }
                    // `..` at the top of the archive: back to its folder, on it
                    None => {
                        let file = view.file.file_name().map(|n| n.to_string_lossy().into_owned());
                        self.archive = None;
                        self.reload(file.as_deref());
                    }
                }
            } else if entry.is_dir {
                view.dir = view.inner(&entry.name);
                self.reload(None);
            }
            return Ok(());
        }
        if !entry.is_dir {
            // Enter on an archive opens it (arclite's OpenFilePlugin)
            let path = self.dir.join(&entry.name);
            if crate::archive::detect(&path).is_some() {
                let listing = crate::archive::list(&path, None).map_err(|error| format!("{}: {error}", path.display()))?;
                self.archive = Some(ArchiveView { file: path, listing, dir: String::new(), password: None });
                self.reload(None);
            }
            return Ok(());
        }
        if entry.is_up() {
            let from = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
            if let Some(parent) = self.dir.parent() {
                self.dir = parent.to_path_buf();
            }
            self.reload(from.as_deref());
        } else {
            let path = self.dir.join(&entry.name);
            self.dir = match std::fs::read_dir(&path) {
                Ok(_) => path,
                Err(error) => match entry.reparse.and_then(|_| link_target(&path)).filter(|target| std::fs::read_dir(target).is_ok()) {
                    Some(target) => target,
                    None => return Err(format!("{}: {error}", path.display())),
                },
            };
            self.reload(None);
        }
        Ok(())
    }

    /// Far's `GetShowColor`: the colors of the entry at `pos` (panel text past the end).
    fn colors(&self, pos: usize, active: bool) -> (Color, Color) {
        let Some(entry) = self.entries.get(pos) else { return (TEXT(), BG()) };
        let cursor = active && pos == self.cursor;
        let marked = self.marked.contains(&entry.name);
        // Far's GetShowColor: the panel's color of the state, the group's (Options > Files highlighting) over it
        let state = match (cursor, marked) {
            (false, false) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (true, true) => 3,
        };
        let base = ["Panel.Text", "Panel.Text.Selected", "Panel.Cursor", "Panel.Cursor.Selected"][state];
        let (mut fg, mut bg) = (crate::colors::fg(base), crate::colors::bg(base));
        if crate::settings::bool("Panel.Highlight")
            && let Some((f, b)) = crate::highlight::colors(entry).0[state]
        {
            // black on black is "the panel's own"; a black background is transparent
            if (f, b) != (0, 0) {
                fg = crate::palette::fg(f);
                if b != 0 {
                    bg = crate::palette::bg(b);
                }
            }
        }
        (fg, bg)
    }

    fn cell(&self, entry: &Entry, col: Col, width: usize, status: bool) -> (String, bool) {
        let (flags, marks, groups) = self.mode_flags;
        let (marks, groups) = (marks && !status, groups && !status);
        // PVS_FOLDERUPPERCASE, PVS_FILELOWERCASE, PVS_FILEUPPERTOLOWERCASE
        let cased;
        let entry = {
            let name = &entry.name;
            let new = if entry.is_dir && flags & PVS_FOLDERUPPERCASE != 0 {
                Some(name.to_uppercase())
            } else if !entry.is_dir && flags & PVS_FILELOWERCASE != 0 {
                Some(name.to_lowercase())
            } else if !entry.is_dir && flags & PVS_FILEUPPERTOLOWERCASE != 0 && *name == name.to_uppercase() {
                Some(name.to_lowercase())
            } else {
                None
            };
            match new {
                Some(name) if !entry.is_up() => {
                    cased = Entry { name, ..entry.clone() };
                    &cased
                }
                _ => entry,
            }
        };
        let align = |e: &Entry| flags & PVS_ALIGNEXTENSIONS != 0 && (!e.is_dir || flags & PVS_FOLDERALIGNEXTENSIONS != 0);
        // Ctrl+N: AlternateOrNormal, the short name where there is one
        let shown;
        let entry = if self.short_names && !entry.short_name.is_empty() {
            shown = Entry { name: entry.short_name.clone(), ..entry.clone() };
            &shown
        } else {
            entry
        };
        match col {
            Col::Name if status => (listing::status_name(&entry.name, width), false),
            // COLFLAGS_MARK: `√ ` before a marked name, two spaces before the rest
            Col::Name if marks && width > 2 => {
                let mark = if self.marked.contains(&entry.name) { "\u{221A} " } else { "  " };
                let (name, cut) = listing::list_name(&entry.name, width - 2, align(entry));
                (format!("{mark}{name}"), cut)
            }
            Col::Name => listing::list_name(&entry.name, width, align(entry)),
            Col::Size if groups => (listing::size_column_grouped(entry, width), false),
            Col::Size => (listing::size_column(entry, width), false),
            Col::Allocated => (listing::size_column(&Entry { size: entry.allocated, ..entry.clone() }, width), false),
            Col::Date => (listing::right(&listing::date_time(entry.modified).0, width), false),
            Col::Time => (listing::right(&listing::time_of_width(entry.modified, width), width), false),
            Col::Write => (listing::right(&listing::date_and_time(entry.modified, width), width), false),
            Col::Created => (listing::right(&listing::date_and_time(entry.created, width), width), false),
            Col::Accessed => (listing::right(&listing::date_and_time(entry.accessed, width), width), false),
            Col::Changed => (listing::right(&listing::date_and_time(entry.changed, width), width), false),
            Col::Attributes => (listing::attribute_letters(entry.attributes, width), false),
            Col::Description => {
                let text = entry.description.as_deref().unwrap_or("");
                let text = text.split('\u{4}').next().unwrap_or("");
                (listing::left(text, width), false)
            }
            // DOMAIN\name shows as name
            Col::Owner => {
                let owner = entry.owner.as_deref().unwrap_or("");
                let owner = owner.rsplit(['\\', '/']).next().unwrap_or(owner);
                (listing::left(owner, width), false)
            }
            Col::Links => (listing::right(&entry.links.map(|n| n.to_string()).unwrap_or_default(), width), false),
        }
    }

    fn paint(&mut self, grid: &mut Grid, col: u16, width: u16, height: u16, active: bool) {
        self.area = (col, 0, width, height);
        if width < 3 || height < 3 {
            return;
        }
        let (left, right, bottom) = (col, col + width - 1, height - 1);
        grid.fill(col, 0, width, height, BG());
        let inner = (width - 2) as usize;
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(left, 0, &format!("\u{2554}{horizontal}\u{2557}"), TEXT(), BG());
        grid.put(left, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT(), BG());
        for row in 1..bottom {
            grid.put(left, row, "\u{2551}", TEXT(), BG());
            grid.put(right, row, "\u{2551}", TEXT(), BG());
        }

        let text = self.mode.text();
        let list = parse_columns(&text.columns, &text.widths);
        self.mode_flags = (text.flags, list.iter().any(|c| c.mark), list.iter().any(|c| c.group));
        let cols = layout(&list.iter().map(|c| (c.col, c.width)).collect::<Vec<_>>(), inner);
        let in_stripe = stripe_len(&cols);
        self.stripes = (cols.len() / in_stripe).max(1);
        // Panel settings: Show column titles, Show status line
        let titles = crate::settings::bool("Panel.Layout.ColumnTitles");
        let status = crate::settings::bool("Panel.Layout.StatusLine");
        self.list_top = if titles { 2 } else { 1 };
        self.height = height.saturating_sub(2 + u16::from(titles) + if status { 2 } else { 0 }) as usize;

        // column titles, with the joints of the dividers in the top border
        self.stripe_cols.clear();
        let mut x = left + 1;
        let mut stripe_start = x;
        for (i, &(kind, w)) in cols.iter().enumerate() {
            if titles {
                grid.put(x, 1, &listing::center(crate::lang::tr(kind.title()), w), TITLE(), BG());
            }
            x += w as u16;
            if (i + 1) % in_stripe == 0 {
                self.stripe_cols.push((stripe_start, x));
                stripe_start = x + 1;
            }
            if i == cols.len() - 1 {
                break;
            }
            let double = (i + 1) % in_stripe == 0;
            grid.put(x, 0, if double { "\u{2566}" } else { "\u{2564}" }, TEXT(), BG());
            if titles {
                grid.put(x, 1, if double { "\u{2551}" } else { "\u{2502}" }, TEXT(), BG());
            }
            x += 1;
        }
        // Panel settings: Show sort mode letter
        if crate::settings::bool("Panel.Layout.SortMode") {
            let mut indicator = self.sorting.mode.indicator(self.sorting.reverse).to_string();
            if self.sorting.selected_first {
                indicator.push('^');
            }
            grid.put(left + 1, if titles { 1 } else { 0 }, &indicator, TITLE(), BG());
        }

        // the folder in the top border
        let max = inner;
        let mut title = match &self.archive {
            Some(view) => view.title(),
            None => self.dir.display().to_string(),
        };
        if max >= 2 {
            title = listing::truncate_path(&title, max - 2);
        }
        let title = format!(" {title} ");
        let title: String = title.chars().take(max).collect();
        let title_x = left + 1 + (width - 1).saturating_sub(listing::chars(&title) as u16) / 2;
        let (fg, bg) = if active { (BLACK(), CURSOR_BG()) } else { (TEXT(), BG()) };
        grid.put(title_x, 0, &title, fg, bg);

        self.correct_position();
        self.paint_list(grid, &cols, in_stripe, active);
        if status {
            self.paint_status(grid, left, right, bottom);
        }
        self.paint_totals(grid, left, width, bottom, &cols, in_stripe, status);
        // Panel settings: Show scrollbar (Far's on the right border when the files do not fit)
        let shown = self.height * self.stripes;
        if crate::settings::bool("Panel.Layout.Scrollbar") && self.entries.len() > shown && self.height >= 2 {
            let bar = crate::vmenu::scroll_bar(self.height, self.top, self.top + shown, self.entries.len());
            for (i, c) in bar.chars().enumerate() {
                grid.put(right, self.list_top + i as u16, &c.to_string(), TEXT(), BG());
            }
        }
    }

    /// Ctrl+Q's file panel (the user's call): a framed panel of one column of
    /// names, Brief's (extensions aligned), the folder in the top border - no
    /// column titles, status line or totals. The view mode itself is not
    /// changed, so leaving Ctrl+Q shows the panel as it was.
    fn paint_compact(&mut self, grid: &mut Grid, col: u16, width: u16, height: u16, active: bool) {
        self.area = (col, 0, width, height);
        grid.fill(col, 0, width, height, BG());
        if width < 4 || height < 3 {
            return;
        }
        let (right, bottom) = (col + width - 1, height - 1);
        let inner = (width - 2) as usize;
        let horizontal = "\u{2550}".repeat(inner);
        grid.put(col, 0, &format!("\u{2554}{horizontal}\u{2557}"), TEXT(), BG());
        grid.put(col, bottom, &format!("\u{255A}{horizontal}\u{255D}"), TEXT(), BG());
        for row in 1..bottom {
            grid.put(col, row, "\u{2551}", TEXT(), BG());
            grid.put(right, row, "\u{2551}", TEXT(), BG());
        }
        // the folder in the top border, as in a panel
        let folder = match &self.archive {
            Some(view) => view.title(),
            None => self.dir.display().to_string(),
        };
        let folder = format!(" {} ", if inner >= 2 { listing::truncate_path(&folder, inner - 2) } else { folder });
        let folder: String = folder.chars().take(inner).collect();
        let folder_x = col + 1 + (width - 1).saturating_sub(listing::chars(&folder) as u16) / 2;
        let (title_fg, title_bg) = if active { (BLACK(), CURSOR_BG()) } else { (TEXT(), BG()) };
        grid.put(folder_x, 0, &folder, title_fg, title_bg);

        let brief = ViewMode::Brief.text();
        self.mode_flags = (brief.flags, false, false);
        self.list_top = 1;
        self.height = usize::from(height - 2);
        self.stripes = 1;
        self.stripe_cols = vec![(col + 1, right)];
        self.correct_position();
        for line in 0..self.height {
            let pos = self.top + line;
            let (fg, bg) = self.colors(pos, active);
            let (text, too_long) = match self.entries.get(pos) {
                Some(entry) => self.cell(entry, Col::Name, inner, false),
                None => (" ".repeat(inner), false),
            };
            grid.put(col + 1, 1 + line as u16, &text, fg, bg);
            if too_long {
                grid.put(right - 1, 1 + line as u16, "}", TEXT(), bg);
            }
        }
    }
    fn paint_list(&self, grid: &mut Grid, cols: &[(Col, usize)], in_stripe: usize, active: bool) {
        let right = self.area.0 + self.area.2 - 1;
        for line in 0..self.height {
            let y = self.list_top + line as u16;
            let mut x = self.area.0 + 1;
            let mut stripe = 0;
            let mut level = 1;
            for (k, &(kind, w)) in cols.iter().enumerate() {
                let pos = self.top + line + stripe * self.height;
                let (fg, bg) = self.colors(pos, active);
                let (text, too_long) = match self.entries.get(pos) {
                    Some(entry) => self.cell(entry, kind, w, false),
                    None => (" ".repeat(w), false),
                };
                grid.put(x, y, &text, fg, bg);
                // Far's `HighlightBorder`: inside a stripe the divider takes the entry's background
                let border = if level == in_stripe { (TEXT(), BG()) } else { (TEXT(), bg) };
                let last = k == cols.len() - 1;
                if too_long {
                    grid.put(x + w as u16, y, "}", border.0, border.1);
                } else if last {
                    let edge = if x + w as u16 == right { "\u{2551}" } else { " " };
                    grid.put(x + w as u16, y, edge, TEXT(), BG());
                } else {
                    let divider = if level == in_stripe { "\u{2551}" } else { "\u{2502}" };
                    grid.put(x + w as u16, y, divider, border.0, border.1);
                }
                x += w as u16 + 1;
                if level == in_stripe {
                    level = 0;
                    stripe += 1;
                }
                level += 1;
            }
            if x < right {
                grid.put(x, y, &" ".repeat((right - x) as usize), TEXT(), BG());
            }
        }
    }

    /// The divider over the status line and the status line: the entry under the cursor.
    fn paint_status(&self, grid: &mut Grid, left: u16, right: u16, bottom: u16) {
        let inner = (right - left - 1) as usize;
        let y = bottom - 1;
        grid.put(left, y - 1, &format!("\u{255F}{}\u{2562}", "\u{2500}".repeat(inner)), TEXT(), BG());
        let Some(entry) = self.current() else { return };
        let cols = layout(&self.mode.status(), inner);
        let mut x = left + 1;
        for (k, &(kind, w)) in cols.iter().enumerate() {
            let (text, _) = self.cell(entry, kind, w, true);
            grid.put(x, y, &text, TEXT(), BG());
            let edge = if k == cols.len() - 1 && x + w as u16 == right { "\u{2551}" } else { " " };
            grid.put(x + w as u16, y, edge, TEXT(), BG());
            x += w as u16 + 1;
        }
        if x < right {
            grid.put(x, y, &" ".repeat((right - x) as usize), TEXT(), BG());
        }
    }

    /// The joints of the dividers, the marked total on the divider and the
    /// folder's total in the bottom border.
    #[allow(clippy::too_many_arguments)]
    fn paint_totals(&self, grid: &mut Grid, left: u16, width: u16, bottom: u16, cols: &[(Col, usize)], in_stripe: usize, status: bool) {
        let mut x = left + 1;
        // the dividers end on the status line's divider, else on the bottom border
        let joint_row = if status { bottom - 2 } else { bottom };
        for (i, &(_, w)) in cols.iter().enumerate().take(cols.len().saturating_sub(1)) {
            x += w as u16;
            let joint = if (i + 1) % in_stripe == 0 { if status { "\u{2568}" } else { "\u{2569}" } } else if status { "\u{2534}" } else { "\u{2567}" };
            grid.put(x, joint_row, joint, TEXT(), BG());
            x += 1;
        }
        let room = (width as usize).saturating_sub(4);
        let centered = |text: &str| left + 1 + (room.saturating_sub(listing::chars(text)) / 2) as u16;

        let files = self.entries.iter().filter(|e| !e.is_up() && !e.is_dir);
        let (count, bytes) = files.fold((0, 0), |(n, sum), e| (n + 1, sum + e.size));
        let folders = self.entries.iter().filter(|e| !e.is_up() && e.is_dir).count();
        // Panel settings: Show files total information, Show free size
        let mut parts = Vec::new();
        if crate::settings::bool("Panel.Layout.TotalInfo") {
            parts.push(crate::lang::trs(&format!("Bytes: {}, files: {count}, folders: {folders}", listing::float_size(bytes))));
        }
        if crate::settings::bool("Panel.Layout.FreeInfo")
            && let Some(free) = self.free
        {
            parts.push(crate::lang::trs(&format!("{} free bytes", listing::float_size(free))));
        }
        if !parts.is_empty() {
            let total = listing::truncate_right(&parts.join(", "), room);
            grid.put(centered(&total), bottom, &format!(" {total} "), TEXT(), BG());
        }

        if !self.marked.is_empty() && status {
            let marked = self.entries.iter().filter(|e| self.marked.contains(&e.name));
            let (files, dirs, bytes) = marked.fold((0, 0, 0), |(f, d, sum), e| {
                if e.is_dir { (f, d + 1, sum + e.tree_size.unwrap_or(0)) } else { (f + 1, d, sum + e.size) }
            });
            let line = |size: String| format!("Bytes: {size}, files: {files}, folders: {dirs}");
            let mut text = line(listing::group_digits(bytes));
            if listing::chars(&text) > room {
                let short = bytes.to_string();
                text = line(if short.len() <= 6 { short } else { listing::file_size(bytes, 6).trim().to_string() });
                text = listing::truncate_right(&text, room);
            }
            grid.put(centered(&text), bottom - 2, &format!(" {text} "), TITLE(), CURSOR_BG());
        }
    }
}

/// One folder being counted by F3: a thread adds up the sizes of the files in it
/// (links are not followed) and stops when this is dropped.
struct Scan {
    right: bool,
    path: PathBuf,
    name: String,
    total: std::sync::Arc<std::sync::atomic::AtomicU64>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Scan {
    fn start(right: bool, path: PathBuf, name: String) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::Arc;
        let total = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (worker_total, worker_done, worker_cancel) = (total.clone(), done.clone(), cancel.clone());
        let root = path.clone();
        std::thread::spawn(move || {
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let Ok(read) = std::fs::read_dir(&dir) else { continue };
                for item in read.flatten() {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(meta) = item.metadata() else { continue };
                    let kind = meta.file_type();
                    if kind.is_symlink() {
                        continue;
                    }
                    if kind.is_dir() {
                        #[cfg(windows)]
                        {
                            use std::os::windows::fs::MetadataExt;
                            if meta.file_attributes() & 0x400 != 0 {
                                continue;
                            }
                        }
                        stack.push(item.path());
                    } else {
                        worker_total.fetch_add(meta.len(), Ordering::Relaxed);
                    }
                }
            }
            worker_done.store(true, Ordering::Relaxed);
        });
        Self { right, path, name, total, done, cancel }
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// What a key asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// F1: Far's help.
    Help,
    View(PathBuf),
    Edit(PathBuf),
    /// Shift+F4: asks for the name of a file (new or not) to edit in this folder.
    EditNew(PathBuf),
    /// Sources and the other panel's folder (the default destination).
    Copy(Vec<PathBuf>, PathBuf),
    Move(Vec<PathBuf>, PathBuf),
    MkDir,
    Delete(Vec<PathBuf>),
    /// Alt+Del.
    Wipe(Vec<PathBuf>),
    Quit,
    /// F9: the menu bar.
    Menu,
    /// Alt+F1 / Alt+F2: another drive (mount point) for the left / right panel.
    ChangeDrive { right: bool },
    /// Ctrl+F12: the "Sort by" menu for the active panel.
    SortMenu,
    /// Something went wrong (a folder that cannot be read): tell the user.
    Error(String),
    /// Ctrl+A: the attributes of these.
    Attributes(Vec<PathBuf>),
    /// Shift+F1: put these into a new archive.
    CreateArchive(Vec<PathBuf>),
    /// F5/F6 in an archive (to the other panel's folder; F6 also takes the
    /// files out of it), Shift+F2 on archives.
    Extract(ArchivePick, PathBuf, bool),
    /// F5/F6 into a zip the other panel shows (F6: the files go).
    AddToArchive(Vec<PathBuf>, ArchiveView, bool),
    /// F8 in a zip.
    DeleteInArchive(ArchivePick),
    /// F7 in a zip.
    CreateDirInArchive(ArchiveView),
    /// F3 on a file in an archive.
    ViewInArchive(ArchivePick),
    /// Alt++ (true) / Alt+-: Select group / Deselect group.
    SelectGroup(bool),
    /// Alt+F6: links to these, in the other panel's folder by default.
    Link(Vec<PathBuf>, PathBuf),
    /// Ctrl+G: a command for each of these names.
    ApplyCommand(Vec<String>),
    /// Ctrl+Z: describe these names of this folder.
    Describe(PathBuf, Vec<String>),
    /// Shift+F3: arclite's menu for these archives (or the items picked in one).
    ArchiveCommands(Vec<ArchivePick>),
    /// F2, F11, F12 (also from the key bar): the user menu, the plugins, the screens.
    Command(crate::menu::Cmd),
}

/// Where the link at `path` points, as an ordinary path: a relative target
/// is taken from the link's folder, Windows' verbatim prefix is dropped.
fn link_target(path: &Path) -> Option<PathBuf> {
    let target = std::fs::read_link(path).ok()?;
    let text = target.to_string_lossy();
    let target = match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => target.clone(),
    };
    Some(if target.is_relative() { path.parent()?.join(target) } else { target })
}

/// What the panels made of a key.
#[derive(Debug, PartialEq)]
pub enum Key {
    /// Not theirs: it belongs to the shell's command line.
    Pass,
    Taken,
    Ask(Request),
}

pub struct Panels {
    /// Folders F3 is counting.
    scans: Vec<Scan>,
    left: Panel,
    right: Panel,
    right_active: bool,
    /// What the information panel shows, read for this folder (Far reads it when it draws).
    info: Option<(PathBuf, crate::infopanel::Info)>,
    /// What the quick view panel shows.
    qview: Option<crate::qview::State>,
    /// The quick view's text: one fresh session for as long as a quick view is
    /// shown, moved from file to file (opening an editor per cursor step would be
    /// slow), and where it was last drawn (to take the wheel and the keys).
    qeditor: Option<crate::fresh_view::EditorView>,
    /// The mouse is selecting text in the quick view.
    qdrag: bool,
    qrect: (u16, u16, u16, u16),
    /// The screen size at the last paint.
    screen: (u16, u16),
    /// Each key of the key bar at the last paint: its first column, the column
    /// past it, and which F-key it is.
    key_ends: Vec<(u16, u16, u8)>,
    /// Alt and a letter opened the quick search box (Far's FastFind): what is typed in it.
    fast: Option<String>,
    /// Where the box was last drawn: its cursor.
    fast_cursor: Option<(u16, u16)>,
}

impl Panels {
    pub fn new(dir: PathBuf) -> Self {
        let mut right = Panel::new(dir.clone());
        right.filter_area = 1;
        right.reload(None);
        Self { scans: Vec::new(), left: Panel::new(dir), right, right_active: false, info: None, qview: None, qeditor: None, qdrag: false, qrect: (0, 0, 0, 0), screen: (0, 0), key_ends: Vec::new(), fast: None, fast_cursor: None }
    }

    /// Save setup: each panel's folder, view and sort (Far's Panel.Left / Panel.Right).
    pub fn save_state(&self) {
        use crate::settings::{set_bool, set_int, set_string};
        for (side, panel) in [("Left", &self.left), ("Right", &self.right)] {
            let key = |name: &str| -> &'static str { Box::leak(format!("Panel.{side}.{name}").into_boxed_str()) };
            set_string(key("Folder"), &panel.dir.display().to_string());
            set_string(key("CurFile"), panel.current().map(|e| e.name.as_str()).unwrap_or(""));
            set_int(key("ViewMode"), Self::VIEWS.iter().position(|&m| m == panel.mode).unwrap_or(2) as i64);
            set_bool(key("Visible"), panel.visible);
            set_bool(key("ShortNames"), panel.short_names);
            set_int(key("SortMode"), panel.sorting.mode as i64);
            set_bool(key("ReverseSortOrder"), panel.sorting.reverse);
            set_bool(key("SortGroups"), panel.sorting.groups);
            set_bool(key("SelectedFirst"), panel.sorting.selected_first);
            set_bool(key("DirectoriesFirst"), panel.sorting.directories_first);
        }
        set_bool("Panel.LeftFocus", !self.right_active);
    }

    /// At start: what Save setup kept (a folder that is gone is left as it was).
    pub fn restore_state(&mut self) {
        use crate::settings::{bool, int, string, value};
        if value("Panel.Left.Folder").is_none() {
            return;
        }
        for (side, right) in [("Left", false), ("Right", true)] {
            let key = |name: &str| format!("Panel.{side}.{name}");
            let panel = if right { &mut self.right } else { &mut self.left };
            let folder = PathBuf::from(string(&key("Folder")));
            if folder.is_dir() {
                panel.dir = folder;
            }
            if let Some(&mode) = Self::VIEWS.get(int(&key("ViewMode")) as usize) {
                panel.mode = mode;
            }
            panel.visible = value(&key("Visible")).and_then(|v| v.as_bool()).unwrap_or(true);
            panel.short_names = bool(&key("ShortNames"));
            if let Some(mode) = SortMode::from_index(int(&key("SortMode")) as usize) {
                panel.sorting.mode = mode;
            }
            panel.sorting.reverse = bool(&key("ReverseSortOrder"));
            panel.sorting.groups = bool(&key("SortGroups"));
            panel.sorting.selected_first = bool(&key("SelectedFirst"));
            panel.sorting.directories_first = value(&key("DirectoriesFirst")).and_then(|v| v.as_bool()).unwrap_or(true);
            let current = string(&key("CurFile"));
            panel.reload(if current.is_empty() { None } else { Some(&current) });
        }
        self.right_active = !value("Panel.LeftFocus").and_then(|v| v.as_bool()).unwrap_or(true);
    }

    /// The file under the active panel's cursor (not a folder, not in an archive).
    pub fn current_file(&self) -> Option<PathBuf> {
        let panel = self.active_ref();
        if panel.kind != Kind::Files || !panel.visible || panel.archive.is_some() {
            return None;
        }
        panel.current().filter(|e| !e.is_dir).map(|e| panel.dir.join(&e.name))
    }

    /// The active panel's filter marks (Ctrl+I), and its names for the file type items.
    pub fn filter_area(&self) -> usize {
        self.active_ref().filter_area
    }

    pub fn names(&self) -> Vec<(String, bool)> {
        self.active_ref().entries.iter().map(|e| (e.name.clone(), e.is_dir)).collect()
    }

    /// The marks changed (Ctrl+I): both panels read again.
    pub fn refilter(&mut self) {
        self.refresh(None);
    }

    /// Find file's "Selected folders": the folders marked (or under the cursor).
    pub fn selected_dirs(&self) -> Vec<PathBuf> {
        self.active_ref().sources().into_iter().filter(|p| p.is_dir()).collect()
    }

    /// Takes the active panel to `dir`, the cursor on `select` (Find file's
    /// Go to, a history's folder).
    pub fn open_at(&mut self, dir: PathBuf, select: Option<&str>) {
        let panel = self.active();
        if panel.kind != Kind::Files {
            panel.kind = Kind::Files;
        }
        panel.visible = true;
        panel.dir = dir;
        panel.archive = None;
        panel.reload(select);
    }

    /// Commands > Compare folders (FileList::CompareDir): every file of both
    /// panels is marked, then a file with the same name in the other panel
    /// unmarks the older one (both when they are the same, unless the sizes
    /// differ). None when the other panel is not a file panel shown; else
    /// whether nothing stayed marked ("The folders contents seems to be identical").
    pub fn compare_folders(&mut self) -> Option<bool> {
        let both = [&self.left, &self.right].iter().all(|p| p.visible && p.kind == Kind::Files);
        if !both {
            return None;
        }
        for panel in [&mut self.left, &mut self.right] {
            panel.marked = panel.entries.iter().filter(|e| !e.is_dir && !e.is_up()).map(|e| e.name.clone()).collect();
        }
        let (left, right) = (&mut self.left, &mut self.right);
        let (this, that) = if self.right_active { (right, left) } else { (left, right) };
        let others: Vec<(String, Option<std::time::SystemTime>, u64)> =
            that.entries.iter().filter(|e| !e.is_dir && !e.is_up()).map(|e| (e.name.clone(), e.modified, e.size)).collect();
        for entry in this.entries.iter().filter(|e| !e.is_dir && !e.is_up()) {
            if !this.marked.contains(&entry.name) {
                continue;
            }
            let Some((name, modified, size)) = others.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(&entry.name) && that.marked.contains(n)) else { continue };
            let order = entry.modified.cmp(modified);
            if order.is_eq() && entry.size != *size {
                continue;
            }
            if order.is_le() {
                this.marked.remove(&entry.name);
            }
            if order.is_ge() {
                that.marked.remove(name);
            }
        }
        let identical = this.marked.is_empty() && that.marked.is_empty();
        for panel in [&mut self.left, &mut self.right] {
            if panel.sorting.selected_first {
                panel.resort();
            }
        }
        Some(identical)
    }

    pub fn active_dir(&self) -> &Path {
        &self.active_ref().dir
    }

    /// The name under the cursor of the active panel.
    pub fn current_name(&self) -> Option<&str> {
        self.active_ref().current().map(|e| e.name.as_str())
    }

    pub fn other_dir(&self) -> &Path {
        if self.right_active {
            &self.left.dir
        } else {
            &self.right.dir
        }
    }

    /// Takes the active panel to `dir` (the shell went there).
    pub fn go_to(&mut self, dir: PathBuf) {
        if self.active_dir() != dir {
            let panel = self.active();
            panel.dir = dir;
            panel.archive = None;
            panel.reload(None);
        }
    }

    /// What the selection (or the entry under the cursor) is inside the
    /// archive the active panel shows, if it shows one.
    pub fn archive_pick(&self) -> Option<ArchivePick> {
        let panel = self.active_ref();
        let view = panel.archive.clone()?;
        let names: Vec<String> = if panel.marked.is_empty() {
            panel.current().filter(|e| !e.is_up()).map(|e| vec![e.name.clone()]).unwrap_or_default()
        } else {
            panel.entries.iter().filter(|e| panel.marked.contains(&e.name)).map(|e| e.name.clone()).collect()
        };
        let items = names.iter().map(|name| view.inner(name)).collect();
        Some(ArchivePick { view, items })
    }

    /// Ctrl+L/Q (Far's FilePanels::ProcessKey; Ctrl+T is off: Far's
    /// Tree.TurnOffCompletely is on by default): the passive panel becomes
    /// that kind, or the active one if it is already; again, a file panel.
    pub fn toggle_kind(&mut self, kind: Kind) -> Key {
        if !self.active_ref().visible {
            return Key::Taken;
        }
        let target_right = if self.active_ref().kind == kind { self.right_active } else { !self.right_active };
        let panel = if target_right { &mut self.right } else { &mut self.left };
        panel.kind = if panel.kind == kind { Kind::Files } else { kind };
        if panel.kind == Kind::Files {
            panel.reload(None);
        }
        self.info = None;
        Key::Taken
    }

    /// The Left/Right menu's Info panel, Quick view: that panel
    /// becomes the kind (a file panel again if it is already).
    pub fn set_kind(&mut self, right: bool, kind: Kind) {
        let panel = if right { &mut self.right } else { &mut self.left };
        panel.kind = if panel.kind == kind { Kind::Files } else { kind };
        if panel.kind == Kind::Files {
            panel.reload(None);
        }
        self.info = None;
    }

    /// Ctrl+F1 / Ctrl+F2 (Far's FilePanels): hide the panel (the other one
    /// becomes active if it is shown), or show it (it becomes active if the
    /// other one is hidden).
    pub fn toggle_visible(&mut self, right: bool) -> Key {
        let other_visible = if right { self.left.visible } else { self.right.visible };
        let panel = if right { &mut self.right } else { &mut self.left };
        if panel.visible {
            panel.visible = false;
            if other_visible {
                self.right_active = !right;
            }
        } else {
            panel.visible = true;
            if !other_visible {
                self.right_active = right;
            }
        }
        Key::Taken
    }

    /// Whether something is still being counted for the quick view.
    pub fn busy(&mut self) -> bool {
        let shown = |panel: &Panel| panel.visible && panel.kind == Kind::QuickView;
        (shown(&self.left) || shown(&self.right))
            && (self.qview.as_ref().is_some_and(|state| state.scanning() || state.viewer_busy() || state.pending())
                || self.qeditor.as_mut().is_some_and(|view| view.next_frame().is_some()))
    }

    /// A failure of what the quick view shows (a file Som could not play), once.
    pub fn take_failure(&mut self) -> Option<String> {
        let shown = |panel: &Panel| panel.visible && panel.kind == Kind::QuickView;
        if !(shown(&self.left) || shown(&self.right)) {
            return None;
        }
        self.qview.as_mut().and_then(crate::qview::State::take_failure)
    }

    /// The quick view's editor goes about its own business (a file read, the
    /// highlighting, an animation): `true` when what it shows has changed.
    pub fn tick(&mut self) -> bool {
        let counted = self.poll_scans();
        let shown = |panel: &Panel| panel.visible && panel.kind == Kind::QuickView;
        if !(shown(&self.left) || shown(&self.right)) {
            return counted;
        }
        self.qeditor.as_mut().is_some_and(|view| view.tick().unwrap_or(false)) || counted
    }

    /// F3 on folders (Far's "view" of a folder): each is counted in a thread of
    /// its own and its size appears in the panel as it grows.
    fn count_folders(&mut self, sources: &[PathBuf]) {
        let right = self.right_active;
        for path in sources.iter().filter(|path| path.is_dir()) {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            self.scans.retain(|scan| !(scan.right == right && scan.name == name));
            self.scans.push(Scan::start(right, path.clone(), name));
        }
    }

    /// Brings the counted sizes into the panels; `true` when something moved.
    fn poll_scans(&mut self) -> bool {
        let mut changed = false;
        let (left, right) = (&mut self.left, &mut self.right);
        self.scans.retain(|scan| {
            let panel = if scan.right { &mut *right } else { &mut *left };
            let parent = scan.path.parent().map(Path::to_path_buf);
            let Some(entry) = panel.entries.iter_mut().find(|e| e.is_dir && e.name == scan.name) else { return false };
            if panel.archive.is_some() || parent.as_deref() != Some(panel.dir.as_path()) {
                return false;
            }
            let size = scan.total.load(std::sync::atomic::Ordering::Relaxed);
            if entry.tree_size != Some(size) {
                entry.tree_size = Some(size);
                changed = true;
            }
            !scan.done.load(std::sync::atomic::Ordering::Relaxed)
        });
        changed
    }

    /// The panels' kinds, visibility and long names, for the menu.
    pub fn states(&self) -> [(Kind, bool, bool); 2] {
        [(self.left.kind, self.left.visible, !self.left.short_names), (self.right.kind, self.right.visible, !self.right.short_names)]
    }

    pub fn toggle_short_names(&mut self, right: bool) {
        let panel = if right { &mut self.right } else { &mut self.left };
        panel.short_names = !panel.short_names;
    }

    /// Shift+F2 on an archive file: all of it, to the other panel's folder.
    pub fn extract_archive_request(&mut self) -> Key {
        if self.in_archive() {
            return Key::Taken;
        }
        let Some(path) = self.active_ref().sources().into_iter().next().filter(|p| p.is_file()) else { return Key::Taken };
        if crate::archive::detect(&path).is_none() {
            return Key::Taken;
        }
        let other = self.other_dir().to_path_buf();
        match crate::archive::list(&path, None) {
            Ok(listing) => {
                let items = listing.children("").iter().map(|i| i.path.clone()).collect();
                let view = ArchiveView { file: path, listing, dir: String::new(), password: None };
                Key::Ask(Request::Extract(ArchivePick { view, items }, other, false))
            }
            Err(error) => Key::Ask(Request::Error(format!("{}: {error}", path.display()))),
        }
    }

    /// Whether the active panel shows an archive.
    pub fn in_archive(&self) -> bool {
        self.active_ref().archive.is_some()
    }

    /// The folder the left or right panel shows.
    pub fn dir(&self, right: bool) -> &Path {
        if right { &self.right.dir } else { &self.left.dir }
    }

    /// Takes the left or right panel to `dir` and makes it the active one.
    pub fn open_dir(&mut self, right: bool, dir: PathBuf) {
        self.right_active = right;
        let panel = self.active();
        panel.dir = dir;
        panel.reload(None);
    }

    pub fn right_active(&self) -> bool {
        self.right_active
    }

    /// The mode Far numbers `n` (Ctrl+N).
    pub fn view_of(n: usize) -> ViewMode {
        Self::VIEWS[n % 10]
    }

    /// The active panel's mode number, if it is a file panel.
    pub fn active_mode(&self) -> usize {
        let panel = self.active_ref();
        Self::VIEWS.iter().position(|&m| m == panel.mode).unwrap_or(2)
    }

    /// Far's numbering: Ctrl+1 is Brief .. Ctrl+9 File links, Ctrl+0 Alternative full.
    pub const VIEWS: [ViewMode; 10] = [
        ViewMode::AlternativeFull,
        ViewMode::Brief,
        ViewMode::Medium,
        ViewMode::Full,
        ViewMode::Wide,
        ViewMode::Detailed,
        ViewMode::Descriptions,
        ViewMode::LongDescriptions,
        ViewMode::Owners,
        ViewMode::Links,
    ];

    /// The left and right panels' views as Far numbers them (1 Brief .. 9, 0).
    pub fn view_modes(&self) -> [u8; 2] {
        let number = |mode: ViewMode| Self::VIEWS.iter().position(|&m| m == mode).map_or(2, |i| i as u8);
        [number(self.left.mode), number(self.right.mode)]
    }

    /// Ctrl+0..Ctrl+9 (or the F9 menu) for the left or right panel.
    pub fn set_view(&mut self, right: bool, mode: u8) {
        if let Some(&view) = Self::VIEWS.get(usize::from(mode)) {
            let panel = if right { &mut self.right } else { &mut self.left };
            panel.mode = view;
            panel.fill_for_view();
        }
    }

    /// Makes the left or right panel the active one.
    pub fn activate(&mut self, right: bool) {
        self.right_active = right;
    }

    /// Ctrl+F12: the "Sort by" menu over the active panel.
    pub fn sort_menu(&self) -> crate::sortmenu::SortMenu {
        let panel = self.active_ref();
        crate::sortmenu::SortMenu::new(&panel.sorting, panel.area.0)
    }

    /// Far's `SelectSortMode` after a choice: a mode is chosen (Enter flips
    /// the current one, `+`/`-` set the direction), an option toggled or set.
    pub fn apply_sort(&mut self, picked: crate::sortmenu::Picked) {
        use crate::sortmenu::Choice;
        let panel = self.active();
        match picked.choice {
            Choice::Mode(mode) if picked.invert => panel.sorting.choose(mode),
            Choice::Mode(mode) => {
                panel.sorting.reverse = !picked.plus;
                panel.sorting.mode = mode;
            }
            Choice::Option(option) => {
                let sorting = &mut panel.sorting;
                let flag = match option {
                    0 => &mut sorting.groups,
                    1 => &mut sorting.selected_first,
                    _ => &mut sorting.directories_first,
                };
                *flag = picked.plus || (picked.invert && !*flag);
            }
        }
        panel.resort();
    }

    /// Ctrl+U: the panels change places; the active one stays the same panel.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.left, &mut self.right);
        self.right_active = !self.right_active;
    }

    /// Alt+* (Far's Gray *, SELECT_INVERT): files are marked or unmarked,
    /// a marked folder is unmarked, an unmarked one stays so.
    pub fn invert_marks(&mut self) {
        let panel = self.active();
        panel.save_selection();
        let entries: Vec<(String, bool)> = panel.entries.iter().filter(|e| !e.is_up()).map(|e| (e.name.clone(), e.is_dir)).collect();
        let folders = crate::settings::bool("Panel.SelectFolders");
        for (name, is_dir) in entries {
            if !panel.marked.remove(&name) && (!is_dir || folders) {
                panel.marked.insert(name);
            }
        }
        if panel.sorting.selected_first {
            panel.resort();
        }
    }

    /// Reads both panels again (after a copy, a delete...); the active
    /// panel's cursor goes to `select` if given.
    pub fn refresh(&mut self, select: Option<&str>) {
        // an archive is read again (it may have changed); one that is gone is left
        for panel in [&mut self.left, &mut self.right] {
            let Some(view) = &mut panel.archive else { continue };
            match crate::archive::list(&view.file, view.password.as_deref()) {
                Ok(listing) => view.listing = listing,
                Err(_) => panel.archive = None,
            }
        }
        let keep = self.active_ref().current().map(|e| e.name.clone());
        let other_keep = {
            let other = if self.right_active { &self.left } else { &self.right };
            other.current().map(|e| e.name.clone())
        };
        let right_active = self.right_active;
        for (is_right, panel) in [(false, &mut self.left), (true, &mut self.right)] {
            let wanted = if is_right == right_active { select.map(String::from).or(keep.clone()) } else { other_keep.clone() };
            let (was_at, was_top) = (panel.cursor, panel.top);
            panel.reload(wanted.as_deref());
            // the file the cursor was on is gone (deleted, moved): the cursor stays where it was,
            // now on what followed it, not thrown to the top of the folder
            if let Some(name) = &wanted
                && panel.current().is_some_and(|entry| &entry.name != name)
                && !panel.entries.iter().any(|entry| &entry.name == name)
            {
                panel.cursor = was_at.min(panel.entries.len().saturating_sub(1));
                panel.top = was_top;
            }
        }
    }

    fn passive_ref(&self) -> &Panel {
        if self.right_active { &self.left } else { &self.right }
    }

    fn active_ref(&self) -> &Panel {
        if self.right_active {
            &self.right
        } else {
            &self.left
        }
    }

    fn active(&mut self) -> &mut Panel {
        if self.right_active {
            &mut self.right
        } else {
            &mut self.left
        }
    }

    /// Handles a key while the panels are up. Everything but navigation,
    /// marking and the function keys (typing, editing, history, Enter over a
    /// typed command) is the shell's own command line; while that line is
    /// empty Enter, Home and End also belong to the panel.
    pub fn key(&mut self, key: KeyEvent, line_is_empty: bool) -> Key {
        // the quick search box takes the keys while it is open (and goes with the panel it searched)
        if self.fast.is_some() {
            if self.active_ref().visible && self.active_ref().kind == Kind::Files {
                return self.fast_key(key, line_is_empty);
            }
            self.fast = None;
        }
        let plain = !key.modifiers.contains(KeyModifiers::CONTROL);
        let ctrl_only = key.modifiers == KeyModifiers::CONTROL;
        let alt_only = key.modifiers == KeyModifiers::ALT;
        // Far's FilePanels keys: Ctrl+L, Ctrl+Q, Ctrl+T, Ctrl+F1, Ctrl+F2
        match key.code {
            KeyCode::Char('l') if ctrl_only => return self.toggle_kind(Kind::Info),
            KeyCode::Char('q') if ctrl_only => return self.toggle_kind(Kind::QuickView),
            KeyCode::F(1) if ctrl_only => return self.toggle_visible(false),
            KeyCode::F(2) if ctrl_only => return self.toggle_visible(true),
            KeyCode::Tab => {
                if self.passive_ref().visible {
                    self.right_active = !self.right_active;
                }
                return Key::Taken;
            }
            _ => {}
        }
        // no panel shown: everything is the command line's
        if !self.active_ref().visible {
            return Key::Pass;
        }
        if self.active_ref().kind == Kind::QuickView {
            return self.quick_view_key(key);
        }
        if self.active_ref().kind != Kind::Files {
            // the information panel takes no keys of its own; the function keys still work
            return match key.code {
                KeyCode::F(9 | 10) => self.function_key(if key.code == KeyCode::F(9) { 9 } else { 10 }),
                _ => Key::Pass,
            };
        }
        match key.code {
            KeyCode::Char('n') if ctrl_only => {
                let panel = self.active();
                panel.short_names = !panel.short_names;
            }
            KeyCode::Up if plain => self.active().move_cursor(-1),
            KeyCode::Down if plain => self.active().move_cursor(1),
            KeyCode::PageUp => self.active().page_move(false),
            KeyCode::PageDown => self.active().page_move(true),
            // Far: Left/Right go over stripes, or (one stripe) when nothing is typed
            KeyCode::Left | KeyCode::Right if plain && (self.active_ref().stripes > 1 || line_is_empty) => {
                let right = key.code == KeyCode::Right;
                self.active().stripe_move(right);
            }
            // Ctrl+1..Ctrl+4: Far's Brief, Medium, Full and Wide views
            KeyCode::Char(c @ '0'..='9') if ctrl_only => {
                let right = self.right_active;
                self.set_view(right, c as u8 - b'0');
            }
            KeyCode::F(12) if ctrl_only => return Key::Ask(Request::SortMenu),
            // Shift+F1: arclite's "Add to archive"
            KeyCode::F(1) if key.modifiers == KeyModifiers::SHIFT => return self.add_to_archive_request(),
            // Shift+F2: arclite's "Extract files" of the archive under the cursor
            KeyCode::F(2) if key.modifiers == KeyModifiers::SHIFT => return self.extract_archive_request(),
            // Far's Alt+Del: Wipe
            KeyCode::Delete if key.modifiers == KeyModifiers::ALT => return self.wipe_request(),
            // Far's Ctrl+A, whatever is typed
            KeyCode::Char('a') if ctrl_only => return self.attributes_request(),
            // Far's Gray +, Gray - and Gray * are Alt++, Alt+- and Alt+* here
            // (keyboards without a keypad), and without Shift Alt+= and Alt+8,
            // the keys + and * are on; Ctrl+M brings the marks back
            KeyCode::Char('+' | '=') if alt_only => return Key::Ask(Request::SelectGroup(true)),
            KeyCode::Char('-') if alt_only => return Key::Ask(Request::SelectGroup(false)),
            KeyCode::Char('*' | '8') if alt_only => self.invert_marks(),
            // Far's FastFind: any other Alt and a character starts the quick search
            KeyCode::Char(c) if alt_only && c != ' ' => return self.fast_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT), line_is_empty),
            KeyCode::Char('m') if ctrl_only => self.restore_selection(),
            KeyCode::Char('g') if ctrl_only => return self.apply_command_request(),
            KeyCode::Char('z') if ctrl_only => return self.describe_request(),
            KeyCode::F(6) if key.modifiers == KeyModifiers::ALT => return self.link_request(),
            KeyCode::F(3) if key.modifiers == KeyModifiers::SHIFT => return self.archive_commands_request(),
            KeyCode::F(4) if key.modifiers == KeyModifiers::SHIFT && !self.in_archive() => return Key::Ask(Request::EditNew(self.active_dir().to_path_buf())),
            KeyCode::F(n) if ctrl_only && SortMode::by_key(n).is_some() => {
                self.active().set_sort(SortMode::by_key(n).unwrap_or(SortMode::Name));
            }
            KeyCode::F(11) if key.modifiers == KeyModifiers::SHIFT => {
                let panel = self.active();
                panel.sorting.groups = !panel.sorting.groups;
                panel.resort();
            }
            KeyCode::F(12) if key.modifiers == KeyModifiers::SHIFT => {
                let panel = self.active();
                panel.sorting.selected_first = !panel.sorting.selected_first;
                panel.resort();
            }
            KeyCode::Home if line_is_empty => self.active().move_cursor(isize::MIN / 2),
            KeyCode::End if line_is_empty => self.active().move_cursor(isize::MAX / 2),
            KeyCode::Enter if line_is_empty => {
                if let Err(error) = self.active().enter() {
                    return Key::Ask(Request::Error(error));
                }
            }
            KeyCode::Insert => self.active().toggle_mark(),
            // Far's Ctrl+R and Ctrl+U, while nothing is typed (else they are the shell's)
            KeyCode::Char('r') if !plain && line_is_empty => self.refresh(None),
            KeyCode::Char('u') if !plain && line_is_empty => self.swap(),
            KeyCode::F(n @ (1 | 2)) if key.modifiers.contains(KeyModifiers::ALT) => {
                return Key::Ask(Request::ChangeDrive { right: n == 2 });
            }
            KeyCode::F(n) => return self.function_key(n),
            _ => return Key::Pass,
        }
        Key::Taken
    }

    /// Far's quick search (fastfind.cpp): what is typed goes to the box and the
    /// cursor to the first name it begins; Ctrl+Enter goes to the next one;
    /// Esc or F10 closes it; any other key closes it and is the panel's own.
    fn fast_key(&mut self, key: KeyEvent, line_is_empty: bool) -> Key {
        let typed = self.fast.clone();
        let opening = typed.is_none();
        let text = typed.unwrap_or_default();
        self.fast = Some(text.clone());
        let ctrl_only = key.modifiers == KeyModifiers::CONTROL;
        match key.code {
            KeyCode::Esc | KeyCode::F(10) if !opening => self.fast = None,
            KeyCode::Enter if ctrl_only => {
                self.active().find_part(&text, true, 1);
            }
            KeyCode::Char('y') if ctrl_only => self.fast = Some(String::new()),
            KeyCode::Backspace if key.modifiers.is_empty() => {
                let mut shorter = text.clone();
                shorter.pop();
                if self.active().find_part(&shorter, false, 1) {
                    self.fast = Some(shorter);
                }
            }
            // a character: with Alt (the first one) in lower case, as Far does
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
                let c = if key.modifiers.contains(KeyModifiers::ALT) { c.to_lowercase().next().unwrap_or(c) } else { c };
                let longer = crate::fastfind::with_char(&text, c);
                if self.active().find_part(&longer, false, 1) {
                    self.fast = Some(longer);
                }
            }
            _ => {
                self.fast = None;
                if opening {
                    return Key::Pass;
                }
                return self.key(key, line_is_empty);
            }
        }
        Key::Taken
    }

    /// Handles a mouse event over the panels. `double` is a second click on
    /// the same spot in quick succession.
    pub fn mouse(&mut self, event: MouseEvent, double: bool) -> Key {
        // a click closes the quick search box, and is nothing else
        if self.fast.is_some() && matches!(event.kind, MouseEventKind::Down(_)) {
            self.fast = None;
            return Key::Taken;
        }
        // text of the quick view is selected with the mouse and copied when it is let go
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left))
            && self.qview.as_ref().is_some_and(|state| state.text())
            && let Some(view) = &mut self.qeditor
        {
            let (qx, qy, qw, qh) = self.qrect;
            let inside = event.column >= qx && event.column < qx + qw && event.row >= qy && event.row < qy + qh;
            if inside || self.qdrag {
                let at = MouseEvent { column: event.column, row: event.row, ..event };
                let _ = view.event(crossterm::event::Event::Mouse(at), qx, qy);
                self.qdrag = crate::viewer::copy_on_select(view, at, self.qdrag, qx, qy);
                return Key::Taken;
            }
        }
        let (cols, rows) = self.screen;
        let col = event.column;
        // the panels are a row lower under the menu bar (not the key bar)
        let row = if Self::menu_bar_shown() && event.row + 1 < rows { event.row.saturating_sub(1) } else { event.row };
        let side = if self.left.contains(col, row) {
            Some(false)
        } else if self.right.contains(col, row) {
            Some(true)
        } else {
            None
        };
        match event.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some(right) = side else { return Key::Pass };
                // over the quick view the wheel scrolls its text
                if (if right { &self.right } else { &self.left }).kind == Kind::QuickView {
                    if self.qview.as_ref().is_some_and(|state| state.text())
                        && let Some(view) = &mut self.qeditor
                    {
                        let wheel = crossterm::event::Event::Mouse(MouseEvent { kind: event.kind, column: col, row, modifiers: event.modifiers });
                        let _ = view.event(wheel, self.qrect.0, self.qrect.1);
                    }
                    return Key::Taken;
                }
                self.right_active = right;
                let step = if event.kind == MouseEventKind::ScrollUp { -3 } else { 3 };
                self.active().move_cursor(step);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if rows > 0 && row == rows - 1 && col < cols {
                    // a column with no key (Ctrl+Q's key bar covers only its panel) presses nothing
                    let Some(&(_, _, key)) = self.key_ends.iter().find(|&&(start, end, _)| col >= start && col < end) else { return Key::Taken };
                    return self.function_key(key);
                }
                let Some(right) = side else { return Key::Pass };
                // a click in the quick view leaves the focus where it is
                if (if right { &self.right } else { &self.left }).kind == Kind::QuickView {
                    return Key::Taken;
                }
                self.right_active = right;
                if let Some(index) = self.active().entry_at(col, row) {
                    self.active().cursor = index;
                    if double
                        && self.active().entries[index].is_dir
                        && let Err(error) = self.active().enter()
                    {
                        return Key::Ask(Request::Error(error));
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                let Some(right) = side else { return Key::Pass };
                if (if right { &self.right } else { &self.left }).kind == Kind::QuickView {
                    return Key::Taken;
                }
                self.right_active = right;
                if let Some(index) = self.active().entry_at(col, row) {
                    self.active().cursor = index;
                    self.active().toggle_mark_at(index);
                }
            }
            _ => return Key::Pass,
        }
        Key::Taken
    }

    /// Alt+Del: wipe the selection, or the entry under the cursor (not `..`).
    pub fn wipe_request(&mut self) -> Key {
        if self.in_archive() {
            return Key::Taken;
        }
        let sources = self.active_ref().sources();
        if sources.is_empty() { Key::Taken } else { Key::Ask(Request::Wipe(sources)) }
    }

    /// Ctrl+A: the selection's attributes, or the entry's under the cursor (not `..`).
    /// Ctrl+G: not in an archive (arclite has no real names).
    pub fn apply_command_request(&mut self) -> Key {
        let names = self.active_ref().selected_names();
        if self.in_archive() || names.is_empty() {
            return Key::Taken;
        }
        self.active().save_selection();
        Key::Ask(Request::ApplyCommand(names))
    }

    /// Ctrl+Z: only on a folder of the disk (Far's NORMAL_PANEL).
    pub fn describe_request(&mut self) -> Key {
        let names = self.active_ref().selected_names();
        if self.in_archive() || names.is_empty() {
            return Key::Taken;
        }
        let panel = self.active();
        panel.save_selection();
        Key::Ask(Request::Describe(panel.dir.clone(), names))
    }

    /// Alt+F6: links to what is selected.
    pub fn link_request(&mut self) -> Key {
        let sources = if self.in_archive() { vec![] } else { self.active_ref().sources() };
        if sources.is_empty() {
            return Key::Taken;
        }
        self.active().save_selection();
        Key::Ask(Request::Link(sources, self.other_dir().to_path_buf()))
    }

    /// Shift+F3 (Far's ProcessHostFile): in an archive, what is picked in
    /// it; outside, the archives selected (or under the cursor).
    pub fn archive_commands_request(&mut self) -> Key {
        if self.in_archive() {
            return match self.archive_pick() {
                Some(pick) if !pick.items.is_empty() => Key::Ask(Request::ArchiveCommands(vec![pick])),
                _ => Key::Taken,
            };
        }
        let mut picks = Vec::new();
        for path in self.active_ref().sources().into_iter().filter(|p| p.is_file() && crate::archive::detect(p).is_some()) {
            match crate::archive::list(&path, None) {
                Ok(listing) => {
                    let items = listing.children("").iter().map(|i| i.path.clone()).collect();
                    picks.push(ArchivePick { view: ArchiveView { file: path, listing, dir: String::new(), password: None }, items });
                }
                Err(error) => return Key::Ask(Request::Error(format!("{}: {error}", path.display()))),
            }
        }
        if picks.is_empty() { Key::Taken } else { Key::Ask(Request::ArchiveCommands(picks)) }
    }

    /// Shift+F1: arclite's "Add to archive" of what is selected.
    pub fn add_to_archive_request(&mut self) -> Key {
        let sources = if self.in_archive() { vec![] } else { self.active_ref().sources() };
        if sources.is_empty() { Key::Taken } else { Key::Ask(Request::CreateArchive(sources)) }
    }

    /// Ctrl+M over the active panel.
    pub fn restore_selection(&mut self) {
        self.active().restore_selection();
    }

    /// The descriptions changed (Ctrl+Z): panels that show them read them
    /// again, keeping their marks and cursor.
    pub fn refresh_descriptions(&mut self) {
        for panel in [&mut self.left, &mut self.right] {
            if panel.archive.is_none() && panel.entries.iter().any(|e| e.description.is_some()) {
                crate::sorting::fill(&mut panel.entries, &[SortMode::Descriptions], &panel.dir);
                panel.resort();
            }
        }
    }

    /// Alt++ / Alt+- accepted: the mask over the active panel.
    pub fn select_group(&mut self, add: bool, masks: &crate::masks::Masks) {
        self.active().select_mask(masks, add);
    }

    /// A mark no longer wanted (a file a command or a description was done for).
    pub fn unmark(&mut self, name: &str) {
        let panel = self.active();
        panel.marked.remove(name);
        if panel.sorting.selected_first {
            panel.resort();
        }
    }

    /// The other panel's folder and the name under its cursor (Far's `!#` metasymbols).
    pub fn passive_current(&self) -> (PathBuf, String, String) {
        let panel = self.passive_ref();
        let entry = panel.current().filter(|e| !e.is_up());
        (panel.dir.clone(), entry.map(|e| e.name.clone()).unwrap_or_default(), entry.map(|e| e.short_name.clone()).unwrap_or_default())
    }

    /// The short name of `name` in the active panel ("" when it has none).
    pub fn short_name_of(&self, name: &str) -> String {
        self.active_ref().entries.iter().find(|e| e.name == name).map(|e| e.short_name.clone()).unwrap_or_default()
    }

    pub fn attributes_request(&mut self) -> Key {
        if self.in_archive() {
            return Key::Taken;
        }
        let sources = self.active_ref().sources();
        if sources.is_empty() { Key::Taken } else { Key::Ask(Request::Attributes(sources)) }
    }

    pub fn function_key(&mut self, n: u8) -> Key {
        if let Some(pick) = self.archive_pick() {
            // arclite's panel: F5/F6 extract (the archive is not updated:
            // farsh does not write into archives), F3 views a file
            let other = self.other_dir().to_path_buf();
            let file_picked = pick.items.len() == 1 && pick.view.listing.items.iter().any(|i| i.path == pick.items[0] && !i.is_dir);
            let updatable = crate::archive::updatable(&pick.view.listing);
            let request = match n {
                3 => file_picked.then(|| Request::ViewInArchive(pick)),
                // F6: extract, and out of the archive if it can be changed
                5 | 6 => (!pick.items.is_empty()).then(|| Request::Extract(pick, other, n == 6)),
                8 if !updatable => Some(Request::Error(crate::archive::NOT_UPDATABLE.into())),
                8 => (!pick.items.is_empty()).then(|| Request::DeleteInArchive(pick)),
                7 if !updatable => Some(Request::Error(crate::archive::NOT_UPDATABLE.into())),
                7 => Some(Request::CreateDirInArchive(pick.view)),
                9 => Some(Request::Menu),
                10 => Some(Request::Quit),
                11 => Some(Request::Command(crate::menu::Cmd::PluginCommands)),
                12 => Some(Request::Command(crate::menu::Cmd::ScreensList)),
                _ => None,
            };
            return request.map_or(Key::Taken, Key::Ask);
        }
        let sources = self.active_ref().sources();
        // F5/F6 into an archive the other panel shows: arclite's PutFiles
        if let (5 | 6, Some(view)) = (n, self.passive_ref().archive.clone()) {
            if sources.is_empty() {
                return Key::Taken;
            }
            if !crate::archive::updatable(&view.listing) {
                return Key::Ask(Request::Error(crate::archive::NOT_UPDATABLE.into()));
            }
            return Key::Ask(Request::AddToArchive(sources, view, n == 6));
        }
        let file = sources.first().filter(|path| path.is_file()).cloned();
        let other = self.other_dir().to_path_buf();
        // copy.cpp saves the selection: Ctrl+M brings it back after the copy
        if matches!(n, 5 | 6) && !sources.is_empty() {
            self.active().save_selection();
        }
        if n == 3 && file.is_none() {
            self.count_folders(&sources);
        }
        let request = match n {
            3 => file.map(Request::View),
            4 => file.map(Request::Edit),
            5 => (!sources.is_empty()).then(|| Request::Copy(sources, other)),
            6 => (!sources.is_empty()).then(|| Request::Move(sources, other)),
            7 => Some(Request::MkDir),
            8 => (!sources.is_empty()).then(|| Request::Delete(sources)),
            1 => Some(Request::Help),
            9 => Some(Request::Menu),
            10 => Some(Request::Quit),
            2 => Some(Request::Command(crate::menu::Cmd::UserMenu)),
            11 => Some(Request::Command(crate::menu::Cmd::PluginCommands)),
            12 => Some(Request::Command(crate::menu::Cmd::ScreensList)),
            _ => None,
        };
        request.map_or(Key::Taken, Key::Ask)
    }

    /// Draws the panels and the key bar over `grid`, with the shell's own
    /// command line between them: `line` is the shell's cursor row exactly as
    /// it is drawn (colors and all), its cursor at `cursor_col`. Returns where
    /// the cursor goes.
    pub fn paint(&mut self, grid: &mut Grid, line: &[Cell], cursor_col: u16) -> Option<(u16, u16)> {
        if grid.rows < 5 || grid.cols < 20 {
            return None;
        }
        self.screen = (grid.cols, grid.rows);
        // Interface settings: Show key bar, Always show the menu bar
        let key_bar = crate::settings::bool("Screen.KeyBar");
        let menu_bar = crate::settings::bool("Interface.ShowMenuBar");
        let height = grid.rows - 1 - u16::from(key_bar) - u16::from(menu_bar);
        // Far: the left panel ends at ScrX / 2
        let left_width = grid.cols.div_ceil(2);
        let right_width = grid.cols - left_width;
        let right_active = self.right_active;
        // PVS_FULLSCREEN: such a panel takes the whole screen; the active panel is drawn last, on top
        let place = |panel: &Panel, left: bool| {
            if panel.kind == Kind::Files && panel.mode.full_screen() { (0, grid.cols) } else if left { (0, left_width) } else { (left_width, right_width) }
        };
        if let Some(((files_x, files_width), (view_x, view_width))) = self.quick_layout(grid.cols) {
            // Ctrl+Q's layout: the file panel one column of names in a sixth of the
            // screen, the viewer in the rest, both framed
            let files_right = self.right.kind == Kind::Files;
            let panel = if files_right { &mut self.right } else { &mut self.left };
            panel.paint_compact(grid, files_x, files_width, height, right_active == files_right);
            self.paint_one(grid, !files_right, view_x, view_width, height, right_active != files_right);
        } else {
            let (left_at, right_at) = (place(&self.left, true), place(&self.right, false));
            for right in [!right_active, right_active] {
                let at = if right { right_at } else { left_at };
                self.paint_one(grid, right, at.0, at.1, height, right == right_active);
            }
        }

        // no quick view shown: its editor session goes
        let quick = |panel: &Panel| panel.visible && panel.kind == Kind::QuickView;
        if !(quick(&self.left) || quick(&self.right)) {
            self.qeditor = None;
        }
        if menu_bar {
            for row in (0..height).rev() {
                let cells = grid.row(row);
                grid.set_row(row + 1, &cells);
            }
            crate::menu::paint_bar(grid);
        }
        let row = grid.rows - 1 - u16::from(key_bar);
        grid.set_row(row, line);

        if key_bar {
            self.paint_key_bar(grid);
        } else {
            self.key_ends.clear();
        }
        // Far's quick search box, over the bottom of the panel that is searched
        self.fast_cursor = None;
        if let Some(text) = self.fast.clone() {
            let (left, _, _, panel_height) = self.active_ref().area;
            let bottom = (panel_height + u16::from(menu_bar)).saturating_sub(1);
            let (x, y) = crate::fastfind::place(left, bottom, grid.cols, grid.rows);
            self.fast_cursor = Some(crate::fastfind::paint(grid, x, y, &text));
        }
        // Interface settings: Clock (Far's in the top right corner)
        if crate::settings::bool("Screen.Clock") {
            let clock = chrono::Local::now().format("%H:%M").to_string();
            grid.put(grid.cols.saturating_sub(5), 0, &clock, crate::colors::fg("Clock"), crate::colors::bg("Clock"));
        }
        Some(self.fast_cursor.unwrap_or((cursor_col.min(grid.cols - 1), row)))
    }

    /// Whether the panels start one row lower (the menu bar always shown).
    pub fn menu_bar_shown() -> bool {
        crate::settings::bool("Interface.ShowMenuBar")
    }

    /// Ctrl+Q's layout (the user's call, 2026-11-07), when one panel is a quick
    /// view and the other a file list: `((files x, width), (viewer x, width))` —
    /// the file panel a sixth of the screen on its own side, the viewer the rest.
    fn quick_layout(&self, screen_cols: u16) -> Option<((u16, u16), (u16, u16))> {
        let is = |panel: &Panel, kind: Kind| panel.visible && panel.kind == kind;
        let files_right = if is(&self.left, Kind::QuickView) && is(&self.right, Kind::Files) {
            true
        } else if is(&self.right, Kind::QuickView) && is(&self.left, Kind::Files) {
            false
        } else {
            return None;
        };
        let files_width = (screen_cols / 6).clamp(10.min(screen_cols / 2), screen_cols.saturating_sub(10).max(1));
        let view_width = screen_cols - files_width;
        Some(if files_right { ((view_width, files_width), (0, view_width)) } else { ((0, files_width), (files_width, view_width)) })
    }

    /// One panel, as what it is; a hidden one is not drawn (the shell shows there).
    fn paint_one(&mut self, grid: &mut Grid, right: bool, x: u16, width: u16, height: u16, active: bool) {
        let (panel, other) = if right { (&mut self.right, &self.left) } else { (&mut self.left, &self.right) };
        if !panel.visible {
            panel.area = (0, 0, 0, 0);
            return;
        }
        match panel.kind {
            Kind::Files => panel.paint(grid, x, width, height, active),
            Kind::Info => {
                panel.area = (x, 0, width, height);
                let dir = other.dir.clone();
                let in_archive = other.archive.is_some();
                if self.info.as_ref().is_none_or(|(at, _)| *at != dir) {
                    self.info = Some((dir.clone(), crate::infopanel::read(&dir)));
                }
                if let Some((_, info)) = &self.info {
                    crate::infopanel::paint(grid, x, width, height, active, info, &dir, in_archive);
                }
            }
            Kind::QuickView => {
                panel.area = (x, 0, width, height);
                self.qrect = (x + 1, 1, width.saturating_sub(2), height.saturating_sub(2));
                // the other panel's entry under the cursor (not `..`, not inside an archive)
                let target = (other.kind == Kind::Files && other.archive.is_none())
                    .then(|| other.current().filter(|e| !e.is_up()).map(|e| other.dir.join(&e.name)))
                    .flatten();
                match target {
                    Some(path) => {
                        if self.qview.as_ref().is_none_or(|state| state.path != path) {
                            self.qview = Some(crate::qview::State::new(path));
                        }
                        crate::qview::paint(grid, x, width, height, active, self.qview.as_mut(), &mut self.qeditor);
                    }
                    None => {
                        // nothing under the cursor (`..`, an archive): what played before stops
                        self.qview = None;
                        crate::qview::paint(grid, x, width, height, active, None, &mut self.qeditor)
                    }
                }
            }
        }
    }

    /// Keys of an active quick view (QuickView::ProcessKey): the viewer's
    /// scrolling, `+`/`-` move the other panel's cursor, F3 views the file.
    fn quick_view_key(&mut self, key: KeyEvent) -> Key {
        let step = |panels: &mut Panels, delta: isize| {
            let other = if panels.right_active { &mut panels.left } else { &mut panels.right };
            other.move_cursor(delta);
        };
        match key.code {
            // `=` is the `+` key without Shift
            KeyCode::Char('+' | '=') => step(self, 1),
            KeyCode::Char('-') => step(self, -1),
            KeyCode::F(3) => {
                self.right_active = !self.right_active;
                let asked = self.function_key(3);
                self.right_active = !self.right_active;
                return asked;
            }
            KeyCode::F(9 | 10) => return self.function_key(if key.code == KeyCode::F(9) { 9 } else { 10 }),
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End | KeyCode::Left | KeyCode::Right => {
                // a placement in the quick view takes no keys: Som's player there is used with
                // the mouse (the user's call), and Som leaves the keys of a preview to farsh
                if self.qview.as_ref().is_some_and(|state| state.text())
                    && let Some(view) = &mut self.qeditor
                {
                    let _ = view.event(crossterm::event::Event::Key(key), 0, 0);
                }
            }
            _ => return Key::Pass,
        }
        Key::Taken
    }

    /// The key bar with the panels' labels, under the panels' columns (the
    /// user's call, not Far's even row): see [`Self::key_spans`].
    fn paint_key_bar(&mut self, grid: &mut Grid) {
        // Ctrl+Q has a set of its own
        let set = if self.quick_layout(grid.cols).is_some() { crate::keybar::Set::QuickView } else { crate::keybar::Set::Main };
        let labels: Vec<&'static str> = crate::keybar::labels(set, crate::modkeys::held(), self.qview.as_ref().is_some_and(|state| state.text())).to_vec();
        let keys = self.key_spans(grid.cols).unwrap_or_else(|| numbered(even_spans(0, grid.cols, (grid.cols as usize / 8).clamp(1, 12))));
        self.key_ends = paint_keys(grid, &labels, &keys);
    }

    /// Where each key goes, as `(F-key, first column, width)`: F1-F6 under the
    /// left panel, F7-F12 under the right one, each panel's six shared out over
    /// its stripes - two under each column of Brief (three columns), three under
    /// each of Medium (two), six under a one-stripe mode - and evenly over a panel
    /// that is not a file list. `None` (one even row of twelve across the screen)
    /// while a panel takes the whole screen, and in Ctrl+Q's layout: its narrow
    /// panel has no room for six keys of its own.
    fn key_spans(&self, screen_cols: u16) -> Option<Vec<(u8, u16, u16)>> {
        if self.quick_layout(screen_cols).is_some() {
            return None;
        }
        let full = |panel: &Panel| panel.visible && panel.kind == Kind::Files && panel.mode.full_screen();
        if full(&self.left) || full(&self.right) {
            return None;
        }
        let left_width = screen_cols.div_ceil(2);
        let mut spans = Vec::with_capacity(12);
        for (panel, x, width) in [(&self.left, 0, left_width), (&self.right, left_width, screen_cols - left_width)] {
            // where each stripe's keys start: the panel's left edge, then the first
            // column of each further stripe
            let mut starts = vec![x];
            if panel.visible && panel.kind == Kind::Files && panel.area.2 == width {
                starts.extend(panel.stripe_cols.iter().skip(1).map(|&(first, _)| first));
            }
            starts.truncate(6);
            let end = x + width;
            let groups = starts.len();
            for (group, &start) in starts.iter().enumerate() {
                let stop = starts.get(group + 1).copied().unwrap_or(end);
                // six keys shared out: the first groups take one more when it does not divide
                let keys = 6 / groups + usize::from(group < 6 % groups);
                spans.extend(even_spans(start, stop.saturating_sub(start), keys).into_iter().map(|(x, w)| (0u8, x, w)));
            }
        }
        // the keys in order are F1..F12
        Some(spans.into_iter().enumerate().map(|(i, (_, x, w))| (i as u8 + 1, x, w)).collect())
    }
}

/// Spans numbered F1, F2... in order.
fn numbered(spans: Vec<(u16, u16)>) -> Vec<(u8, u16, u16)> {
    spans.into_iter().enumerate().map(|(i, (x, w))| (i as u8 + 1, x, w)).collect()
}

/// The key bar row: each key `(F-key, first column, width)` shows its number
/// and its label fitted to what is left less one space; the rest of the row is
/// the key bar's background. Returns each key's columns and number (for the mouse).
fn paint_keys(grid: &mut Grid, labels: &[&str], keys: &[(u8, u16, u16)]) -> Vec<(u16, u16, u8)> {
    let y = grid.rows - 1;
    use crate::colors::{bg, fg};
    let (number, text, gap) = ((fg("Keybar.Num"), bg("Keybar.Num")), (fg("Keybar.Text"), bg("Keybar.Text")), (fg("Keybar.Background"), bg("Keybar.Background")));
    grid.put(0, y, &" ".repeat(grid.cols as usize), gap.0, gap.1);
    let mut ends = Vec::new();
    for &(key, x, width) in keys {
        let digits = key.to_string();
        let width = width as usize;
        grid.put(x, y, &digits.chars().take(width).collect::<String>(), number.0, number.1);
        let label_width = width.saturating_sub(digits.len() + 1);
        let label: String = labels.get(usize::from(key).saturating_sub(1)).copied().unwrap_or("").chars().take(label_width).collect();
        grid.put(x + digits.len().min(width) as u16, y, &format!("{label:<label_width$}"), text.0, text.1);
        if width > digits.len() {
            grid.put(x + width as u16 - 1, y, " ", gap.0, gap.1);
        }
        ends.push((x, x + width as u16, key));
    }
    ends
}

/// The key bar across the whole width (the help screen, a full-screen panel):
/// twelve keys - fewer when the screen is narrower than 8 columns a key, as in
/// Far - sharing the width evenly, the columns left over one each to the first
/// keys rather than all to the last (Far's (ScrX-1)/12 left F12 up to twice
/// as long as the others). Returns where each key shown ends (for the mouse).
pub fn paint_labels(grid: &mut Grid, labels: &[&str]) -> Vec<u16> {
    let keys = numbered(even_spans(0, grid.cols, (grid.cols as usize / 8).clamp(1, 12)));
    paint_keys(grid, labels, &keys).into_iter().map(|(_, end, _)| end).collect()
}

/// `count` spans `(first column, width)` over `width` columns from `x`, as
/// equal as columns allow: the remainder goes one column each to the first spans.
fn even_spans(x: u16, width: u16, count: usize) -> Vec<(u16, u16)> {
    let width = width as usize;
    let mut at = x as usize;
    (0..count)
        .map(|i| {
            let w = width / count + usize::from(i < width % count);
            let span = (at as u16, w as u16);
            at += w;
            span
        })
        .collect()
}

/// Ctrl+Enter: put the highlighted name on the command line. Terminals send
/// it as Enter with Ctrl, or as the line feed (Ctrl+J) a Ctrl+Enter often is.
pub fn is_insert_name(key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    ctrl && matches!(key.code, KeyCode::Enter | KeyCode::Char('j') | KeyCode::Char('\n'))
}

/// A key as the shell should get it: Ctrl+E and Ctrl+X are Far's history
/// keys (the arrows belong to the panel), so they go as Up and Down.
pub fn to_shell(key: KeyEvent) -> KeyEvent {
    if key.modifiers == KeyModifiers::CONTROL {
        match key.code {
            KeyCode::Char('e') => return KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyCode::Char('x') => return KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            _ => {}
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("farsh-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("beta")).unwrap();
        std::fs::create_dir_all(dir.join("Alpha")).unwrap();
        std::fs::write(dir.join("zeta.txt"), "abc").unwrap();
        std::fs::write(dir.join("apple.txt"), "").unwrap();
        dir
    }

    fn names(panel: &Panel) -> Vec<&str> {
        panel.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn listing_puts_up_then_folders_then_files() {
        let dir = scratch("listing");
        let panel = Panel::new(dir.clone());
        assert_eq!(names(&panel), ["..", "Alpha", "beta", "apple.txt", "zeta.txt"]);
        assert_eq!(panel.entries[4].size, 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn entering_a_folder_and_coming_back_keeps_the_cursor_on_it() {
        let dir = scratch("enter");
        let mut panel = Panel::new(dir.clone());
        panel.move_cursor(2); // beta
        panel.enter().unwrap();
        assert_eq!(panel.dir, dir.join("beta"));
        assert_eq!(names(&panel), [".."]);
        panel.enter().unwrap(); // ".." from inside beta
        assert_eq!(panel.dir, dir);
        assert_eq!(panel.entries[panel.cursor].name, "beta");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `My Documents` in every Windows profile is a junction that denies
    /// listing itself; Enter goes where it points, as Far does.
    #[cfg(windows)]
    #[test]
    fn enter_follows_a_link_that_cannot_be_listed_to_its_target() {
        let profile = PathBuf::from(std::env::var("USERPROFILE").unwrap());
        let link = profile.join("My Documents");
        if std::fs::read_dir(&link).is_ok() || !link.exists() {
            return; // a profile without the compatibility junction
        }
        let mut panel = Panel::new(profile.clone());
        panel.cursor = panel.entries.iter().position(|e| e.name == "My Documents").unwrap();
        panel.enter().unwrap();
        assert_eq!(panel.dir, profile.join("Documents"));
        assert!(names(&panel).len() > 1 || std::fs::read_dir(&panel.dir).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn enter_goes_through_a_symlink_to_a_folder() {
        let dir = scratch("symlink");
        std::os::unix::fs::symlink(dir.join("beta"), dir.join("link")).unwrap();
        let mut panel = Panel::new(dir.clone());
        panel.cursor = panel.entries.iter().position(|e| e.name == "link").unwrap();
        panel.enter().unwrap();
        assert_eq!(panel.dir, dir.join("link"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_read_is_an_error_and_the_panel_stays() {
        let dir = scratch("gone");
        let mut panel = Panel::new(dir.clone());
        panel.cursor = panel.entries.iter().position(|e| e.name == "beta").unwrap();
        std::fs::remove_dir(dir.join("beta")).unwrap();
        assert!(panel.enter().is_err());
        assert_eq!(panel.dir, dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_panel_follows_the_shell_to_another_directory() {
        let dir = scratch("follow");
        let mut panels = Panels::new(dir.clone());
        panels.go_to(dir.join("beta"));
        assert_eq!(panels.active_dir(), dir.join("beta"));
        panels.go_to(dir.join("beta"));
        assert_eq!(panels.active_dir(), dir.join("beta"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_cursor_stays_inside_the_list() {
        let dir = scratch("clamp");
        let mut panel = Panel::new(dir.clone());
        panel.move_cursor(-5);
        assert_eq!(panel.cursor, 0);
        panel.move_cursor(100);
        assert_eq!(panel.cursor, panel.entries.len() - 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_panels_take_navigation_and_leave_the_command_line_to_the_shell() {
        let dir = scratch("keys");
        let mut panels = Panels::new(dir.clone());
        assert_eq!(panels.key(key(KeyCode::Down), false), Key::Taken);
        assert_eq!(panels.key(key(KeyCode::Char('l')), true), Key::Pass);
        assert_eq!(panels.key(key(KeyCode::Backspace), true), Key::Pass);
        // Enter, Home and End are the panel's only while the line is empty.
        assert_eq!(panels.key(key(KeyCode::Enter), false), Key::Pass);
        assert_eq!(panels.key(key(KeyCode::Home), false), Key::Pass);
        assert_eq!(panels.key(key(KeyCode::Enter), true), Key::Taken);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn f3_on_a_folder_counts_its_size_and_shows_it() {
        let dir = scratch("foldersize");
        std::fs::create_dir_all(dir.join("Alpha/deep")).unwrap();
        std::fs::write(dir.join("Alpha/one.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.join("Alpha/deep/two.bin"), vec![0u8; 234]).unwrap();
        let mut panels = Panels::new(dir.clone());
        panels.key(key(KeyCode::Down), true); // Alpha
        assert_eq!(panels.key(key(KeyCode::F(3)), true), Key::Taken);
        for _ in 0..200 {
            panels.tick();
            if panels.scans.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let entry = panels.active_ref().current().unwrap().clone();
        assert_eq!(entry.name, "Alpha");
        assert_eq!(entry.tree_size, Some(1234));
        assert_eq!(listing::size_column(&entry, 8).trim(), "1234");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn insert_marks_and_moves_down_and_the_marked_are_the_sources() {
        let dir = scratch("marks");
        let mut panels = Panels::new(dir.clone());
        // on ".." nothing can be marked or sent anywhere
        assert!(panels.active_ref().sources().is_empty());
        panels.key(key(KeyCode::Down), true); // Alpha
        panels.key(key(KeyCode::Insert), true); // marks Alpha, now on beta
        panels.key(key(KeyCode::Down), true); // apple.txt
        panels.key(key(KeyCode::Insert), true);
        assert_eq!(panels.active_ref().sources(), [dir.join("Alpha"), dir.join("apple.txt")]);
        // Insert again on a marked entry unmarks it
        panels.key(key(KeyCode::Up), true);
        panels.key(key(KeyCode::Insert), true);
        assert_eq!(panels.active_ref().sources(), [dir.join("Alpha")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quick_search_follows_what_is_typed_and_gives_the_other_keys_back() {
        let dir = scratch("fast");
        let mut panels = Panels::new(dir.clone());
        let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
        let name = |panels: &mut Panels| panels.active().current().map(|e| e.name.clone()).unwrap();
        // Alt and a letter open the box and go to the first name with it, case ignored
        assert_eq!(panels.key(alt('z'), true), Key::Taken);
        assert_eq!(panels.fast.as_deref(), Some("z"));
        assert_eq!(name(&mut panels), "zeta.txt");
        // a letter that no name continues with is not taken into the text
        panels.key(key(KeyCode::Char('x')), true);
        assert_eq!(panels.fast.as_deref(), Some("z"));
        // Backspace shortens it, Esc closes the box and leaves the cursor
        panels.key(key(KeyCode::Backspace), true);
        assert_eq!(panels.fast.as_deref(), Some(""));
        panels.key(key(KeyCode::Esc), true);
        assert_eq!(panels.fast, None);
        assert_eq!(name(&mut panels), "zeta.txt");
        // the next letters narrow the search; from the cursor on, then round the end
        panels.key(alt('a'), true);
        assert_eq!(name(&mut panels), "Alpha");
        panels.key(key(KeyCode::Char('p')), true);
        assert_eq!(panels.fast.as_deref(), Some("ap"));
        assert_eq!(name(&mut panels), "apple.txt");
        // Ctrl+Enter goes to the next such name: there is none, the cursor stays
        panels.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), true);
        assert_eq!(name(&mut panels), "apple.txt");
        // any other key closes the box and is the panel's: Up moves the cursor
        panels.key(key(KeyCode::Up), true);
        assert_eq!(panels.fast, None);
        assert_eq!(name(&mut panels), "beta");
        // a letter nothing begins with opens an empty box
        panels.key(alt('q'), true);
        assert_eq!(panels.fast.as_deref(), Some(""));
        assert_eq!(name(&mut panels), "beta");
        panels.key(key(KeyCode::Esc), true);
        // a closing separator asks for folders
        panels.key(alt('b'), true);
        for c in "eta\\".chars() {
            panels.key(key(KeyCode::Char(c)), true);
        }
        assert_eq!(panels.fast.as_deref(), Some("beta\\"));
        panels.key(key(KeyCode::Esc), true);
        // Alt+Minus is still the group mark, not a search
        assert!(matches!(panels.key(alt('-'), true), Key::Ask(Request::SelectGroup(false))));
        assert_eq!(panels.fast, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quick_search_box_is_drawn_over_the_panel_bottom_with_the_cursor_in_it() {
        let dir = scratch("fastbox");
        let mut panels = Panels::new(dir.clone());
        panels.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT), true);
        let mut grid = Grid::new(100, 30);
        let line = vec![Cell::default(); 100];
        let cursor = panels.paint(&mut grid, &line, 0).unwrap();
        // the left panel's bottom row is row 27 (one for the command line, one for the key bar)
        assert_eq!(grid.cell_mut(9, 27).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(10, 28).unwrap().text, " ");
        assert_eq!(grid.cell_mut(11, 28).unwrap().text, "a");
        assert_eq!(cursor, (12, 28));
        panels.key(key(KeyCode::Esc), true);
        let cursor = panels.paint(&mut grid, &line, 3).unwrap();
        assert_eq!(cursor, (3, 28), "the command line's cursor is back");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn function_keys_ask_for_the_right_things() {
        let dir = scratch("fkeys");
        let mut panels = Panels::new(dir.clone());
        assert_eq!(panels.key(key(KeyCode::F(5)), true), Key::Taken, "nothing to copy on ..");
        assert_eq!(panels.key(key(KeyCode::F(7)), true), Key::Ask(Request::MkDir));
        assert_eq!(panels.key(key(KeyCode::F(10)), true), Key::Ask(Request::Quit));
        panels.key(key(KeyCode::Down), true); // Alpha, a folder
        assert_eq!(panels.key(key(KeyCode::F(3)), true), Key::Taken, "a folder is not viewed");
        assert_eq!(
            panels.key(key(KeyCode::F(5)), true),
            Key::Ask(Request::Copy(vec![dir.join("Alpha")], dir.clone()))
        );
        assert_eq!(panels.key(key(KeyCode::F(8)), true), Key::Ask(Request::Delete(vec![dir.join("Alpha")])));
        panels.key(key(KeyCode::Down), true);
        panels.key(key(KeyCode::Down), true); // apple.txt
        assert_eq!(panels.key(key(KeyCode::F(3)), true), Key::Ask(Request::View(dir.join("apple.txt"))));
        assert_eq!(panels.key(key(KeyCode::F(4)), true), Key::Ask(Request::Edit(dir.join("apple.txt"))));
        assert_eq!(panels.key(KeyEvent::new(KeyCode::F(4), KeyModifiers::SHIFT), true), Key::Ask(Request::EditNew(dir.clone())));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn after_a_delete_the_cursor_stays_where_the_file_was() {
        let dir = scratch("delete-cursor");
        for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            std::fs::write(dir.join(name), name).unwrap();
        }
        let mut panels = Panels::new(dir.clone());
        let at = panels.active().entries.iter().position(|e| e.name == "c.txt").unwrap();
        panels.active().cursor = at;
        assert_eq!(panels.active().current().unwrap().name, "c.txt");
        std::fs::remove_file(dir.join("c.txt")).unwrap();
        panels.refresh(None);
        assert_eq!(panels.active().cursor, at, "the cursor did not move");
        assert_eq!(panels.active().current().unwrap().name, "d.txt", "on what followed the deleted file");
        // the last one deleted: the cursor goes to the new last one, not to the top
        std::fs::remove_file(dir.join("d.txt")).unwrap();
        panels.refresh(None);
        assert_eq!(panels.active().current().unwrap().name, "zeta.txt", "the last one now");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refresh_picks_up_new_files_and_can_land_on_one() {
        let dir = scratch("refresh");
        let mut panels = Panels::new(dir.clone());
        std::fs::create_dir(dir.join("new")).unwrap();
        panels.refresh(Some("new"));
        assert_eq!(panels.active_ref().current().unwrap().name, "new");
        assert!(panels.other_dir() == dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ctrl_enter_is_told_from_enter_and_the_name_under_the_cursor_is_found() {
        let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
        assert!(is_insert_name(ctrl(KeyCode::Enter)));
        assert!(is_insert_name(ctrl(KeyCode::Char('j'))));
        assert!(!is_insert_name(key(KeyCode::Enter)));
        assert!(!is_insert_name(ctrl(KeyCode::Char('o'))));
        let dir = scratch("name");
        let mut panels = Panels::new(dir.clone());
        panels.key(key(KeyCode::Down), true);
        assert_eq!(panels.current_name(), Some("Alpha"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn click(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn the_mouse_picks_entries_opens_folders_marks_and_presses_the_key_bar() {
        let dir = scratch("mouse");
        let mut panels = Panels::new(dir.clone());
        let mut grid = Grid::new(80, 24);
        panels.paint(&mut grid, &[], 0);
        let left = MouseEventKind::Down(MouseButton::Left);
        // rows: 2 is "..", 3 Alpha, 4 beta, 5 apple.txt (row 1 has the column titles); the right panel starts at column 40
        assert_eq!(panels.mouse(click(left, 50, 4), false), Key::Taken);
        assert_eq!(panels.active_ref().current().unwrap().name, "beta");
        assert!(panels.right_active, "the click made the right panel active");
        // a double click opens a folder
        panels.mouse(click(left, 50, 4), true);
        assert_eq!(panels.active_dir(), dir.join("beta"));
        // right button marks
        panels.go_to(dir.clone());
        panels.mouse(click(MouseEventKind::Down(MouseButton::Right), 50, 5), false);
        assert!(panels.active_ref().marked.contains("apple.txt"));
        // the wheel moves the cursor by three
        panels.mouse(click(MouseEventKind::ScrollDown, 50, 5), false);
        assert_eq!(panels.active_ref().cursor, 4);
        // F5 sits where the key bar put it (under the left panel's second column)
        let f5 = panels.key_spans(80).unwrap()[4].1;
        assert_eq!(
            panels.mouse(click(left, f5 + 1, 23), false),
            Key::Ask(Request::Copy(vec![dir.join("apple.txt")], dir.clone()))
        );
        // empty space and the command line are not the panels'
        assert_eq!(panels.mouse(click(left, 10, 22), false), Key::Pass);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn far_history_keys_reach_the_shell_as_arrows() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(to_shell(ctrl('e')).code, KeyCode::Up);
        assert_eq!(to_shell(ctrl('x')).code, KeyCode::Down);
        assert_eq!(to_shell(ctrl('a')).code, KeyCode::Char('a'));
    }

    #[test]
    fn one_modifier_changes_the_key_bar_two_do_not() {
        use crate::modkeys::{ALT, CTRL, SHIFT};
        let first = |held| key_bar_labels(held).next().unwrap();
        assert_eq!(first(CTRL), "Left");
        assert_eq!(first(SHIFT), "Add");
        assert_eq!(first(CTRL | SHIFT), "Help");
        assert_eq!(first(CTRL | ALT | SHIFT), "Help");
    }

    #[test]
    fn painting_shows_both_panels_and_the_key_bar() {
        let dir = scratch("paint");
        let mut panels = Panels::new(dir.clone());
        let mut grid = Grid::new(80, 24);
        let red = Color::Rgb { r: 255, g: 0, b: 0 };
        let line: Vec<Cell> =
            "~/src$ ls".chars().map(|c| Cell { text: c.to_string(), fg: red, ..Cell::default() }).collect();
        let cursor = panels.paint(&mut grid, &line, 9);
        assert_eq!(cursor, Some((9, 22)));
        // the shell's own line, in the colors it drew it with
        assert_eq!(grid.cell_mut(1, 22).unwrap().text, "/");
        assert_eq!(grid.cell_mut(1, 22).unwrap().fg, red);
        assert_eq!(grid.cell_mut(0, 0).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(40, 0).unwrap().text, "\u{2554}");
        assert_eq!(grid.cell_mut(0, 23).unwrap().text, "1");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_key_is_longer_than_another_by_more_than_a_column() {
        // Far's (ScrX-1)/12 gave the remainder to F12, up to twice as long
        for cols in [80u16, 100, 119, 150, 211] {
            let spans = even_spans(0, cols, 12);
            let widths: Vec<u16> = spans.iter().map(|s| s.1).collect();
            assert!(widths.iter().max().unwrap() - widths.iter().min().unwrap() <= 1, "{cols}: {widths:?}");
            assert_eq!(widths.iter().sum::<u16>(), cols, "the keys fill the row");
            let mut grid = Grid::new(cols, 3);
            let ends = paint_labels(&mut grid, &["Help"; 12]);
            assert_eq!(ends.last().copied(), Some(cols), "{cols}: F12 ends at the edge, no longer than the rest");
        }
        // a screen too narrow for twelve keys of 8 shows fewer, as Far does
        assert_eq!(paint_labels(&mut Grid::new(60, 3), &[]).len(), 7);
    }

    #[test]
    fn the_key_bar_stands_under_the_panels_columns() {
        // six keys under each panel: Medium (two columns) three under each,
        // Brief (three columns) two under each
        let dir = scratch("keybar");
        let mut panels = Panels::new(dir.clone());
        panels.set_view(false, 2); // left Medium
        panels.set_view(true, 1); // right Brief
        let mut grid = Grid::new(100, 24);
        panels.paint(&mut grid, &[], 0);
        let spans = panels.key_spans(100).expect("no full-screen panel");
        assert_eq!(spans.len(), 12);
        let starts: Vec<u16> = spans.iter().map(|s| s.1).collect();
        // F1 at the left edge, F4 at the left panel's second column, F7 at the right panel
        assert_eq!(starts[0], 0);
        assert_eq!(starts[3], panels.left.stripe_cols[1].0);
        assert_eq!(starts[6], 50);
        // the right panel's Brief: F9 and F11 at its second and third columns
        assert_eq!(starts[8], panels.right.stripe_cols[1].0);
        assert_eq!(starts[10], panels.right.stripe_cols[2].0);
        // the key bar row shows the numbers where the spans start
        for (i, &(_, x, _)) in spans.iter().enumerate() {
            let digit = (i + 1).to_string();
            assert_eq!(grid.cell_mut(x, 23).unwrap().text, digit[..1], "F{} at {x}", i + 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn zip_of_scratch(name: &str) -> PathBuf {
        let dir = scratch(name);
        let options = crate::archive::Options {
            path: dir.join("out.zip"),
            format: crate::archive::Format::Zip,
            level: 5,
            method: "Deflate".into(),
            solid: false,
            password: None,
            encrypt_header: false,
            volume_size: None,
            move_files: false,
            ignore_errors: false,
            overwrite: crate::archive::Overwrite::Ask,
        };
        struct Quiet;
        impl crate::archive::Progress for Quiet {
            fn file(&mut self, _: &crate::archive::Entry) -> std::io::Result<()> {
                Ok(())
            }
            fn bytes(&mut self, _: u64) -> std::io::Result<()> {
                Ok(())
            }
        }
        crate::archive::create(&options, &[dir.join("Alpha"), dir.join("zeta.txt")], &mut Quiet).unwrap();
        std::fs::write(dir.join("Alpha").join("inner.txt"), "in").unwrap();
        dir
    }

    #[test]
    fn enter_opens_an_archive_as_arclite_does_and_dots_leave_it_on_the_archive() {
        let dir = zip_of_scratch("arcpanel");
        let mut panel = Panel::new(dir.clone());
        panel.cursor = panel.entries.iter().position(|e| e.name == "out.zip").unwrap();
        panel.enter().unwrap();
        assert_eq!(panel.archive.as_ref().unwrap().title(), "ArcLite:zip:out.zip");
        assert_eq!(names(&panel), ["..", "Alpha", "zeta.txt"]);
        panel.cursor = 1;
        panel.enter().unwrap();
        assert_eq!(panel.archive.as_ref().unwrap().title(), "ArcLite:zip:out.zip:\\Alpha");
        assert_eq!(names(&panel), [".."], "Alpha was empty when it went in");
        panel.enter().unwrap();
        assert_eq!(panel.entries[panel.cursor].name, "Alpha", "back up, on the folder");
        panel.cursor = 0;
        panel.enter().unwrap();
        assert!(panel.archive.is_none());
        assert_eq!(panel.dir, dir);
        assert_eq!(panel.entries[panel.cursor].name, "out.zip", "out of the archive, on it");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn f5_in_an_archive_asks_to_extract_and_f8_says_farsh_does_not_change_archives() {
        let dir = zip_of_scratch("arcf5");
        let mut panels = Panels::new(dir.clone());
        let at = panels.left.entries.iter().position(|e| e.name == "out.zip").unwrap();
        panels.left.cursor = at;
        panels.key(key(KeyCode::Enter), true);
        panels.key(key(KeyCode::Down), true);
        panels.key(key(KeyCode::Down), true); // zeta.txt
        let Key::Ask(Request::Extract(pick, dest, false)) = panels.key(key(KeyCode::F(5)), true) else { panic!("F5 extracts") };
        assert_eq!(pick.items, ["zeta.txt"]);
        assert_eq!(dest, dir);
        assert!(matches!(panels.key(key(KeyCode::F(8)), true), Key::Ask(Request::DeleteInArchive(_))), "a zip can lose files");
        assert!(matches!(panels.key(key(KeyCode::F(3)), true), Key::Ask(Request::ViewInArchive(_))));
        assert_eq!(panels.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL), true), Key::Taken, "no Ctrl+A in an archive");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn f5_into_a_zip_adds_and_into_another_archive_or_f8_there_is_not_supported() {
        let dir = zip_of_scratch("arcput");
        // a tar beside the zip
        let tar = crate::archive::Options {
            path: dir.join("t.tar"),
            format: crate::archive::Format::Tar,
            level: 0,
            method: "Copy".into(),
            solid: false,
            password: None,
            encrypt_header: false,
            volume_size: None,
            move_files: false,
            ignore_errors: false,
            overwrite: crate::archive::Overwrite::Ask,
        };
        struct Quiet;
        impl crate::archive::Progress for Quiet {
            fn file(&mut self, _: &crate::archive::Entry) -> std::io::Result<()> {
                Ok(())
            }
            fn bytes(&mut self, _: u64) -> std::io::Result<()> {
                Ok(())
            }
        }
        crate::archive::create(&tar, &[dir.join("zeta.txt")], &mut Quiet).unwrap();
        let mut panels = Panels::new(dir.clone());
        panels.right.reload(None);
        // the right panel opens the zip, the left stays on the folder
        panels.right.cursor = panels.right.entries.iter().position(|e| e.name == "out.zip").unwrap();
        panels.right.enter().unwrap();
        panels.left.cursor = panels.left.entries.iter().position(|e| e.name == "apple.txt").unwrap();
        let Key::Ask(Request::AddToArchive(sources, view, false)) = panels.key(key(KeyCode::F(5)), true) else { panic!("F5 adds") };
        assert_eq!(sources, [dir.join("apple.txt")]);
        assert_eq!(view.file, dir.join("out.zip"));
        assert!(matches!(panels.key(key(KeyCode::F(6)), true), Key::Ask(Request::AddToArchive(_, _, true))));
        // a tar cannot be changed: F5 into it, F8 in it
        panels.right.archive = None;
        panels.right.reload(None);
        panels.right.cursor = panels.right.entries.iter().position(|e| e.name == "t.tar").unwrap();
        panels.right.enter().unwrap();
        let Key::Ask(Request::Error(message)) = panels.key(key(KeyCode::F(5)), true) else { panic!("into a tar") };
        assert_eq!(message, "Archive modification not supported");
        panels.key(key(KeyCode::Tab), true);
        panels.key(key(KeyCode::Down), true);
        let Key::Ask(Request::Error(message)) = panels.key(key(KeyCode::F(8)), true) else { panic!("F8 in a tar") };
        assert_eq!(message, "Archive modification not supported");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn painted(panels: &mut Panels) -> Vec<String> {
        let mut grid = Grid::new(80, 25);
        panels.paint(&mut grid, &[], 0);
        (0..25).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect()
    }

    #[test]
    fn the_widths_are_fars_prepare_column_widths() {
        // Descriptions in a 38-cell panel: the name 40% of what is left, the description the rest
        assert_eq!(layout(&ViewMode::Descriptions.columns(), 38), [(Col::Name, 14), (Col::Description, 23)]);
        // Detailed on the whole 80-cell screen: the fixed columns, the name what is left
        let detailed = layout(&ViewMode::Detailed.columns(), 78);
        assert_eq!(detailed.iter().map(|c| c.1).collect::<Vec<_>>(), [12, 6, 6, 14, 14, 14, 6]);
        // Long descriptions: the description 70% of what is left (no zero-width column but the name)
        let long = layout(&ViewMode::LongDescriptions.columns(), 78);
        assert_eq!(long, [(Col::Name, 21), (Col::Size, 6), (Col::Description, 49)], "70 left: 70 * 70 / 100 = 49, the name the 21 after");
    }

    #[test]
    fn ctrl_5_to_ctrl_0_switch_to_fars_other_modes() {
        let dir = scratch("modes");
        let mut panels = Panels::new(dir.clone());
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        panels.key(ctrl('5'), true);
        assert_eq!(panels.view_modes()[0], 5);
        let rows = painted(&mut panels);
        // the whole screen, Far's titles
        assert_eq!(rows[0].chars().next(), Some('\u{2554}'));
        // the clock is in the top right corner (Interface settings' Clock)
        assert_eq!(rows[0].chars().nth(77), Some(':'));
        for title in ["Name", "Size", "Alloca", "Write", "Created", "Accessed", "Attr"] {
            assert!(rows[1].contains(title), "{title} in {:?}", rows[1]);
        }
        // a folder's attributes letter D, a file's A (Windows) or nothing extra
        assert!(rows.iter().any(|r| r.contains("Alpha") && r.trim_end_matches(['\u{2551}', ' ']).ends_with('D')), "{rows:#?}");
        for (key, number) in [('6', 6), ('7', 7), ('8', 8), ('9', 9), ('0', 0), ('1', 1)] {
            panels.key(ctrl(key), true);
            assert_eq!(panels.view_modes()[0], number);
            painted(&mut panels);
        }
        // Alternative full: the mark before a marked name
        panels.key(ctrl('0'), true);
        panels.key(key(KeyCode::Down), true);
        panels.key(key(KeyCode::Insert), true);
        let rows = painted(&mut panels);
        assert!(rows.iter().any(|r| r.contains("\u{221A} Alpha")), "{rows:#?}");
        let rows_titles = &rows[1];
        assert!(rows_titles.contains("Size") && rows_titles.contains("Date") && !rows_titles.contains("Time"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ctrl_l_and_ctrl_q_turn_the_other_panel_into_info_and_quick_view() {
        let dir = scratch("kinds");
        let mut panels = Panels::new(dir.clone());
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        panels.key(ctrl('l'), true);
        assert_eq!((panels.left.kind, panels.right.kind), (Kind::Files, Kind::Info));
        let rows = painted(&mut panels);
        assert!(rows[0].contains(" Information "), "{:?}", rows[0]);
        // Ctrl+Q over it: the other panel becomes the quick view of the file under the cursor
        panels.key(ctrl('q'), true);
        assert_eq!(panels.right.kind, Kind::QuickView);
        for _ in 0..3 {
            panels.key(key(KeyCode::Down), true);
        }
        assert_eq!(panels.left.current().unwrap().name, "apple.txt");
        let rows = painted(&mut panels);
        // Ctrl+Q's layout: two framed panels, the viewer's titled (the next test has the details)
        assert!(rows[0].starts_with('\u{2554}') && rows[0].contains(" Quick view "), "{:?}", rows[0]);
        // Brief aligns the extensions: "apple" and "txt" apart
        assert!(rows.iter().any(|r| r.starts_with("\u{2551}apple") && r.contains("txt")), "{rows:#?}");
        // again: a file panel
        panels.key(ctrl('q'), true);
        assert_eq!(panels.right.kind, Kind::Files);
        // Ctrl+T does nothing: Far's tree is off by default
        panels.key(ctrl('t'), true);
        assert_eq!(panels.right.kind, Kind::Files);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ctrl_q_is_a_framed_one_column_panel_in_a_sixth_and_a_framed_viewer_in_the_rest() {
        let dir = scratch("ctrlq");
        std::fs::write(dir.join("text.txt"), "hello from the quick view\n").unwrap();
        let mut panels = Panels::new(dir.clone());
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let before = panels.view_modes();
        let framed = painted(&mut panels);
        assert!(framed[0].starts_with('\u{2554}'));
        panels.key(ctrl('q'), true);
        // the cursor to text.txt: "..", Alpha, beta, apple.txt, text.txt
        while panels.left.current().is_some_and(|e| e.name != "text.txt") {
            panels.key(key(KeyCode::Down), true);
        }
        let mut grid = Grid::new(120, 25);
        panels.paint(&mut grid, &[], 0);
        let rows: Vec<String> = (0..25).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect();
        // the file panel: a sixth of the screen, one column of names, framed, the folder in its border
        assert_eq!(panels.left.area, (0, 0, 20, panels.left.area.3));
        assert_eq!(panels.left.stripes, 1);
        assert!(rows[0].starts_with('\u{2554}') && rows[0].contains('\u{2557}'), "{:?}", rows[0]);
        assert!(rows[1].starts_with("\u{2551}..") && rows[1].chars().nth(19) == Some('\u{2551}'), "names from the first row inside the frame: {:?}", rows[1]);
        // the viewer: the other five sixths, framed and titled, the file's text inside the frame
        assert_eq!(panels.right.area.0, 20);
        assert_eq!(panels.right.area.2, 100);
        assert!(from_col(&rows[0], 20).starts_with('\u{2554}') && rows[0].contains(" Quick view "), "{:?}", rows[0]);
        assert!(from_col(&rows[1], 21).starts_with("hello from the quick view"), "{:?}", rows[1]);
        // the command line is the row above the key bar, and the key bar is all twelve keys
        // in one even row (the narrow panel has no room for six of its own)
        assert!(panels.key_spans(120).is_none());
        for label in ["1Help", "3View", "4Edit", "9ConfMn", "10Quit", "12Screen"] {
            assert!(rows[24].contains(label), "{label}: {:?}", rows[24]);
        }
        assert_eq!(panels.key_ends.len(), 12);
        // F3 is the key at column 20: a click there views the file
        let f3_x = panels.key_ends[2].0;
        assert_eq!(panels.mouse(click(MouseEventKind::Down(MouseButton::Left), f3_x + 1, 24), false), Key::Ask(Request::View(dir.join("text.txt"))));
        // leaving Ctrl+Q: the panels as they were (frames, titles, mode), the cursor where it went
        panels.key(ctrl('q'), true);
        assert_eq!(panels.view_modes(), before);
        let after = painted(&mut panels);
        assert!(after[0].starts_with('\u{2554}'));
        assert_eq!(after[1], framed[1], "the column titles again");
        assert_eq!(panels.left.current().unwrap().name, "text.txt");
        std::fs::remove_dir_all(dir).unwrap();
    }
    /// A quick view (Ctrl+Q's layout) of `files`, the cursor on `name`.
    fn quick_view_of(files: &[(&str, &str)], name: &str) -> (Panels, PathBuf) {
        let dir = scratch(&format!("qtext-{name}"));
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        let mut panels = Panels::new(dir.clone());
        panels.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL), true);
        while panels.left.current().is_some_and(|e| e.name != name) {
            panels.key(key(KeyCode::Down), true);
        }
        (panels, dir)
    }

    /// What a screen row shows from column `col` on (rows hold frame characters
    /// of several bytes, so a byte slice is not a column).
    fn from_col(row: &str, col: usize) -> String {
        row.chars().skip(col).collect()
    }

    fn quick_rows(panels: &mut Panels) -> Vec<String> {
        let mut grid = Grid::new(120, 25);
        panels.paint(&mut grid, &[], 0);
        (0..25).map(|row| grid.row(row).iter().map(|cell| cell.text.clone()).collect()).collect()
    }

    #[test]
    fn the_quick_view_shows_text_in_fresh_highlighted_and_follows_the_cursor_in_one_session() {
        let (mut panels, dir) = quick_view_of(
            &[("main.rs", "fn main() {\n    let answer = \"forty-two\"; // a comment\n}\n"), ("note.txt", "just a note\n")],
            "main.rs",
        );
        let mut grid = Grid::new(120, 25);
        // the highlighting arrives with the editor's own ticks
        let mut colors = std::collections::HashSet::new();
        for _ in 0..200 {
            panels.paint(&mut grid, &[], 0);
            panels.tick();
            let (x0, y0, w, h) = panels.qrect;
            colors = (y0..y0 + h)
                .flat_map(|y| (x0..x0 + w).map(move |x| (x, y)))
                .filter_map(|(x, y)| grid.cell_mut(x, y).cloned())
                .filter(|cell| cell.text.trim() != "")
                .map(|cell| format!("{:?}", cell.fg))
                .collect();
            if colors.len() > 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(colors.len() > 2, "keywords, strings and comments differ in color: {colors:?}");
        let rows = quick_rows(&mut panels);
        assert!(from_col(&rows[1], 21).starts_with("fn main()"), "{:?}", rows[0]);
        // the cursor moves to the next file: the same session shows it
        let session = panels.qeditor.as_ref().map(|view| view as *const _ as usize);
        panels.key(key(KeyCode::Down), true);
        while panels.left.current().is_some_and(|e| e.name != "note.txt") {
            panels.key(key(KeyCode::Down), true);
        }
        let rows = quick_rows(&mut panels);
        assert!(from_col(&rows[1], 21).starts_with("just a note"), "{rows:#?}");
        assert_eq!(panels.qeditor.as_ref().map(|view| view as *const _ as usize), session, "one session, not one per file");
        // it settles (fresh's own short timers run out) and then asks for nothing:
        // the panels do not redraw forever
        let started = std::time::Instant::now();
        while panels.busy() && started.elapsed() < std::time::Duration::from_secs(3) {
            panels.tick();
            quick_rows(&mut panels);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!panels.busy(), "an idle quick view must not keep the screen redrawing");
        // no quick view shown: the session is gone
        panels.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL), true);
        quick_rows(&mut panels);
        assert!(panels.qeditor.is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_wheel_scrolls_the_quick_views_text_and_the_view_is_read_only() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let (mut panels, dir) = quick_view_of(&[("long.txt", &text)], "long.txt");
        let rows = quick_rows(&mut panels);
        assert!(from_col(&rows[1], 21).starts_with("line 1"), "{:?}", rows[0]);
        let wheel = |kind, column| MouseEvent { kind, column, row: 5, modifiers: KeyModifiers::NONE };
        // over the viewer (to the right of the narrow panel)
        for _ in 0..3 {
            assert_eq!(panels.mouse(wheel(MouseEventKind::ScrollDown, 60), false), Key::Taken);
            panels.tick();
        }
        let rows = quick_rows(&mut panels);
        let first: usize = from_col(&rows[1], 21).trim_start_matches("line ").trim().split_whitespace().next().unwrap().parse().unwrap();
        assert!(first > 1, "scrolled: {:?}", rows[0]);
        // the wheel did not move the file panel's cursor
        assert_eq!(panels.left.current().unwrap().name, "long.txt");
        // a click in the quick view leaves the focus on the file panel
        let was = panels.right_active;
        for button in [MouseButton::Left, MouseButton::Right] {
            panels.mouse(wheel(MouseEventKind::Down(button), 60), false);
            assert_eq!(panels.right_active, was);
        }
        // typing is refused: the text of the file is not changed by keys sent to a quick view
        panels.right_active = true;
        panels.key(key(KeyCode::Char('Z')), true);
        assert!(panels.qeditor.as_mut().is_some_and(|view| !view.session().modified()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_waits_for_the_cursor_to_stay_before_it_is_read() {
        let dir = scratch("debounce");
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        let mut state = crate::qview::State::new(dir.join("a.txt"));
        state.set_debounce(std::time::Duration::from_millis(60));
        let mut editor = None;
        let mut grid = Grid::new(40, 10);
        crate::qview::paint(&mut grid, 0, 40, 10, false, Some(&mut state), &mut editor);
        assert!(state.pending() && editor.is_none(), "the cursor has just come: nothing is opened");
        std::thread::sleep(std::time::Duration::from_millis(80));
        crate::qview::paint(&mut grid, 0, 40, 10, false, Some(&mut state), &mut editor);
        assert!(!state.pending() && editor.is_some());
        assert!(grid.row(1).iter().skip(1).map(|c| c.text.as_str()).collect::<String>().starts_with("alpha"), "inside the frame");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_binary_file_and_an_unreadable_one_say_so() {
        let (mut panels, dir) = quick_view_of(&[("data.bin", "MZ\u{0}\u{0}\u{3}\u{0}")], "data.bin");
        let rows = quick_rows(&mut panels);
        assert!(from_col(&rows[1], 21).starts_with("A binary file"), "{:?}", rows[0]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_binary_file_is_told_apart_from_text() {
        let dir = scratch("binary");
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        assert!(crate::qview::is_binary(&file("nul.bin", b"MZ\x90\x00\x03\x00\x00\x00")));
        assert!(crate::qview::is_binary(&file("ctl.bin", &[1, 2, 3, 4, 5, 6, 7, 8, b'a', b'b'])));
        assert!(!crate::qview::is_binary(&file("text.txt", "plain text\twith tabs\r\nand \x1b[1mescapes\x1b[0m\n".as_bytes())));
        assert!(!crate::qview::is_binary(&file("utf16.txt", &[0xFF, 0xFE, b'h', 0, b'i', 0])));
        assert!(!crate::qview::is_binary(&file("empty.txt", b"")));
        assert!(!crate::qview::is_binary(&file("\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}.txt", "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}".as_bytes())));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ctrl_f1_and_ctrl_f2_hide_and_show_a_panel_and_tab_does_not_go_to_a_hidden_one() {
        let dir = scratch("onoff");
        let mut panels = Panels::new(dir.clone());
        let ctrl_f = |n| KeyEvent::new(KeyCode::F(n), KeyModifiers::CONTROL);
        panels.key(ctrl_f(1), true);
        assert!(!panels.left.visible);
        assert!(panels.right_active, "the other panel took over");
        panels.key(key(KeyCode::Tab), true);
        assert!(panels.right_active, "nothing to Tab to");
        let rows = painted(&mut panels);
        assert!(!rows[0].starts_with('\u{2554}'), "the left half is the shell's: {:?}", rows[0]);
        panels.key(ctrl_f(2), true);
        assert!(!panels.right.visible && !panels.left.visible);
        // showing one when the other is hidden makes it active
        panels.key(ctrl_f(1), true);
        assert!(panels.left.visible && !panels.right_active);
        panels.key(ctrl_f(2), true);
        assert!(panels.right.visible && !panels.right_active, "the other was shown: the active stays");
        // Ctrl+N switches the active panel to short names and back
        panels.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL), true);
        assert!(panels.left.short_names);
        assert_eq!(panels.states()[0], (Kind::Files, true, false), "the menu: long names unchecked");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn alt_plus_minus_star_select_by_mask_and_ctrl_m_brings_the_marks_back() {
        let dir = scratch("select");
        let mut panels = Panels::new(dir.clone());
        let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
        assert_eq!(panels.key(alt('+'), false), Key::Ask(Request::SelectGroup(true)));
        assert_eq!(panels.key(alt('='), false), Key::Ask(Request::SelectGroup(true)), "the + key without Shift");
        assert_eq!(panels.key(alt('-'), false), Key::Ask(Request::SelectGroup(false)));
        panels.select_group(true, &crate::masks::Masks::new("*.*").unwrap());
        let mut marked: Vec<&String> = panels.left.marked.iter().collect();
        marked.sort();
        assert_eq!(marked, ["apple.txt", "zeta.txt"], "files only: Far's SelectFolders is off");
        panels.select_group(false, &crate::masks::Masks::new("z*").unwrap());
        assert!(panels.left.marked.contains("apple.txt") && !panels.left.marked.contains("zeta.txt"));
        // Ctrl+M: the marks before the last operation; again, back
        panels.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL), true);
        assert_eq!(panels.left.marked.len(), 2);
        panels.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL), true);
        assert_eq!(panels.left.marked.len(), 1);
        // Alt+*: files flip, a marked folder is unmarked, an unmarked one stays so
        panels.left.marked.insert("Alpha".into());
        panels.key(alt('*'), true);
        let mut marked: Vec<&String> = panels.left.marked.iter().collect();
        marked.sort();
        assert_eq!(marked, ["zeta.txt"]);
        // Alt+8, the * key without Shift, flips them back
        panels.key(alt('8'), true);
        assert!(panels.left.marked.contains("apple.txt") && !panels.left.marked.contains("zeta.txt"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ctrl_g_ctrl_z_alt_f6_and_shift_f3_ask_for_what_is_selected() {
        let dir = scratch("files-menu");
        let mut panels = Panels::new(dir.clone());
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        assert_eq!(panels.key(ctrl('g'), true), Key::Taken, "nothing on ..");
        panels.key(key(KeyCode::End), true); // zeta.txt
        assert_eq!(panels.key(ctrl('g'), true), Key::Ask(Request::ApplyCommand(vec!["zeta.txt".into()])));
        assert_eq!(panels.key(ctrl('z'), true), Key::Ask(Request::Describe(dir.clone(), vec!["zeta.txt".into()])));
        assert_eq!(panels.key(KeyEvent::new(KeyCode::F(6), KeyModifiers::ALT), true), Key::Ask(Request::Link(vec![dir.join("zeta.txt")], dir.clone())));
        assert_eq!(panels.key(KeyEvent::new(KeyCode::F(3), KeyModifiers::SHIFT), true), Key::Taken, "not an archive");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
