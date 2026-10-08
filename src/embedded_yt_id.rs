//! Embed a YouTube video ID into audio files as a custom tag, and read it back.
//!
//! Since `ItemKey::Unknown` was removed from lofty, custom fields have to go
//! through the format-specific tag types:
//!
//! | Format          | Storage                                        |
//! |-----------------|------------------------------------------------|
//! | MP3             | ID3v2 `TXXX` frame with description `YTID`     |
//! | M4A / MP4 / M4B | `----:com.apple.iTunes:YTID` freeform atom     |
//! | FLAC / Opus/OGG | `YTID` Vorbis comment                          |
//! | WAV             | not supported                                  |

use anyhow::{Context, Result, bail};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
use lofty::ogg::tag::VorbisComments;
use std::borrow::Cow;
use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{TaggedFile, TaggedFileExt};
use lofty::id3::v2::Id3v2Tag;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};

/// The custom-field key used for the YouTube video ID in every supported format.
pub const YT_ID_KEY: &str = "YTID";
const MP4_ATOM: AtomIdent<'_> = AtomIdent::Freeform {
  mean: Cow::Borrowed("com.apple.iTunes"),
  name: Cow::Borrowed(YT_ID_KEY),
};

/// Outcome of an embedding request. Formats that simply cannot store the field
/// are reported here rather than as an error, because that is a successful,
/// well-understood outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedResult {
  /// The ID was embedded.
  Embedded,
  /// The format has no place to store a custom tag.
  Unsupported { reason: &'static str },
}

/// Embed `yt_id` as a custom field into `file`.
///
/// The format is inferred from the file extension:
/// `mp3`, `m4a`/`mp4`/`m4b`/`aac`, `flac`, `opus`/`ogg`, `wav`.
pub fn embed_yt_id(file: &Path, yt_id: &str) -> Result<EmbedResult> {
  let yt_id = yt_id.trim();
  if yt_id.is_empty() {
    bail!("YouTube ID must not be empty");
  }

  let tag_type = match file_ext(file)?.as_str() {
    "mp3" => TagType::Id3v2,
    "m4a" | "mp4" | "m4b" | "aac" => TagType::Mp4Ilst,
    "flac" | "opus" | "ogg" => TagType::VorbisComments,
    "wav" => {
      return Ok(EmbedResult::Unsupported {
        reason: "WAV files do not have a widely-supported custom-tag container",
      });
    },
    _ => {
      return Ok(EmbedResult::Unsupported {
        reason: "this audio format does not support custom tags",
      });
    },
  };

  let mut tagged = open_and_ensure_tag(file, tag_type)?;

  // Take the primary tag out so we can round-trip it through the
  // format-specific type to add the custom field.
  let tag = tagged
    .primary_tag()
    .context("file has no primary tag after initialisation")?;
  let updated = add_custom_tag(tag.clone(), yt_id)?;
  tagged.primary_tag().replace(&updated);

  save_tagged(file, &mut tagged)?;
  Ok(EmbedResult::Embedded)
}

/// Try to read the YouTube video ID back from `file`.
///
/// Returns `Ok(None)` when the format is unsupported or the field is simply
/// not present.
pub fn get_yt_id(file: &Path) -> Result<Option<String>> {
  match file_ext(file)?.as_str() {
    "mp3" | "m4a" | "mp4" | "m4b" | "aac" | "flac" | "opus" | "ogg" => {},
    _ => return Ok(None),
  }

  let tagged = Probe::open(file)
    .with_context(|| format!("failed to open {}", file.display()))?
    .read()
    .with_context(|| format!("failed to read tags from {}", file.display()))?;

  let Some(tag) = tagged.primary_tag() else {
    return Ok(None);
  };

  read_custom_tag(tag.clone())
}

// --- format-specific plumbing ------------------------------------------------

fn add_custom_tag(tag: Tag, yt_id: &str) -> Result<Tag> {
  match tag.tag_type() {
    TagType::Id3v2 => {
      let mut id3v2: Id3v2Tag = tag.into();
      // Replace any previous value with the same description.
      id3v2.remove_user_text(YT_ID_KEY);
      id3v2.insert_user_text(YT_ID_KEY.to_string(), yt_id.to_string());
      Ok(id3v2.into())
    },
    TagType::Mp4Ilst => {
      let mut mp4: Ilst = tag.into();
      let _ = mp4.remove(&MP4_ATOM);
      mp4.insert(Atom::new(MP4_ATOM, AtomData::UTF8(yt_id.to_string())));
      Ok(mp4.into())
    },
    TagType::VorbisComments => {
      let mut vorbis: VorbisComments = tag.into();
      let _ = vorbis.remove(YT_ID_KEY);
      vorbis.push(YT_ID_KEY.to_string(), yt_id.to_string());
      Ok(vorbis.into())
    },
    other => bail!("unsupported tag type for custom fields: {other:?}"),
  }
}

fn read_custom_tag(tag: Tag) -> Result<Option<String>> {
  match tag.tag_type() {
    TagType::Id3v2 => {
      let id3v2: Id3v2Tag = tag.into();
      Ok(id3v2.get_user_text(YT_ID_KEY).map(str::to_owned))
    },
    TagType::Mp4Ilst => {
      let mp4: Ilst = tag.into();
      let Some(atom) = mp4.get(&MP4_ATOM) else {
        return Ok(None);
      };

      let Some(data) = atom.data().next() else {
        return Ok(None);
      };

      match data {
        AtomData::UTF8(value) => Ok(Some(value.clone())),
        _ => bail!("Unexpected atom data type"),
      }
    },
    TagType::VorbisComments => {
      let vorbis: VorbisComments = tag.into();
      Ok(vorbis.get(YT_ID_KEY).map(str::to_owned))
    },
    _ => Ok(None),
  }
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

fn file_ext(file: &Path) -> Result<String> {
  Ok(
    file
      .extension()
      .and_then(|s| s.to_str())
      .map(str::to_ascii_lowercase)
      .context("audio file has no extension")?,
  )
}
