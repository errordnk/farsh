//! SRP, Som's rich-content protocol, for the F3 viewer and the quick view.
//!
//! A placement is text: a grid of placeholder cells (`srp_client::placeholder`)
//! that farsh draws like any other cells, and the file's bytes, which go to
//! the `somsrv` daemon on this machine over a side-channel ([`Channel`]).
//! Som finds the cells, asks the daemon for the bytes and paints the picture
//! (or the player) over them, fitted into the rectangle with its proportions
//! kept. Only inside Som (`SOM_WINDOW_ID`), and only to a daemon that is
//! running: farsh never starts or replaces one.
//!
//! The protocol is Som's `srp_client` crate (the types `somsrv` itself uses;
//! Som's `SRP_INTEGRATION_GUIDE.md` describes it).

pub mod audio;
pub mod play;
pub mod radio;
pub mod streams;

pub use srp_client::channel::SrvChannel as Channel;
pub use srp_client::cue;

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use srp_client::protocol::{ContentMetadata, ContentType, SrvRequest, VideoCodec};
use srp_client::channel::Incoming;

/// The size of one `PutChunk`.
const CHUNK: usize = 64 * 1024;

/// A file read whole into memory is at most this big.
const IMAGE_LIMIT: u64 = 256 << 20;

/// How long a quick view waits on one file before it reads and sends it, so
/// that moving the cursor over many files does not start a transfer for each.
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

/// Whether this farsh runs in Som, which paints placements.
pub fn in_som() -> bool {
    std::env::var_os("SOM_WINDOW_ID").is_some() || std::env::var_os("FARSH_SRP").is_some()
}

/// The ids of a transfer: both time-derived, 24 bits, never 0 (they travel
/// in the colors of the placeholder cells).
pub fn new_ids() -> (u32, u32) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u32).unwrap_or(1);
    static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // two placements started in the same millisecond must differ
    let mut chosen = now;
    let _ = LAST.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |previous| {
        chosen = now.max(previous.wrapping_add(1));
        Some(chosen)
    });
    ((chosen & 0xFF_FFFF).max(1), (chosen.wrapping_mul(2_654_435_761) & 0xFF_FFFF).max(1))
}

/// What the viewer can show through SRP, by the file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image(ContentType),
    /// Played by Som; the file stays open here and Som asks for its ranges.
    Audio,
    /// The same, with a picture; its size is not probed (see [`video_metadata`]).
    Video,
    /// An `.m3u` list: its entries are played one after the other.
    Playlist,
    /// A `.cue` sheet: its audio file(s), with the track list announced.
    Cue,
    /// A document Som renders; its cells grow as Som lays it out.
    Markdown,
}

pub fn kind_of(path: &Path) -> Option<Kind> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    Some(match extension.as_str() {
        "png" => Kind::Image(ContentType::Png),
        "jpg" | "jpeg" => Kind::Image(ContentType::Jpeg),
        "gif" => Kind::Image(ContentType::Gif),
        "mp3" | "flac" | "ogg" | "oga" => Kind::Audio,
        "mp4" | "mkv" | "avi" => Kind::Video,
        "m3u" | "m3u8" => Kind::Playlist,
        "cue" => Kind::Cue,
        "md" => Kind::Markdown,
        _ => return None,
    })
}

/// Which audio and subtitle streams of a video Som plays: the container's stream
/// numbers as FFmpeg counts them (what `somsrp -a <n> -s <n>` takes). `None` is
/// Som's default. farsh cannot list a container's streams, so the viewer takes the
/// number from the user (`a` or `s` and a digit).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Streams {
    pub audio: Option<u32>,
    pub subtitle: Option<u32>,
}

/// What a video is announced with: no size, rate or codec (`somsrp` probes
/// them with FFmpeg on Windows only and sends zeros elsewhere; Som reads the
/// real ones itself when it opens the stream, and fits the picture into the
/// placement with its proportions kept, so the placement needs no exact size).
/// No audio or subtitle stream is chosen: Som takes the default ones.
pub(super) fn video_metadata(path: &Path, streams: Streams) -> ContentMetadata {
    ContentMetadata::Video {
        width_px: 0,
        height_px: 0,
        fps_numerator: 0,
        fps_denominator: 0,
        codec: VideoCodec::Unknown,
        audio_stream_index: streams.audio,
        subtitle_stream_index: streams.subtitle,
        extension: path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string(),
    }
}

/// A non-faststart MP4 has its index at the end, a Matroska file may keep its
/// cues there: the last bytes go out with the first, so that Som's probe does
/// not wait for a round trip (the same figure as `somsrp`).
pub(super) const TAIL_LEN: u64 = 4 * 1024 * 1024;

/// How much of the front of an audio file goes out unasked: enough for the
/// daemon to open its cache entry and for the probe to start. All the rest is
/// sent when Som asks for it.
pub(super) const ANNOUNCE_LEN: u64 = 256 * 1024;

/// What a viewer reads off a connection: the requests the daemon forwards
/// from Som (`RequestByteRange`, `EndPlayback`, `GrowMarkdownRows`...); the
/// daemon's own answers on it are skipped. An error is the end of the connection.
pub trait Forwarded {
    fn next_forwarded(&self) -> Result<SrvRequest, String>;
}

impl Forwarded for Channel {
    fn next_forwarded(&self) -> Result<SrvRequest, String> {
        loop {
            if let Incoming::Request(request) = self.read_incoming()? {
                return Ok(request);
            }
        }
    }
}

/// `bytes` of `file` from `offset` as chunks on `channel`; `total_size` is the
/// whole file's. Seeks for each chunk: the announce and the answers to Som's
/// requests come from two threads over one file.
#[allow(clippy::too_many_arguments)]
pub(super) fn send_range(
    channel: &Channel,
    file: &Mutex<std::fs::File>,
    ids: (u32, u32),
    content_type: ContentType,
    metadata: &ContentMetadata,
    total_size: u64,
    offset: u64,
    len: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let end = offset.saturating_add(len).min(total_size);
    let mut at = offset;
    let mut buffer = vec![0u8; CHUNK];
    while at < end {
        if cancelled() {
            return Ok(());
        }
        let piece = (end - at).min(CHUNK as u64) as usize;
        {
            let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
            file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
            file.read_exact(&mut buffer[..piece]).map_err(|e| e.to_string())?;
        }
        channel.send(&SrvRequest::PutChunk {
            session_id: ids.0,
            file_id: ids.1,
            offset: at,
            data: buffer[..piece].to_vec(),
            total_size,
            content_type,
            metadata: metadata.clone(),
        })?;
        at += piece as u64;
    }
    Ok(())
}

/// The requests that carry `bytes` of an image: chunks in order.
pub fn image_requests(ids: (u32, u32), bytes: &[u8], content_type: ContentType, metadata: &ContentMetadata) -> Vec<SrvRequest> {
    let total_size = bytes.len() as u64;
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .chunks(CHUNK)
        .enumerate()
        .map(|(index, data)| SrvRequest::PutChunk {
            session_id: ids.0,
            file_id: ids.1,
            offset: (index * CHUNK) as u64,
            data: data.to_vec(),
            total_size,
            content_type,
            metadata: metadata.clone(),
        })
        .collect()
}

/// An image's size from its header, and whether a GIF has more than one frame
/// (a second image-descriptor byte after the first; no frame is decoded).
fn image_metadata(bytes: &[u8], content_type: ContentType) -> Result<ContentMetadata, String> {
    let (width_px, height_px) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    let is_animated = content_type == ContentType::Gif && bytes.iter().filter(|&&b| b == 0x2C).count() > 1;
    Ok(ContentMetadata::Image { width_px, height_px, color_bits: 32, is_animated })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Reading the file and sending it.
    Sending,
    /// Everything is with the daemon; Som has it.
    Sent,
    Failed(String),
}

/// What the transfer thread and the viewer share.
pub struct Shared {
    streams: Streams,
    /// What a radio keeps between its runner and the clicks on its widget.
    pub(super) radio: radio::State,
    cancel: AtomicBool,
    status: Mutex<Status>,
    /// The ids the placement's cells carry now; a playlist changes them with each track.
    ids: Mutex<(u32, u32)>,
    /// Counts what the viewer would draw differently (ids, status, position).
    generation: AtomicU32,
    /// The widget's previous/next (or a key) waiting to be taken by the list runner.
    step: AtomicI32,
    /// The track playing and the length of its list, for a list.
    position: Mutex<Option<(usize, usize)>>,
    /// How many rows of cells a document has now (it starts with one; Som asks for more).
    rows: AtomicU32,
}

impl Shared {
    fn new(ids: (u32, u32), streams: Streams) -> Self {
        Self {
            streams,
            radio: radio::State::default(),
            cancel: AtomicBool::new(false),
            status: Mutex::new(Status::Sending),
            ids: Mutex::new(ids),
            generation: AtomicU32::new(1),
            step: AtomicI32::new(0),
            position: Mutex::new(None),
            rows: AtomicU32::new(1),
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn set_status(&self, status: Status) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
        self.touch();
    }

    fn set_ids(&self, ids: (u32, u32)) {
        *self.ids.lock().unwrap_or_else(|e| e.into_inner()) = ids;
        self.touch();
    }

    fn ids(&self) -> (u32, u32) {
        *self.ids.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Som asked for `more` rows for a document, at most as many as the marks can number.
    fn grow_rows(&self, more: u32) {
        let rows = self.rows.load(Ordering::Relaxed).saturating_add(more).min(srp_client::placeholder::MAX_CELLS);
        self.rows.store(rows, Ordering::Relaxed);
        self.touch();
    }

    /// Som told the document's real number of rows (at least one, as it starts).
    fn set_rows(&self, rows: u32) {
        self.rows.store(rows.clamp(1, srp_client::placeholder::MAX_CELLS), Ordering::Relaxed);
        self.touch();
    }

    fn set_position(&self, position: Option<(usize, usize)>) {
        *self.position.lock().unwrap_or_else(|e| e.into_inner()) = position;
        self.touch();
    }
}

/// One placement: the ids its cells carry, and the transfer that serves it.
/// Dropping it stops the transfer; the cells go when farsh stops drawing them.
pub struct Media {
    pub kind: Kind,
    shared: Arc<Shared>,
    /// The generation the viewer last drew.
    painted: AtomicU32,
}

impl Media {
    /// Starts the placement of `path` if SRP applies to it here.
    pub fn start(path: &Path, debounce: bool) -> Option<Self> {
        Self::start_with(path, debounce, Streams::default())
    }

    /// The same for a video with chosen audio and subtitle streams.
    pub fn start_with(path: &Path, debounce: bool, streams: Streams) -> Option<Self> {
        if !in_som() {
            return None;
        }
        let kind = kind_of(path)?;
        let shared = Arc::new(Shared::new(new_ids(), streams));
        let (worker, path) = (shared.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let result = transfer(&path, kind, &worker, debounce);
            worker.set_status(match result {
                Ok(()) => Status::Sent,
                Err(message) => Status::Failed(message),
            });
        });
        Some(Self { kind, shared, painted: AtomicU32::new(0) })
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The ids to draw the cells with; what is drawn counts as seen ([`Self::busy`]).
    pub fn ids(&self) -> (u32, u32) {
        let seen = self.shared.generation.load(Ordering::Acquire);
        let ids = self.shared.ids();
        self.painted.store(seen, Ordering::Relaxed);
        ids
    }

    /// Something changed that the viewer has not drawn yet: the transfer
    /// moved on (a new track, a new status), with no key to make a redraw.
    pub fn busy(&self) -> bool {
        self.shared.generation.load(Ordering::Acquire) != self.painted.load(Ordering::Relaxed)
    }

    /// Previous (`-1`) or next (`1`) in a list; nothing for a single file.
    pub fn step(&self, delta: i32) {
        if self.is_list() {
            self.shared.step.store(delta.signum(), Ordering::Release);
        }
    }

    pub fn is_list(&self) -> bool {
        matches!(self.kind, Kind::Playlist | Kind::Cue)
    }

    /// The rows of cells a document has now.
    pub fn rows(&self) -> u32 {
        self.shared.rows.load(Ordering::Relaxed)
    }

    /// The track playing and the number of tracks, for a list.
    pub fn position(&self) -> Option<(usize, usize)> {
        *self.shared.position.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}

fn transfer(path: &Path, kind: Kind, shared: &Arc<Shared>, debounce: bool) -> Result<(), String> {
    if debounce {
        std::thread::sleep(DEBOUNCE);
    }
    if shared.cancelled() {
        return Ok(());
    }
    match kind {
        Kind::Image(content_type) => send_image(path, content_type, shared),
        Kind::Audio | Kind::Video => play::run_single(path, kind, shared),
        Kind::Playlist => play::run_playlist_file(path, shared),
        Kind::Cue => play::run_cue_file(path, shared),
        Kind::Markdown => play::run_markdown(path, shared),
    }
}

/// The picture's chunks, then its range responder until the placement goes: Som
/// may see the cells in the middle of the transfer (its first chunk is then from
/// the middle) or after it, and asks for the whole file from the start. The
/// transfer's own connection is only written to, so without a responder that
/// request was never answered - somsrp's 8 MB GIF showed nothing (2026-11-06).
fn send_image(path: &Path, content_type: ContentType, shared: &Arc<Shared>) -> Result<(), String> {
    let ids = shared.ids();
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > IMAGE_LIMIT {
        return Err(format!("{} MB is too much for a picture", size >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let metadata = image_metadata(&bytes, content_type)?;
    let file = Arc::new(Mutex::new(std::fs::File::open(path).map_err(|e| e.to_string())?));
    let responder = Arc::new(Channel::connect()?);
    responder.send(&SrvRequest::RegisterRangeResponder { session_id: ids.0, file_id: ids.1 })?;
    let ended = Arc::new(AtomicBool::new(false));
    let thread = {
        let metadata = metadata.clone();
        play::watch_responder(responder.clone(), ids, shared.clone(), ended.clone(), move |responder, offset, len| {
            let _ = send_range(responder, &file, ids, content_type, &metadata, size, offset, len, &|| false);
        })
    };
    let channel = Channel::connect()?;
    for request in image_requests(ids, &bytes, content_type, &metadata) {
        if shared.cancelled() {
            break;
        }
        channel.send(&request)?;
    }
    drop(channel);
    shared.set_status(Status::Sent);
    while !shared.cancelled() && !ended.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    responder.shutdown();
    let _ = thread.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_comes_from_the_extension() {
        assert_eq!(kind_of(Path::new("a/B.MP3")), Some(Kind::Audio));
        assert_eq!(kind_of(Path::new("x.flac")), Some(Kind::Audio));
        assert_eq!(kind_of(Path::new("x.oga")), Some(Kind::Audio));
        assert_eq!(kind_of(Path::new("film.MKV")), Some(Kind::Video));
        assert_eq!(kind_of(Path::new("clip.mp4")), Some(Kind::Video));
        assert_eq!(kind_of(Path::new("a.M3U8")), Some(Kind::Playlist));
        assert_eq!(kind_of(Path::new("a.cue")), Some(Kind::Cue));
        assert_eq!(kind_of(Path::new("README.md")), Some(Kind::Markdown));
        assert_eq!(kind_of(Path::new("a/B.PNG")), Some(Kind::Image(ContentType::Png)));
        assert_eq!(kind_of(Path::new("x.jpeg")), Some(Kind::Image(ContentType::Jpeg)));
        assert_eq!(kind_of(Path::new("x.gif")), Some(Kind::Image(ContentType::Gif)));
        assert_eq!(kind_of(Path::new("x.txt")), None);
        assert_eq!(kind_of(Path::new("noext")), None);
    }

    #[test]
    fn a_video_is_announced_with_the_streams_chosen_for_it() {
        let streams = Streams { audio: Some(2), subtitle: None };
        match video_metadata(Path::new("a/film.mkv"), streams) {
            ContentMetadata::Video { audio_stream_index: Some(2), subtitle_stream_index: None, width_px: 0, extension, .. } => {
                assert_eq!(extension, "mkv")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ids_are_24_bit_nonzero_and_differ_between_placements() {
        let a = new_ids();
        let b = new_ids();
        for id in [a.0, a.1, b.0, b.1] {
            assert!((1..=0xFF_FFFF).contains(&id));
        }
        assert_ne!(a, b);
    }

    #[test]
    fn a_file_goes_in_chunks_with_its_total_size_and_metadata_on_each() {
        let metadata = ContentMetadata::Image { width_px: 1, height_px: 1, color_bits: 32, is_animated: false };
        let bytes = vec![5u8; CHUNK * 2 + 10];
        let requests = image_requests((7, 8), &bytes, ContentType::Png, &metadata);
        assert_eq!(requests.len(), 3);
        let offsets: Vec<u64> = requests
            .iter()
            .map(|r| match r {
                SrvRequest::PutChunk { offset, total_size, session_id: 7, file_id: 8, .. } => {
                    assert_eq!(*total_size, bytes.len() as u64);
                    *offset
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(offsets, [0, CHUNK as u64, 2 * CHUNK as u64]);
        assert!(image_requests((1, 1), &[], ContentType::Png, &metadata).is_empty());
    }

    #[test]
    fn sizes_are_read_from_headers_and_an_animation_is_told_by_a_second_frame() {
        // a 2x3 PNG made by the image crate
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(2, 3).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(
            image_metadata(&png, ContentType::Png).unwrap(),
            ContentMetadata::Image { width_px: 2, height_px: 3, color_bits: 32, is_animated: false }
        );
        assert!(image_metadata(b"not an image", ContentType::Png).is_err());
    }
}

#[cfg(test)]
mod daemon_tests {
    use super::*;

    /// Against a test daemon (`SOMSRV_INSTANCE=<name> somsrv --daemon`), never
    /// the user's own: a picture is sent and the daemon keeps the connection.
    #[test]
    #[ignore = "needs a running test daemon and SOMSRV_INSTANCE set to its name"]
    fn an_image_goes_to_a_running_test_daemon() {
        assert!(std::env::var_os("SOMSRV_INSTANCE").is_some(), "never the user's daemon");
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(40, 30).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let metadata = image_metadata(&png, ContentType::Png).unwrap();
        let channel = Channel::connect().unwrap();
        for request in image_requests(new_ids(), &png, ContentType::Png, &metadata) {
            channel.send(&request).unwrap();
        }
        // a daemon that did not like the chunk would have closed the connection by now
        std::thread::sleep(std::time::Duration::from_millis(400));
        channel.send(&SrvRequest::StopPlayback { session_id: 1, file_id: 1 }).unwrap();
    }

    /// Som may see a picture's cells in the middle of the transfer, or after it:
    /// it then asks for the whole file from the start, and farsh answers for as
    /// long as the placement is shown. The picture is larger than the daemon's
    /// trailing window (8 MB), so nothing but farsh can answer (before, farsh's
    /// connection was only written to and then closed; somsrp had the same bug:
    /// an 8 MB GIF showed nothing, 2026-11-06).
    #[test]
    #[ignore = "needs a running test daemon and SOMSRV_INSTANCE set to its name"]
    fn a_picture_is_sent_again_whole_when_som_asks() {
        assert!(std::env::var_os("SOMSRV_INSTANCE").is_some(), "never the user's daemon");
        let path = std::env::temp_dir().join(format!("farsh-daemon-noise-{}.png", std::process::id()));
        let (width, height) = (2000u32, 1500u32);
        // noise (xorshift): it does not compress, so the PNG stays this large
        let mut state = 0x2545_f491u32;
        let pixels: Vec<u8> = (0..width * height * 3)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        image::RgbImage::from_raw(width, height, pixels).unwrap().save(&path).unwrap();
        let total = std::fs::metadata(&path).unwrap().len();
        assert!(total > 8 * 1024 * 1024, "the picture must be larger than the daemon's window");

        // SAFETY: the tests that read the variable all set it to the same value
        unsafe { std::env::set_var("FARSH_SRP", "1") };
        let media = Media::start(&path, false).expect("a picture is a kind");
        let started = std::time::Instant::now();
        while media.status() == Status::Sending && started.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(media.status(), Status::Sent);
        let ids = media.shared.ids();

        // Som: subscribed only now, and asking for the whole file
        let subscriber = Channel::connect().unwrap();
        subscriber.send(&SrvRequest::SubscribeProgress { session_id: ids.0, file_id: ids.1 }).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(incoming) = subscriber.read_incoming() {
                if let Incoming::Response(srp_client::protocol::SrvResponse::Progress { chunk_offset, chunk_data, .. }) = incoming
                    && tx.send((chunk_offset, chunk_data.len() as u64)).is_err()
                {
                    return;
                }
            }
        });
        Channel::connect()
            .unwrap()
            .send(&SrvRequest::RequestByteRange { session_id: ids.0, file_id: ids.1, offset: 0, len: total })
            .unwrap();
        let mut next = 0u64;
        while next < total {
            let Ok((offset, len)) = rx.recv_timeout(std::time::Duration::from_secs(10)) else { break };
            if offset == next {
                next += len;
            }
        }
        assert_eq!(next, total, "the whole file did not come again from the start");

        let shared = media.shared.clone();
        drop(media);
        let ended = std::time::Instant::now();
        while Arc::strong_count(&shared) > 1 && ended.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(Arc::strong_count(&shared), 1, "the transfer thread is still running");
        std::fs::remove_file(path).unwrap();
    }

    /// A document is sent and registered for growing; dropping the placement ends
    /// the transfer (the grower's read is cut).
    #[test]
    #[ignore = "needs a running test daemon and SOMSRV_INSTANCE set to its name"]
    fn a_document_is_sent_on_a_running_test_daemon() {
        assert!(std::env::var_os("SOMSRV_INSTANCE").is_some(), "never the user's daemon");
        let path = std::env::temp_dir().join(format!("farsh-daemon-{}.md", std::process::id()));
        std::fs::write(&path, b"# Title

text
").unwrap();
        // SAFETY: this test is the only one in the process that reads the variable
        unsafe { std::env::set_var("FARSH_SRP", "1") };
        let media = Media::start(&path, false).expect("a document is a kind");
        let started = std::time::Instant::now();
        while media.status() == Status::Sending && started.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(media.status(), Status::Sent);
        assert_eq!(media.rows(), 1);
        let shared = media.shared.clone();
        drop(media);
        let ended = std::time::Instant::now();
        while Arc::strong_count(&shared) > 1 && ended.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(Arc::strong_count(&shared), 1, "the transfer thread is still running");
        std::fs::remove_file(path).unwrap();
    }

    /// A track is announced, its front sent and a responder registered; dropping
    /// the placement ends the transfer promptly (the responder's read is cut).
    #[test]
    #[ignore = "needs a running test daemon and SOMSRV_INSTANCE set to its name"]
    fn audio_is_announced_and_stopped_on_a_running_test_daemon() {
        assert!(std::env::var_os("SOMSRV_INSTANCE").is_some(), "never the user's daemon");
        let path = std::env::temp_dir().join(format!("farsh-daemon-{}.flac", std::process::id()));
        std::fs::write(&path, include_bytes!("tone.flac")).unwrap();
        // SAFETY: this test is the only one in the process that reads the variable
        unsafe { std::env::set_var("FARSH_SRP", "1") };
        let media = Media::start(&path, false).expect("audio is a kind");
        let started = std::time::Instant::now();
        while media.status() == Status::Sending && started.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(media.status(), Status::Sent);
        let shared = media.shared.clone();
        drop(media);
        // the worker notices the cancel and ends: nobody else holds the Arc then
        let ended = std::time::Instant::now();
        while Arc::strong_count(&shared) > 1 && ended.elapsed() < std::time::Duration::from_secs(3) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(Arc::strong_count(&shared), 1, "the transfer thread is still running");
        std::fs::remove_file(path).unwrap();
    }
}
