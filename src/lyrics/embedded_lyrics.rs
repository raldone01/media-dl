//! Embed plain and/or synced (LRC) lyrics into audio files.
//!
//! Many formats lack the distinction between synced and unsynced lyrics and the lyric format.
//! We just store timed LRC lyrics when ever possible for maximum player compatibility.
//! If we only have plain unsynced lyrics we store those instead.
//! For id3 we also do the SYLT format which most players don't use.
//!
//! | Format      | Plain lyrics                | Synced lyrics               |
//! |-------------|-----------------------------|-----------------------------|
//! | MP3         | `USLT` (ID3v2.4)            | `SYLT` (ID3v2.4)            |
//! | M4A/MP4     | `©lyr` atom                 | `©lyr` atom                 |
//! | FLAC        | `UNSYNCEDLYRICS` Vorbis cmt | `LYRICS` Vorbis cmt         |
//! | Opus / OGG  | `UNSYNCEDLYRICS` Vorbis cmt | `LYRICS` Vorbis cmt         |

use crate::lyrics::parse_lrc::parse_lrc;
use anyhow::Result;
use lofty::config::WriteOptions;
use lofty::id3::v2::{BinaryFrame, FrameId, Id3v2Tag, UnsynchronizedTextFrame};
use lofty::id3::v2::{SynchronizedTextFrame, TimestampFormat};
use lofty::tag::{ItemKey, Tag, TagType};
use std::convert::Infallible;

mod embed {
  use super::*;

  #[must_use]
  pub fn embed_lyrics_tag(
    plain_lyrics: Option<&str>,
    synced_lyrics: Option<&str>,
  ) -> impl Fn(Tag) -> Result<Tag, Infallible> {
    move |mut tag| {
      // Choose the richest text we have. Synced wins.
      let maybe_synced_lyrics = match (plain_lyrics, synced_lyrics) {
        (_, Some(lrc)) => lrc,
        (Some(plain), None) => plain,
        (None, None) => return Ok(tag),
      };

      if let Some(plain_lyrics) = plain_lyrics {
        tag.insert_text(ItemKey::UnsyncLyrics, plain_lyrics.to_string());
      }
      tag.insert_text(ItemKey::Lyrics, maybe_synced_lyrics.to_string());

      match tag.tag_type() {
        TagType::Id3v2 => {
          // MP3 - ID3v2.4 USLT (plain) + SYLT (synced)
          // for id3v2 insert replaces based on id and description
          let mut id3v2: Id3v2Tag = tag.into();

          if let Some(text) = plain_lyrics {
            id3v2.insert(
              UnsynchronizedTextFrame::new(
                lofty::TextEncoding::UTF8,
                *b"eng",
                "",
                text.to_string(),
              )
              .into(),
            );
          }

          if let Some(lrc) = synced_lyrics {
            // Because lofty doesn't support SYLT we must reach for the id3 crate and do it ourselves.
            let lines = parse_lrc(lrc);

            let sync_text_frame = SynchronizedTextFrame::new(
              lofty::TextEncoding::UTF8,
              *b"eng",
              TimestampFormat::MS,
              lofty::id3::v2::SyncTextContentType::Lyrics,
              None,
              lines,
            );
            id3v2.insert(
              BinaryFrame::new(
                FrameId::new("SYLT").expect("valid constant frame id"),
                sync_text_frame
                  .as_bytes(WriteOptions::default())
                  .expect("valid sync text frame"),
              )
              .into(),
            );
          }

          Ok(id3v2.into())
        },
        _ => {
          // other formats are handled correctly by lofty itself
          Ok(tag)
        },
      }
    }
  }
}

mod check_lyrics_presence {
  use super::*;

  #[derive(Debug, Clone)]
  pub struct LyricsPresence {
    pub has_unsynced_lyrics: bool,
    pub has_synced_lyrics: bool,
  }

  impl LyricsPresence {
    #[must_use]
    pub fn from_tag(tag: &Tag) -> Self {
      let unsynced_lyrics = tag.get_string(ItemKey::UnsyncLyrics);
      let has_unsynced_lyrics = unsynced_lyrics.is_some_and(|s| !s.trim().is_empty());

      let maybe_synced_lyrics = tag.get_string(ItemKey::Lyrics);
      let mut has_synced_lyrics = maybe_synced_lyrics
        .map(|maybe_synced_lyrics| {
          Self::classify_lyrics_text(maybe_synced_lyrics).has_synced_lyrics
        })
        .unwrap_or(false);

      if tag.tag_type() == TagType::Id3v2 {
        let id3v2: Id3v2Tag = tag.clone().into();
        has_synced_lyrics |= id3v2
          .get(&FrameId::new("SYLT").expect("constant valid frame id"))
          .is_some();
      }

      Self {
        has_unsynced_lyrics,
        has_synced_lyrics,
      }
    }

    /// Shared heuristic for formats that only have a single lyrics field:
    /// if the text contains at least one LRC timestamp line, treat it as
    /// synced lyrics; otherwise as plain lyrics.
    #[must_use]
    fn classify_lyrics_text(text: &str) -> Self {
      if !text.trim().is_empty() {
        let synced = !parse_lrc(text).is_empty();
        return LyricsPresence {
          has_unsynced_lyrics: !synced,
          has_synced_lyrics: synced,
        };
      }

      Self {
        has_synced_lyrics: false,
        has_unsynced_lyrics: false,
      }
    }
  }
}

pub use check_lyrics_presence::LyricsPresence;
pub use embed::embed_lyrics_tag;
