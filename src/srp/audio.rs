//! What Som's audio widget is told of a file before the first byte goes out:
//! the PCM shape and the real length (read from the headers with symphonia,
//! nothing is decoded; decoding and playing happen in Som, which is where the
//! speakers are) and the tags. The same as `somsrp` does.

use std::path::Path;

use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataRevision, StandardTagKey};
use symphonia::core::probe::{Hint, ProbeResult};

use super::wire::{ContentMetadata, TrackTags};

fn probe(path: &Path) -> Result<ProbeResult, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    symphonia::default::get_probe()
        .format(&hint, stream, &Default::default(), &Default::default())
        .map_err(|e| format!("probing {}: {e}", path.display()))
}

/// The sample rate, channels, bits and length of the first decodable track.
pub fn metadata(path: &Path) -> Result<ContentMetadata, String> {
    let probed = probe(path)?;
    let track = probed
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "no decodable audio track".to_string())?;
    let params = &track.codec_params;
    let sample_rate = params.sample_rate.ok_or("unknown sample rate")?;
    let channels = params.channels.map(|c| c.count() as u8).ok_or("unknown channel count")?;
    let bits_per_sample = params.bits_per_sample.map(|b| b as u8).unwrap_or(16);
    let duration_ms = match (params.n_frames, params.time_base) {
        (Some(frames), Some(base)) => {
            let time = base.calc_time(frames);
            (time.seconds.saturating_mul(1000) as u32).saturating_add((time.frac * 1000.0) as u32)
        }
        _ => 0,
    };
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
    Ok(ContentMetadata::Audio { sample_rate, channels, bits_per_sample, duration_ms, extension })
}

/// The title, artist, album and year in the file's tags; a title missing from
/// them is the file's name. `None` if the file cannot be read at all.
pub fn tags(path: &Path) -> Option<TrackTags> {
    let mut probed = probe(path).ok()?;
    let mut tags = TrackTags::default();
    let mut album_artist = String::new();
    let mut absorb = |revision: &MetadataRevision| {
        for tag in revision.tags() {
            let value = tag.value.to_string();
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if tags.title.is_empty() => tags.title = value.to_string(),
                Some(StandardTagKey::Artist) if tags.artist.is_empty() => tags.artist = value.to_string(),
                Some(StandardTagKey::AlbumArtist) if album_artist.is_empty() => album_artist = value.to_string(),
                Some(StandardTagKey::Album) if tags.album.is_empty() => tags.album = value.to_string(),
                Some(StandardTagKey::Date | StandardTagKey::OriginalDate | StandardTagKey::ReleaseDate) if tags.year.is_none() => {
                    tags.year = value.get(..4).and_then(|year| year.parse().ok());
                }
                _ => {}
            }
        }
    };
    // ID3 in an MP3 is found by the probe itself, Vorbis comments by the format reader
    if let Some(metadata) = probed.metadata.get()
        && let Some(revision) = metadata.current()
    {
        absorb(revision);
    }
    if let Some(revision) = probed.format.metadata().current() {
        absorb(revision);
    }
    if tags.artist.is_empty() {
        tags.artist = album_artist;
    }
    if tags.title.is_empty() {
        tags.title = path.file_stem()?.to_string_lossy().into_owned();
    }
    Some(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real half-second 8 kHz mono FLAC made by ffmpeg, with title and artist tags.
    const TONE: &[u8] = include_bytes!("tone.flac");

    fn tone() -> std::path::PathBuf {
        // two tests run at once and each removes its file
        static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("farsh-audio-{}-{n}-tone.flac", std::process::id()));
        std::fs::write(&path, TONE).unwrap();
        path
    }

    #[test]
    fn a_flac_gives_its_shape_and_length() {
        let path = tone();
        let result = metadata(&path);
        std::fs::remove_file(&path).unwrap();
        match result {
            Ok(ContentMetadata::Audio { sample_rate: 8000, channels: 1, duration_ms, extension, .. }) => {
                assert!((450..=550).contains(&duration_ms), "{duration_ms}");
                assert_eq!(extension, "flac");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_tags_are_read_and_a_missing_title_is_the_file_name() {
        let path = tone();
        let found = tags(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!((found.title.as_str(), found.artist.as_str()), ("Tone", "Farsh"));
    }
}