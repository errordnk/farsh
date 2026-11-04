//! Copy, move and delete in a worker thread, with progress, cancelling and
//! the questions Far asks (a file that already exists).

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
    Delete,
    /// Alt+Del: each file is overwritten, cut to nothing, renamed, then removed.
    Wipe,
    /// Shift+F1: an archive is made (`crate::archive`).
    Pack,
}

impl Kind {
    /// The word for the progress box title.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Copy => "Copy",
            Kind::Move => "Move",
            Kind::Delete => "Delete",
            Kind::Wipe => "Wipe",
            Kind::Pack => "Creating archive...",
        }
    }
}

/// What to do with a file that already exists at the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    Ask,
    Overwrite,
    Skip,
}

/// Far's "Access rights": which permissions the copy gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// What the system gives a new file (the permissions of the source).
    Default,
    /// The source's permissions, always.
    Copy,
    /// Left to the destination folder: nothing is set.
    Inherit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub existing: Existing,
    /// Also take the access time (the modification time is always kept).
    pub timestamps: bool,
    /// A symbolic link is copied as the file it points to, not as a link.
    pub follow_links: bool,
    pub access: Access,
}

impl Default for Options {
    fn default() -> Self {
        Self { existing: Existing::Ask, timestamps: false, follow_links: false, access: Access::Default }
    }
}

/// What the progress box shows.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub from: String,
    pub to: String,
    pub file_done: u64,
    pub file_size: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: usize,
    pub files_total: usize,
}

/// A file that is in the way: the worker waits for an [`Answer`].
#[derive(Clone, Debug)]
pub struct Conflict {
    pub dst: PathBuf,
    pub new_size: u64,
    pub new_modified: Option<SystemTime>,
    pub old_size: u64,
    pub old_modified: Option<SystemTime>,
}

/// What the worker stops to ask (Far's messages during a copy or a delete).
#[derive(Clone, Debug)]
pub enum Question {
    Conflict(Conflict),
    /// Delete/Wipe: "The following folder will be deleted:" (it has something in it).
    FolderNotEmpty(PathBuf),
    /// Delete/Wipe: "The file is read only".
    ReadOnly(PathBuf),
    /// Wipe: "Several hard links link to this file."
    HardLinks(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Overwrite { remember: bool },
    Skip { remember: bool },
    /// Delete/Wipe: this one (Far's first button).
    Proceed,
    /// Delete/Wipe: this one and every other like it.
    All,
    /// Delete/Wipe: skip this one and every other like it.
    SkipAll,
    Cancel,
}

enum Event {
    Progress(Progress),
    Ask(Question),
    Done(Result<(), String>),
}

pub struct Job {
    pub kind: Kind,
    pub progress: Progress,
    pub started: Instant,
    /// Set when the worker is finished: `Ok`, or what went wrong (cancelling is an error).
    pub result: Option<Result<(), String>>,
    /// A question the worker is waiting on.
    pub question: Option<Question>,
    events: Receiver<Event>,
    answers: Sender<Answer>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Takes what the worker has reported so far.
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Progress(progress) => self.progress = progress,
                Event::Ask(question) => self.question = Some(question),
                Event::Done(result) => self.result = Some(result),
            }
        }
    }

    pub fn answer(&mut self, answer: Answer) {
        self.question = None;
        let _ = self.answers.send(answer);
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// Starts `kind` over `items` (source, target); a delete ignores the target.
pub fn start(kind: Kind, items: Vec<(PathBuf, PathBuf)>, options: Options) -> Job {
    let (sender, events) = mpsc::channel();
    let (answers, answer_receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    std::thread::spawn(move || {
        let result = Worker::new(kind, options, sender.clone(), answer_receiver, worker_cancel).run(&items);
        let _ = sender.send(Event::Done(result.map_err(|e| e.to_string())));
    });
    Job { kind, progress: Progress::default(), started: Instant::now(), result: None, question: None, events, answers, cancel }
}

/// Work of another kind (an archive being made) run as a job: it reports
/// through the [`Reporter`], Esc cancels it.
pub type Work = Box<dyn FnOnce(&mut Reporter) -> std::io::Result<()> + Send>;

pub fn spawn(kind: Kind, work: Work) -> Job {
    let (sender, events) = mpsc::channel();
    let (answers, answer_receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    std::thread::spawn(move || {
        let mut worker = Worker::new(kind, Options::default(), sender.clone(), answer_receiver, worker_cancel);
        let result = work(&mut Reporter(&mut worker));
        worker.send(true);
        let _ = sender.send(Event::Done(result.map_err(|e| e.to_string())));
    });
    Job { kind, progress: Progress::default(), started: Instant::now(), result: None, question: None, events, answers, cancel }
}

/// What a [`Work`] tells the progress box, and where it learns it was cancelled.
pub struct Reporter<'a>(&'a mut Worker);

impl Reporter<'_> {
    pub fn progress(&mut self) -> &mut Progress {
        &mut self.0.progress
    }

    /// Sends the progress (at most every 50 ms); `Err` when the user cancelled.
    pub fn tick(&mut self) -> std::io::Result<()> {
        self.0.send(false);
        self.0.check_cancel()
    }
}

/// Where each of `sources` goes when the user typed `typed` in the Copy/Move
/// box with the active panel in `base`: into the folder `typed` names, or, for
/// a single source, to the new name `typed` gives it. With `multiple`, `typed`
/// may name several destinations separated by `;` or `,`: each gets every source.
pub fn targets(sources: &[PathBuf], typed: &str, base: &Path, multiple: bool) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let names: Vec<&str> = if multiple { typed.split([';', ',']).map(str::trim).filter(|n| !n.is_empty()).collect() } else { vec![typed.trim()] };
    if names.is_empty() || names.iter().any(|n| n.is_empty()) {
        return Err("The destination is empty".into());
    }
    let mut pairs = Vec::new();
    for name in names {
        pairs.extend(targets_one(sources, name, base)?);
    }
    Ok(pairs)
}

fn targets_one(sources: &[PathBuf], typed: &str, base: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let dest = base.join(typed);
    let pairs: Vec<(PathBuf, PathBuf)> = if dest.is_dir() {
        sources
            .iter()
            .map(|src| (src.clone(), dest.join(src.file_name().unwrap_or_default())))
            .collect()
    } else if let [src] = sources {
        if dest.parent().is_some_and(|parent| !parent.is_dir()) {
            return Err(format!("The folder {} does not exist", dest.parent().unwrap().display()));
        }
        vec![(src.clone(), dest)]
    } else {
        return Err(format!("{} is not a folder", dest.display()));
    };
    for (src, dst) in &pairs {
        if src == dst {
            return Err(format!("{} cannot be copied onto itself", src.display()));
        }
        if dst.starts_with(src) {
            return Err(format!("{} cannot be copied into itself", src.display()));
        }
    }
    Ok(pairs)
}

struct Entry {
    src: PathBuf,
    dst: PathBuf,
    is_dir: bool,
    is_link: bool,
    size: u64,
}

/// `src` and everything under it, folders before their contents.
fn walk(src: &Path, dst: &Path, follow_links: bool, out: &mut Vec<Entry>) -> std::io::Result<()> {
    let link = fs::symlink_metadata(src)?;
    let is_link = link.file_type().is_symlink();
    // A link that is not followed is one entry of its own.
    let meta = if is_link && follow_links { fs::metadata(src)? } else { link };
    if meta.is_dir() && !(is_link && !follow_links) {
        out.push(Entry { src: src.into(), dst: dst.into(), is_dir: true, is_link: false, size: 0 });
        let mut children: Vec<_> = fs::read_dir(src)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|child| child.file_name());
        for child in children {
            walk(&child.path(), &dst.join(child.file_name()), follow_links, out)?;
        }
    } else {
        let is_link = is_link && !follow_links;
        out.push(Entry { src: src.into(), dst: dst.into(), is_dir: false, is_link, size: if is_link { 0 } else { meta.len() } });
    }
    Ok(())
}

struct Worker {
    kind: Kind,
    options: Options,
    events: Sender<Event>,
    answers: Receiver<Answer>,
    cancel: Arc<AtomicBool>,
    progress: Progress,
    last_sent: Instant,
    /// What "remember" turned the policy into.
    existing: Existing,
    /// Delete/Wipe: "All" was the answer about a folder with something in it.
    delete_folders: bool,
    /// Delete/Wipe: "All" (`Some(true)`) or "Skip all" about a read-only file.
    read_only: Option<bool>,
    /// Delete/Wipe: something was skipped, so the folders holding it stay.
    kept: bool,
    /// Wipe: "All" (`Some(true)`) or "Skip all" about a file with hard links.
    hard_links: Option<bool>,
    /// Sources a copy skipped: a move must not delete them.
    skipped: HashSet<PathBuf>,
}

impl Worker {
    fn new(kind: Kind, options: Options, events: Sender<Event>, answers: Receiver<Answer>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            kind,
            existing: options.existing,
            delete_folders: false,
            read_only: None,
            kept: false,
            hard_links: None,
            options,
            events,
            answers,
            cancel,
            progress: Progress::default(),
            last_sent: Instant::now() - Duration::from_secs(1),
            skipped: HashSet::new(),
        }
    }

    fn check_cancel(&self) -> std::io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("Cancelled"));
        }
        Ok(())
    }

    fn send(&mut self, force: bool) {
        if force || self.last_sent.elapsed() >= Duration::from_millis(50) {
            self.last_sent = Instant::now();
            let _ = self.events.send(Event::Progress(self.progress.clone()));
        }
    }

    fn run(&mut self, items: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
        match self.kind {
            Kind::Copy => {
                let plan = plan(items, self.options.follow_links)?;
                self.start_totals(&plan);
                self.copy_all(&plan)
            }
            Kind::Move => self.move_all(items),
            Kind::Pack => Ok(()),
            Kind::Delete | Kind::Wipe => {
                for (src, _) in items {
                    self.remove(src)?;
                }
                self.send(true);
                Ok(())
            }
        }
    }

    fn start_totals(&mut self, plan: &[Entry]) {
        self.progress.bytes_total = plan.iter().map(|e| e.size).sum();
        self.progress.files_total = plan.iter().filter(|e| !e.is_dir).count();
        self.send(true);
    }

    fn copy_all(&mut self, plan: &[Entry]) -> std::io::Result<()> {
        for entry in plan {
            self.check_cancel()?;
            if entry.is_dir {
                if entry.dst.exists() && !entry.dst.is_dir() {
                    return Err(std::io::Error::other(format!("{} already exists", entry.dst.display())));
                }
                fs::create_dir_all(&entry.dst)?;
            } else {
                self.copy_file(entry)?;
                self.progress.files_done += 1;
            }
        }
        self.send(true);
        Ok(())
    }

    /// Decides what to do with a file in the way: `true` to overwrite it.
    fn overwrite(&mut self, entry: &Entry) -> std::io::Result<bool> {
        let policy = match self.existing {
            Existing::Ask => {
                let old = fs::metadata(&entry.dst)?;
                if old.is_dir() {
                    return Err(std::io::Error::other(format!("{} is a folder", entry.dst.display())));
                }
                let new = fs::metadata(&entry.src).ok();
                let conflict = Conflict {
                    dst: entry.dst.clone(),
                    new_size: entry.size,
                    new_modified: new.and_then(|m| m.modified().ok()),
                    old_size: old.len(),
                    old_modified: old.modified().ok(),
                };
                let _ = self.events.send(Event::Ask(Question::Conflict(conflict)));
                // The worker waits here for the user; cancelling answers too.
                match self.answers.recv() {
                    Ok(Answer::Overwrite { remember }) => {
                        if remember {
                            self.existing = Existing::Overwrite;
                        }
                        Existing::Overwrite
                    }
                    Ok(Answer::Skip { remember }) => {
                        if remember {
                            self.existing = Existing::Skip;
                        }
                        Existing::Skip
                    }
                    Ok(_) | Err(_) => return Err(std::io::Error::other("Cancelled")),
                }
            }
            other => other,
        };
        Ok(policy == Existing::Overwrite)
    }

    fn copy_file(&mut self, entry: &Entry) -> std::io::Result<()> {
        self.progress.from = entry.src.display().to_string();
        self.progress.to = entry.dst.display().to_string();
        self.progress.file_size = entry.size;
        self.progress.file_done = 0;
        let exists = fs::symlink_metadata(&entry.dst).is_ok();
        if exists && !self.overwrite(entry)? {
            self.skipped.insert(entry.src.clone());
            self.progress.bytes_done += entry.size;
            self.send(true);
            return Ok(());
        }
        if entry.is_link {
            if exists {
                fs::remove_file(&entry.dst)?;
            }
            return copy_link(&entry.src, &entry.dst);
        }
        let mut input = fs::File::open(&entry.src)?;
        let mut output = fs::OpenOptions::new().write(true).create(true).truncate(true).open(&entry.dst)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            self.check_cancel()?;
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n])?;
            self.progress.file_done += n as u64;
            self.progress.bytes_done += n as u64;
            self.send(false);
        }
        if let Ok(meta) = input.metadata() {
            let mut times = fs::FileTimes::new();
            if let Ok(modified) = meta.modified() {
                times = times.set_modified(modified);
            }
            if self.options.timestamps {
                if let Ok(accessed) = meta.accessed() {
                    times = times.set_accessed(accessed);
                }
            }
            let _ = output.set_times(times);
            if self.options.access != Access::Inherit {
                let _ = fs::set_permissions(&entry.dst, meta.permissions());
            }
        }
        Ok(())
    }

    fn move_all(&mut self, items: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
        let plan = plan(items, false)?;
        self.start_totals(&plan);
        for (src, dst) in items {
            self.check_cancel()?;
            self.progress.from = src.display().to_string();
            self.progress.to = dst.display().to_string();
            let own: Vec<Entry> = plan.iter().filter(|e| e.src.starts_with(src)).map(clone_entry).collect();
            let size: u64 = own.iter().map(|e| e.size).sum();
            let files = own.iter().filter(|e| !e.is_dir).count();
            // Nothing in the way: a rename is all it takes (on one volume).
            if fs::symlink_metadata(dst).is_err() && fs::rename(src, dst).is_ok() {
                self.progress.bytes_done += size;
                self.progress.files_done += files;
                self.send(true);
                continue;
            }
            // Something there already, or another volume: copy it over (asking
            // about what is in the way), then remove the original.
            let copy: Vec<Entry> = own
                .iter()
                .map(|e| Entry { dst: dst.join(e.src.strip_prefix(src).unwrap_or(Path::new(""))), ..clone_entry(e) })
                .collect();
            self.copy_all(&copy)?;
            self.delete_all(&own)?;
        }
        Ok(())
    }

    /// Asks the user and waits for the answer (cancelling answers too).
    fn ask(&mut self, question: Question) -> std::io::Result<Answer> {
        let _ = self.events.send(Event::Ask(question));
        match self.answers.recv() {
            Ok(Answer::Cancel) | Err(_) => Err(std::io::Error::other("Cancelled")),
            Ok(answer) => Ok(answer),
        }
    }

    /// Delete/Wipe of `path` and everything in it, as Far's ShellDelete
    /// goes: a folder with something in it is asked about (until "All"),
    /// a read-only file too; the folder goes after what was in it.
    fn remove(&mut self, path: &Path) -> std::io::Result<()> {
        self.check_cancel()?;
        let meta = fs::symlink_metadata(path)?;
        if meta.is_dir() {
            let has_something = fs::read_dir(path)?.next().is_some();
            if has_something && !self.delete_folders {
                match self.ask(Question::FolderNotEmpty(path.to_path_buf()))? {
                    Answer::All => self.delete_folders = true,
                    Answer::Skip { .. } | Answer::SkipAll => {
                        self.kept = true;
                        return Ok(());
                    }
                    _ => {}
                }
            }
            let mut children: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
            children.sort_by_key(|child| child.file_name());
            for child in children {
                self.remove(&child.path())?;
            }
            self.progress.from = path.display().to_string();
            self.send(false);
            let removed = if self.kind == Kind::Wipe { erase_dir(path) } else { fs::remove_dir(path) };
            return match removed {
                Err(_) if self.kept => Ok(()),
                other => other,
            };
        }
        if is_read_only(&meta) {
            let go = match self.read_only {
                Some(all) => all,
                None => match self.ask(Question::ReadOnly(path.to_path_buf()))? {
                    Answer::All => {
                        self.read_only = Some(true);
                        true
                    }
                    Answer::SkipAll => {
                        self.read_only = Some(false);
                        false
                    }
                    Answer::Skip { .. } => false,
                    _ => true,
                },
            };
            if !go {
                self.kept = true;
                return Ok(());
            }
            let mut permissions = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
        }
        self.progress.from = path.display().to_string();
        self.progress.file_size = meta.len();
        self.progress.file_done = 0;
        self.send(false);
        // Far's confirm_erase_file_with_hardlinks: wiping voids every other name of it
        if self.kind == Kind::Wipe && !meta.file_type().is_symlink() && link_count(path) > 1 {
            let go = match self.hard_links {
                Some(all) => all,
                None => match self.ask(Question::HardLinks(path.to_path_buf()))? {
                    Answer::All => {
                        self.hard_links = Some(true);
                        true
                    }
                    Answer::SkipAll => {
                        self.hard_links = Some(false);
                        false
                    }
                    Answer::Skip { .. } => false,
                    _ => true,
                },
            };
            if !go {
                self.kept = true;
                return Ok(());
            }
        }
        if is_dir_link(&meta) {
            fs::remove_dir(path)?;
        } else if self.kind == Kind::Wipe && !meta.file_type().is_symlink() {
            self.erase_file(path)?;
        } else {
            fs::remove_file(path)?;
        }
        self.progress.files_done += 1;
        self.send(false);
        Ok(())
    }

    /// Far's EraseFile: the data overwritten with zeros (Far's WipeSymbol)
    /// to the end, written through, the file cut to nothing, renamed to a
    /// temporary name and removed.
    fn erase_file(&mut self, path: &Path) -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
            const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
            options.custom_flags(FILE_FLAG_WRITE_THROUGH | FILE_FLAG_SEQUENTIAL_SCAN);
        }
        let mut file = options.open(path)?;
        let size = file.metadata()?.len();
        let zeros = vec![0u8; 65536];
        let mut done = 0u64;
        while done < size {
            self.check_cancel()?;
            let chunk = (size - done).min(zeros.len() as u64) as usize;
            file.write_all(&zeros[..chunk])?;
            done += chunk as u64;
            self.progress.file_done = done;
            self.send(false);
        }
        file.sync_all()?;
        file.set_len(0)?;
        drop(file);
        let temp = temp_name(path);
        fs::rename(path, &temp)?;
        fs::remove_file(&temp)
    }

    /// Removes the files, then the folders, the deepest first; what a copy
    /// skipped stays (and so do the folders holding it).
    fn delete_all(&mut self, plan: &[Entry]) -> std::io::Result<()> {
        for entry in plan.iter().rev() {
            self.check_cancel()?;
            self.progress.from = entry.src.display().to_string();
            if entry.is_dir {
                if let Err(error) = fs::remove_dir(&entry.src) {
                    if self.skipped.is_empty() {
                        return Err(error);
                    }
                }
            } else if !self.skipped.contains(&entry.src) {
                fs::remove_file(&entry.src)?;
                if self.kind == Kind::Delete {
                    self.progress.files_done += 1;
                    self.progress.bytes_done += entry.size;
                }
            }
            self.send(false);
        }
        self.send(true);
        Ok(())
    }
}

/// Far's EraseDirectory: the folder renamed to a temporary name, then removed.
fn erase_dir(path: &Path) -> std::io::Result<()> {
    let temp = temp_name(path);
    fs::rename(path, &temp)?;
    fs::remove_dir(&temp)
}

/// A name nobody has, next to `path` (Far's MakeTemp: `F3T` and hex digits).
fn temp_name(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) ^ std::process::id();
    (0u32..)
        .map(|n| dir.join(format!("F3T{:05X}.tmp", seed.wrapping_add(n) & 0xF_FFFF)))
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .unwrap_or_else(|| dir.join("F3T.tmp"))
}

/// How many names the file has (hard links).
fn link_count(path: &Path) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
        let Ok(file) = fs::File::open(path) else { return 1 };
        // SAFETY: plain data, filled in by the call.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `file` is open for the call.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return 1;
        }
        info.nNumberOfLinks
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(path).map_or(1, |m| m.nlink() as u32)
    }
}

fn is_read_only(meta: &fs::Metadata) -> bool {
    meta.permissions().readonly() && !meta.file_type().is_symlink()
}

/// A link to a folder (a junction or a directory symlink): removed as a folder on Windows.
fn is_dir_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() && meta.file_attributes() & 0x10 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// A symbolic link made again at `dst` with the target `src` has.
fn copy_link(src: &Path, dst: &Path) -> std::io::Result<()> {
    let target = fs::read_link(src)?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, dst)
    }
    #[cfg(windows)]
    {
        let dir = fs::metadata(src).is_ok_and(|m| m.is_dir());
        if dir {
            std::os::windows::fs::symlink_dir(target, dst)
        } else {
            std::os::windows::fs::symlink_file(target, dst)
        }
    }
}

fn plan(items: &[(PathBuf, PathBuf)], follow_links: bool) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for (src, dst) in items {
        walk(src, dst, follow_links, &mut out)?;
    }
    Ok(out)
}

fn clone_entry(entry: &Entry) -> Entry {
    Entry { src: entry.src.clone(), dst: entry.dst.clone(), is_dir: entry.is_dir, is_link: entry.is_link, size: entry.size }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("farsh-ops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a").join("sub")).unwrap();
        fs::write(dir.join("a").join("x.txt"), "hello").unwrap();
        fs::write(dir.join("a").join("sub").join("y.txt"), vec![7u8; 3_000_000]).unwrap();
        dir
    }

    fn wait(mut job: Job) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while job.result.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            job.poll();
        }
        job.result.expect("the job finished")
    }

    fn options(existing: Existing) -> Options {
        Options { existing, ..Options::default() }
    }

    fn one(sources: &[PathBuf], typed: &str, base: &Path) -> Vec<(PathBuf, PathBuf)> {
        targets(sources, typed, base, false).unwrap()
    }

    #[test]
    fn copying_a_folder_to_a_new_name_copies_the_whole_tree() {
        let dir = scratch("copy");
        let pairs = one(&[dir.join("a")], "b", &dir);
        wait(start(Kind::Copy, pairs, Options::default())).unwrap();
        assert_eq!(fs::read_to_string(dir.join("b").join("x.txt")).unwrap(), "hello");
        assert_eq!(fs::metadata(dir.join("b").join("sub").join("y.txt")).unwrap().len(), 3_000_000);
        assert!(dir.join("a").join("x.txt").exists(), "the original stays");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copying_into_an_existing_folder_puts_the_item_inside_it() {
        let dir = scratch("into");
        fs::create_dir(dir.join("dest")).unwrap();
        let pairs = one(&[dir.join("a")], "dest", &dir);
        assert_eq!(pairs[0].1, dir.join("dest").join("a"));
        wait(start(Kind::Copy, pairs, Options::default())).unwrap();
        assert!(dir.join("dest").join("a").join("sub").join("y.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_files_are_skipped_or_overwritten_by_policy() {
        let dir = scratch("policy");
        fs::write(dir.join("keep.txt"), "mine").unwrap();
        let pairs = vec![(dir.join("a").join("x.txt"), dir.join("keep.txt"))];
        wait(start(Kind::Copy, pairs.clone(), options(Existing::Skip))).unwrap();
        assert_eq!(fs::read_to_string(dir.join("keep.txt")).unwrap(), "mine");
        wait(start(Kind::Copy, pairs, options(Existing::Overwrite))).unwrap();
        assert_eq!(fs::read_to_string(dir.join("keep.txt")).unwrap(), "hello");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Runs a copy with Ask over two clashing files, answering each question with `answers`.
    fn ask(name: &str, answers: &[Answer]) -> (PathBuf, usize) {
        let dir = scratch(name);
        fs::create_dir(dir.join("b")).unwrap();
        fs::write(dir.join("b").join("x.txt"), "old").unwrap();
        fs::create_dir(dir.join("b").join("sub")).unwrap();
        fs::write(dir.join("b").join("sub").join("y.txt"), "old").unwrap();
        let pairs = vec![(dir.join("a").join("x.txt"), dir.join("b").join("x.txt")), (dir.join("a").join("sub").join("y.txt"), dir.join("b").join("sub").join("y.txt"))];
        let mut job = start(Kind::Copy, pairs, Options::default());
        let mut asked = 0;
        let deadline = Instant::now() + Duration::from_secs(20);
        while job.result.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            job.poll();
            if job.question.is_some() {
                job.answer(answers[asked.min(answers.len() - 1)]);
                asked += 1;
            }
        }
        (dir, asked)
    }

    #[test]
    fn the_worker_asks_about_each_clash_and_obeys() {
        let (dir, asked) = ask("ask", &[Answer::Overwrite { remember: false }, Answer::Skip { remember: false }]);
        assert_eq!(asked, 2);
        assert_eq!(fs::read_to_string(dir.join("b").join("x.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dir.join("b").join("sub").join("y.txt")).unwrap(), "old");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remembering_an_answer_stops_the_questions() {
        let (dir, asked) = ask("remember", &[Answer::Overwrite { remember: true }]);
        assert_eq!(asked, 1);
        assert_eq!(fs::read_to_string(dir.join("b").join("sub").join("y.txt")).unwrap().len(), 3_000_000);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelling_the_question_cancels_the_job() {
        let dir = scratch("cancelask");
        fs::write(dir.join("keep.txt"), "mine").unwrap();
        let mut job = start(Kind::Copy, vec![(dir.join("a").join("x.txt"), dir.join("keep.txt"))], Options::default());
        let deadline = Instant::now() + Duration::from_secs(20);
        while job.question.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            job.poll();
        }
        job.answer(Answer::Cancel);
        assert_eq!(wait(job), Err("Cancelled".into()));
        assert_eq!(fs::read_to_string(dir.join("keep.txt")).unwrap(), "mine");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moving_takes_the_original_away() {
        let dir = scratch("move");
        let pairs = one(&[dir.join("a")], "moved", &dir);
        wait(start(Kind::Move, pairs, Options::default())).unwrap();
        assert!(!dir.join("a").exists());
        assert_eq!(fs::read_to_string(dir.join("moved").join("x.txt")).unwrap(), "hello");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moving_onto_a_folder_merges_and_keeps_what_was_skipped() {
        let dir = scratch("merge");
        fs::create_dir_all(dir.join("m")).unwrap();
        fs::write(dir.join("m").join("x.txt"), "old").unwrap();
        let pairs = vec![(dir.join("a"), dir.join("m"))];
        wait(start(Kind::Move, pairs, options(Existing::Skip))).unwrap();
        assert_eq!(fs::read_to_string(dir.join("m").join("x.txt")).unwrap(), "old", "skipped, not replaced");
        assert!(dir.join("m").join("sub").join("y.txt").exists(), "the rest moved in");
        assert!(dir.join("a").join("x.txt").exists(), "the skipped original is still there");
        assert!(!dir.join("a").join("sub").exists(), "what moved is gone from the source");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Runs `job`, answering its questions in turn with `answers`; what was asked.
    fn answering(mut job: Job, answers: &[Answer]) -> (Result<(), String>, Vec<String>) {
        let mut asked = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while job.result.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            job.poll();
            if let Some(question) = job.question.clone() {
                asked.push(match question {
                    Question::Conflict(c) => format!("conflict {}", c.dst.display()),
                    Question::FolderNotEmpty(path) => format!("folder {}", path.file_name().unwrap().to_string_lossy()),
                    Question::ReadOnly(path) => format!("read only {}", path.file_name().unwrap().to_string_lossy()),
                    Question::HardLinks(path) => format!("hard links {}", path.file_name().unwrap().to_string_lossy()),
                });
                job.answer(answers.get(asked.len() - 1).copied().unwrap_or(Answer::Cancel));
            }
        }
        (job.result.expect("the job finished"), asked)
    }

    #[test]
    fn deleting_asks_about_each_folder_with_something_in_it_until_all() {
        let dir = scratch("delete");
        let (result, asked) = answering(start(Kind::Delete, vec![(dir.join("a"), dir.join("a"))], Options::default()), &[Answer::Proceed, Answer::Proceed]);
        result.unwrap();
        assert_eq!(asked, ["folder a", "folder sub"]);
        assert!(!dir.join("a").exists());

        let dir = scratch("deleteall");
        let (result, asked) = answering(start(Kind::Delete, vec![(dir.join("a"), dir.join("a"))], Options::default()), &[Answer::All]);
        result.unwrap();
        assert_eq!(asked, ["folder a"], "All: no more questions");
        assert!(!dir.join("a").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_skipped_folder_stays_and_so_does_the_folder_holding_it() {
        let dir = scratch("skipfolder");
        let (result, asked) = answering(start(Kind::Delete, vec![(dir.join("a"), dir.join("a"))], Options::default()), &[Answer::Proceed, Answer::Skip { remember: false }]);
        result.unwrap();
        assert_eq!(asked, ["folder a", "folder sub"]);
        assert!(!dir.join("a").join("x.txt").exists());
        assert!(dir.join("a").join("sub").join("y.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_read_only_file_is_asked_about_and_skip_all_keeps_them() {
        let dir = scratch("readonly");
        for name in ["x.txt", "sub/y.txt"] {
            let path = dir.join("a").join(name);
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions).unwrap();
        }
        let (result, asked) = answering(start(Kind::Delete, vec![(dir.join("a"), dir.join("a"))], Options::default()), &[Answer::All, Answer::SkipAll]);
        result.unwrap();
        assert_eq!(asked, ["folder a", "read only y.txt"], "sub comes first; Skip all asks no more");
        assert!(dir.join("a").join("x.txt").exists() && dir.join("a").join("sub").join("y.txt").exists());
        for name in ["x.txt", "sub/y.txt"] {
            let path = dir.join("a").join(name);
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(&path, permissions).unwrap();
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn wiping_overwrites_the_data_and_leaves_nothing_behind() {
        let dir = scratch("wipe");
        let file = dir.join("a").join("sub").join("y.txt");
        // what the wipe writes is in the file before it goes: watch it through a hard link
        let witness = dir.join("witness");
        fs::hard_link(&file, &witness).unwrap();
        let (result, asked) = answering(start(Kind::Wipe, vec![(dir.join("a"), dir.join("a"))], Options::default()), &[Answer::All, Answer::Proceed]);
        result.unwrap();
        assert_eq!(asked, ["folder a", "hard links y.txt"]);
        assert!(!dir.join("a").exists());
        assert_eq!(fs::metadata(&witness).unwrap().len(), 0, "the data is cut to nothing");
        let left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, [std::ffi::OsString::from("witness")], "no temporary names left");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn progress_counts_files_and_bytes() {
        let dir = scratch("progress");
        let pairs = one(&[dir.join("a")], "b", &dir);
        let mut job = start(Kind::Copy, pairs, Options::default());
        let deadline = Instant::now() + Duration::from_secs(20);
        while job.result.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            job.poll();
        }
        assert_eq!((job.progress.files_done, job.progress.files_total), (2, 2));
        assert_eq!((job.progress.bytes_done, job.progress.bytes_total), (3_000_005, 3_000_005));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_cancelled_copy_stops() {
        let dir = scratch("cancel");
        let pairs = one(&[dir.join("a")], "b", &dir);
        let job = start(Kind::Copy, pairs, Options::default());
        job.cancel();
        let result = wait(job);
        // Either it was stopped, or it was already done; it never hangs.
        assert!(result.is_ok() || result == Err("Cancelled".into()), "{result:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn several_destinations_get_every_source() {
        let dir = scratch("multi");
        fs::create_dir(dir.join("d1")).unwrap();
        fs::create_dir(dir.join("d2")).unwrap();
        let x = dir.join("a").join("x.txt");
        let pairs = targets(&[x.clone()], "d1; d2", &dir, true).unwrap();
        assert_eq!(pairs, vec![(x.clone(), dir.join("d1").join("x.txt")), (x, dir.join("d2").join("x.txt"))]);
        assert!(targets(&[dir.join("a")], "d1;;", &dir, true).is_ok(), "empty names are ignored");
        assert!(targets(&[dir.join("a")], " ; ", &dir, true).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_destinations_are_refused() {
        let dir = scratch("targets");
        let a = dir.join("a");
        assert!(targets(&[a.clone()], "", &dir, false).is_err());
        assert!(targets(&[a.clone()], "a", &dir, false).is_err(), "onto itself");
        assert!(targets(&[a.clone()], "a/sub/inside", &dir, false).is_err(), "into itself");
        assert!(targets(&[a.clone()], "nowhere/b", &dir, false).is_err(), "missing parent");
        assert!(targets(&[a.clone(), dir.join("a").join("x.txt")], "newname", &dir, false).is_err(), "many into a file name");
        fs::remove_dir_all(dir).unwrap();
    }
}
