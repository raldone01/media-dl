use crate::{yt_id::YtMediaId, yt_playlist_id::YtPlaylistId};
use std::path::PathBuf;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
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

  #[must_use]
  pub fn to_url(&self) -> Result<Url, anyhow::Error> {
    Ok(match self {
      ResourceId::YtMediaId(yt_media_id) => yt_media_id.to_url(),
      ResourceId::YtPlaylistId(yt_playlist_id) => yt_playlist_id.to_url(),
      ResourceId::UnknownUrl(url) => url.clone(),
      ResourceId::LocalFile(file_path) => Url::from_file_path(file_path)
        .map_err(|_| anyhow::anyhow!(format!("Path {} not absolute", file_path.display())))?,
    })
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
