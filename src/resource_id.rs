use std::path::PathBuf;

use url::Url;

use crate::parse_id_from_yt_url;

#[derive(PartialEq, Eq)]
pub enum ResourceId {
  YtId(String),
  UnknownUrl(Url),
  LocalFile(PathBuf),
}

impl From<&'_ Url> for ResourceId {
  fn from(remote_url: &'_ Url) -> Self {
    if let Some(yt_id) = parse_id_from_yt_url::yt_id_from_url(remote_url) {
      return Self::YtId(yt_id);
    }
    Self::UnknownUrl(remote_url.clone())
  }
}
