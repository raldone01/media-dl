//! Embed plain and/or synced (LRC) lyrics into audio files.
//!
//! | Format      | Plain lyrics                | Synced lyrics               |
//! |-------------|-----------------------------|-----------------------------|
//! | MP3         | `USLT` (ID3v2.4)            | `SYLT` (ID3v2.4)            |
//! | M4A/MP4     | `©lyr` atom                 | `©lyr` atom                 |
//! | FLAC        | `UNSYNCEDLYRICS` Vorbis cmt | `LYRICS` Vorbis cmt         |
//! | Opus / OGG  | `UNSYNCEDLYRICS` Vorbis cmt | `LYRICS` Vorbis cmt         |

// TODO: rewrite to make use of infra and patterns established in embedded_yt_id

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

use crate::utils::file_ext;

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
/// `mp3`, `m4a`/`mp4`/`m4b`/`aac`, `flac`, `opus`/`ogg`.
pub fn embed_lyrics(
  file: &Path,
  plain_lyrics: Option<&str>,
  synced_lyrics: Option<&str>,
) -> Result<EmbedResult> {
  if plain_lyrics.is_none() && synced_lyrics.is_none() {
    return Ok(EmbedResult::Embedded);
  }

  let ext = file_ext(file).context("Audio file has no extension")?;

  match ext.as_str() {
    "mp3" => embed_mp3(file, plain_lyrics, synced_lyrics),
    "m4a" | "mp4" | "m4b" | "aac" => embed_mp4(file, plain_lyrics, synced_lyrics),
    "flac" | "opus" | "ogg" => embed_vorbis(file, plain_lyrics, synced_lyrics),
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
#[must_use]
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

/// Presence of the two kinds of lyrics in an audio file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LyricsPresence {
  pub has_unsynced_lyrics: bool,
  pub has_synced_lyrics: bool,
}

/// Inspect `file` and report which kinds of lyrics are embedded.
///
/// The format is inferred from the extension, matching [`embed_lyrics`]:
///
/// * MP3 — `USLT` ⇒ unsynced, `SYLT` ⇒ synced (both can coexist).
/// * M4A/MP4 — the single `©lyr` atom is classified by inspecting its
///   content: if the text parses as LRC it counts as *synced*, otherwise
///   as *unsynced*.
/// * FLAC/Opus/OGG — `UNSYNCEDLYRICS` ⇒ unsynced; `LYRICS` is classified
///   by content (LRC ⇒ synced, otherwise unsynced).
/// * anything else — both flags are `false`.
pub fn detect_lyrics(file: &Path) -> Result<LyricsPresence> {
  let ext = file_ext(file).context("Audio file has no extension")?;

  match ext.as_str() {
    "mp3" => detect_lyrics_mp3(file),
    "m4a" | "mp4" | "m4b" | "aac" => detect_lyrics_mp4(file),
    "flac" | "opus" | "ogg" => detect_lyrics_vorbis(file),
    // WAV / unknown: no lyrics containers are ever written, so nothing
    // to detect.
    _ => Ok(LyricsPresence::default()),
  }
}

/// MP3: `USLT` and `SYLT` are distinct frames, so this is exact.
fn detect_lyrics_mp3(file: &Path) -> Result<LyricsPresence> {
  let tag = id3::Tag::read_from_path(file).unwrap_or_else(|_| id3::Tag::new());
  Ok(LyricsPresence {
    has_unsynced_lyrics: tag.get("USLT").is_some(),
    has_synced_lyrics: tag.get("SYLT").is_some(),
  })
}

/// MP4 / M4A: only `©lyr` exists. Classify by content.
fn detect_lyrics_mp4(file: &Path) -> Result<LyricsPresence> {
  let tagged = Probe::open(file)
    .with_context(|| format!("failed to open {}", file.display()))?
    .read()
    .with_context(|| format!("failed to read tags from {}", file.display()))?;

  let text = tagged
    .primary_tag()
    .and_then(|t| t.get_string(ItemKey::Lyrics));

  Ok(classify_lyrics_text(text))
}

/// FLAC / Opus / OGG: `UNSYNCEDLYRICS` is explicitly unsynced; `LYRICS`
/// is classified by content (matches what `embed_vorbis` writes).
fn detect_lyrics_vorbis(file: &Path) -> Result<LyricsPresence> {
  let tagged = Probe::open(file)
    .with_context(|| format!("failed to open {}", file.display()))?
    .read()
    .with_context(|| format!("failed to read tags from {}", file.display()))?;

  let tag = match tagged.primary_tag() {
    Some(t) => t,
    None => return Ok(LyricsPresence::default()),
  };

  let unsynced = tag
    .get_string(ItemKey::UnsyncLyrics)
    .filter(|s| !s.trim().is_empty());
  let lyrics = tag
    .get_string(ItemKey::Lyrics)
    .filter(|s| !s.trim().is_empty());

  let lyrics_is_synced = lyrics.map(|s| !parse_lrc(s).is_empty()).unwrap_or(false);

  Ok(LyricsPresence {
    has_unsynced_lyrics: unsynced.is_some() || (lyrics.is_some() && !lyrics_is_synced),
    has_synced_lyrics: lyrics_is_synced,
  })
}

/// Shared heuristic for formats that only have a single lyrics field:
/// if the text contains at least one LRC timestamp line, treat it as
/// synced lyrics; otherwise as plain lyrics.
#[must_use]
fn classify_lyrics_text(text: Option<&str>) -> LyricsPresence {
  match text {
    Some(s) if !s.trim().is_empty() => {
      let synced = !parse_lrc(s).is_empty();
      LyricsPresence {
        has_unsynced_lyrics: !synced,
        has_synced_lyrics: synced,
      }
    },
    _ => LyricsPresence::default(),
  }
}
