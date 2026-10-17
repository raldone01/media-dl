use url::Url;

use crate::{utils::AbsoluteFilePath, yt::YtMediaId, yt::YtPlaylistId};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceId {
  YtMediaId(YtMediaId),
  YtPlaylistId(YtPlaylistId),
  UnknownUrl(Url),
  LocalFile(AbsoluteFilePath),
}

impl ResourceId {
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
