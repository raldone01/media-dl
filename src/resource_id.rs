use crate::{absolute_file_path::AbsoluteFilePath, yt_id::YtMediaId, yt_playlist_id::YtPlaylistId};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceId {
  YtMediaId(YtMediaId),
  YtPlaylistId(YtPlaylistId),
  UnknownUrl(Url),
  LocalFile(AbsoluteFilePath),
}

impl ResourceId {
  #[must_use]
  pub fn yt_media_id(&self) -> Option<&YtMediaId> {
    match self {
      ResourceId::YtMediaId(inner) => Some(inner),
      _ => None,
    }
  }

  #[must_use]
  pub fn yt_playlist_id(&self) -> Option<&YtPlaylistId> {
    match self {
      ResourceId::YtPlaylistId(inner) => Some(inner),
      _ => None,
    }
  }

  pub fn to_url(&self) -> Url {
    match self {
      ResourceId::YtMediaId(yt_media_id) => yt_media_id.to_url(),
      ResourceId::YtPlaylistId(yt_playlist_id) => yt_playlist_id.to_url(),
      ResourceId::UnknownUrl(url) => url.clone(),
      ResourceId::LocalFile(local_file) => local_file.to_url(),
    }
  }

  #[must_use]
  pub fn to_string(&self) -> String {
    match self {
      ResourceId::LocalFile(local_file) => local_file.as_path().to_string_lossy().to_string(),
      ResourceId::UnknownUrl(url) => url.to_string(),
      ResourceId::YtMediaId(yt_media_id) => yt_media_id.to_string(),
      ResourceId::YtPlaylistId(yt_playlist_id) => yt_playlist_id.to_string(),
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
    if let Ok(yt_id) = YtPlaylistId::try_from(remote_url) {
      return Self::YtPlaylistId(yt_id);
    }
    if let Ok(yt_id) = YtMediaId::try_from(remote_url) {
      return Self::YtMediaId(yt_id);
    }
    Self::UnknownUrl(remote_url.clone())
  }
}

impl From<&'_ ResourceId> for String {
  fn from(value: &'_ ResourceId) -> Self {
    value.to_string()
  }
}
