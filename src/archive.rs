//! Archives: what Far's arclite does with 7z, zip, tar and gzip (the
//! formats farsh knows; arclite has 7z.dll and many more). Creating is
//! here; the dialogs are `crate::arcdialog`.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// A format arclite can create (its c_7z, c_zip and the "Other" list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    SevenZ,
    Zip,
    Gzip,
    Tar,
}

impl Format {
    /// The name 7z.dll gives the format (what arclite lists).
    pub fn name(self) -> &'static str {
        match self {
            Format::SevenZ => "7z",
            Format::Zip => "zip",
            Format::Gzip => "gzip",
            Format::Tar => "tar",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::SevenZ => ".7z",
            Format::Zip => ".zip",
            Format::Gzip => ".gz",
            Format::Tar => ".tar",
        }
    }

    /// gzip holds one file (ArcAPI::is_single_file_format).
    pub fn single_file(self) -> bool {
        self == Format::Gzip
    }
}

/// arclite's c_min_volume_size.
pub const MIN_VOLUME_SIZE: u64 = 64 * 1024;

/// arclite's parse_size_string: digits, spaces, then K, M or G (of 1024).
pub fn parse_size(text: &str) -> Option<u64> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let size: u64 = digits.parse().ok()?;
    let unit = text[digits.len()..].trim_start().chars().next();
    let multiplier = match unit {
        Some('K' | 'k') => 1024,
        Some('M' | 'm') => 1024 * 1024,
        Some('G' | 'g') => 1024 * 1024 * 1024,
        _ => 1,
    };
    Some(size * multiplier)
}

/// What Create archive asked for (arclite's UpdateOptions that farsh does).
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub path: PathBuf,
    pub format: Format,
    /// 7-Zip's level: 0 (store) to 9.
    pub level: u32,
    /// "Copy" for store, else one of `arcdialog::METHODS`.
    pub method: String,
    pub solid: bool,
    pub password: Option<String>,
    pub encrypt_header: bool,
    pub volume_size: Option<u64>,
    /// "Delete files after archiving".
    pub move_files: bool,
    pub ignore_errors: bool,
    /// Adding to an archive: "When destination file exists".
    pub overwrite: Overwrite,
}

/// One thing that goes into the archive: the file on disk and its name inside.
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// `sources` and everything under them, named from their own folder
/// (as arclite names them from the panel's folder).
pub fn entries(sources: &[PathBuf]) -> io::Result<Vec<Entry>> {
    fn walk(path: &Path, name: String, out: &mut Vec<Entry>) -> io::Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if meta.is_dir() {
            out.push(Entry { path: path.into(), name: name.clone(), is_dir: true, size: 0 });
            let mut children: Vec<_> = fs::read_dir(path)?.collect::<Result<_, _>>()?;
            children.sort_by_key(|child| child.file_name());
            for child in children {
                walk(&child.path(), format!("{name}/{}", child.file_name().to_string_lossy()), out)?;
            }
        } else {
            out.push(Entry { path: path.into(), name, is_dir: false, size: meta.len() });
        }
        Ok(())
    }
    let mut out = Vec::new();
    for source in sources {
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        walk(source, name, &mut out)?;
    }
    Ok(out)
}

/// Reports a file starting (`Some`) or bytes read; `Err` stops the work (cancelled).
pub trait Progress {
    fn file(&mut self, entry: &Entry) -> io::Result<()>;
    fn bytes(&mut self, count: u64) -> io::Result<()>;
}

/// Reads a file into the archive, telling `progress` how far it got.
struct Counting<'a, R> {
    inner: R,
    progress: &'a mut dyn Progress,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.progress.bytes(n as u64)?;
        Ok(n)
    }
}

/// Creates the archive `options` describe from `sources`.
pub fn create(options: &Options, sources: &[PathBuf], progress: &mut dyn Progress) -> io::Result<()> {
    let entries = entries(sources)?;
    if options.format.single_file() && (entries.len() != 1 || entries[0].is_dir) {
        return Err(io::Error::other("gzip holds a single file"));
    }
    // volumes: the archive is written whole, then cut (7-Zip's name.7z.001, .002...)
    let target = match options.volume_size {
        Some(_) => options.path.with_extension(""),
        None => options.path.clone(),
    };
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let written = match options.format {
        Format::SevenZ => create_7z(options, &target, &entries, progress),
        Format::Zip => create_zip(options, &target, &entries, progress),
        Format::Tar => create_tar(&target, &entries, progress),
        Format::Gzip => create_gzip(options, &target, &entries[0], progress),
    };
    if let Err(error) = written {
        fs::remove_file(&target).ok();
        return Err(error);
    }
    if let Some(size) = options.volume_size {
        split(&target, size)?;
    }
    if options.move_files {
        for source in sources {
            if fs::symlink_metadata(source)?.is_dir() { fs::remove_dir_all(source)? } else { fs::remove_file(source)? }
        }
    }
    Ok(())
}

/// Cuts `path` into `path.001`, `path.002`... of `size` bytes, then removes it.
fn split(path: &Path, size: u64) -> io::Result<()> {
    let mut input = File::open(path)?;
    let mut buffer = vec![0u8; 1 << 20];
    for number in 1u32.. {
        let mut part = None;
        let mut left = size;
        while left > 0 {
            let n = input.read(&mut buffer[..left.min(1 << 20) as usize])?;
            if n == 0 {
                break;
            }
            let name = PathBuf::from(format!("{}.{number:03}", path.display()));
            let file = match &mut part {
                Some(file) => file,
                None => part.insert(File::create(name)?),
            };
            file.write_all(&buffer[..n])?;
            left -= n as u64;
        }
        if part.is_none() || left > 0 {
            break;
        }
    }
    drop(input);
    fs::remove_file(path)
}

fn create_7z(options: &Options, target: &Path, entries: &[Entry], progress: &mut dyn Progress) -> io::Result<()> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, DeflateOptions, Lzma2Options, LzmaOptions, PpmdOptions};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password};
    let level = options.level;
    let mut methods: Vec<EncoderConfiguration> = Vec::new();
    if let Some(password) = &options.password {
        methods.push(AesEncoderOptions::new(Password::from(password.as_str())).into());
    }
    methods.push(match options.method.as_str() {
        "Copy" => EncoderMethod::COPY.into(),
        "LZMA" => LzmaOptions::from_level(level).into(),
        "LZMA2" => Lzma2Options::from_level(level).into(),
        "PPMD" => PpmdOptions::from_level(level).into(),
        "Deflate" => DeflateOptions::from_level(level).into(),
        other => return Err(io::Error::other(format!("{other} is not a method farsh can write to 7z"))),
    });
    let io_error = |e: sevenz_rust2::Error| io::Error::other(e.to_string());
    let mut writer = ArchiveWriter::create(target).map_err(io_error)?;
    writer.set_content_methods(methods);
    writer.set_encrypt_header(options.encrypt_header);
    let mut files = Vec::new();
    for entry in entries {
        progress.file(entry)?;
        if entry.is_dir {
            writer.push_archive_entry::<File>(ArchiveEntry::from_path(&entry.path, entry.name.clone()), None).map_err(io_error)?;
        } else if options.solid {
            files.push(entry);
        } else {
            let reader = Counting { inner: File::open(&entry.path)?, progress: &mut *progress };
            writer.push_archive_entry(ArchiveEntry::from_path(&entry.path, entry.name.clone()), Some(reader)).map_err(io_error)?;
        }
    }
    if !files.is_empty() {
        // solid: one block; the files are read as it is written
        let archive_entries = files.iter().map(|e| ArchiveEntry::from_path(&e.path, e.name.clone())).collect();
        let readers = files.iter().map(|e| File::open(&e.path).map(sevenz_rust2::SourceReader::new)).collect::<io::Result<Vec<_>>>()?;
        writer.push_archive_entries(archive_entries, readers).map_err(io_error)?;
    }
    writer.finish()?;
    Ok(())
}

fn create_zip(options: &Options, target: &Path, entries: &[Entry], progress: &mut dyn Progress) -> io::Result<()> {
    let file_options = zip_file_options(options)?;
    let mut writer = zip::ZipWriter::new(File::create(target)?);
    for entry in entries {
        progress.file(entry)?;
        if entry.is_dir {
            writer.add_directory(entry.name.as_str(), file_options).map_err(io::Error::other)?;
        } else {
            writer.start_file(entry.name.as_str(), file_options).map_err(io::Error::other)?;
            let mut reader = Counting { inner: File::open(&entry.path)?, progress: &mut *progress };
            io::copy(&mut reader, &mut writer)?;
        }
    }
    writer.finish().map_err(io::Error::other)?;
    Ok(())
}

/// The zip entry options for what Create archive / Add files asked.
fn zip_file_options(options: &Options) -> io::Result<zip::write::FileOptions<'_, '_, ()>> {
    use zip::write::SimpleFileOptions;
    use zip::{AesMode, CompressionMethod};
    let method = match options.method.as_str() {
        "Copy" => CompressionMethod::Stored,
        "Deflate" => CompressionMethod::Deflated,
        "PPMD" => CompressionMethod::Ppmd,
        other => return Err(io::Error::other(format!("{other} is not a method farsh can write to zip"))),
    };
    let mut file_options = SimpleFileOptions::default().compression_method(method).large_file(true);
    if method != CompressionMethod::Stored {
        file_options = file_options.compression_level(Some(i64::from(options.level)));
    }
    if let Some(password) = &options.password {
        file_options = file_options.with_aes_encryption(AesMode::Aes256, password);
    }
    Ok(file_options)
}

fn create_tar(target: &Path, entries: &[Entry], progress: &mut dyn Progress) -> io::Result<()> {
    let mut builder = tar::Builder::new(File::create(target)?);
    builder.follow_symlinks(false);
    for entry in entries {
        progress.file(entry)?;
        if entry.is_dir {
            builder.append_dir(&entry.name, &entry.path)?;
        } else {
            let mut header = tar::Header::new_gnu();
            header.set_metadata(&fs::symlink_metadata(&entry.path)?);
            let reader = Counting { inner: File::open(&entry.path)?, progress: &mut *progress };
            builder.append_data(&mut header, &entry.name, reader)?;
        }
    }
    builder.into_inner()?.flush()
}

fn create_gzip(options: &Options, target: &Path, entry: &Entry, progress: &mut dyn Progress) -> io::Result<()> {
    progress.file(entry)?;
    let mut encoder = flate2::GzBuilder::new()
        .filename(entry.name.as_bytes())
        .write(File::create(target)?, flate2::Compression::new(options.level.clamp(1, 9)));
    let mut reader = Counting { inner: File::open(&entry.path)?, progress };
    io::copy(&mut reader, &mut encoder)?;
    encoder.finish()?.flush()
}

// ---------------------------------------------------------------- reading

/// Something in an archive: its path inside (`/`-separated, no leading
/// slash), what it is, its size and time.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// An archive opened: the formats it is wrapped in (arclite's arc_chain:
/// `gzip→tar` for a .tar.gz) and what is in it, with the folders the
/// paths imply.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub chain: Vec<Format>,
    pub items: Vec<Item>,
}

impl Listing {
    /// arclite's arc_chain.to_string().
    pub fn chain_name(&self) -> String {
        self.chain.iter().map(|f| f.name()).collect::<Vec<_>>().join("\u{2192}")
    }

    /// The items right inside `dir` (`""` is the top).
    pub fn children(&self, dir: &str) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| match item.path.rsplit_once('/') {
                Some((parent, _)) => parent == dir,
                None => dir.is_empty(),
            })
            .collect()
    }
}

/// What the file is, by its first bytes (and a gzip's first decompressed
/// ones, for a tar inside).
pub fn detect(path: &Path) -> Option<Vec<Format>> {
    let mut head = [0u8; 512];
    let mut file = File::open(path).ok()?;
    let n = read_full(&mut file, &mut head).ok()?;
    let head = &head[..n];
    if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
        Some(vec![Format::SevenZ])
    } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        Some(vec![Format::Zip])
    } else if head.starts_with(&[0x1F, 0x8B]) {
        let mut inner = [0u8; 512];
        let mut decoder = flate2::read::GzDecoder::new(File::open(path).ok()?);
        let m = read_full(&mut decoder, &mut inner).unwrap_or(0);
        if is_tar(&inner[..m]) { Some(vec![Format::Gzip, Format::Tar]) } else { Some(vec![Format::Gzip]) }
    } else if is_tar(head) {
        Some(vec![Format::Tar])
    } else {
        None
    }
}

fn is_tar(block: &[u8]) -> bool {
    block.len() >= 262 && &block[257..262] == b"ustar"
}

fn read_full(reader: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        let n = reader.read(&mut buf[done..])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Ok(done)
}

fn to_io(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// A tar's reader: the file, or the gzip around it.
fn tar_reader(path: &Path, chain: &[Format]) -> io::Result<Box<dyn Read>> {
    let file = File::open(path)?;
    Ok(if chain.first() == Some(&Format::Gzip) { Box::new(flate2::read::GzDecoder::new(file)) } else { Box::new(file) })
}

/// The name a gzip keeps for what it holds: its header's, else its own without `.gz`.
fn gzip_inner_name(path: &Path) -> String {
    let header_name = File::open(path).ok().and_then(|file| {
        let decoder = flate2::read::GzDecoder::new(file);
        decoder.header().and_then(|h| h.filename()).map(|n| String::from_utf8_lossy(n).into_owned())
    });
    header_name.unwrap_or_else(|| {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.strip_suffix(".gz").or_else(|| name.strip_suffix(".tgz").map(|_| "")).map(str::to_string).filter(|n| !n.is_empty()).unwrap_or(name)
    })
}

fn system_time_of_unix(seconds: u64) -> Option<std::time::SystemTime> {
    (seconds > 0).then(|| std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
}

/// Lists the archive at `path` (a 7z with an encrypted header needs its `password`).
pub fn list(path: &Path, password: Option<&str>) -> io::Result<Listing> {
    let chain = detect(path).ok_or_else(|| io::Error::other("Not an archive of a format farsh knows (7z, zip, tar, gzip)"))?;
    let mut items: Vec<Item> = Vec::new();
    match chain.last() {
        Some(Format::SevenZ) => {
            let archive = match password {
                Some(p) => sevenz_rust2::Archive::open_with_password(path, &p.into()),
                None => sevenz_rust2::Archive::open(path),
            }
            .map_err(to_io)?;
            for file in &archive.files {
                let modified = file.has_last_modified_date.then(|| std::time::SystemTime::from(file.last_modified_date));
                items.push(Item { path: file.name.replace('\\', "/"), is_dir: file.is_directory, size: file.size, modified });
            }
        }
        Some(Format::Zip) => {
            let mut zip = zip::ZipArchive::new(File::open(path)?).map_err(to_io)?;
            for i in 0..zip.len() {
                let entry = zip.by_index_raw(i).map_err(to_io)?;
                let modified = entry.last_modified().and_then(|t| {
                    let date = chrono::NaiveDate::from_ymd_opt(t.year().into(), t.month().into(), t.day().into())?;
                    let local = date.and_hms_opt(t.hour().into(), t.minute().into(), t.second().into())?;
                    chrono::TimeZone::from_local_datetime(&chrono::Local, &local).earliest().map(Into::into)
                });
                items.push(Item { path: entry.name().map_err(to_io)?.trim_end_matches('/').to_string(), is_dir: entry.is_dir(), size: entry.size(), modified });
            }
        }
        Some(Format::Tar) => {
            let mut archive = tar::Archive::new(tar_reader(path, &chain)?);
            for entry in archive.entries()? {
                let entry = entry?;
                let header = entry.header();
                let name = entry.path()?.to_string_lossy().replace('\\', "/").trim_start_matches("./").trim_end_matches('/').to_string();
                if name.is_empty() {
                    continue;
                }
                let modified = header.mtime().ok().and_then(system_time_of_unix);
                items.push(Item { path: name, is_dir: header.entry_type().is_dir(), size: header.size().unwrap_or(0), modified });
            }
        }
        Some(Format::Gzip) => {
            let meta = fs::metadata(path)?;
            // gzip's ISIZE: the size modulo 4 GiB, in its last 4 bytes
            let mut file = File::open(path)?;
            let mut size = [0u8; 4];
            use std::io::Seek;
            file.seek(io::SeekFrom::End(-4))?;
            file.read_exact(&mut size)?;
            items.push(Item { path: gzip_inner_name(path), is_dir: false, size: u32::from_le_bytes(size).into(), modified: meta.modified().ok() });
        }
        None => {}
    }
    // the folders only the paths name
    let mut implied = Vec::new();
    for item in &items {
        let mut path = item.path.as_str();
        while let Some((parent, _)) = path.rsplit_once('/') {
            if !items.iter().any(|i| i.is_dir && i.path == parent) && !implied.iter().any(|i: &Item| i.path == parent) {
                implied.push(Item { path: parent.to_string(), is_dir: true, size: 0, modified: None });
            }
            path = parent;
        }
    }
    items.extend(implied);
    Ok(Listing { chain, items })
}

/// arclite's "When destination file exists".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overwrite {
    Ask,
    Overwrite,
    Skip,
    /// The new one gets a free name: `name.0`, `name.1`... (auto_rename).
    Rename,
    /// The new one is written at the end of the old.
    Append,
}

/// What a file that is there already asks: extracting (`old` is `None`:
/// the file on the disk) or adding (`old` is the archive's).
pub trait Extracting: Progress {
    fn exists(&mut self, target: &Path, item: &Item, old: Option<&Item>) -> io::Result<Overwrite>;

    /// Whether the last answer was "Remember choice".
    fn remembers(&self) -> bool {
        false
    }
}

/// arclite's auto_rename.
pub fn auto_rename(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    (0u32..).map(|n| PathBuf::from(format!("{}.{n}", path.display()))).find(|p| !p.exists()).unwrap_or_else(|| path.to_path_buf())
}

/// Extracts the items whose paths are in `wanted` (a folder brings what is
/// in it) from the archive into `dest`, named from `base` (the panel's
/// folder inside the archive) down.
pub fn extract(path: &Path, listing: &Listing, wanted: &[String], base: &str, dest: &Path, password: Option<&str>, policy: Overwrite, progress: &mut dyn Extracting) -> io::Result<()> {
    let picked = |name: &str| wanted.iter().any(|w| name == w || name.starts_with(&format!("{w}/")));
    let target_of = |name: &str| -> PathBuf {
        let relative = if base.is_empty() { name } else { name.strip_prefix(base).map(|r| r.trim_start_matches('/')).unwrap_or(name) };
        dest.join(relative.split('/').collect::<PathBuf>())
    };
    fs::create_dir_all(dest)?;
    let mut policy = policy;
    let write = |progress: &mut dyn Extracting, item: &Item, reader: &mut dyn Read, policy: &mut Overwrite| -> io::Result<()> {
        let mut target = target_of(&item.path);
        if item.is_dir {
            return fs::create_dir_all(&target);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut append = false;
        if target.exists() {
            let choice = if *policy == Overwrite::Ask { progress.exists(&target, item, None)? } else { *policy };
            match choice {
                Overwrite::Skip => return io::copy(reader, &mut io::sink()).map(|_| ()),
                Overwrite::Rename => target = auto_rename(&target),
                Overwrite::Append => append = true,
                _ => {}
            }
        }
        progress.file(&Entry { path: target.clone(), name: item.path.clone(), is_dir: false, size: item.size })?;
        let mut out = fs::OpenOptions::new().write(true).create(true).append(append).truncate(!append).open(&target)?;
        let mut counted = Counting { inner: reader, progress: &mut *progress };
        io::copy(&mut counted, &mut out)?;
        if let Some(modified) = item.modified {
            out.set_modified(modified).ok();
        }
        Ok(())
    };
    match listing.chain.last() {
        Some(Format::SevenZ) => {
            let password = password.map(sevenz_rust2::Password::from).unwrap_or_else(sevenz_rust2::Password::empty);
            let mut reader = sevenz_rust2::ArchiveReader::open(path, password).map_err(to_io)?;
            let mut failure = None;
            reader
                .for_each_entries(|entry, data| {
                    let name = entry.name.replace('\\', "/");
                    if !picked(&name) {
                        return Ok(true);
                    }
                    let item = listing.items.iter().find(|i| i.path == name).cloned().unwrap_or(Item { path: name, is_dir: entry.is_directory, size: entry.size, modified: None });
                    match write(progress, &item, data, &mut policy) {
                        Ok(()) => Ok(true),
                        Err(error) => {
                            failure = Some(error);
                            Ok(false)
                        }
                    }
                })
                .map_err(to_io)?;
            if let Some(error) = failure {
                return Err(error);
            }
        }
        Some(Format::Zip) => {
            let mut zip = zip::ZipArchive::new(File::open(path)?).map_err(to_io)?;
            for i in 0..zip.len() {
                let mut entry = match password {
                    Some(p) => zip.by_index_decrypt(i, p.as_bytes()).map_err(to_io)?,
                    None => zip.by_index(i).map_err(to_io)?,
                };
                let name = entry.name().map_err(to_io)?.trim_end_matches('/').to_string();
                if !picked(&name) {
                    continue;
                }
                let item = listing.items.iter().find(|it| it.path == name).cloned().unwrap_or(Item { path: name, is_dir: entry.is_dir(), size: entry.size(), modified: None });
                write(progress, &item, &mut entry, &mut policy)?;
            }
        }
        Some(Format::Tar) => {
            let mut archive = tar::Archive::new(tar_reader(path, &listing.chain)?);
            for entry in archive.entries()? {
                let mut entry = entry?;
                let name = entry.path()?.to_string_lossy().replace('\\', "/").trim_start_matches("./").trim_end_matches('/').to_string();
                if name.is_empty() || !picked(&name) {
                    continue;
                }
                let item = listing.items.iter().find(|i| i.path == name).cloned().unwrap_or(Item { path: name, is_dir: entry.header().entry_type().is_dir(), size: 0, modified: None });
                write(progress, &item, &mut entry, &mut policy)?;
            }
        }
        Some(Format::Gzip) => {
            let item = listing.items[0].clone();
            if picked(&item.path) {
                let mut decoder = flate2::read::GzDecoder::new(File::open(path)?);
                write(progress, &item, &mut decoder, &mut policy)?;
            }
        }
        None => {}
    }
    // folders the archive only implies
    for item in listing.items.iter().filter(|i| i.is_dir && picked(&i.path)) {
        fs::create_dir_all(target_of(&item.path))?;
    }
    Ok(())
}

// ---------------------------------------------------------------- changing a zip

/// Whether farsh can change the archive (arclite's updatable()): a zip.
/// Anything else is "Archive modification not supported".
pub fn updatable(listing: &Listing) -> bool {
    listing.chain == [Format::Zip]
}

/// arclite's message for an archive farsh cannot change.
pub const NOT_UPDATABLE: &str = "Archive modification not supported";

fn picked(wanted: &[String], name: &str) -> bool {
    wanted.iter().any(|w| name == w || name.starts_with(&format!("{w}/")))
}

/// Writes the zip again (as 7-Zip does, into a new file that then takes
/// its place): the entries `keep` says stay are copied as they are, not
/// recompressed (encrypted ones too); `add` writes the new ones.
fn rewrite_zip(path: &Path, mut keep: impl FnMut(&str) -> bool, add: impl FnOnce(&mut zip::ZipWriter<File>) -> io::Result<()>) -> io::Result<()> {
    let temp = auto_rename(&PathBuf::from(format!("{}.farsh-tmp", path.display())));
    let result = (|| {
        let mut reader = zip::ZipArchive::new(File::open(path)?).map_err(to_io)?;
        let mut writer = zip::ZipWriter::new(File::create(&temp)?);
        for i in 0..reader.len() {
            let entry = reader.by_index_raw(i).map_err(to_io)?;
            let name = entry.name().map_err(to_io)?.trim_end_matches('/').to_string();
            if keep(&name) {
                writer.raw_copy_file(entry).map_err(to_io)?;
            }
        }
        add(&mut writer)?;
        writer.finish().map_err(to_io)?.sync_all()
    })();
    match result {
        Ok(()) => fs::rename(&temp, path),
        Err(error) => {
            fs::remove_file(&temp).ok();
            Err(error)
        }
    }
}

/// F8 in a zip: the items (a folder with what is in it) are gone.
pub fn zip_delete(path: &Path, items: &[String]) -> io::Result<()> {
    rewrite_zip(path, |name| !picked(items, name), |_| Ok(()))
}

/// F5/F6 into a zip's panel: `sources` go into the folder `inside` of the
/// archive with the method, level and password `options` say; a file
/// that is there already is asked about (Overwrite or Skip, arclite's
/// odkUpdate) unless `options.overwrite` decided.
pub fn zip_add(path: &Path, listing: &Listing, sources: &[PathBuf], inside: &str, options: &Options, progress: &mut dyn Extracting) -> io::Result<()> {
    let mut new = entries(sources)?;
    for entry in &mut new {
        if !inside.is_empty() {
            entry.name = format!("{inside}/{}", entry.name);
        }
    }
    let mut replaced = Vec::new();
    let mut skipped = Vec::new();
    let mut policy = options.overwrite;
    for entry in new.iter().filter(|e| !e.is_dir) {
        let Some(old) = listing.items.iter().find(|i| i.path == entry.name && !i.is_dir) else { continue };
        let modified = fs::metadata(&entry.path).and_then(|m| m.modified()).ok();
        let item = Item { path: entry.name.clone(), is_dir: false, size: entry.size, modified };
        let choice = if policy == Overwrite::Ask { progress.exists(Path::new(&entry.name), &item, Some(old))? } else { policy };
        if choice == Overwrite::Skip {
            skipped.push(entry.name.clone());
        } else {
            replaced.push(entry.name.clone());
        }
        if policy == Overwrite::Ask && progress.remembers() {
            policy = choice;
        }
    }
    let file_options = zip_file_options(options)?;
    let had_dirs: Vec<String> = listing.items.iter().filter(|i| i.is_dir).map(|i| i.path.clone()).collect();
    rewrite_zip(path, |name| !replaced.iter().any(|r| r == name), |writer| {
        for entry in &new {
            if skipped.contains(&entry.name) {
                continue;
            }
            progress.file(entry)?;
            if entry.is_dir {
                if !had_dirs.contains(&entry.name) {
                    writer.add_directory(entry.name.as_str(), file_options).map_err(to_io)?;
                }
            } else {
                writer.start_file(entry.name.as_str(), file_options).map_err(to_io)?;
                let mut reader = Counting { inner: File::open(&entry.path)?, progress: &mut *progress };
                io::copy(&mut reader, writer)?;
            }
        }
        Ok(())
    })?;
    // Delete files after archiving: what went in (not what was skipped)
    if options.move_files {
        for entry in new.iter().rev() {
            if skipped.iter().any(|s| s == &entry.name || s.starts_with(&format!("{}/", entry.name))) {
                continue;
            }
            if entry.is_dir { fs::remove_dir(&entry.path).ok() } else { fs::remove_file(&entry.path).ok() };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;

    impl Progress for Quiet {
        fn file(&mut self, _: &Entry) -> io::Result<()> {
            Ok(())
        }
        fn bytes(&mut self, _: u64) -> io::Result<()> {
            Ok(())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("farsh-arc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src").join("sub")).unwrap();
        fs::write(dir.join("src").join("a.txt"), "hello hello hello").unwrap();
        fs::write(dir.join("src").join("sub").join("b.bin"), vec![7u8; 200_000]).unwrap();
        dir
    }

    fn options(dir: &Path, format: Format, method: &str) -> Options {
        Options {
            path: dir.join(format!("out{}", format.extension())),
            format,
            level: 5,
            method: method.into(),
            solid: false,
            password: None,
            encrypt_header: false,
            volume_size: None,
            move_files: false,
            ignore_errors: false,
            overwrite: Overwrite::Ask,
        }
    }

    #[test]
    fn sizes_are_read_as_arclite_reads_them() {
        assert_eq!(parse_size("100"), Some(100));
        assert_eq!(parse_size("64 K"), Some(65536));
        assert_eq!(parse_size("2m"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size("1G"), Some(1 << 30));
        assert_eq!(parse_size("M"), None);
    }

    #[test]
    fn every_format_and_method_makes_an_archive_its_own_reader_reads() {
        let dir = scratch("formats");
        let sources = vec![dir.join("src")];
        for (format, method, solid) in [
            (Format::SevenZ, "LZMA", true),
            (Format::SevenZ, "LZMA2", false),
            (Format::SevenZ, "PPMD", true),
            (Format::SevenZ, "Deflate", false),
            (Format::SevenZ, "Copy", false),
            (Format::Zip, "Deflate", false),
            (Format::Zip, "Copy", false),
            (Format::Zip, "PPMD", false),
            (Format::Tar, "", false),
        ] {
            let mut o = options(&dir, format, method);
            o.solid = solid;
            create(&o, &sources, &mut Quiet).unwrap_or_else(|e| panic!("{format:?} {method}: {e}"));
            let names: Vec<String> = match format {
                Format::SevenZ => sevenz_rust2::Archive::open(&o.path).unwrap().files.iter().map(|f| f.name.clone()).collect(),
                Format::Zip => {
                    let mut zip = zip::ZipArchive::new(File::open(&o.path).unwrap()).unwrap();
                    let mut text = String::new();
                    zip.by_name("src/a.txt").unwrap().read_to_string(&mut text).unwrap();
                    assert_eq!(text, "hello hello hello");
                    zip.file_names().map(|name| name.unwrap().into_owned()).collect()
                }
                _ => tar::Archive::new(File::open(&o.path).unwrap()).entries().unwrap().map(|e| e.unwrap().path().unwrap().display().to_string()).collect(),
            };
            assert!(names.iter().any(|n| n.trim_end_matches('/') == "src/sub/b.bin"), "{format:?} {method}: {names:?}");
            fs::remove_file(&o.path).unwrap();
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_method_farsh_cannot_write_is_an_error_not_a_broken_archive() {
        let dir = scratch("cannot");
        for (format, method) in [(Format::Zip, "LZMA"), (Format::Zip, "Deflate64"), (Format::SevenZ, "Deflate64")] {
            let o = options(&dir, format, method);
            let error = create(&o, &[dir.join("src")], &mut Quiet).unwrap_err();
            assert!(error.to_string().contains(method), "{error}");
            assert!(!o.path.exists());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_password_volumes_and_deleting_the_files() {
        let dir = scratch("extras");
        let sources = vec![dir.join("src")];
        let mut o = options(&dir, Format::SevenZ, "LZMA2");
        o.password = Some("secret".into());
        o.encrypt_header = true;
        create(&o, &sources, &mut Quiet).unwrap();
        assert!(sevenz_rust2::Archive::open(&o.path).is_err(), "the header is encrypted");
        assert!(sevenz_rust2::Archive::open_with_password(&o.path, &"secret".into()).is_ok());

        let mut o = options(&dir, Format::Zip, "Copy");
        o.path = dir.join("vol.zip.001");
        o.volume_size = Some(MIN_VOLUME_SIZE);
        o.move_files = true;
        create(&o, &sources, &mut Quiet).unwrap();
        assert!(dir.join("vol.zip.001").exists() && dir.join("vol.zip.004").exists());
        assert!(!dir.join("vol.zip").exists());
        assert!(!dir.join("src").exists(), "deleted after archiving");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gzip_takes_one_file() {
        let dir = scratch("gzip");
        let o = options(&dir, Format::Gzip, "");
        assert!(create(&o, &[dir.join("src")], &mut Quiet).is_err());
        create(&o, &[dir.join("src").join("a.txt")], &mut Quiet).unwrap();
        let mut text = String::new();
        flate2::read::GzDecoder::new(File::open(&o.path).unwrap()).read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello hello hello");
        fs::remove_dir_all(dir).unwrap();
    }

    struct Answering(Overwrite);

    impl Progress for Answering {
        fn file(&mut self, _: &Entry) -> io::Result<()> {
            Ok(())
        }
        fn bytes(&mut self, _: u64) -> io::Result<()> {
            Ok(())
        }
    }

    impl Extracting for Answering {
        fn exists(&mut self, _: &Path, _: &Item, _: Option<&Item>) -> io::Result<Overwrite> {
            Ok(self.0)
        }
    }

    #[test]
    fn every_format_lists_and_extracts_what_went_in() {
        let dir = scratch("roundtrip");
        let sources = vec![dir.join("src")];
        for (format, method) in [(Format::SevenZ, "LZMA2"), (Format::Zip, "Deflate"), (Format::Tar, "")] {
            let o = options(&dir, format, method);
            create(&o, &sources, &mut Quiet).unwrap();
            assert_eq!(detect(&o.path), Some(vec![format]));
            let listing = list(&o.path, None).unwrap();
            let top: Vec<_> = listing.children("").iter().map(|i| i.path.clone()).collect();
            assert_eq!(top, ["src"], "{format:?}");
            let mut inside: Vec<_> = listing.children("src").iter().map(|i| (i.path.clone(), i.is_dir)).collect();
            inside.sort();
            assert_eq!(inside, [("src/a.txt".to_string(), false), ("src/sub".to_string(), true)], "{format:?}");
            // F5 inside "src" of sub: named from there down
            let out = dir.join(format!("out-{}", format.name()));
            extract(&o.path, &listing, &["src/sub".into()], "src", &out, None, Overwrite::Ask, &mut Answering(Overwrite::Overwrite)).unwrap();
            assert_eq!(fs::read(out.join("sub").join("b.bin")).unwrap().len(), 200_000, "{format:?}");
            fs::remove_file(&o.path).unwrap();
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_tar_gz_is_gzip_then_tar_and_a_lone_gzip_holds_its_file() {
        let dir = scratch("targz");
        let tar_path = dir.join("x.tar");
        create(&options(&dir, Format::Tar, ""), &[dir.join("src")], &mut Quiet).unwrap();
        fs::rename(dir.join("out.tar"), &tar_path).unwrap();
        let mut gz = flate2::write::GzEncoder::new(File::create(dir.join("x.tar.gz")).unwrap(), flate2::Compression::default());
        io::copy(&mut File::open(&tar_path).unwrap(), &mut gz).unwrap();
        gz.finish().unwrap();
        let listing = list(&dir.join("x.tar.gz"), None).unwrap();
        assert_eq!(listing.chain_name(), "gzip\u{2192}tar");
        assert!(listing.items.iter().any(|i| i.path == "src/sub/b.bin"));

        create(&options(&dir, Format::Gzip, ""), &[dir.join("src").join("a.txt")], &mut Quiet).unwrap();
        let listing = list(&dir.join("out.gz"), None).unwrap();
        assert_eq!(listing.chain_name(), "gzip");
        assert_eq!(listing.items, vec![Item { path: "a.txt".into(), is_dir: false, size: 17, modified: listing.items[0].modified }]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_is_there_is_skipped_renamed_or_appended_to() {
        let dir = scratch("exists");
        let o = options(&dir, Format::Zip, "Deflate");
        create(&o, &[dir.join("src").join("a.txt")], &mut Quiet).unwrap();
        let listing = list(&o.path, None).unwrap();
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("a.txt"), "old").unwrap();
        let run = |answer: Overwrite| extract(&o.path, &listing, &["a.txt".into()], "", &out, None, Overwrite::Ask, &mut Answering(answer)).unwrap();
        run(Overwrite::Skip);
        assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "old");
        run(Overwrite::Rename);
        assert_eq!(fs::read_to_string(out.join("a.txt.0")).unwrap(), "hello hello hello");
        run(Overwrite::Append);
        assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "oldhello hello hello");
        run(Overwrite::Overwrite);
        assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "hello hello hello");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn encrypted_archives_open_and_extract_with_the_password() {
        let dir = scratch("secret");
        for format in [Format::SevenZ, Format::Zip] {
            let mut o = options(&dir, format, if format == Format::Zip { "Deflate" } else { "LZMA2" });
            o.password = Some("pw".into());
            create(&o, &[dir.join("src").join("a.txt")], &mut Quiet).unwrap();
            let listing = list(&o.path, Some("pw")).unwrap();
            let out = dir.join(format!("plain-{}", format.name()));
            extract(&o.path, &listing, &["a.txt".into()], "", &out, Some("pw"), Overwrite::Overwrite, &mut Answering(Overwrite::Overwrite)).unwrap();
            assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "hello hello hello", "{format:?}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    fn zip_names(path: &Path) -> Vec<String> {
        let zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().trim_end_matches('/').to_string()).collect();
        names.sort();
        names
    }

    fn zip_bytes(path: &Path, name: &str) -> Vec<u8> {
        let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        let mut entry = zip.by_name(name).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn deleting_from_a_zip_keeps_the_rest_as_it_was_and_takes_a_folder_whole() {
        let dir = scratch("zipdel");
        let o = options(&dir, Format::Zip, "Deflate");
        create(&o, &[dir.join("src")], &mut Quiet).unwrap();
        let raw = |path: &Path| {
            let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
            let entry = zip.by_name("src/a.txt").unwrap();
            (entry.compressed_size(), entry.crc32())
        };
        let before = raw(&o.path);
        zip_delete(&o.path, &["src/sub".into()]).unwrap();
        assert_eq!(zip_names(&o.path), ["src", "src/a.txt"]);
        assert_eq!(raw(&o.path), before, "copied, not recompressed");
        assert_eq!(zip_bytes(&o.path, "src/a.txt"), b"hello hello hello");
        let left: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!left.iter().any(|n| n.contains("farsh-tmp")), "{left:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn adding_to_a_zip_goes_into_the_panel_folder_and_asks_about_what_is_there() {
        let dir = scratch("zipadd");
        let o = options(&dir, Format::Zip, "Deflate");
        create(&o, &[dir.join("src")], &mut Quiet).unwrap();
        fs::create_dir_all(dir.join("new")).unwrap();
        fs::write(dir.join("new").join("a.txt"), "replaced").unwrap();
        fs::write(dir.join("new").join("c.txt"), "added").unwrap();
        let listing = list(&o.path, None).unwrap();
        let sources = vec![dir.join("new").join("a.txt"), dir.join("new").join("c.txt")];
        // Skip: the old a.txt stays
        zip_add(&o.path, &listing, &sources, "src", &o, &mut Answering(Overwrite::Skip)).unwrap();
        assert_eq!(zip_bytes(&o.path, "src/a.txt"), b"hello hello hello");
        assert_eq!(zip_bytes(&o.path, "src/c.txt"), b"added");
        // Overwrite, and the files go after archiving
        let listing = list(&o.path, None).unwrap();
        let mut moving = o.clone();
        moving.move_files = true;
        zip_add(&o.path, &listing, &sources, "src", &moving, &mut Answering(Overwrite::Overwrite)).unwrap();
        assert_eq!(zip_bytes(&o.path, "src/a.txt"), b"replaced");
        assert_eq!(zip_names(&o.path).iter().filter(|n| *n == "src/a.txt").count(), 1, "replaced, not twice");
        assert!(!dir.join("new").join("a.txt").exists() && !dir.join("new").join("c.txt").exists());
        assert_eq!(zip_bytes(&o.path, "src/sub/b.bin").len(), 200_000, "the rest is there");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_a_zip_can_be_changed() {
        let zip = Listing { chain: vec![Format::Zip], items: vec![] };
        assert!(updatable(&zip));
        for chain in [vec![Format::SevenZ], vec![Format::Tar], vec![Format::Gzip], vec![Format::Gzip, Format::Tar]] {
            assert!(!updatable(&Listing { chain, items: vec![] }));
        }
    }
}
