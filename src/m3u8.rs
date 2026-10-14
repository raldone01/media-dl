//! m3u8 ser/des.
//!
//! ```m3u8
//! #EXTM3U
//! # Indicates this local playlist tracks a remote playlist.
//! #EXTPLAYLIST_REMOTE <URL>
//! # Indicates that remote download should be skipped.
//! #EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD
//! # Indicates that this playlist intentionally might not track a remote playlist.
//! #EXTPLAYLIST_UNTRACKED
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
//! song4.opus
//! ```
//!
//! TODO: error on duplicate EXT tags we know
//! TODO: make sure repeated files are handled properly

use std::path::PathBuf;

use anyhow::bail;
use thiserror::Error;
use url::{ParseError, Url};

use crate::resource_id::ResourceId;

pub enum MediaSource {
  Url(Url),
  Path(PathBuf),
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
      .meta
      .iter()
      .find_map(|item| match item {
        MediaMeta::ExtComment(ExtMediaComment::RemoteMedia(url)) => Some(ResourceId::from(url)),
        _ => None,
      })
      .unwrap_or_else(|| match self.media_source {
        MediaSource::Url(ref url) => ResourceId::from(url),
        MediaSource::Path(ref path) => ResourceId::LocalFile(path.to_owned()),
      });
    resource_id
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
  UntrackedPlaylistMarker,
  UnknownExtComment(String),
}

/// media-level #EXT...
pub enum ExtMediaComment {
  LocalAdditionMarker,
  SkipRemoteDownloadMarker,
  RemoteMedia(Url),
  RemoteFormat(String),
  UnknownExtComment(String),
}

#[derive(Error, Debug)]
pub enum M3U8PlaylistParseError {
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
}

impl M3U8Playlist {
  pub fn set_playlist_resource_id(&mut self, resource_id: ResourceId) -> Result<(), anyhow::Error> {
    // Drop any previous remote reference or untracked marker.
    self.playlist_meta.retain(|meta| {
      !matches!(
        meta,
        PlaylistMeta::PlaylistExt(
          ExtPlaylistComment::RemotePlaylist(_) | ExtPlaylistComment::UntrackedPlaylistMarker
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
          ExtPlaylistComment::UntrackedPlaylistMarker,
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

  pub fn deserialize(source: &str) -> Result<Self, M3U8PlaylistParseError> {
    let lines = source.lines();

    let mut playlist_meta = Vec::new();
    let mut entries = Vec::new();
    let mut pending_media_meta = Vec::new();
    let mut seen_entry = false;

    for line in lines {
      if line.trim().is_empty() {
        // skip empty lines
        continue;
      }
      if line.starts_with('#') {
        if !seen_entry {
          if let Some(ext) = parse_playlist_ext(line)? {
            playlist_meta.push(PlaylistMeta::PlaylistExt(ext));
          } else {
            playlist_meta.push(PlaylistMeta::Comment(line[1..].to_string()));
          }
        } else {
          if let Some(ext) = parse_media_ext(line)? {
            pending_media_meta.push(MediaMeta::ExtComment(ext));
          } else {
            pending_media_meta.push(MediaMeta::Comment(line[1..].to_string()));
          }
        }
      } else {
        let path = parse_media_source(line)?;
        entries.push(MediaEntry {
          meta: std::mem::take(&mut pending_media_meta),
          media_source: path,
        });
        seen_entry = true;
      }
    }

    if !pending_media_meta.is_empty() {
      flush_pending_media_meta(pending_media_meta, &mut playlist_meta)?;
    }

    Ok(M3U8Playlist {
      playlist_meta,
      entries,
    })
  }

  #[must_use]
  pub fn serialize(&self) -> String {
    let mut out = String::from("#EXTM3U\n");

    for meta in &self.playlist_meta {
      match meta {
        PlaylistMeta::Comment(c) => {
          out.push('#');
          out.push_str(c);
          out.push('\n');
        },
        PlaylistMeta::PlaylistExt(ext) => {
          serialize_playlist_ext(&mut out, ext);
        },
      }
    }

    for entry in &self.entries {
      for meta in &entry.meta {
        match meta {
          MediaMeta::Comment(c) => {
            out.push('#');
            out.push_str(c);
            out.push('\n');
          },
          MediaMeta::ExtComment(ext) => {
            serialize_media_ext(&mut out, ext);
          },
        }
      }

      match &entry.media_source {
        MediaSource::Url(url) => {
          out.push_str(url.as_str());
          out.push('\n');
        },
        MediaSource::Path(path) => {
          out.push_str(&path.to_string_lossy());
          out.push('\n');
        },
      }
    }

    out
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
}

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
        // At EOF, an unknown media-level "#EXT..." may actually be a trailing
        // playlist-level extension/comment. Try parsing it as playlist-level.
        let line = format!("#{comment}");
        if let Some(ext) = parse_playlist_ext(&line)? {
          playlist_meta.push(PlaylistMeta::PlaylistExt(ext));
        } else {
          playlist_meta.push(PlaylistMeta::Comment(comment));
        }
      },
      MediaMeta::ExtComment(_) => {
        return Err(M3U8PlaylistParseError::DanglingMediaMetadata);
      },
    }
  }

  Ok(())
}

fn parse_urls_from_rest(rest: &str, tag: &'static str) -> Result<Vec<Url>, M3U8PlaylistParseError> {
  let mut urls = Vec::new();
  for part in rest.split_whitespace() {
    let url = Url::parse(part).map_err(|parse_error| M3U8PlaylistParseError::InvalidUrl {
      tag,
      url: part.to_string(),
      parse_error,
    })?;
    urls.push(url);
  }
  Ok(urls)
}

fn parse_url_from_rest(rest: &str, tag: &'static str) -> Result<Url, M3U8PlaylistParseError> {
  let url = Url::parse(rest).map_err(|parse_error| M3U8PlaylistParseError::InvalidUrl {
    tag,
    url: rest.to_string(),
    parse_error,
  })?;
  Ok(url)
}

fn parse_playlist_ext(line: &str) -> Result<Option<ExtPlaylistComment>, M3U8PlaylistParseError> {
  if let Some(rest) = line.strip_prefix("#EXTPLAYLIST_REMOTE") {
    if rest.chars().next().map_or(true, char::is_whitespace) {
      let url = parse_url_from_rest(rest, "#EXTPLAYLIST_REMOTE")?;
      return Ok(Some(ExtPlaylistComment::RemotePlaylist(url)));
    }
  }

  if line.trim_end() == "#EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD" {
    return Ok(Some(ExtPlaylistComment::SkipRemoteDownloadPlaylist));
  }

  if line.trim_end() == "#EXTPLAYLIST_UNTRACKED" {
    return Ok(Some(ExtPlaylistComment::UntrackedPlaylistMarker));
  }

  if line.trim_end() == "#EXTM3U" {
    return Ok(Some(ExtPlaylistComment::EXTM3UMarker));
  }

  if line.starts_with("#EXT") {
    return Ok(Some(ExtPlaylistComment::UnknownExtComment(
      line[1..].to_string(),
    )));
  }

  Ok(None)
}

fn parse_media_ext(line: &str) -> Result<Option<ExtMediaComment>, M3U8PlaylistParseError> {
  if line.trim_end() == "#EXTMEDIA_LOCAL_ADDITION" {
    return Ok(Some(ExtMediaComment::LocalAdditionMarker));
  }

  if line.trim_end() == "#EXTMEDIA_REMOTE_SKIP_DOWNLOAD" {
    return Ok(Some(ExtMediaComment::SkipRemoteDownloadMarker));
  }

  if let Some(rest) = line.strip_prefix("#EXTMEDIA_REMOTE") {
    let url = parse_url_from_rest(rest, "#EXTMEDIA_REMOTE")?;
    return Ok(Some(ExtMediaComment::RemoteMedia(url)));
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
      return Ok(Some(ExtMediaComment::RemoteFormat(format.to_string())));
    }
  }

  if line.starts_with("#EXT") {
    return Ok(Some(ExtMediaComment::UnknownExtComment(
      line[1..].to_string(),
    )));
  }

  Ok(None)
}

fn parse_media_source(line: &str) -> Result<MediaSource, M3U8PlaylistParseError> {
  match Url::parse(line) {
    Ok(url) if url.scheme().len() > 1 || line.contains("://") => Ok(MediaSource::Url(url)),
    _ => Ok(MediaSource::Path(PathBuf::from(line))),
  }
}

fn serialize_playlist_ext(out: &mut String, ext: &ExtPlaylistComment) {
  match ext {
    ExtPlaylistComment::EXTM3UMarker => {
      // ignore additional stray markers
      // we always add one at the start
    },
    ExtPlaylistComment::RemotePlaylist(url) => {
      out.push_str("#EXTPLAYLIST_REMOTE ");
      out.push_str(url.as_str());
      out.push('\n');
    },
    ExtPlaylistComment::SkipRemoteDownloadPlaylist => {
      out.push_str("#EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD\n");
    },
    ExtPlaylistComment::UntrackedPlaylistMarker => {
      out.push_str("#EXTPLAYLIST_UNTRACKED\n");
    },
    ExtPlaylistComment::UnknownExtComment(c) => {
      out.push('#');
      out.push_str(c);
      out.push('\n');
    },
  }
}

fn serialize_media_ext(out: &mut String, ext: &ExtMediaComment) {
  match ext {
    ExtMediaComment::LocalAdditionMarker => {
      out.push_str("#EXTMEDIA_LOCAL_ADDITION\n");
    },
    ExtMediaComment::SkipRemoteDownloadMarker => {
      out.push_str("#EXTMEDIA_REMOTE_SKIP_DOWNLOAD\n");
    },
    ExtMediaComment::RemoteMedia(url) => {
      out.push_str("#EXTMEDIA_REMOTE ");
      out.push_str(url.as_str());
      out.push('\n');
    },
    ExtMediaComment::RemoteFormat(format) => {
      out.push_str("#EXTMEDIA_REMOTE_FORMAT ");
      out.push_str(format);
      out.push('\n');
    },
    ExtMediaComment::UnknownExtComment(c) => {
      out.push('#');
      out.push_str(c);
      out.push('\n');
    },
  }
}
