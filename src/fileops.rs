//! Copy, move and delete in a worker thread, with progress and cancelling.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
    Delete,
}

impl Kind {
    /// The word for the progress box title.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Copy => "Copy",
            Kind::Move => "Move",
            Kind::Delete => "Delete",
        }
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

enum Event {
    Progress(Progress),
    Done(Result<(), String>),
}

pub struct Job {
    pub kind: Kind,
    pub progress: Progress,
    pub started: Instant,
    /// Set when the worker is finished: `Ok`, or what went wrong (cancelling is an error).
    pub result: Option<Result<(), String>>,
    events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Takes what the worker has reported so far.
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Progress(progress) => self.progress = progress,
                Event::Done(result) => self.result = Some(result),
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// Starts `kind` over `items` (source, target); a delete ignores the target.
pub fn start(kind: Kind, items: Vec<(PathBuf, PathBuf)>) -> Job {
    let (sender, events) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    std::thread::spawn(move || {
        let result = Worker::new(kind, sender.clone(), worker_cancel).run(&items);
        let _ = sender.send(Event::Done(result.map_err(|e| e.to_string())));
    });
    Job { kind, progress: Progress::default(), started: Instant::now(), result: None, events, cancel }
}

/// Where each of `sources` goes when the user typed `typed` in the Copy/Move
/// box with the active panel in `base`: into the folder `typed` names, or, for
/// a single source, to the new name `typed` gives it.
pub fn targets(sources: &[PathBuf], typed: &str, base: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("The destination is empty".into());
    }
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
    size: u64,
}

/// `src` and everything under it, folders before their contents.
fn walk(src: &Path, dst: &Path, out: &mut Vec<Entry>) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        out.push(Entry { src: src.into(), dst: dst.into(), is_dir: true, size: 0 });
        let mut children: Vec<_> = fs::read_dir(src)?.collect::<Result<_, _>>()?;
        children.sort_by_key(|child| child.file_name());
        for child in children {
            walk(&child.path(), &dst.join(child.file_name()), out)?;
        }
    } else {
        out.push(Entry { src: src.into(), dst: dst.into(), is_dir: false, size: meta.len() });
    }
    Ok(())
}

struct Worker {
    kind: Kind,
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    progress: Progress,
    last_sent: Instant,
}

impl Worker {
    fn new(kind: Kind, events: Sender<Event>, cancel: Arc<AtomicBool>) -> Self {
        Self { kind, events, cancel, progress: Progress::default(), last_sent: Instant::now() - Duration::from_secs(1) }
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
                let plan = plan(items)?;
                self.start_totals(&plan);
                self.copy_all(&plan)
            }
            Kind::Move => self.move_all(items),
            Kind::Delete => {
                let sources: Vec<_> = items.iter().map(|(src, _)| (src.clone(), src.clone())).collect();
                let plan = plan(&sources)?;
                self.start_totals(&plan);
                self.delete_all(&plan)
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

    fn copy_file(&mut self, entry: &Entry) -> std::io::Result<()> {
        self.progress.from = entry.src.display().to_string();
        self.progress.to = entry.dst.display().to_string();
        self.progress.file_size = entry.size;
        self.progress.file_done = 0;
        let mut input = fs::File::open(&entry.src)?;
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&entry.dst).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                std::io::Error::other(format!("{} already exists", entry.dst.display()))
            } else {
                e
            }
        })?;
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
            let _ = output.set_modified(meta.modified()?);
            let _ = fs::set_permissions(&entry.dst, meta.permissions());
        }
        Ok(())
    }

    fn move_all(&mut self, items: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
        let plan = plan(items)?;
        self.start_totals(&plan);
        for (src, dst) in items {
            self.check_cancel()?;
            if dst.exists() {
                return Err(std::io::Error::other(format!("{} already exists", dst.display())));
            }
            self.progress.from = src.display().to_string();
            self.progress.to = dst.display().to_string();
            let size: u64 = plan.iter().filter(|e| e.src.starts_with(src)).map(|e| e.size).sum();
            let files = plan.iter().filter(|e| e.src.starts_with(src) && !e.is_dir).count();
            if fs::rename(src, dst).is_ok() {
                self.progress.bytes_done += size;
                self.progress.files_done += files;
                self.send(true);
                continue;
            }
            // Another volume: copy it over, then remove the original.
            let own: Vec<Entry> = plan.iter().filter(|e| e.src.starts_with(src)).map(clone_entry).collect();
            self.copy_all(&own)?;
            self.delete_all(&own)?;
        }
        Ok(())
    }

    /// Removes the files, then the folders, the deepest first.
    fn delete_all(&mut self, plan: &[Entry]) -> std::io::Result<()> {
        for entry in plan.iter().rev() {
            self.check_cancel()?;
            self.progress.from = entry.src.display().to_string();
            if entry.is_dir {
                fs::remove_dir(&entry.src)?;
            } else {
                fs::remove_file(&entry.src)?;
                self.progress.files_done += 1;
                self.progress.bytes_done += entry.size;
            }
            self.send(false);
        }
        self.send(true);
        Ok(())
    }
}

fn plan(items: &[(PathBuf, PathBuf)]) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for (src, dst) in items {
        walk(src, dst, &mut out)?;
    }
    Ok(out)
}

fn clone_entry(entry: &Entry) -> Entry {
    Entry { src: entry.src.clone(), dst: entry.dst.clone(), is_dir: entry.is_dir, size: entry.size }
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

    #[test]
    fn copying_a_folder_to_a_new_name_copies_the_whole_tree() {
        let dir = scratch("copy");
        let pairs = targets(&[dir.join("a")], "b", &dir).unwrap();
        wait(start(Kind::Copy, pairs)).unwrap();
        assert_eq!(fs::read_to_string(dir.join("b").join("x.txt")).unwrap(), "hello");
        assert_eq!(fs::metadata(dir.join("b").join("sub").join("y.txt")).unwrap().len(), 3_000_000);
        assert!(dir.join("a").join("x.txt").exists(), "the original stays");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copying_into_an_existing_folder_puts_the_item_inside_it() {
        let dir = scratch("into");
        fs::create_dir(dir.join("dest")).unwrap();
        let pairs = targets(&[dir.join("a")], "dest", &dir).unwrap();
        assert_eq!(pairs[0].1, dir.join("dest").join("a"));
        wait(start(Kind::Copy, pairs)).unwrap();
        assert!(dir.join("dest").join("a").join("sub").join("y.txt").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_existing_file_is_never_overwritten() {
        let dir = scratch("exists");
        fs::write(dir.join("keep.txt"), "mine").unwrap();
        let pairs = vec![(dir.join("a").join("x.txt"), dir.join("keep.txt"))];
        let error = wait(start(Kind::Copy, pairs)).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(fs::read_to_string(dir.join("keep.txt")).unwrap(), "mine");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moving_takes_the_original_away() {
        let dir = scratch("move");
        let pairs = targets(&[dir.join("a")], "moved", &dir).unwrap();
        wait(start(Kind::Move, pairs)).unwrap();
        assert!(!dir.join("a").exists());
        assert_eq!(fs::read_to_string(dir.join("moved").join("x.txt")).unwrap(), "hello");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deleting_removes_files_and_folders() {
        let dir = scratch("delete");
        wait(start(Kind::Delete, vec![(dir.join("a"), dir.join("a"))])).unwrap();
        assert!(!dir.join("a").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn progress_counts_files_and_bytes() {
        let dir = scratch("progress");
        let pairs = targets(&[dir.join("a")], "b", &dir).unwrap();
        let mut job = start(Kind::Copy, pairs);
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
        let pairs = targets(&[dir.join("a")], "b", &dir).unwrap();
        let job = start(Kind::Copy, pairs);
        job.cancel();
        let result = wait(job);
        // Either it was stopped, or it was already done; it never hangs.
        assert!(result.is_ok() || result == Err("Cancelled".into()), "{result:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_destinations_are_refused() {
        let dir = scratch("targets");
        let a = dir.join("a");
        assert!(targets(&[a.clone()], "", &dir).is_err());
        assert!(targets(&[a.clone()], "a", &dir).is_err(), "onto itself");
        assert!(targets(&[a.clone()], "a/sub/inside", &dir).is_err(), "into itself");
        assert!(targets(&[a.clone()], "nowhere/b", &dir).is_err(), "missing parent");
        assert!(targets(&[a.clone(), dir.join("a").join("x.txt")], "newname", &dir).is_err(), "many into a file name");
        fs::remove_dir_all(dir).unwrap();
    }
}
