use std::path::PathBuf;

use url::Url;

use crate::yt_id::YtId;

#[derive(PartialEq, Eq)]
pub enum ResourceId {
  YtId(YtId),
  UnknownUrl(Url),
  LocalFile(PathBuf),
}

impl ResourceId {
  #[must_use]
  pub fn yt_id(&self) -> Option<&YtId> {
    match self {
      ResourceId::YtId(yt_id) => Some(yt_id),
      _ => None,
    }
  }
}

impl From<&'_ Url> for ResourceId {
  fn from(remote_url: &'_ Url) -> Self {
    if let Ok(yt_id) = YtId::try_from(remote_url) {
      return Self::YtId(yt_id);
    }
    Self::UnknownUrl(remote_url.clone())
  }
}
