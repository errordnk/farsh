//! What farsh says to `somsrv` and hears from it: the JSON messages of Som's
//! binary side-channel (`somsrv::protocol` in the `som` repository, whose
//! `SRP_INTEGRATION_GUIDE.md` describes them). Only the part a viewer needs is
//! written out here; serde's default (externally tagged) enum form is the
//! wire form, so the names of variants and fields must stay as they are in Som.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize)]
pub enum Os {
    Windows,
    Darwin,
    Linux,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum Arch {
    Amd64,
    Arm64,
}

/// First message on a connection. `somsrv` only logs it and answers with its own.
#[derive(Debug, Clone, Serialize)]
pub struct HandshakeInfo {
    pub version: String,
    pub os: Os,
    pub arch: Arch,
    pub build_id: u64,
    pub pid: u32,
}

impl HandshakeInfo {
    pub fn current() -> Self {
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Darwin
        } else {
            Os::Linux
        };
        let arch = if cfg!(target_arch = "x86_64") { Arch::Amd64 } else { Arch::Arm64 };
        Self { version: env!("CARGO_PKG_VERSION").to_string(), os, arch, build_id: 0, pid: std::process::id() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ContentType {
    Gif,
    Audio,
    Markdown,
    Video,
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum VideoCodec {
    Unknown,
    H264,
    H265,
    Vp9,
    Av1,
    Mpeg4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ContentMetadata {
    Image { width_px: u32, height_px: u32, color_bits: u8, is_animated: bool },
    Audio { sample_rate: u32, channels: u8, bits_per_sample: u8, duration_ms: u32, extension: String },
    Video {
        width_px: u32,
        height_px: u32,
        fps_numerator: u32,
        fps_denominator: u32,
        codec: VideoCodec,
        audio_stream_index: Option<u32>,
        subtitle_stream_index: Option<u32>,
        extension: String,
    },
    Markdown { base_dir: String },
}

/// What a track's file says of it (title, artist, album, year).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TrackTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: Option<u32>,
}

/// What a station announces about a track: its tags and where its cover is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextTrack {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub year: Option<u32>,
    pub duration_ms: u32,
    pub cover_url: Option<String>,
}

/// One track of a radio batch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadioTrack {
    pub id: String,
    /// "artist - title", the way a list shows it.
    pub title: String,
    /// The link that plays it.
    pub target: String,
    pub liked: bool,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(default)]
    pub cover_url: Option<String>,
}

/// What a listener did to a track of a radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RadioEvent {
    RadioStarted,
    TrackStarted,
    TrackFinished,
    Skip,
}

/// What a click on an icon of the radio widget asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum RadioControl {
    ToggleLike,
    ToggleTrackWave,
}

/// One track of a [`CueSheet`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CueTrack {
    pub number: u32,
    /// Where the track starts in the file, in milliseconds.
    pub start_ms: u64,
    pub title: String,
    /// Empty when the track has no performer of its own.
    pub artist: String,
}

/// The track list of a CUE sheet: one audio file cut into `tracks`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CueSheet {
    pub album: String,
    pub artist: String,
    pub year: Option<u32>,
    /// Where the album's picture is (a path).
    pub cover: Option<String>,
    pub tracks: Vec<CueTrack>,
}

/// The requests farsh sends.
#[derive(Debug, Clone, Serialize)]
pub enum Request {
    Handshake(HandshakeInfo),
    /// Registers this connection as the one that hears `GrowMarkdownRows` of a document.
    RegisterMarkdownGrower { session_id: u32, file_id: u32 },
    /// The track list of a CUE sheet, for the placement's widget.
    AnnounceCue { session_id: u32, file_id: u32, sheet: CueSheet },
    /// Som's daemon fetches `target` (a URL) itself and streams it into the cache.
    FetchResource { session_id: u32, file_id: u32, target: String, base_dir: Option<String> },
    /// Names the track of a placement for Som's widget. `len: 0` is "not in a
    /// playlist" (no previous/next buttons). `current` and `next` are for
    /// radio batches and stay empty.
    AnnounceTrack {
        session_id: u32,
        file_id: u32,
        title: Option<String>,
        index: u32,
        len: u32,
        target: String,
        tags: Option<TrackTags>,
        looping: bool,
        current: Option<NextTrack>,
        next: Option<NextTrack>,
    },
    /// The next tracks of a radio after the track `queue`; answered with [`Response::RadioTracks`] or [`Response::RadioFailed`].
    RadioTracks { service: String, station: String, queue: Option<String> },
    /// What the listener did, so the radio adapts; nothing is answered.
    RadioFeedback { service: String, station: String, batch_id: String, event: RadioEvent, track_id: String, played_seconds: f32 },
    /// Likes or un-likes a track; nothing is answered.
    RadioLike { service: String, track_id: String, liked: bool },
    /// What the widget's radio icons show for the placement.
    AnnounceRadioState { session_id: u32, file_id: u32, liked: bool, track_wave: bool, wave_label: Option<String> },
    PutChunk {
        session_id: u32,
        file_id: u32,
        offset: u64,
        data: Vec<u8>,
        total_size: u64,
        content_type: ContentType,
        metadata: ContentMetadata,
    },
    /// Registers this connection as the one `RequestByteRange` is sent to.
    RegisterRangeResponder { session_id: u32, file_id: u32 },
    /// Asks Som to stop the player of a placement.
    StopPlayback { session_id: u32, file_id: u32 },
}

/// What can arrive on a connection that sends chunks: Som's requests for a
/// range of the file (forwarded as they are) and its end of a playback. The
/// rest (the handshake answer, progress) is of no use to a viewer.
#[derive(Debug, Clone, Deserialize)]
pub enum Incoming {
    RequestByteRange { session_id: u32, file_id: u32, offset: u64, len: u64 },
    EndPlayback { session_id: u32, file_id: u32 },
    /// Som laid the document out and needs `additional_rows` more rows of cells.
    GrowMarkdownRows { session_id: u32, file_id: u32, additional_rows: u32 },
    /// A click on one of the radio widget's icons.
    RadioControl { session_id: u32, file_id: u32, control: RadioControl },
    /// The widget's previous (`-1`) or next (`1`) button.
    PlaylistStep { session_id: u32, file_id: u32, delta: i32 },
}

/// What the daemon answers on a connection that asked it to fetch a URL.
#[derive(Debug, Clone, Deserialize)]
pub enum Response {
    FetchFailed { session_id: u32, file_id: u32, reason: String },
    RadioTracks { batch_id: String, tracks: Vec<RadioTrack> },
    RadioFailed { reason: String },
}

/// The connection kind byte that precedes everything: `Srv`.
pub const KIND_SRV: u8 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_has_the_shape_somsrv_reads() {
        let chunk = Request::PutChunk {
            session_id: 1,
            file_id: 2,
            offset: 0,
            data: vec![7, 8],
            total_size: 2,
            content_type: ContentType::Png,
            metadata: ContentMetadata::Image { width_px: 3, height_px: 4, color_bits: 32, is_animated: false },
        };
        assert_eq!(
            serde_json::to_string(&chunk).unwrap(),
            r#"{"PutChunk":{"session_id":1,"file_id":2,"offset":0,"data":[7,8],"total_size":2,"content_type":"Png","metadata":{"Image":{"width_px":3,"height_px":4,"color_bits":32,"is_animated":false}}}}"#
        );
    }

    #[test]
    fn a_lone_track_is_announced_as_somsrp_does() {
        let announce = Request::AnnounceTrack {
            session_id: 1,
            file_id: 2,
            title: Some("song".into()),
            index: 0,
            len: 0,
            target: "a.mp3".into(),
            tags: Some(TrackTags { title: "song".into(), artist: "me".into(), album: String::new(), year: Some(1999) }),
            looping: true,
            current: None,
            next: None,
        };
        assert_eq!(
            serde_json::to_string(&announce).unwrap(),
            r#"{"AnnounceTrack":{"session_id":1,"file_id":2,"title":"song","index":0,"len":0,"target":"a.mp3","tags":{"title":"song","artist":"me","album":"","year":1999},"looping":true,"current":null,"next":null}}"#
        );
    }

    #[test]
    fn requests_without_fields_and_the_handshake() {
        assert_eq!(
            serde_json::to_string(&Request::StopPlayback { session_id: 5, file_id: 6 }).unwrap(),
            r#"{"StopPlayback":{"session_id":5,"file_id":6}}"#
        );
        let handshake = serde_json::to_value(Request::Handshake(HandshakeInfo::current())).unwrap();
        let info = &handshake["Handshake"];
        assert!(info["version"].is_string() && info["os"].is_string() && info["arch"].is_string());
        assert_eq!(info["build_id"], 0);
    }

    #[test]
    fn what_som_asks_is_read_and_the_rest_is_not_an_error_to_skip() {
        let range: Incoming =
            serde_json::from_str(r#"{"RequestByteRange":{"session_id":1,"file_id":2,"offset":10,"len":20}}"#).unwrap();
        assert!(matches!(range, Incoming::RequestByteRange { offset: 10, len: 20, .. }));
        let end: Incoming = serde_json::from_str(r#"{"EndPlayback":{"session_id":1,"file_id":2}}"#).unwrap();
        assert!(matches!(end, Incoming::EndPlayback { file_id: 2, .. }));
        assert!(serde_json::from_str::<Incoming>(r#"{"Handshake":{"version":"x"}}"#).is_err());
    }
}
