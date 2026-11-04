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
    use zip::write::SimpleFileOptions;
    use zip::{AesMode, CompressionMethod, ZipWriter};
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
    let mut writer = ZipWriter::new(File::create(target)?);
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
}
