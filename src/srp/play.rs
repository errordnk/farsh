//! What plays in Som: one audio or video file, an `.m3u` list or a `.cue` sheet,
//! as `somsrp` does it. Each track is a placement of its own (new ids, drawn by
//! the viewer when it sees the generation change); a list moves on when Som says
//! the track ended, or when the widget's previous/next (or `n`/`p`) is used.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use srp_client::protocol::{ContentMetadata, ContentType, CueSheet, NextTrack, SrvRequest, SrvResponse, TrackTags};
use super::{ANNOUNCE_LEN, Channel, Forwarded as _, Kind, Shared, TAIL_LEN, audio, cue, kind_of, new_ids, send_range, video_metadata};

/// One thing in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// A path, or an `http(s)` URL that the daemon fetches itself.
    pub target: String,
    pub title: Option<String>,
    /// Set for a file played as the tracks of a CUE sheet.
    pub cue: Option<CueSheet>,
}

/// Where a track sits in its list.
#[derive(Debug, Clone)]
pub(super) struct Position {
    pub(super) index: usize,
    /// `0` = not in a list (no previous/next buttons).
    pub(super) len: usize,
    pub(super) title: Option<String>,
    pub(super) cue: Option<CueSheet>,
    pub(super) looping: bool,
    /// The track belongs to a radio: its widget shows the radio's icons.
    pub(super) radio: bool,
    /// What a radio's batch already says of this track and of the next one.
    pub(super) current: Option<NextTrack>,
    pub(super) next: Option<NextTrack>,
}

impl Position {
    fn plain(index: usize, len: usize, title: Option<String>, cue: Option<CueSheet>, looping: bool) -> Self {
        Self { index, len, title, cue, looping, radio: false, current: None, next: None }
    }
}

fn is_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// The entries of an M3U/M3U8 text, in order: blank lines and `#` lines are
/// skipped (`#EXTINF:<secs>,<title>` titles the entry after it), URLs are kept
/// as they are, anything else is a path relative to `base_dir` unless absolute.
pub fn parse_m3u(text: &str, base_dir: Option<&Path>) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut pending_title: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            pending_title = info.split_once(',').map(|(_, title)| title.trim().to_string()).filter(|title| !title.is_empty());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let path = Path::new(line);
        // a line naming the radio service is no file name either
        let named_service = line.to_ascii_lowercase().contains("yandex");
        let target = match (is_url(line) || named_service, path.is_absolute(), base_dir) {
            (false, false, Some(base_dir)) => base_dir.join(path).to_string_lossy().into_owned(),
            _ => line.to_string(),
        };
        entries.push(Entry { target, title: pending_title.take(), cue: None });
    }
    entries
}

/// The entries a `.cue` sheet makes: each `FILE` with tracks is one entry, with
/// its track list when it has more than one track.
pub fn cue_entries(sheet_path: &Path, text: &str) -> Result<Vec<Entry>, String> {
    let parsed = cue::parse(text);
    let cover = cue::find_cover(sheet_path).map(|cover| cover.to_string_lossy().into_owned());
    let mut entries = Vec::new();
    for file in &parsed.files {
        let sheet = parsed.sheet_for(file, cover.clone());
        if sheet.tracks.is_empty() {
            continue;
        }
        let audio = cue::find_audio_file(sheet_path, &file.name)
            .ok_or_else(|| format!("the audio file {:?} was not found next to the sheet", file.name))?;
        let target = audio.to_string_lossy().into_owned();
        if sheet.tracks.len() == 1 {
            entries.push(Entry { target, title: Some(sheet.tracks[0].title.clone()), cue: None });
        } else {
            entries.push(Entry { target, title: None, cue: Some(sheet) });
        }
    }
    if entries.is_empty() {
        return Err("the cue sheet lists no tracks".to_string());
    }
    Ok(entries)
}

/// A lone audio or video file.
pub fn run_single(path: &Path, kind: Kind, shared: &Arc<Shared>) -> Result<(), String> {
    // a lone audio file is a track of no list, titled by its name; a video has no title to announce
    let position = (kind == Kind::Audio)
        .then(|| Position::plain(0, 0, path.file_stem().map(|stem| stem.to_string_lossy().into_owned()), None, true));
    play_file(path, kind, position.as_ref(), shared.ids(), shared)
}

/// A document is read whole and sent; Som renders it into the cells of the
/// placement and asks for more rows as it lays the text out (`GrowMarkdownRows`,
/// heard on a connection registered as the document's grower), which the viewer
/// shows by scrolling through the rows.
pub fn run_markdown(path: &Path, shared: &Arc<Shared>) -> Result<(), String> {
    const LIMIT: u64 = 32 << 20;
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > LIMIT {
        return Err(format!("{} MB is too much for a document", size >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // absolute: the daemon resolves the document's relative links against it
    let base_dir = path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_string_lossy().into_owned()))
        .map(|dir| dir.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(dir))
        .unwrap_or_default();
    let metadata = ContentMetadata::Markdown { base_dir };
    let ids = shared.ids();
    {
        // closed after the last chunk: the daemon takes it for the document's sender, and a
        // connection that nobody reads would swallow Som's requests for the bytes it missed
        let channel = Channel::connect()?;
        for request in super::image_requests(ids, &bytes, ContentType::Markdown, &metadata) {
            if shared.cancelled() {
                return Ok(());
            }
            channel.send(&request)?;
        }
    }

    let grower = Arc::new(Channel::connect()?);
    grower.send(&SrvRequest::RegisterMarkdownGrower { session_id: ids.0, file_id: ids.1 })?;
    shared.set_status(super::Status::Sent);
    let ended = Arc::new(AtomicBool::new(false));
    let thread = {
        let (grower, ended, shared) = (grower.clone(), ended.clone(), shared.clone());
        std::thread::spawn(move || {
            while let Ok(incoming) = grower.next_forwarded() {
                match incoming {
                    SrvRequest::GrowMarkdownRows { session_id, file_id, additional_rows } if (session_id, file_id) == ids => {
                        shared.grow_rows(additional_rows)
                    }
                    SrvRequest::EndPlayback { session_id, file_id } if (session_id, file_id) == ids => break,
                    _ => {}
                }
            }
            ended.store(true, Ordering::Release);
        })
    };
    while !shared.cancelled() && !ended.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    grower.shutdown();
    let _ = thread.join();
    Ok(())
}

pub fn run_playlist_file(path: &Path, shared: &Arc<Shared>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading the playlist: {e}"))?;
    if text.contains("#EXT-X-") {
        return Err("HLS (.m3u8 segment) playlists are not supported - only plain track lists".to_string());
    }
    let base_dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let entries = parse_m3u(&text, base_dir);
    if entries.is_empty() {
        return Err("the playlist has no entries".to_string());
    }
    if let Some((service, station)) = super::radio::radio_in_playlist(&entries)? {
        return super::radio::run(service, &station, shared);
    }
    run_list(&entries, true, shared)
}

pub fn run_cue_file(path: &Path, shared: &Arc<Shared>) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading the cue sheet: {e}"))?;
    let entries = cue_entries(path, &cue::decode_text(&bytes))?;
    // one file cut into tracks plays through once: it is not a circle
    if entries.len() == 1 && entries[0].cue.is_some() {
        let entry = &entries[0];
        let position = Position::plain(0, 1, entry.title.clone(), entry.cue.clone(), true);
        return play_target(entry, &position, shared.ids(), shared);
    }
    run_list(&entries, true, shared)
}

/// Plays `entries` one after the other; a circle when `looping`. An entry that
/// cannot start is skipped; when all of them fail in a row the last error is returned.
fn run_list(entries: &[Entry], looping: bool, shared: &Arc<Shared>) -> Result<(), String> {
    let len = entries.len();
    let mut index = 0usize;
    let mut failures = 0usize;
    let mut first = true;
    loop {
        if shared.cancelled() {
            return Ok(());
        }
        shared.step.store(0, Ordering::Release);
        let entry = &entries[index];
        let ids = if first {
            shared.ids()
        } else {
            let ids = new_ids();
            shared.set_ids(ids);
            ids
        };
        first = false;
        shared.set_position(Some((index, len)));
        let position = Position::plain(index, len, entry.title.clone(), entry.cue.clone(), looping);
        let result = play_target(entry, &position, ids, shared);
        let step = shared.step.swap(0, Ordering::AcqRel);
        match result {
            Ok(()) => failures = 0,
            Err(error) => {
                failures += 1;
                if failures >= len {
                    return Err(format!("{}: {error}", entry.target));
                }
            }
        }
        if shared.cancelled() {
            return Ok(());
        }
        index = match (looping, step < 0) {
            (true, true) => (index + len - 1) % len,
            (true, false) => (index + 1) % len,
            (false, true) => index.saturating_sub(1),
            (false, false) if index + 1 < len => index + 1,
            (false, false) => return Ok(()),
        };
    }
}

fn play_target(entry: &Entry, position: &Position, ids: (u32, u32), shared: &Arc<Shared>) -> Result<(), String> {
    if is_url(&entry.target) {
        return play_url(&entry.target, position, ids, shared, None);
    }
    let path = PathBuf::from(&entry.target);
    match kind_of(&path) {
        Some(kind @ (Kind::Audio | Kind::Video)) => play_file(&path, kind, Some(position), ids, shared),
        _ => Err("not something Som plays".to_string()),
    }
}

/// Tells Som which track the placement is, and its CUE track list.
pub(super) fn announce(ids: (u32, u32), path: Option<&Path>, target: &str, position: &Position) {
    let tags = match (&position.cue, path) {
        // the tags of a CUE sheet's file describe the whole disc, not a track: the sheet is announced instead
        (Some(sheet), _) if sheet.cover.is_none() => Some(TrackTags {
            title: sheet.album.clone(),
            artist: sheet.artist.clone(),
            album: sheet.album.clone(),
            year: sheet.year,
        }),
        (Some(_), _) => None,
        (None, Some(path)) => audio::tags(path),
        (None, None) => None,
    };
    let result = Channel::connect().and_then(|channel| {
        channel.send(&SrvRequest::AnnounceTrack {
            session_id: ids.0,
            file_id: ids.1,
            title: position.title.clone(),
            index: position.index as u32,
            len: position.len as u32,
            target: target.to_string(),
            tags,
            looping: position.looping,
            current: position.current.clone(),
            next: position.next.clone(),
        })?;
        if let Some(sheet) = &position.cue {
            channel.send(&SrvRequest::AnnounceCue { session_id: ids.0, file_id: ids.1, sheet: sheet.clone() })?;
        }
        Ok(())
    });
    // the title and the previous/next buttons are lost, not the playback
    let _ = result;
}

/// Reads what Som sends to a placement's responder connection until the track
/// ends (`ended` is set) or the connection goes. `serve` answers a range request.
pub(super) fn watch_responder(
    responder: Arc<Channel>,
    ids: (u32, u32),
    shared: Arc<Shared>,
    ended: Arc<AtomicBool>,
    mut serve: impl FnMut(&Channel, u64, u64) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(incoming) = responder.next_forwarded() {
            match incoming {
                SrvRequest::RequestByteRange { session_id, file_id, offset, len } if (session_id, file_id) == ids => {
                    serve(&responder, offset, len)
                }
                SrvRequest::EndPlayback { session_id, file_id } if (session_id, file_id) == ids => {
                    ended.store(true, Ordering::Release);
                    return;
                }
                SrvRequest::PlaylistStep { session_id, file_id, delta } if (session_id, file_id) == ids => shared.step.store(delta.signum(), Ordering::Release),
                SrvRequest::RadioControl { session_id, file_id, control } if (session_id, file_id) == ids => {
                    super::radio::handle_control(&shared, ids, control)
                }
                _ => {}
            }
        }
        // the connection is gone: nothing will ask again
        ended.store(true, Ordering::Release);
    })
}

/// Waits until the track ended, the viewer went, or a step was asked.
fn wait_for_end(shared: &Shared, ended: &AtomicBool) {
    while !shared.cancelled() && !ended.load(Ordering::Acquire) && shared.step.load(Ordering::Acquire) == 0 {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Plays one local file: the track is announced, the front of the file goes out
/// (a video's end too), and a second connection registered as the range responder
/// answers what Som asks for.
fn play_file(path: &Path, kind: Kind, position: Option<&Position>, ids: (u32, u32), shared: &Arc<Shared>) -> Result<(), String> {
    let total_size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let (content_type, metadata) = match kind {
        Kind::Video => (ContentType::Video, video_metadata(path, shared.streams)),
        _ => (ContentType::Audio, audio::metadata(path)?),
    };
    let file = Arc::new(Mutex::new(std::fs::File::open(path).map_err(|e| e.to_string())?));
    let channel = Channel::connect()?;

    if let Some(position) = position {
        announce(ids, Some(path), &path.display().to_string(), position);
    }

    // the responder: its connection is the one Som's requests for ranges come on
    let responder = Arc::new(Channel::connect()?);
    responder.send(&SrvRequest::RegisterRangeResponder { session_id: ids.0, file_id: ids.1 })?;
    let ended = Arc::new(AtomicBool::new(false));
    let thread = {
        let (file, metadata) = (file.clone(), metadata.clone());
        watch_responder(responder.clone(), ids, shared.clone(), ended.clone(), move |responder, offset, len| {
            let _ = send_range(responder, &file, ids, content_type, &metadata, total_size, offset, len, &|| false);
        })
    };

    send_range(&channel, &file, ids, content_type, &metadata, total_size, 0, ANNOUNCE_LEN, &|| shared.cancelled())?;
    if kind == Kind::Video {
        let tail = total_size.saturating_sub(TAIL_LEN).max(ANNOUNCE_LEN.min(total_size));
        let _ = send_range(&channel, &file, ids, content_type, &metadata, total_size, tail, TAIL_LEN, &|| shared.cancelled());
    }
    shared.set_status(super::Status::Sent);

    wait_for_end(shared, &ended);
    responder.shutdown();
    let _ = thread.join();
    // a no-op if the playback already ended
    let _ = channel.send(&SrvRequest::StopPlayback { session_id: ids.0, file_id: ids.1 });
    Ok(())
}

/// A URL placement being made ready: the daemon is told what it is (`AnnounceTrack`) and asked for
/// its bytes (`FetchResource`), and the fetch is watched for its failure. A radio makes the next
/// track ready while this one plays, so that when this one ends the next one is in the daemon's cache
/// with its tags and cover known, and nothing is waited for.
pub(super) struct Fetch {
    pub(super) ids: (u32, u32),
    channel: Arc<Channel>,
    /// Set when the fetch failed, and later by the end of the playback.
    ended: Arc<AtomicBool>,
    /// Set when the daemon has the whole of it: it can be played at once.
    ready: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Fetch {
    fn drop(&mut self) {
        self.close();
    }
}

impl Fetch {
    /// Gives it up unplayed (the radio went another way): the placement is stopped, nothing is left in the daemon.
    pub(super) fn abandon(mut self) {
        self.close();
        if let Ok(channel) = Channel::connect() {
            let _ = channel.send(&SrvRequest::StopPlayback { session_id: self.ids.0, file_id: self.ids.1 });
        }
    }

    /// Whether the daemon has the bytes of this one (its start waits for nothing).
    pub(super) fn ready_flag(&self) -> Arc<AtomicBool> {
        self.ready.clone()
    }

    fn close(&mut self) {
        self.channel.shutdown();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Announces `ids` and starts the daemon's fetch of `url`.
pub(super) fn begin_fetch(url: &str, position: &Position, ids: (u32, u32)) -> Result<Fetch, String> {
    let channel = Arc::new(Channel::connect()?);
    announce(ids, None, url, position);
    let ended = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    channel.send(&SrvRequest::FetchResource { session_id: ids.0, file_id: ids.1, target: url.to_string(), base_dir: None })?;
    // The daemon takes the requests of a connection one after another, and `FetchResource` is not
    // answered when it succeeds: a request that has an answer, sent after it, is answered once the
    // fetch is done. That answer says the bytes are all there.
    channel.send(&SrvRequest::ListSessions { client_id: Some("farsh-fetch-done".to_string()) })?;
    let reader = {
        let (channel, ended, failure, ready) = (channel.clone(), ended.clone(), failure.clone(), ready.clone());
        std::thread::spawn(move || {
            while let Ok(message) = channel.read_raw() {
                match serde_json::from_slice::<SrvResponse>(&message) {
                    Ok(SrvResponse::FetchFailed { reason, .. }) => {
                        *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
                        ended.store(true, Ordering::Release);
                        return;
                    }
                    Ok(SrvResponse::Sessions(_)) => ready.store(true, Ordering::Release),
                    _ => {}
                }
            }
        })
    };
    Ok(Fetch { ids, channel, ended, ready, failure, reader: Some(reader) })
}

/// A URL: the daemon fetches it into its cache itself, so farsh only asks, and
/// listens for the end of the playback and for a failure of the fetch. `prepared` is the
/// fetch begun ahead for these `ids`.
pub(super) fn play_url(url: &str, position: &Position, ids: (u32, u32), shared: &Arc<Shared>, prepared: Option<Fetch>) -> Result<(), String> {
    let mut fetch = match prepared {
        Some(fetch) => fetch,
        None => begin_fetch(url, position, ids)?,
    };

    // registered only to hear the end of the playback: the daemon never forwards ranges for a fetched key
    let responder = Arc::new(Channel::connect()?);
    responder.send(&SrvRequest::RegisterRangeResponder { session_id: ids.0, file_id: ids.1 })?;
    if position.radio {
        super::radio::announce_state(shared, ids);
    }
    let ended = fetch.ended.clone();
    let thread = watch_responder(responder.clone(), ids, shared.clone(), ended.clone(), |_, _, _| {});
    shared.set_status(super::Status::Sent);

    wait_for_end(shared, &ended);
    responder.shutdown();
    fetch.close();
    let _ = thread.join();
    if let Ok(channel) = Channel::connect() {
        let _ = channel.send(&SrvRequest::StopPlayback { session_id: ids.0, file_id: ids.1 });
    }
    let failure = fetch.failure.lock().unwrap_or_else(|e| e.into_inner()).take();
    match failure {
        Some(reason) => Err(format!("fetching: {reason}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_skips_comments_and_keeps_titles_and_urls() {
        let text = "\u{feff}#EXTM3U\n#EXTINF:123,First song\na.mp3\n\n# a comment\nhttps://example.com/radio\n#EXTINF:5,\nb.flac\n";
        let entries = parse_m3u(text, None);
        assert_eq!(entries.len(), 3);
        assert_eq!((entries[0].target.as_str(), entries[0].title.as_deref()), ("a.mp3", Some("First song")));
        assert_eq!(entries[1].target, "https://example.com/radio");
        assert_eq!(entries[2].title, None, "an empty title is no title");
    }

    #[test]
    fn m3u_paths_are_relative_to_the_lists_folder_unless_absolute_or_urls() {
        let base = Path::new("music");
        let absolute = if cfg!(windows) { "C:\\x\\z.mp3" } else { "/x/z.mp3" };
        let entries = parse_m3u(&format!("a.mp3\n{absolute}\nhttp://h/s\n"), Some(base));
        assert_eq!(Path::new(&entries[0].target), base.join("a.mp3"));
        assert_eq!(entries[1].target, absolute);
        assert_eq!(entries[2].target, "http://h/s");
    }

    #[test]
    fn a_sheet_with_one_file_and_many_tracks_is_one_entry_with_its_track_list() {
        let dir = std::env::temp_dir().join(format!("farsh-cue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("album.flac"), b"x").unwrap();
        let sheet = dir.join("album.cue");
        let text = "PERFORMER \"Band\"\nTITLE \"Album\"\nFILE \"album.flac\" WAVE\n TRACK 01 AUDIO\n  TITLE \"One\"\n  INDEX 01 00:00:00\n TRACK 02 AUDIO\n  TITLE \"Two\"\n  INDEX 01 03:00:00\n";
        let entries = cue_entries(&sheet, text).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        let list = entries[0].cue.as_ref().unwrap();
        assert_eq!((list.album.as_str(), list.tracks.len(), list.tracks[1].start_ms), ("Album", 2, 180_000));
    }

    #[test]
    fn a_sheet_whose_audio_is_missing_or_that_has_no_tracks_is_an_error() {
        let sheet = std::env::temp_dir().join("farsh-nowhere").join("a.cue");
        assert!(cue_entries(&sheet, "FILE \"gone.flac\" WAVE\n TRACK 01 AUDIO\n  INDEX 01 00:00:00\n").is_err());
        assert!(cue_entries(&sheet, "TITLE \"x\"\n").is_err());
    }
}
