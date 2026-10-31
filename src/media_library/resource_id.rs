use url::Url;

use crate::{utils::AbsoluteFilePath, yt::YtMediaId, yt::YtPlaylistId};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaylistResourceId {
  YtPlaylistId(YtPlaylistId),
  LocalPlaylistFile(AbsoluteFilePath),
  UnknownUrl(Url),
}

impl PlaylistResourceId {
  #[must_use]
  pub fn to_url(&self) -> Url {
    match self {
      Self::YtPlaylistId(yt_playlist_id) => yt_playlist_id.to_url(),
      Self::LocalPlaylistFile(local_file) => local_file.to_url(),
      Self::UnknownUrl(url) => url.clone(),
    }
  }

  #[must_use]
  pub fn to_string(&self) -> String {
    match self {
      Self::YtPlaylistId(yt_playlist_id) => yt_playlist_id.to_string(),
      Self::LocalPlaylistFile(local_file) => local_file.as_path().to_string_lossy().to_string(),
      Self::UnknownUrl(url) => url.to_string(),
    }
  }
}

impl From<&'_ Url> for PlaylistResourceId {
  fn from(remote_url: &'_ Url) -> Self {
    if let Ok(yt_id) = YtPlaylistId::try_from(remote_url) {
      return Self::YtPlaylistId(yt_id);
    }
    if remote_url.scheme() == "file"
      && let Ok(file_path) = remote_url.to_file_path()
    {
      return Self::LocalPlaylistFile(
        AbsoluteFilePath::new(file_path).expect("Path must be absolute"),
      );
    }
    Self::UnknownUrl(remote_url.clone())
  }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaResourceId {
  YtMediaId(YtMediaId),
  LocalMediaFile(AbsoluteFilePath),
  UnknownUrl(Url),
}

impl MediaResourceId {
  #[must_use]
  pub fn to_url(&self) -> Url {
    match self {
      Self::YtMediaId(yt_media_id) => yt_media_id.to_url(),
      Self::LocalMediaFile(local_file) => local_file.to_url(),
      Self::UnknownUrl(url) => url.clone(),
    }
  }

  #[must_use]
  pub fn to_string(&self) -> String {
    match self {
      Self::YtMediaId(yt_media_id) => yt_media_id.to_string(),
      Self::LocalMediaFile(local_file) => local_file.as_path().to_string_lossy().to_string(),
      Self::UnknownUrl(url) => url.to_string(),
    }
  }
}

impl From<&'_ Url> for MediaResourceId {
  fn from(remote_url: &'_ Url) -> Self {
    if let Ok(yt_id) = YtMediaId::try_from(remote_url) {
      return Self::YtMediaId(yt_id);
    }
    if remote_url.scheme() == "file"
      && let Ok(file_path) = remote_url.to_file_path()
    {
      return Self::LocalMediaFile(
        AbsoluteFilePath::new(file_path).expect("Path must be absolute"),
      );
    }
    Self::UnknownUrl(remote_url.clone())
  }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceId {
  Playlist(PlaylistResourceId),
  Media(MediaResourceId),
  UnknownUrl(Url),
}

impl ResourceId {
  #[must_use]
  pub fn to_url(&self) -> Url {
    match self {
      Self::Playlist(playlist_id) => playlist_id.to_url(),
      Self::Media(media_id) => media_id.to_url(),
      Self::UnknownUrl(url) => url.clone(),
    }
  }

  #[must_use]
  pub fn to_string(&self) -> String {
    match self {
      Self::Media(media_id) => media_id.to_string(),
      Self::Playlist(playlist_id) => playlist_id.to_string(),
      Self::UnknownUrl(url) => url.to_string(),
    }
  }
}

impl From<&'_ ResourceId> for Url {
  fn from(value: &'_ ResourceId) -> Self {
    value.to_url()
  }
}

impl From<&'_ Url> for ResourceId {
  fn from(remote_url: &'_ Url) -> Self {
    // Try to decode a playlist url first.
    let playlist_id = PlaylistResourceId::from(remote_url);
    if matches!(playlist_id, PlaylistResourceId::UnknownUrl(..)) {
      // Try to decode a media url next.
      let media_id = MediaResourceId::from(remote_url);
      if let MediaResourceId::UnknownUrl(remote_url) = media_id {
        // We can't classify the url as either a playlist or media.
        return Self::UnknownUrl(remote_url);
      }

      return Self::Media(media_id);
    }

    return Self::Playlist(playlist_id);
  }
}

impl From<&'_ ResourceId> for String {
  fn from(value: &'_ ResourceId) -> Self {
    value.to_string()
  }
}
