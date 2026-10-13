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
//! local_only_song.mp3
//! # Indicates that remote download should be skipped.
//! #EXTMEDIA_REMOTE_SKIP_DOWNLOAD
//! song3.opus
//! # Indicates what remote media this media entry tracks.
//! # If the local file is unavailable the urls will be tried in order.
//! #EXTMEDIA_REMOTE <URL> <URL> <...>
//! song4.opus

use std::path::PathBuf;

use thiserror::Error;
use url::{ParseError, Url};

pub enum MediaSource {
  Url(Url),
  Path(PathBuf),
}

pub struct M3U8Playlist {
  pub playlist_meta: Vec<PlaylistMeta>,
  pub entries: Vec<MediaEntry>,
}

pub enum PlaylistMeta {
  EmptyLine,
  Comment(String),
  PlaylistExt(ExtPlaylistComment),
}

pub struct MediaEntry {
  pub meta: Vec<MediaMeta>,
  pub path: MediaSource,
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
  GenericComment(String),
}

/// media-level #EXT...
pub enum ExtMediaComment {
  LocalAdditionMarker,
  SkipRemoteDownloadMarker,
  RemoteMedia(Vec<Url>),
  GenericComment(String),
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
}

impl M3U8Playlist {
  pub fn deserialize(source: String) -> Result<Self, M3U8PlaylistParseError> {
    let lines = source.lines();

    let mut playlist_meta = Vec::new();
    let mut entries = Vec::new();
    let mut pending_media_meta = Vec::new();
    let mut seen_entry = false;

    for line in lines {
      if line.trim().is_empty() {
        playlist_meta.push(PlaylistMeta::EmptyLine);
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
          path,
        });
        seen_entry = true;
      }
    }

    if !pending_media_meta.is_empty() {
      return Err(M3U8PlaylistParseError::DanglingMediaMetadata);
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
        PlaylistMeta::EmptyLine => {
          out.push('\n');
        },
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

      match &entry.path {
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

  if line.starts_with("#EXT") {
    return Ok(Some(ExtPlaylistComment::GenericComment(
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
    if rest.chars().next().map_or(true, char::is_whitespace) {
      let urls = parse_urls_from_rest(rest, "#EXTMEDIA_REMOTE")?;
      return Ok(Some(ExtMediaComment::RemoteMedia(urls)));
    }
  }

  if line.starts_with("#EXT") {
    return Ok(Some(ExtMediaComment::GenericComment(line[1..].to_string())));
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
      out.push_str("#EXTPLAYLIST_REMOTE");
      out.push(' ');
      out.push_str(url.as_str());
      out.push('\n');
    },
    ExtPlaylistComment::SkipRemoteDownloadPlaylist => {
      out.push_str("#EXTPLAYLIST_REMOTE_SKIP_DOWNLOAD\n");
    },
    ExtPlaylistComment::UntrackedPlaylistMarker => {
      out.push_str("#EXTPLAYLIST_UNTRACKED\n");
    },
    ExtPlaylistComment::GenericComment(c) => {
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
    ExtMediaComment::RemoteMedia(urls) => {
      out.push_str("#EXTMEDIA_REMOTE");
      for url in urls {
        out.push(' ');
        out.push_str(url.as_str());
      }
      out.push('\n');
    },
    ExtMediaComment::GenericComment(c) => {
      out.push('#');
      out.push_str(c);
      out.push('\n');
    },
  }
}
