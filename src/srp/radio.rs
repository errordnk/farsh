//! A streaming service's radio (Yandex Music's "My Wave"), ported from `somsrp`:
//! an endless run of tracks the service picks in batches, played one after the
//! other as URLs the daemon fetches itself (the account is linked in `somsrv`).
//! `n`/next goes on, `p`/previous back along what was played (the history is kept
//! here), what was heard is reported so the radio adapts, and the widget's icons
//! like the track or switch between the session's radio and the radio built from
//! the track playing.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::play::Entry;
use super::wire::{NextTrack, RadioControl, RadioEvent, RadioTrack, Request, Response};
use super::{Channel, Shared, new_ids};

pub const YANDEX: &str = "yandex";
pub const MY_WAVE: &str = "user:onyourwave";

/// The step that enters a branch: forward like a "next", but not a skip - the
/// track was not rejected, a wave was asked of it.
pub const WAVE_JUMP: i32 = 2;

/// This many tracks in a row failing to start ends the radio.
const MAX_FAILURES_IN_A_ROW: usize = 3;

/// What the last click on the wave icon asks of the radio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaveRequest {
    /// Back to the main line: the session's own radio.
    Main,
    /// A branch off the main line, the radio built from this track.
    Branch(String),
}

/// The track of the radio that is playing now.
struct Current {
    id: String,
    liked: bool,
}

/// What the runner and the thread that hears the widget's clicks share.
#[derive(Default)]
pub struct State {
    current: Mutex<Option<Current>>,
    /// Likes the user changed this session, by track id: they outlive the batch the track came in.
    likes: Mutex<Vec<(String, bool)>>,
    track_wave: AtomicBool,
    wave_request: Mutex<Option<WaveRequest>>,
    /// Names the track a branch is built from.
    wave_label: Mutex<Option<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl State {
    /// Whether the track is liked: what the user chose this session, else what the batch said.
    fn effective_like(&self, track_id: &str, batch_said: bool) -> bool {
        lock(&self.likes).iter().rev().find(|(id, _)| id == track_id).map(|(_, liked)| *liked).unwrap_or(batch_said)
    }
}

/// A track of the radio being played, with the batch it came in.
pub struct RadioEntry {
    pub track: RadioTrack,
    pub batch_id: String,
    /// The station the track came from: the radio's feedback is about it.
    pub station: String,
}

/// A playlist line that names Yandex Music at all: in a playlist it stands for
/// the service's radio, whatever else the line says.
fn names_yandex(line: &str) -> bool {
    line.to_ascii_lowercase().contains("yandex")
}

/// The radio a playlist stands for: its one line. A radio plays on its own, so it
/// cannot share a playlist with other entries (`Err`); a playlist without a radio
/// line is `Ok(None)`.
pub fn radio_in_playlist(entries: &[Entry]) -> Result<Option<(&'static str, String)>, String> {
    if !entries.iter().any(|entry| names_yandex(&entry.target)) {
        return Ok(None);
    }
    if entries.len() > 1 {
        return Err("a radio plays on its own, it cannot share a playlist with other entries".to_string());
    }
    Ok(Some((YANDEX, MY_WAVE.to_string())))
}

/// What names the track a branch is made from: "artist - title, year".
fn wave_label(track: &RadioTrack) -> String {
    match track.year {
        Some(year) => format!("{}, {year}", track.title),
        None => track.title.clone(),
    }
}

/// What the radio's batch says of a track, as the daemon is told it ahead of the stream.
pub fn announced_track(track: &RadioTrack) -> NextTrack {
    NextTrack {
        artist: track.artist.clone(),
        title: if track.name.is_empty() { track.title.clone() } else { track.name.clone() },
        album: track.album.clone(),
        year: track.year,
        duration_ms: track.duration_ms,
        cover_url: track.cover_url.clone(),
    }
}

/// Tells the daemon what the radio icons of the placement show.
pub fn announce_state(shared: &Shared, ids: (u32, u32)) {
    let liked = lock(&shared.radio.current).as_ref().is_some_and(|track| track.liked);
    let track_wave = shared.radio.track_wave.load(Ordering::Acquire);
    let wave_label = lock(&shared.radio.wave_label).clone();
    let _ = Channel::connect()
        .and_then(|channel| channel.send(&Request::AnnounceRadioState { session_id: ids.0, file_id: ids.1, liked, track_wave, wave_label }));
}

/// A click on one of the radio widget's icons.
pub fn handle_control(shared: &Shared, ids: (u32, u32), control: RadioControl) {
    let radio = &shared.radio;
    match control {
        RadioControl::ToggleLike => {
            let change = lock(&radio.current).as_mut().map(|track| {
                track.liked = !track.liked;
                (track.id.clone(), track.liked)
            });
            if let Some((track_id, liked)) = change {
                lock(&radio.likes).push((track_id.clone(), liked));
                let _ = Channel::connect().and_then(|channel| channel.send(&Request::RadioLike { service: YANDEX.to_string(), track_id, liked }));
            }
        }
        RadioControl::ToggleTrackWave => {
            let lit = radio.track_wave.load(Ordering::Acquire);
            let playing = lock(&radio.current).as_ref().map(|track| track.id.clone());
            // Lit: back to main. Dim: a branch from the track playing now (nothing to branch from before the first starts).
            let request = match (lit, playing) {
                (true, _) => Some(WaveRequest::Main),
                (false, Some(id)) => Some(WaveRequest::Branch(id)),
                (false, None) => None,
            };
            if let Some(request) = request {
                let branch = matches!(request, WaveRequest::Branch(_));
                radio.track_wave.store(!lit, Ordering::Release);
                *lock(&radio.wave_request) = Some(request);
                if branch {
                    shared.step.store(WAVE_JUMP, Ordering::Release);
                }
            }
        }
    }
    announce_state(shared, ids);
}

impl Channel {
    /// The next tracks of `station` after the track `queue`, and the id of their batch.
    fn radio_tracks(&self, service: &str, station: &str, queue: Option<&str>) -> Result<(String, Vec<RadioTrack>), String> {
        self.send(&Request::RadioTracks {
            service: service.to_string(),
            station: station.to_string(),
            queue: queue.map(str::to_string),
        })?;
        loop {
            let message = self.read_raw()?;
            match serde_json::from_slice::<Response>(&message) {
                Ok(Response::RadioTracks { batch_id, tracks }) => return Ok((batch_id, tracks)),
                Ok(Response::RadioFailed { reason }) => return Err(reason),
                _ => {}
            }
        }
    }
}

/// The next batch of `station` after the track `queue`, as entries.
fn fetch_entries(service: &str, station: &str, queue: Option<&str>) -> Result<Vec<RadioEntry>, String> {
    let (batch_id, tracks) = Channel::connect()?.radio_tracks(service, station, queue).map_err(|reason| format!("{station}: {reason}"))?;
    Ok(tracks.into_iter().map(|track| RadioEntry { track, batch_id: batch_id.clone(), station: station.to_string() }).collect())
}

/// Tells the radio what happened to a track; the radio plays on without feedback if this fails.
fn feedback(service: &str, entry: Option<&RadioEntry>, station: &str, batch_id: &str, event: RadioEvent, played_seconds: f32) {
    let track_id = entry.map(|entry| entry.track.id.clone()).unwrap_or_default();
    let _ = Channel::connect().and_then(|channel| {
        channel.send(&Request::RadioFeedback {
            service: service.to_string(),
            station: station.to_string(),
            batch_id: batch_id.to_string(),
            event,
            track_id,
            played_seconds,
        })
    });
}

/// What a click on the wave icon does to the queue when the next track is about
/// to start (for a branch, the click itself ends the track). The session's own
/// radio (`main_station`) is the main line; the radio of a track is a branch off it:
///
/// - `Branch(from)` makes a branch at the track the click came on: what main had
///   queued after the track playing now (`cursor`) waits in `main_queue`, kept as
///   it is however many branches follow, and the tracks queued from here on are
///   the wave of `from`. A branch made while already in one drops that branch's
///   queue and starts anew from `from`.
/// - `Main`, in a branch, drops the branch: the track playing now plays out and
///   the tracks after it are main's own queue; with none waiting, the next ones
///   are asked of main.
///
/// `fetch(station, after)` gets the next tracks of a station after a track.
/// Returns whether the radio is in a branch afterwards.
pub fn apply_wave_request(
    request: Option<WaveRequest>,
    played: &mut Vec<RadioEntry>,
    cursor: usize,
    in_branch: bool,
    main_queue: &mut Vec<RadioEntry>,
    main_station: &str,
    mut fetch: impl FnMut(&str, &str) -> Result<Vec<RadioEntry>, String>,
) -> Result<bool, String> {
    match request {
        Some(WaveRequest::Branch(from)) => {
            if in_branch {
                played.truncate(cursor + 1);
            } else {
                *main_queue = played.split_off(cursor + 1);
            }
            played.extend(fetch(&format!("track:{from}"), &from)?);
            Ok(true)
        }
        Some(WaveRequest::Main) if in_branch => {
            played.truncate(cursor + 1);
            if main_queue.is_empty() {
                let current_id = played[cursor].track.id.clone();
                played.extend(fetch(main_station, &current_id)?);
            } else {
                played.append(main_queue);
            }
            Ok(false)
        }
        Some(WaveRequest::Main) | None => Ok(in_branch),
    }
}

/// Plays the radio until the viewer goes. Each track is a placement of its own
/// (new ids) played as a URL.
pub fn run(service: &'static str, station: &str, shared: &std::sync::Arc<Shared>) -> Result<(), String> {
    let radio = &shared.radio;
    radio.track_wave.store(false, Ordering::Release);
    *lock(&radio.wave_request) = None;
    *lock(&radio.wave_label) = None;
    let mut played = fetch_entries(service, station, None)?;
    if let Some(first) = played.first() {
        feedback(service, None, &first.station, &first.batch_id, RadioEvent::RadioStarted, 0.0);
    }
    // Whether the tracks coming are those of a branch off the main line, and what main had queued meanwhile.
    let mut in_branch = false;
    let mut main_queue: Vec<RadioEntry> = Vec::new();
    let mut cursor = 0usize;
    let mut failures_in_a_row = 0usize;
    let mut first = true;
    loop {
        if shared.cancelled() {
            return Ok(());
        }
        shared.step.store(0, Ordering::Release);
        // The track after this one is wanted before this one starts: its tags and cover are
        // fetched while this one plays. At the end of the batch the next batch is asked for now.
        if cursor + 1 >= played.len() {
            let (station, last) = (played[cursor].station.clone(), played[cursor].track.id.clone());
            if let Ok(more) = fetch_entries(service, &station, Some(&last)) {
                played.extend(more);
            }
            // not fatal: the radio is asked again when this track ends
        }
        let entry = &played[cursor];
        *lock(&radio.current) =
            Some(Current { id: entry.track.id.clone(), liked: radio.effective_like(&entry.track.id, entry.track.liked) });
        let ids = if first {
            shared.ids()
        } else {
            let ids = new_ids();
            shared.set_ids(ids);
            ids
        };
        first = false;
        // one more than what is known, so the next button never goes away
        shared.set_position(Some((cursor, played.len() + 1)));
        let position = super::play::Position {
            index: cursor,
            len: played.len() + 1,
            title: Some(entry.track.title.clone()),
            cue: None,
            looping: false,
            radio: true,
            current: Some(announced_track(&entry.track)),
            next: played.get(cursor + 1).map(|following| announced_track(&following.track)),
        };
        feedback(service, Some(entry), &entry.station, &entry.batch_id, RadioEvent::TrackStarted, 0.0);
        let started = std::time::Instant::now();
        let result = super::play::play_url(&entry.track.target, &position, ids, shared);
        let step = shared.step.swap(0, Ordering::AcqRel);
        let heard = started.elapsed().as_secs_f32();
        match result {
            Ok(()) => {
                failures_in_a_row = 0;
                // going back, or asking a wave of the track, says nothing bad about it
                match step {
                    0 => feedback(service, Some(entry), &entry.station, &entry.batch_id, RadioEvent::TrackFinished, heard),
                    1 => feedback(service, Some(entry), &entry.station, &entry.batch_id, RadioEvent::Skip, heard),
                    _ => {}
                }
            }
            Err(error) => {
                failures_in_a_row += 1;
                if failures_in_a_row >= MAX_FAILURES_IN_A_ROW {
                    return Err(format!("{}: {error}", entry.track.target));
                }
            }
        }
        if shared.cancelled() {
            return Ok(());
        }
        if step < 0 {
            cursor = cursor.saturating_sub(1);
            continue;
        }
        // the wave icon was clicked: a branch asked for plays at once (the click ended the track),
        // going back to main waits for here, the end of the track that played out
        let request = lock(&radio.wave_request).take();
        let branched_from = match &request {
            Some(WaveRequest::Branch(from)) => Some(from.clone()),
            _ => None,
        };
        in_branch = apply_wave_request(request, &mut played, cursor, in_branch, &mut main_queue, station, |radio, after| {
            fetch_entries(service, radio, Some(after))
        })?;
        // the running line names the track the branch is built from
        let label = match (&branched_from, in_branch) {
            (Some(from), _) => played.iter().find(|entry| entry.track.id == *from).map(|entry| wave_label(&entry.track)),
            (None, true) => lock(&shared.radio.wave_label).clone(),
            (None, false) => None,
        };
        *lock(&shared.radio.wave_label) = label;
        cursor += 1;
        if cursor >= played.len() {
            let last = played[played.len() - 1].track.id.clone();
            let radio_station = played[played.len() - 1].station.clone();
            played.extend(fetch_entries(service, &radio_station, Some(&last))?);
            if cursor >= played.len() {
                return Err(format!("{radio_station}: the radio gave no more tracks"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radio_entry(id: &str, station: &str) -> RadioEntry {
        RadioEntry { track: RadioTrack { id: id.to_string(), title: id.to_string(), ..Default::default() }, batch_id: String::new(), station: station.to_string() }
    }

    fn ids(entries: &[RadioEntry]) -> Vec<String> {
        entries.iter().map(|entry| format!("{}@{}", entry.track.id, entry.station)).collect()
    }

    /// A fetch that answers with two tracks named after what it was asked.
    fn fake_fetch(station: &str, after: &str) -> Result<Vec<RadioEntry>, String> {
        Ok(vec![radio_entry(&format!("{after}-a"), station), radio_entry(&format!("{after}-b"), station)])
    }

    #[test]
    fn a_branch_is_made_at_the_playing_track_and_mains_queue_waits() {
        let mut played = vec![radio_entry("m1", "main"), radio_entry("m2", "main"), radio_entry("m3", "main")];
        let mut main_queue = Vec::new();
        let in_branch = apply_wave_request(Some(WaveRequest::Branch("m1".into())), &mut played, 0, false, &mut main_queue, "main", fake_fetch).unwrap();
        assert!(in_branch);
        assert_eq!(ids(&played), ["m1@main", "m1-a@track:m1", "m1-b@track:m1"]);
        assert_eq!(ids(&main_queue), ["m2@main", "m3@main"]);
    }

    #[test]
    fn back_to_main_plays_the_branchs_track_out_and_goes_on_along_mains_queue() {
        let mut played = vec![radio_entry("m1", "main"), radio_entry("b1", "track:m1"), radio_entry("b2", "track:m1")];
        let mut main_queue = vec![radio_entry("m2", "main"), radio_entry("m3", "main")];
        let in_branch = apply_wave_request(Some(WaveRequest::Main), &mut played, 1, true, &mut main_queue, "main", fake_fetch).unwrap();
        assert!(!in_branch);
        assert_eq!(ids(&played), ["m1@main", "b1@track:m1", "m2@main", "m3@main"]);
        assert!(main_queue.is_empty());
    }

    #[test]
    fn off_and_on_again_within_a_track_branches_anew_from_the_track_heard() {
        let mut played = vec![radio_entry("m1", "main"), radio_entry("b1", "track:m1"), radio_entry("b2", "track:m1")];
        let mut main_queue = vec![radio_entry("m2", "main"), radio_entry("m3", "main")];
        let in_branch = apply_wave_request(Some(WaveRequest::Branch("b1".into())), &mut played, 1, true, &mut main_queue, "main", fake_fetch).unwrap();
        assert!(in_branch);
        assert_eq!(ids(&played), ["m1@main", "b1@track:m1", "b1-a@track:b1", "b1-b@track:b1"]);
        assert_eq!(ids(&main_queue), ["m2@main", "m3@main"]);
    }

    #[test]
    fn back_to_main_with_nothing_waiting_asks_main_for_more() {
        let mut played = vec![radio_entry("m1", "main"), radio_entry("b1", "track:m1")];
        let mut main_queue = Vec::new();
        apply_wave_request(Some(WaveRequest::Main), &mut played, 1, true, &mut main_queue, "main", fake_fetch).unwrap();
        assert_eq!(ids(&played), ["m1@main", "b1@track:m1", "b1-a@main", "b1-b@main"]);
    }

    #[test]
    fn no_click_or_a_stray_back_to_main_changes_nothing() {
        let mut played = vec![radio_entry("m1", "main"), radio_entry("m2", "main")];
        let mut main_queue = Vec::new();
        assert!(!apply_wave_request(None, &mut played, 0, false, &mut main_queue, "main", fake_fetch).unwrap());
        assert!(!apply_wave_request(Some(WaveRequest::Main), &mut played, 0, false, &mut main_queue, "main", fake_fetch).unwrap());
        assert_eq!(ids(&played), ["m1@main", "m2@main"]);
    }

    #[test]
    fn a_branch_is_named_by_its_track_and_year() {
        let mut track = radio_entry("m1", "main").track;
        track.title = "Portishead - Roads".to_string();
        assert_eq!(wave_label(&track), "Portishead - Roads");
        track.year = Some(1994);
        assert_eq!(wave_label(&track), "Portishead - Roads, 1994");
    }

    #[test]
    fn a_like_the_user_chose_outlives_the_batch_it_came_in() {
        let state = State::default();
        assert!(!state.effective_like("t", false));
        lock(&state.likes).push(("t".to_string(), true));
        assert!(state.effective_like("t", false));
        lock(&state.likes).push(("t".to_string(), false));
        assert!(!state.effective_like("t", true), "the latest choice wins over the batch");
        assert!(state.effective_like("other", true), "a track nothing was chosen for keeps what its batch said");
    }

    #[test]
    fn a_playlist_line_naming_yandex_is_my_wave() {
        let entry = |target: &str| Entry { target: target.to_string(), title: None, cue: None };
        let my_wave = Ok(Some(("yandex", "user:onyourwave".to_string())));
        assert_eq!(radio_in_playlist(&[entry("https://music.yandex.ru/")]), my_wave);
        assert_eq!(radio_in_playlist(&[entry("yandex:wave")]), my_wave);
        assert_eq!(radio_in_playlist(&[entry("yandex:track:38526")]), my_wave);
        assert_eq!(radio_in_playlist(&[entry("https://stream.example/a.mp3")]), Ok(None));
        assert!(radio_in_playlist(&[entry("yandex:wave"), entry("https://stream.example/a.mp3")]).is_err());
    }

    #[test]
    fn a_radios_requests_have_the_shape_somsrv_reads() {
        let feedback = Request::RadioFeedback {
            service: "yandex".into(),
            station: "s".into(),
            batch_id: "b".into(),
            event: RadioEvent::TrackStarted,
            track_id: "t".into(),
            played_seconds: 0.0,
        };
        assert_eq!(
            serde_json::to_string(&feedback).unwrap(),
            r#"{"RadioFeedback":{"service":"yandex","station":"s","batch_id":"b","event":"TrackStarted","track_id":"t","played_seconds":0.0}}"#
        );
        let control: super::super::wire::Incoming = serde_json::from_str(r#"{"RadioControl":{"session_id":1,"file_id":2,"control":"ToggleLike"}}"#).unwrap();
        assert!(matches!(control, super::super::wire::Incoming::RadioControl { control: RadioControl::ToggleLike, .. }));
    }
}
