//! Embed plain and/or synced (LRC) lyrics into audio files.
//!
//! | Format      | Plain lyrics                | Synced lyrics               |
//! |-------------|-----------------------------|-----------------------------|
//! | MP3         | `USLT` (ID3v2.4)            | `SYLT` (ID3v2.4)            |
//! | M4A/MP4     | `©lyr` atom                 | `©lyr` atom                 |
//! | FLAC        | `UNSYNCEDLYRICS` Vorbis cmt | `LYRICS` Vorbis cmt         |
//! | Opus / OGG  | `UNSYNCEDLYRICS` Vorbis cmt | `LYRICS` Vorbis cmt         |
//! | WAV         | not supported               | not supported               |

use anyhow::{Context, Result, bail};
use std::path::Path;

use id3::TagLike;
use id3::frame::{
  Lyrics as Id3Lyrics, SynchronisedLyrics, SynchronisedLyricsType, TimestampFormat,
};

use lofty::config::WriteOptions;
use lofty::file::{TaggedFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

/// Outcome of an embedding request. Anything an audio format simply does not
/// support is reported *here* rather than as an error, because it is a
/// successful, well-understood outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedResult {
  /// All requested lyrics were embedded.
  Embedded,
  /// The format cannot hold any of the requested lyrics.
  Unsupported { reason: &'static str },
}

/// Embed lyrics into `file`. The format is inferred from the file extension:
/// `mp3`, `m4a`/`mp4`/`m4b`/`aac`, `flac`, `opus`/`ogg`, `wav`.
pub fn embed_lyrics(
  file: &Path,
  plain_lyrics: Option<&str>,
  synced_lyrics: Option<&str>,
) -> Result<EmbedResult> {
  if plain_lyrics.is_none() && synced_lyrics.is_none() {
    return Ok(EmbedResult::Embedded);
  }

  let ext = file
    .extension()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .context("audio file has no extension")?;

  match ext.as_str() {
    "mp3" => embed_mp3(file, plain_lyrics, synced_lyrics),
    "m4a" | "mp4" | "m4b" | "aac" => embed_mp4(file, plain_lyrics, synced_lyrics),
    "flac" | "opus" | "ogg" => embed_vorbis(file, plain_lyrics, synced_lyrics),
    "wav" => Ok(EmbedResult::Unsupported {
      reason: "WAV files do not have a widely-supported lyrics container",
    }),
    _ => Ok(EmbedResult::Unsupported {
      reason: "this audio format does not support embedded lyrics",
    }),
  }
}

/// MP3 — ID3v2.4 USLT (plain) + SYLT (synced)
fn embed_mp3(
  file: &Path,
  plain_lyrics: Option<&str>,
  synced_lyrics: Option<&str>,
) -> Result<EmbedResult> {
  // Read existing tag if present; otherwise start a fresh one.
  let mut tag = id3::Tag::read_from_path(file).unwrap_or_else(|_| id3::Tag::new());

  if let Some(text) = plain_lyrics {
    tag.remove("USLT");
    tag.add_frame(Id3Lyrics {
      lang: "eng".to_string(),
      description: String::new(),
      text: text.to_string(),
    });
  }

  if let Some(lrc) = synced_lyrics {
    let lines = parse_lrc(lrc);
    if lines.is_empty() {
      bail!("synced lyrics did not contain any valid LRC timestamp lines");
    }
    tag.remove("SYLT");
    let synced_lyrics_frame = SynchronisedLyrics {
      lang: "eng".to_string(),
      // yt-dlp / MusicBrainz style timestamps are in ms.
      timestamp_format: TimestampFormat::Ms,
      content_type: SynchronisedLyricsType::Lyrics,
      description: "Lyrics".to_string(),
      content: lines,
    };
    tag.add_frame(synced_lyrics_frame);
  }

  tag
    .write_to_path(file, id3::Version::Id3v24)
    .with_context(|| format!("failed to write ID3 tag to {}", file.display()))?;

  Ok(EmbedResult::Embedded)
}

/// MP4 / M4A — the standard `©lyr` atom.
///
/// ISO-BMFF has no dedicated atom for timed lyrics. The de-facto convention
/// (Picard, beets, MusicBee, foobar2000, …) is to store the LRC text verbatim
/// in `©lyr` and let the player parse the timestamps. Plain lyrics use the
/// same atom, so when both are supplied we prefer the synced version because
/// it already contains the line text.
fn embed_mp4(
  file: &Path,
  plain_lyrics: Option<&str>,
  synced_lyrics: Option<&str>,
) -> Result<EmbedResult> {
  // Choose the richest text we have. Synced wins.
  let text = match (plain_lyrics, synced_lyrics) {
    (_, Some(lrc)) => lrc,
    (Some(plain), None) => plain,
    (None, None) => return Ok(EmbedResult::Embedded),
  };

  let mut tagged = open_and_ensure_tag(file, TagType::Mp4Ilst)?;
  {
    let tag = tagged
      .primary_tag_mut()
      .context("MP4 file has no tag after initialisation")?;
    tag.remove_key(ItemKey::Lyrics);
    tag.insert_text(ItemKey::Lyrics, text.to_string());
  }
  save_tagged(file, &mut tagged)?;

  Ok(EmbedResult::Embedded)
}

/// FLAC / Opus / OGG - Vorbis comments
fn embed_vorbis(
  file: &Path,
  plain_lyrics: Option<&str>,
  synced_lyrics: Option<&str>,
) -> Result<EmbedResult> {
  let mut tagged = open_and_ensure_tag(file, TagType::VorbisComments)?;
  {
    let tag = tagged
      .primary_tag_mut()
      .context("Vorbis comment file has no tag after initialisation")?;

    if let Some(text) = plain_lyrics {
      tag.remove_key(ItemKey::UnsyncLyrics);
      tag.insert_text(ItemKey::UnsyncLyrics, text.to_string());
      tag.remove_key(ItemKey::Lyrics);
      tag.insert_text(ItemKey::Lyrics, text.to_string());
    }

    if let Some(lrc) = synced_lyrics {
      // LRC is stored as-is; players that understand LYRICS will
      // parse the timestamps themselves.
      tag.remove_key(ItemKey::Lyrics);
      tag.insert_text(ItemKey::Lyrics, lrc.to_string());
    }
  }
  save_tagged(file, &mut tagged)?;
  Ok(EmbedResult::Embedded)
}

fn open_and_ensure_tag(file: &Path, tag_type: TagType) -> Result<TaggedFile> {
  let mut tagged = Probe::open(file)
    .with_context(|| format!("failed to open {}", file.display()))?
    .read()
    .with_context(|| format!("failed to read tags from {}", file.display()))?;

  if tagged.primary_tag_mut().is_none() {
    tagged.insert_tag(Tag::new(tag_type));
  }
  Ok(tagged)
}

fn save_tagged(file: &Path, tagged: &mut TaggedFile) -> Result<()> {
  tagged
    .save_to_path(file, WriteOptions::default())
    .with_context(|| format!("failed to write tags to {}", file.display()))
}

/// Parse LRC text into `(timestamp_ms, text)` pairs, sorted by time.
///
/// Handles the common variants: `[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]`,
/// `[mm:ss:xx]`, and multiple timestamps on the same line
/// (e.g. `[00:01.00][00:10.00]Chorus`). Unknown metadata lines such as
/// `[ar:Artist]` are skipped.
fn parse_lrc(lrc: &str) -> Vec<(u32, String)> {
  use regex::Regex;
  // The whole-tag regex, anchored by requiring consecutive tags at the start.
  let re = Regex::new(r"\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]").unwrap();

  let mut entries: Vec<(u32, String)> = Vec::new();

  for line in lrc.lines() {
    let mut last_end = 0usize;
    let mut timestamps: Vec<u32> = Vec::new();

    for cap in re.captures_iter(line) {
      let whole = cap.get(0).unwrap();
      // Timestamps must be consecutive from the start of the line;
      // anything else is treated as text (or a metadata tag).
      if whole.start() != last_end {
        break;
      }
      last_end = whole.end();

      let minutes: u32 = cap[1].parse().unwrap_or(0);
      let seconds: u32 = cap[2].parse().unwrap_or(0);
      let millis: u32 = match cap.get(3) {
        Some(g) => {
          let s = g.as_str();
          let n: u32 = s.parse().unwrap_or(0);
          // Normalize 1/2/3-digit fractions to milliseconds.
          match s.len() {
            1 => n * 100,
            2 => n * 10,
            _ => n,
          }
        },
        None => 0,
      };

      timestamps.push(minutes * 60_000 + seconds * 1_000 + millis);
    }

    if timestamps.is_empty() {
      // Metadata like `[ar:...]`, `[offset:...]`, or blank line.
      continue;
    }

    let text = line[last_end..].trim().to_string();
    for ts in timestamps {
      entries.push((ts, text.clone()));
    }
  }

  entries.sort_by_key(|(ts, _)| *ts);
  entries
}
