//! The audio and subtitle streams of a video, read with FFmpeg, so that the
//! viewer's `a` and `s` go through the streams the file has (the user's call:
//! with FFmpeg, as `somsrp` probes video on Windows).
//!
//! Windows only, as in `somsrp`: FFmpeg's avformat, avcodec and avutil are
//! linked delay-loaded and carried inside farsh.exe (zstd, `build.rs` takes
//! them from the vcpkg tree farsh was linked against); they are written to
//! `%TEMP%\farsh-ffmpeg-<hash>` the first time a video's streams are asked
//! for. Elsewhere there is no list, and a stream is chosen by its number.

/// The streams of one kind, in the order FFmpeg enumerates them (the index
/// Som takes, as `somsrp -a <n> -s <n>`), each as a short label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamList {
    pub audio: Vec<String>,
    pub subtitle: Vec<String>,
}

/// A stream's label: its language, its title and its codec, whichever it has.
#[cfg_attr(not(windows), allow(dead_code))]
fn label(language: Option<&str>, title: Option<&str>, codec: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(language) = language.filter(|language| !language.is_empty() && *language != "und") {
        parts.push(language);
    }
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        parts.push(title);
    }
    if !codec.is_empty() && codec != "none" {
        parts.push(codec);
    }
    parts.join(", ")
}

#[cfg(windows)]
pub fn list(path: &std::path::Path) -> Option<StreamList> {
    if !ffmpeg::ready() {
        return None;
    }
    // Som's FFmpeg has no `file` protocol (it never opens a path): the file is
    // read through a stream of ours, as somsrp does
    let file = std::fs::File::open(path).ok()?;
    let io = ffmpeg_next::format::context::StreamIo::from_read_seek(file).ok()?;
    let name = path.file_name().and_then(|name| name.to_str());
    let input = ffmpeg_next::format::input_from_stream(io, name, None)
        .inspect_err(|error| eprintln!("farsh: {}: {error}", path.display()))
        .ok()?;
    let mut list = StreamList::default();
    for stream in input.streams() {
        let parameters = stream.parameters();
        let codec = parameters.id().name().to_uppercase();
        let metadata = stream.metadata();
        let text = label(metadata.get("language"), metadata.get("title"), &codec);
        match parameters.medium() {
            ffmpeg_next::media::Type::Audio => list.audio.push(text),
            ffmpeg_next::media::Type::Subtitle => list.subtitle.push(text),
            _ => {}
        }
    }
    Some(list)
}

#[cfg(not(windows))]
pub fn list(_path: &std::path::Path) -> Option<StreamList> {
    None
}

#[cfg(windows)]
mod ffmpeg {
    use std::hash::{Hash as _, Hasher as _};
    use std::path::{Path, PathBuf};
    use std::sync::LazyLock;

    /// The DLLs, zstd-compressed by `build.rs`, in the order they are loaded.
    const DLLS: [(&str, &[u8]); 4] = [
        ("avutil-61.dll", include_bytes!(concat!(env!("OUT_DIR"), "/avutil-61.dll.zst"))),
        // avcodec's own import (not delay-loaded); found nowhere else once `main` has
        // limited the search to the system's folders
        ("swresample-7.dll", include_bytes!(concat!(env!("OUT_DIR"), "/swresample-7.dll.zst"))),
        ("avcodec-63.dll", include_bytes!(concat!(env!("OUT_DIR"), "/avcodec-63.dll.zst"))),
        ("avformat-63.dll", include_bytes!(concat!(env!("OUT_DIR"), "/avformat-63.dll.zst"))),
    ];

    static READY: LazyLock<bool> = LazyLock::new(|| match extract() {
        Ok(()) => ffmpeg_next::init().is_ok(),
        Err(error) => {
            eprintln!("farsh: FFmpeg is not available: {error}");
            false
        }
    });

    /// Whether FFmpeg can be called; the first call writes the DLLs out.
    pub fn ready() -> bool {
        *READY
    }

    /// The folder for these DLLs: named by their contents, so another build
    /// of farsh never takes DLLs that are not its own.
    fn folder() -> PathBuf {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for (name, bytes) in DLLS {
            name.hash(&mut hasher);
            bytes.hash(&mut hasher);
        }
        std::env::temp_dir().join(format!("farsh-ffmpeg-{:016x}", hasher.finish()))
    }

    fn extract() -> Result<(), String> {
        let dir = folder();
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for (name, compressed) in DLLS {
            let bytes = zstd::decode_all(compressed).map_err(|error| format!("{name}: {error}"))?;
            write_unless_same(&dir.join(name), &bytes)?;
        }
        add_dll_directory(&dir)
    }

    /// A DLL that another farsh holds loaded cannot be written over: one that
    /// is already there byte for byte is left alone.
    fn write_unless_same(path: &Path, bytes: &[u8]) -> Result<(), String> {
        if std::fs::read(path).is_ok_and(|current| current == bytes) {
            return Ok(());
        }
        std::fs::write(path, bytes).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// `main` has limited the DLL search to the system's folders and farsh's
    /// own (`SetDefaultDllDirectories`), so the folder is added to it: the
    /// delay-loaded imports are found there on the first FFmpeg call.
    fn add_dll_directory(dir: &Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt as _;
        use windows_sys::Win32::System::LibraryLoader::AddDllDirectory;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        if unsafe { AddDllDirectory(wide.as_ptr()) }.is_null() {
            return Err(format!("AddDllDirectory({}) failed", dir.display()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_has_what_the_stream_says_about_itself() {
        assert_eq!(label(Some("rus"), Some("Dub"), "AC3"), "rus, Dub, AC3");
        assert_eq!(label(Some("und"), None, "AAC"), "AAC");
        assert_eq!(label(None, Some(""), "SUBRIP"), "SUBRIP");
    }

    /// The fixture of Som's own tests, when this machine has it. The DLL search
    /// is limited first, as `main` does: `cargo test` puts the vcpkg tree on
    /// PATH, where a DLL farsh does not carry would be found (swresample was,
    /// and farsh itself ended on the first `a`; 2026-11-06).
    #[cfg(windows)]
    #[test]
    fn the_streams_of_a_real_video_are_listed() {
        let clip = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../som/test_fixtures/real_fixture_clip.mkv");
        if !clip.is_file() {
            eprintln!("skipping: {} not present", clip.display());
            return;
        }
        crate::system_dlls_only();
        let list = list(&clip).expect("FFmpeg reads the clip");
        assert!(!list.audio.is_empty(), "{list:?}");
    }
}
