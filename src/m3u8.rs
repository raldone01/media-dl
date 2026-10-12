//! m3u8 ser/des.
//!
//! ```m3u8
//! #EXTM3U
//! # Indicates this local playlist tracks one or more remote playlists.
//! #EXTPLAYLIST_REMOTE <URL> <URL> <...>
//! # Indicates that this playlist intentionally might not track remote playlists.
//! #EXTPLAYLIST_UNTRACKED
//! song1.mp3
//! # some comment
//! song2.mp3
//! # Indicates that the following song is a local addition to the playlist.
//! #EXTMEDIA_LOCAL_ADDITION
//! local_only_song.mp3
//! # Indicates that remote download should be skipped.
//! #EXTMEDIA_SKIP_REMOTE_DOWNLOAD
//! song3.opus
//! # Indicates what remote media this song tracks.
//! # If the local file is unavailable the urls will be tried in order.
//! #EXTMEDIA_REMOTE <URL> <URL> <...>
//! song4.opus

use std::path::PathBuf;

use thiserror::Error;
use url::Url;

pub enum MediaSource {
  Url(Url),
  Path(PathBuf),
}

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
  pub path: MediaSource,
}

pub enum MediaMeta {
  Comment(String),
  ExtComment(ExtMediaComment),
}

/// playlist-level #EXT...
pub enum ExtPlaylistComment {
  RemotePlaylists(Vec<Url>),
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
pub enum M3U8PlaylistParseError {}

impl M3U8Playlist {
  pub fn deserialize(source: String) -> Result<Self, M3U8PlaylistParseError> {
    todo!()
  }

  #[must_use]
  pub fn serialize(&self) -> String {
    todo!()
  }
}
