//! m3u8 ser/des.
//!
//! ```m3u8
//! #EXTM3U
//! # Indicates this local playlist tracks a remote playlist.
//! #EXTPLAYLIST_REMOTE <URL>
//! # Indicates that remote download should be skipped.
//! # Can also be used to mark playlists that intentionally don't track a remote playlist
//! #EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD
//! song1.mp3
//! # some comment
//! song2.mp3
//! # Indicates that the following media entry is a local addition to the playlist.
//! #EXTMEDIA_LOCAL_ADDITION
//! only_part_of_the_playlist_locally.mp3
//! # Indicates that remote download should be skipped.
//! #EXTMEDIA_REMOTE_SKIP_DOWNLOAD
//! local_only_song3.opus
//! # Indicates what remote media this media entry tracks.
//! # If the local file is unavailable the url will be tries.
//! #EXTMEDIA_REMOTE <URL>
//! # Indicates which format the media should be downloaded as.
//! #EXTMEDIA_REMOTE_FORMAT <FORMAT>
//! # Indicates that this local addition has not been confirmed by the user
//! #EXTMEDIA_LOCAL_ADDITION UNCONFIRMED
//! song4.opus
//! ```
//!
//! TODO: make sure repeated files are handled properly

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::bail;
use thiserror::Error;
use url::{ParseError, Url};

use crate::absolute_file_path::AbsoluteFilePath;
use crate::resource_id::ResourceId;

pub enum MediaSource {
  Url(Url),
  File(AbsoluteFilePath),
}

impl MediaSource {
  fn parse(line: &str, playlist_dir: &Path) -> Result<Self, M3U8PlaylistParseError> {
    match Url::parse(line) {
      Ok(url) if url.scheme().len() > 1 || line.contains("://") => Ok(Self::Url(url)),
      _ => {
        let media_path = PathBuf::from(line);
        let absolute_path = playlist_dir.join(media_path);
        Ok(Self::File(
          AbsoluteFilePath::new(absolute_path).expect("We made sure the path is absolute."),
        ))
      },
    }
  }
}

#[derive(Default)]
pub struct M3U8Playlist {
  pub playlist_meta: Vec<PlaylistMeta>,
  pub entries: Vec<MediaEntry>,
}

pub enum PlaylistMeta {
  Comment(String),
  PlaylistExt(ExtPlaylistComment),
}

pub struct MediaEntry {
  pub meta: Vec<MediaMeta>,
  pub media_source: MediaSource,
}

impl MediaEntry {
  #[must_use]
  pub fn resource_id(&self) -> ResourceId {
    let resource_id = self
      .remote_resource_id()
      .unwrap_or_else(|| match self.media_source {
        MediaSource::Url(ref url) => ResourceId::from(url),
        MediaSource::File(ref path) => ResourceId::LocalFile(path.to_owned()),
      });
    resource_id
  }

  pub fn remove_locally_unconfirmed(&mut self) {
    self.meta.retain(|item| match item {
      MediaMeta::ExtComment(ExtMediaComment::LocalAdditionMarker { confirmed: false }) => false,
      _ => true,
    });
  }

  pub fn set_remote_resource_id(&mut self, resource_id: &ResourceId) {
    let url = resource_id.to_url();

    if let Some(MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(existing))) =
      self.meta.iter_mut().find(|item| {
        matches!(
          item,
          MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(..))
        )
      })
    {
      *existing = url;
    } else {
      self
        .meta
        .push(MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(url)));
    }
  }

  #[must_use]
  pub fn remote_resource_id(&self) -> Option<ResourceId> {
    self.meta.iter().find_map(|item| match item {
      MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(url)) => Some(ResourceId::from(url)),
      _ => None,
    })
  }
}

pub enum MediaMeta {
  Comment(String),
  ExtComment(ExtMediaComment),
}

/// playlist-level #EXT...
pub enum ExtPlaylistComment {
  EXTM3UMarker,
  RemotePlaylist(Url),
  SkipRemoteDownloadPlaylist,
  UnknownExtComment(String),
}

impl ExtPlaylistComment {
  fn serialize(&self, out: &mut impl Write) -> io::Result<()> {
    match self {
      ExtPlaylistComment::EXTM3UMarker => {
        // ignore additional stray markers
        // we always add one at the start
      },
      ExtPlaylistComment::RemotePlaylist(url) => {
        writeln!(out, "#EXTPLAYLIST_REMOTE {}", url.as_str())?;
      },
      ExtPlaylistComment::SkipRemoteDownloadPlaylist => {
        writeln!(out, "#EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD")?;
      },
      ExtPlaylistComment::UnknownExtComment(c) => {
        writeln!(out, "#{c}")?;
      },
    }
    Ok(())
  }
}

/// media-level #EXT...
pub enum ExtMediaComment {
  LocalAdditionMarker { confirmed: bool },
  SkipRemoteDownloadMarker,
  RemoteMedia(Url),
  RemoteFormat(String),
  UnknownExtComment(String),
}

impl ExtMediaComment {
  fn serialize(&self, out: &mut impl Write) -> io::Result<()> {
    match self {
      Self::LocalAdditionMarker { confirmed } => {
        write!(out, "#EXTMEDIA_LOCAL_ADDITION")?;
        if !confirmed {
          write!(out, " UNCONFIRMED")?;
        }
        writeln!(out, "")?;
      },
      Self::SkipRemoteDownloadMarker => {
        writeln!(out, "#EXTMEDIA_REMOTE_SKIP_DOWNLOAD")?;
      },
      Self::RemoteMedia(url) => {
        writeln!(out, "#EXTMEDIA_REMOTE {}", url.as_str())?;
      },
      Self::RemoteFormat(format) => {
        writeln!(out, "#EXTMEDIA_REMOTE_FORMAT {format}")?;
      },
      Self::UnknownExtComment(c) => {
        writeln!(out, "#{c}")?;
      },
    }
    Ok(())
  }

  /// Parse a media-level `#EXTMEDIA_*` line.
  ///
  /// Returns `Ok(Some(_))` only for tags with the `#EXTMEDIA_` prefix.
  /// Other `#EXT…` tags return `Ok(None)`.
  ///
  /// Unknown `#EXTMEDIA_*` tags are captured as
  /// [`ExtMediaComment::UnknownExtComment`] so they round-trip untouched.
  fn parse(line: &str) -> Result<Option<Self>, M3U8PlaylistParseError> {
    if let Some(rest) = line.strip_prefix("#EXTMEDIA_LOCAL_ADDITION") {
      if rest.chars().next().map_or(false, char::is_whitespace) {
        match rest.trim() {
          "" => return Ok(Some(Self::LocalAdditionMarker { confirmed: true })),
          "UNCONFIRMED" => return Ok(Some(Self::LocalAdditionMarker { confirmed: false })),
          other => {
            return Err(M3U8PlaylistParseError::InvalidLocalAddition {
              value: other.to_string(),
            });
          },
        }
      }
    }

    if line.trim_end() == "#EXTMEDIA_REMOTE_SKIP_DOWNLOAD" {
      return Ok(Some(Self::SkipRemoteDownloadMarker));
    }

    if let Some(rest) = line.strip_prefix("#EXTMEDIA_REMOTE") {
      // Same whitespace guard as the playlist-level `#EXTPLAYLIST_REMOTE`
      // parsing above, so sibling `#EXTMEDIA_REMOTE_*` tags aren't
      // mistaken for a `#EXTMEDIA_REMOTE <url>` payload.
      if rest.chars().next().map_or(true, char::is_whitespace) {
        let url = parse_url_from_rest(rest, "#EXTMEDIA_REMOTE")?;
        return Ok(Some(Self::RemoteMedia(url)));
      }
    }

    if let Some(rest) = line.strip_prefix("#EXTMEDIA_REMOTE_FORMAT") {
      if rest.is_empty() {
        return Err(M3U8PlaylistParseError::RemoteFormatMissing);
      }

      if rest.chars().next().map_or(false, char::is_whitespace) {
        let format = rest.trim();
        if format.is_empty() {
          return Err(M3U8PlaylistParseError::RemoteFormatMissing);
        }
        return Ok(Some(Self::RemoteFormat(format.to_string())));
      }
    }

    if line.starts_with("#EXTMEDIA_") {
      return Ok(Some(Self::UnknownExtComment(line[1..].to_string())));
    }

    Ok(None)
  }
}

#[derive(Error, Debug)]
pub enum M3U8PlaylistParseError {
  /// I/O failure while reading the underlying stream.
  #[error("i/o error while reading playlist: {0}")]
  Io(#[from] io::Error),
  #[error("invalid URL in {tag}: {url}")]
  InvalidUrl {
    tag: &'static str,
    url: String,
    parse_error: ParseError,
  },
  #[error("media metadata without a following media entry")]
  DanglingMediaMetadata,
  #[error("#EXTMEDIA_REMOTE_FORMAT requires a format value")]
  RemoteFormatMissing,
  #[error("invalid #EXTMEDIA_LOCAL_ADDITION value: {value:?}")]
  InvalidLocalAddition { value: String },
  /// Two instances of a tag that may only appear once (per playlist /
  /// per entry) disagreed with each other.
  #[error("conflicting {tag} values: first {first:?}, then {second:?}")]
  ConflictingExtTag {
    tag: &'static str,
    first: String,
    second: String,
  },
}

impl M3U8Playlist {
  pub fn set_playlist_resource_id(&mut self, resource_id: ResourceId) -> anyhow::Result<()> {
    // Drop any previous remote reference or skip download marker.
    self.playlist_meta.retain(|meta| {
      !matches!(
        meta,
        PlaylistMeta::PlaylistExt(
          ExtPlaylistComment::RemotePlaylist(_) | ExtPlaylistComment::SkipRemoteDownloadPlaylist
        )
      )
    });

    Ok(match resource_id {
      ResourceId::YtPlaylistId(yt_id) => {
        self.playlist_meta.push(PlaylistMeta::PlaylistExt(
          ExtPlaylistComment::RemotePlaylist(yt_id.to_url()),
        ));
      },
      ResourceId::UnknownUrl(url) => {
        self.playlist_meta.push(PlaylistMeta::PlaylistExt(
          ExtPlaylistComment::RemotePlaylist(url),
        ));
      },
      ResourceId::LocalFile(_) => {
        self.playlist_meta.push(PlaylistMeta::PlaylistExt(
          ExtPlaylistComment::SkipRemoteDownloadPlaylist,
        ));
      },
      other_id => bail!(format!(
        "Can't set {other_id:?} as a remote playlist source."
      )),
    })
  }

  #[must_use]
  pub fn resource_id(&self) -> Option<ResourceId> {
    let resource_id = self
      .playlist_meta
      .iter()
      .find_map(|playlist_meta| match playlist_meta {
        PlaylistMeta::PlaylistExt(ExtPlaylistComment::RemotePlaylist(url)) => {
          Some(ResourceId::from(url))
        },
        _ => None,
      });
    resource_id
  }

  /// Parse a playlist from any buffered byte stream.
  pub fn parse(
    mut reader: impl BufRead,
    playlist_dir: &Path,
  ) -> Result<Self, M3U8PlaylistParseError> {
    // Controls when to switch from playlist comments to media level comments.
    let mut saw_first_media_level_item = false;

    // Pools that grow across the whole file.
    let mut playlist_meta: Vec<PlaylistMeta> = Vec::new();
    let mut entries: Vec<MediaEntry> = Vec::new();

    // Metadata waiting to be attached to the next media entry.
    let mut pending_media_meta: Vec<MediaMeta> = Vec::new();

    // Reused for every line so we don't allocate on each iteration.
    let mut line_buf = String::new();

    loop {
      line_buf.clear();
      let bytes_read = reader.read_line(&mut line_buf)?;
      if bytes_read == 0 {
        break; // EOF
      }

      // `read_line` keeps the line terminator; strip LF and (for
      // CRLF input) CR.
      let line = line_buf.trim_end_matches(['\n', '\r']);

      // Blank / whitespace-only lines carry no information.
      if line.trim().is_empty() {
        continue;
      }

      if line.starts_with('#') {
        // Playlist-level EXT tags are recognized in *any* context
        // and always promote to `playlist_meta`.
        if let Some(ext) = parse_playlist_ext(line)? {
          push_playlist_ext_checked(&mut playlist_meta, ext)?;
          continue;
        }

        // Media-level EXT tags are also recognized in any
        // context and accumulate for the *next* media entry.
        // Seeing the first one moves us from playlist preamble mode into media item mode.
        if let Some(ext) = ExtMediaComment::parse(line)? {
          saw_first_media_level_item = true;
          push_media_ext_checked(&mut pending_media_meta, ext)?;
          continue;
        }

        // Anything else is either a plain comment or an unknown
        // `#EXT…` tag we can't classify.
        let content = line[1..].to_string();
        if !saw_first_media_level_item {
          // playlist comments
          if line.starts_with("#EXT") {
            playlist_meta.push(PlaylistMeta::PlaylistExt(
              ExtPlaylistComment::UnknownExtComment(content),
            ));
          } else {
            playlist_meta.push(PlaylistMeta::Comment(content));
          }
        } else {
          if line.starts_with("#EXT") {
            pending_media_meta.push(MediaMeta::ExtComment(ExtMediaComment::UnknownExtComment(
              content,
            )));
          } else {
            pending_media_meta.push(MediaMeta::Comment(content));
          }
        }
      } else {
        // A media source line.
        saw_first_media_level_item = true;
        let media_source = MediaSource::parse(line, playlist_dir)?;
        entries.push(MediaEntry {
          meta: std::mem::take(&mut pending_media_meta),
          media_source,
        });
      }
    }

    // Any metadata accumulated after the last media entry has no media entry to attach to.
    if !pending_media_meta.is_empty() {
      flush_pending_media_meta(pending_media_meta, &mut playlist_meta)?;
    }

    Ok(M3U8Playlist {
      playlist_meta,
      entries,
    })
  }

  /// Serialize the playlist to any `Write` sink.
  /// The output is always UTF-8.
  pub fn serialize(&self, mut writer: impl Write, playlist_dir: &Path) -> io::Result<()> {
    // The header marker is always emitted first.
    writer.write_all(b"#EXTM3U\n")?;

    for meta in &self.playlist_meta {
      match meta {
        PlaylistMeta::Comment(c) => {
          writer.write_all(b"#")?;
          writer.write_all(c.as_bytes())?;
          writer.write_all(b"\n")?;
        },
        PlaylistMeta::PlaylistExt(ext) => {
          ext.serialize(&mut writer)?;
        },
      }
    }

    for entry in &self.entries {
      for meta in &entry.meta {
        match meta {
          MediaMeta::Comment(c) => {
            writer.write_all(b"#")?;
            writer.write_all(c.as_bytes())?;
            writer.write_all(b"\n")?;
          },
          MediaMeta::ExtComment(ext) => {
            ext.serialize(&mut writer)?;
          },
        }
      }

      match &entry.media_source {
        MediaSource::Url(url) => {
          writer.write_all(url.as_str().as_bytes())?;
          writer.write_all(b"\n")?;
        },
        MediaSource::File(path) => {
          writer.write_all(
            pathdiff::diff_paths(path.as_path(), playlist_dir)
              .unwrap_or_else(|| path.as_path().to_path_buf())
              .to_string_lossy()
              .as_bytes(),
          )?;
          writer.write_all(b"\n")?;
        },
      }
    }

    Ok(())
  }

  /// Convenience wrapper around [`serialize`] for callers who want the
  /// whole playlist as a `String`. Writing to a `Vec<u8>` cannot fail,
  /// and the output is always valid UTF-8.
  #[must_use]
  pub fn serialize_to_string(&self, playlist_dir: &Path) -> String {
    let mut buf = Vec::new();
    self
      .serialize(&mut buf, playlist_dir)
      .expect("writing to Vec<u8> cannot fail");
    String::from_utf8(buf).expect("m3u8 output is always valid UTF-8")
  }

  #[must_use]
  pub fn find(&self, resource_id: &ResourceId) -> Option<&MediaEntry> {
    self
      .entries
      .iter()
      .find(|media_entry| media_entry.resource_id() == *resource_id)
  }

  pub fn add_media_entry(&mut self, url: Url, format: String) {
    self.entries.push(MediaEntry {
      media_source: MediaSource::Url(url.clone()),
      meta: vec![
        MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(url.clone())),
        MediaMeta::ExtComment(ExtMediaComment::RemoteFormat(format)),
      ],
    });
  }

  pub fn update_playlist_from_remote<I, R>(&mut self, remote_playlist_items: I, format: &str)
  where
    I: IntoIterator<Item = R>,
    R: Borrow<ResourceId>,
  {
    // Take all existing entries so we can re-order them and detect local additions.
    let old_entries = std::mem::take(&mut self.entries);
    let mut remaining: BTreeMap<ResourceId, MediaEntry> = old_entries
      .into_iter()
      .map(|entry| (entry.resource_id(), entry))
      .collect();

    let remote_playlist_items_iter = remote_playlist_items.into_iter();
    let mut new_entries =
      Vec::with_capacity(remote_playlist_items_iter.size_hint().0 + remaining.len());

    // Apply the remote order and add entries for new media.
    for remote_id in remote_playlist_items_iter {
      let remote_id = remote_id.borrow();
      if let Some(mut existing) = remaining.remove(&remote_id) {
        // Already present locally; keep it (and its metadata) as-is.
        existing.remove_locally_unconfirmed();
        new_entries.push(existing);
      } else {
        // New item: push a media entry pointing at the remote URL.
        let remote_url = remote_id.to_url();
        let mut meta = vec![MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(
          remote_url.clone(),
        ))];
        meta.push(MediaMeta::ExtComment(ExtMediaComment::RemoteFormat(
          format.to_string(),
        )));
        new_entries.push(MediaEntry {
          meta,
          media_source: MediaSource::Url(remote_url),
        });
      }
    }

    // Anything left in `remaining` is a local addition. We don't
    // remove those items; we just mark them and append them after
    // the remote entries.
    for (_resource_id, mut entry) in remaining {
      let already_marked = entry.meta.iter().any(|meta| {
        matches!(
          meta,
          MediaMeta::ExtComment(ExtMediaComment::LocalAdditionMarker { .. })
        )
      });
      if !already_marked {
        entry.meta.push(MediaMeta::ExtComment(
          ExtMediaComment::LocalAdditionMarker { confirmed: false },
        ));
      }
      new_entries.push(entry);
    }

    self.entries = new_entries;
  }
}

/// Reattach trailing pending media metadata to the playlist.
///
/// This runs once, at EOF, for any metadata accumulated after the last
/// media entry:
///
/// * Plain comments can safely become playlist-level comments.
/// * Unknown `#EXT…` tags are ambiguous, but at EOF they clearly aren't
///   decorating a media entry, so we keep them around as playlist-level
///   unknown tags (preserving round-tripping).
/// * Any *known* media-level tag (`#EXTMEDIA_*`) is a genuine error -
///   it's media metadata with no media entry to attach to.
fn flush_pending_media_meta(
  pending_media_meta: Vec<MediaMeta>,
  playlist_meta: &mut Vec<PlaylistMeta>,
) -> Result<(), M3U8PlaylistParseError> {
  for meta in pending_media_meta {
    match meta {
      MediaMeta::Comment(comment) => {
        playlist_meta.push(PlaylistMeta::Comment(comment));
      },
      MediaMeta::ExtComment(ExtMediaComment::UnknownExtComment(comment)) => {
        playlist_meta.push(PlaylistMeta::PlaylistExt(
          ExtPlaylistComment::UnknownExtComment(comment),
        ));
      },
      MediaMeta::ExtComment(_) => {
        return Err(M3U8PlaylistParseError::DanglingMediaMetadata);
      },
    }
  }

  Ok(())
}

/// Insert a playlist-level EXT tag, deduping exact repeats and rejecting
/// mutually-inconsistent duplicates.
///
/// Only tags that are meaningful *singular* are checked:
/// `#EXTPLAYLIST_REMOTE` may appear at most once with a given URL,
/// `#EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD` is idempotent, and `#EXTM3U` is
/// idempotent (the serializer emits exactly one at the head anyway).
/// Unknown tags are passed through untouched.
fn push_playlist_ext_checked(
  playlist_meta: &mut Vec<PlaylistMeta>,
  ext: ExtPlaylistComment,
) -> Result<(), M3U8PlaylistParseError> {
  match &ext {
    ExtPlaylistComment::RemotePlaylist(new_url) => {
      for existing in playlist_meta.iter() {
        if let PlaylistMeta::PlaylistExt(ExtPlaylistComment::RemotePlaylist(old_url)) = existing {
          if old_url == new_url {
            return Ok(()); // idempotent duplicate
          }
          return Err(M3U8PlaylistParseError::ConflictingExtTag {
            tag: "#EXTPLAYLIST_REMOTE",
            first: old_url.to_string(),
            second: new_url.to_string(),
          });
        }
      }
    },
    ExtPlaylistComment::SkipRemoteDownloadPlaylist => {
      let already = playlist_meta.iter().any(|m| {
        matches!(
          m,
          PlaylistMeta::PlaylistExt(ExtPlaylistComment::SkipRemoteDownloadPlaylist)
        )
      });
      if already {
        return Ok(());
      }
    },
    ExtPlaylistComment::EXTM3UMarker => {
      let already = playlist_meta.iter().any(|m| {
        matches!(
          m,
          PlaylistMeta::PlaylistExt(ExtPlaylistComment::EXTM3UMarker)
        )
      });
      if already {
        return Ok(());
      }
    },
    ExtPlaylistComment::UnknownExtComment(_) => {
      // Unknown tags are opaque; we don't attempt to dedupe them.
    },
  }

  playlist_meta.push(PlaylistMeta::PlaylistExt(ext));
  Ok(())
}

/// Insert a media-level EXT tag into the pending metadata for the next
/// entry, deduping exact repeats and rejecting mutually-inconsistent
/// duplicates within the same entry.
fn push_media_ext_checked(
  pending_media_meta: &mut Vec<MediaMeta>,
  ext: ExtMediaComment,
) -> Result<(), M3U8PlaylistParseError> {
  match &ext {
    ExtMediaComment::RemoteMedia(new_url) => {
      for existing in pending_media_meta.iter() {
        if let MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(old_url)) = existing {
          if old_url == new_url {
            return Ok(());
          }
          return Err(M3U8PlaylistParseError::ConflictingExtTag {
            tag: "#EXTMEDIA_REMOTE",
            first: old_url.to_string(),
            second: new_url.to_string(),
          });
        }
      }
    },
    ExtMediaComment::RemoteFormat(new_format) => {
      for existing in pending_media_meta.iter() {
        if let MediaMeta::ExtComment(ExtMediaComment::RemoteFormat(old_format)) = existing {
          if old_format == new_format {
            return Ok(());
          }
          return Err(M3U8PlaylistParseError::ConflictingExtTag {
            tag: "#EXTMEDIA_REMOTE_FORMAT",
            first: old_format.clone(),
            second: new_format.clone(),
          });
        }
      }
    },
    ExtMediaComment::LocalAdditionMarker { confirmed } => {
      for existing in pending_media_meta.iter() {
        if let MediaMeta::ExtComment(ExtMediaComment::LocalAdditionMarker {
          confirmed: old_confirmed,
        }) = existing
        {
          if old_confirmed == confirmed {
            return Ok(());
          }

          let first = (if *old_confirmed {
            "CONFIRMED"
          } else {
            "UNCONFIRMED"
          })
          .to_string();
          let second = (if *confirmed {
            "CONFIRMED"
          } else {
            "UNCONFIRMED"
          })
          .to_string();

          return Err(M3U8PlaylistParseError::ConflictingExtTag {
            tag: "#EXTMEDIA_LOCAL_ADDITION",
            first,
            second,
          });
        }
      }
    },
    ExtMediaComment::SkipRemoteDownloadMarker => {
      let already = pending_media_meta.iter().any(|m| {
        matches!(
          m,
          MediaMeta::ExtComment(ExtMediaComment::SkipRemoteDownloadMarker)
        )
      });
      if already {
        return Ok(());
      }
    },
    ExtMediaComment::UnknownExtComment(_) => {
      // Unknown tags are opaque; we don't attempt to dedupe them.
    },
  }

  pending_media_meta.push(MediaMeta::ExtComment(ext));
  Ok(())
}

fn parse_url_from_rest(rest: &str, tag: &'static str) -> Result<Url, M3U8PlaylistParseError> {
  let url = Url::parse(rest.trim()).map_err(|parse_error| M3U8PlaylistParseError::InvalidUrl {
    tag,
    url: rest.to_string(),
    parse_error,
  })?;
  Ok(url)
}

/// Parse a playlist-level `#EXT…` line.
///
/// Returns `Ok(Some(_))` only for tags with a playlist-level prefix
/// (`#EXTPLAYLIST_*` or `#EXTM3U`). Other `#EXT…` tags return `Ok(None)`
/// so the caller can decide whether they're media-level or unclassified.
///
/// Unknown `#EXTPLAYLIST_*` tags are captured as
/// [`ExtPlaylistComment::UnknownExtComment`] so they round-trip untouched.
fn parse_playlist_ext(line: &str) -> Result<Option<ExtPlaylistComment>, M3U8PlaylistParseError> {
  if let Some(rest) = line.strip_prefix("#EXTPLAYLIST_REMOTE") {
    // Only treat this as `#EXTPLAYLIST_REMOTE <url>` when the following
    // character is whitespace (or the tag is bare, matching the
    // original error path).
    if rest.chars().next().map_or(true, char::is_whitespace) {
      let url = parse_url_from_rest(rest, "#EXTPLAYLIST_REMOTE")?;
      return Ok(Some(ExtPlaylistComment::RemotePlaylist(url)));
    }
  }

  if line.trim_end() == "#EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD" {
    return Ok(Some(ExtPlaylistComment::SkipRemoteDownloadPlaylist));
  }

  if line.trim_end() == "#EXTM3U" {
    return Ok(Some(ExtPlaylistComment::EXTM3UMarker));
  }

  if line.starts_with("#EXTPLAYLIST_") {
    return Ok(Some(ExtPlaylistComment::UnknownExtComment(
      line[1..].to_string(),
    )));
  }

  Ok(None)
}
