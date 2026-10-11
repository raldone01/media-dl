//! Embed a YouTube video ID into audio files as a custom tag, and read it back.
//!
//! ## Written by [`embed_yt_id_tag`]
//!
//! | Format          | Storage                                                        |
//! |-----------------|----------------------------------------------------------------|
//! | MP3             | ID3v2 `TXXX` frame with description `YTID`                     |
//! |                 | ID3v2 `WXXX` frame with description `YouTube Music` (URL)      |
//! | M4A / MP4 / M4B | `----:com.apple.iTunes:YTID` freeform atom                     |
//! | FLAC / Opus/OGG | `YTID` Vorbis comment                                          |
//!
//! ## Read by [`get_yt_id`]
//!
//! In addition to the fields written above, the parser recognises the keys
//! that yt-dlp writes by default, so IDs can be recovered from files that
//! were tagged by external tools.
//!
//! | Format          | Fields consulted                                                               |
//! |-----------------|--------------------------------------------------------------------------------|
//! | MP3             | `TXXX` `YTID`                                                                  |
//! |                 | `WXXX` `YouTube Music` (URL)                                                   |
//! |                 | `TXXX` `purl` (URL, yt-dlp)                                                    |
//! |                 | comment frame with empty description (URL, yt-dlp)                             |
//! | M4A / MP4 / M4B | `----:com.apple.iTunes:YTID` freeform atom                                     |
//! |                 | `©cmt` comment atom (URL, yt-dlp)                                              |
//! | FLAC / Opus/OGG | `YTID` Vorbis comment                                                          |
//! |                 | `DESCRIPTION` Vorbis comments (URL, yt-dlp)                                    |

use lofty::id3::v2::CommentFrame;
use lofty::id3::v2::ExtendedTextFrame;
use lofty::id3::v2::{ExtendedUrlFrame, Id3v2Tag};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
use lofty::ogg::tag::VorbisComments;
use lofty::tag::Accessor;
use lofty::tag::{Tag, TagType};
use std::borrow::Cow;
use thiserror::Error;
use url::Url;

/// The custom-field key used for the YouTube video ID in every supported format.
pub const YTID_KEY: &str = "YTID";
const YTID_KEY_MP4_ATOM: AtomIdent<'_> = AtomIdent::Freeform {
  mean: Cow::Borrowed("com.apple.iTunes"),
  name: Cow::Borrowed(YTID_KEY),
};
const YTID_KEY_EXTENDED_URL_FRAME_DESCRIPTION: &str = "YouTube Music";

mod embed {
  use super::*;

  #[derive(Debug, Error)]
  pub enum EmbedError {
    #[error("Unsupported primary tag type: {tag_type:?}")]
    UnsupportedTag { tag_type: TagType },
  }

  pub fn embed_yt_id_tag(yt_id: &str) -> impl Fn(Tag) -> Result<Tag, EmbedError> {
    |tag| {
      match tag.tag_type() {
        TagType::Id3v2 => {
          // for id3v2 insert replaces based on id and description
          let mut id3v2: Id3v2Tag = tag.into();
          id3v2.insert_user_text(YTID_KEY.to_string(), yt_id.to_string());

          let mut youtube_music_url =
            Url::parse("https://music.youtube.com/watch").expect("valid constant url");
          youtube_music_url.query_pairs_mut().append_pair("v", yt_id);

          id3v2.insert(
            ExtendedUrlFrame::new(
              lofty::TextEncoding::UTF8,
              YTID_KEY_EXTENDED_URL_FRAME_DESCRIPTION,
              youtube_music_url.to_string(),
            )
            .into(),
          );

          Ok(id3v2.into())
        },
        TagType::Mp4Ilst => {
          let mut mp4: Ilst = tag.into();
          mp4.replace_atom(Atom::new(
            YTID_KEY_MP4_ATOM,
            AtomData::UTF8(yt_id.to_string()),
          ));
          Ok(mp4.into())
        },
        TagType::VorbisComments => {
          // FIXME: This code path is broken until lofty is updated to 0.25.5. Waiting for release.
          let mut vorbis: VorbisComments = tag.into();
          let _ = vorbis.remove(YTID_KEY);
          vorbis.push(YTID_KEY.to_string(), yt_id.to_string());
          Ok(vorbis.into())
        },
        other_tag_type => Err(EmbedError::UnsupportedTag {
          tag_type: other_tag_type,
        }),
      }
    }
  }
}

mod parse {
  use super::*;
  use crate::parse_id_from_yt_url::yt_id_from_url_str;

  /// Vendored from lofty
  fn filter_comment_frame_by_description<'a>(
    frame: &'a lofty::id3::v2::Frame<'_>,
    description: &str,
  ) -> Option<&'a CommentFrame<'a>> {
    match frame {
      lofty::id3::v2::Frame::Comment(comment_frame) => {
        (comment_frame.description == description).then_some(comment_frame)
      },
      _ => None,
    }
  }

  fn filter_user_text_frame_by_description<'a>(
    frame: &'a lofty::id3::v2::Frame<'_>,
    description: &str,
  ) -> Option<&'a ExtendedTextFrame<'a>> {
    match frame {
      lofty::id3::v2::Frame::UserText(user_text_frame) => {
        (user_text_frame.description == description).then_some(user_text_frame)
      },
      _ => None,
    }
  }

  fn filter_extended_url_frame_by_description<'a>(
    frame: &'a lofty::id3::v2::Frame<'_>,
    description: &str,
  ) -> Option<&'a ExtendedUrlFrame<'a>> {
    match frame {
      lofty::id3::v2::Frame::UserUrl(extended_url_frame) => {
        (extended_url_frame.description == description).then_some(extended_url_frame)
      },
      _ => None,
    }
  }

  /// Try to read the YouTube video ID back from `Tag`.
  ///
  /// Returns `None` when the format is unsupported or the field is simply
  /// not present.
  pub fn yt_id_from_tag(tag: &Tag) -> Option<String> {
    let tag = tag.clone();

    match tag.tag_type() {
      TagType::Id3v2 => {
        let id3v2: Id3v2Tag = tag.into();

        for frame in id3v2.iter() {
          // we embed our own keys

          // read our user_text
          if let Some(user_text_frame) = filter_user_text_frame_by_description(frame, YTID_KEY) {
            return Some(user_text_frame.content.to_string());
          }

          // read our url frame
          if let Some(extended_url_frame) =
            filter_extended_url_frame_by_description(frame, YTID_KEY_EXTENDED_URL_FRAME_DESCRIPTION)
          {
            let yt_url_str = &extended_url_frame.content;
            if let Some(yt_id) = yt_id_from_url_str(yt_url_str) {
              return Some(yt_id);
            }
          }

          // yt-dlp embeds the following keys by default

          // yt-dlp embeds the youtube url by default in purl
          if let Some(user_text_frame) = filter_user_text_frame_by_description(frame, "purl") {
            let yt_url_str = &user_text_frame.content;
            if let Some(yt_id) = yt_id_from_url_str(yt_url_str) {
              return Some(yt_id);
            }
          }

          // yt-dlp also embeds the youtube url in a comment
          if let Some(comment_frame) = filter_comment_frame_by_description(frame, "") {
            let yt_url_str = &comment_frame.content;
            if let Some(yt_id) = yt_id_from_url_str(yt_url_str) {
              return Some(yt_id);
            }
          }
        }

        None
      },
      TagType::Mp4Ilst => {
        let mp4: Ilst = tag.into();

        // First try our own YTID freeform atom.
        if let Some(atom) = mp4.get(&YTID_KEY_MP4_ATOM) {
          if let Some(AtomData::UTF8(value)) = atom.data().next() {
            return Some(value.clone());
          }
        }

        // Fall back to yt-dlp's default: it writes the YouTube URL into the comment atom.
        if let Some(comment) = mp4.comment() {
          if let Some(yt_id) = yt_id_from_url_str(&comment) {
            return Some(yt_id);
          }
        }

        None
      },
      TagType::VorbisComments => {
        let vorbis: VorbisComments = tag.into();

        let yt_id_comment = vorbis.get(YTID_KEY).map(str::to_owned);
        if yt_id_comment.is_some() {
          return yt_id_comment;
        }

        // yt-dlp adds a description entry with the url by default
        for yt_url_str in vorbis.get_all("DESCRIPTION") {
          if let Some(yt_id) = yt_id_from_url_str(yt_url_str) {
            return Some(yt_id);
          }
        }

        None
      },
      _ => None,
    }
  }
}

pub use embed::{EmbedError, embed_yt_id_tag};
pub use parse::yt_id_from_tag;
