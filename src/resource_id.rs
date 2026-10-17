use crate::{fs_utils::absolute_from_cwd, yt_id::YtMediaId, yt_playlist_id::YtPlaylistId};
use std::path::PathBuf;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceId {
  YtMediaId(YtMediaId),
  YtPlaylistId(YtPlaylistId),
  UnknownUrl(Url),
  LocalFile(PathBuf),
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
      ResourceId::LocalFile(file_path) => {
        let absolute_path = absolute_from_cwd(file_path);
        Url::from_file_path(absolute_path)
          .expect("This can only error if the path is not absolute which we checked")
      },
    }
  }

  #[must_use]
  pub fn to_string(&self) -> String {
    match self {
      ResourceId::LocalFile(file_path) => file_path.to_string_lossy().to_string(),
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
