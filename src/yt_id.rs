use std::fmt::Display;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum YtIdUrlParseError {
  #[error("URL has no host")]
  MissingHost,

  #[error("unsupported YouTube URL host: {host}")]
  UnsupportedHost { host: String },

  #[error("missing YouTube video ID")]
  MissingVideoId,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[serde(transparent)]
pub struct YtMediaId(String);

impl YtMediaId {
  #[must_use]
  pub fn as_str(&self) -> &str {
    &self.0
  }

  #[must_use]
  pub fn to_url(&self) -> Url {
    // `youtu.be` short-link form: https://youtu.be/<VIDEO_ID>
    let mut url = Url::parse("https://youtu.be/").expect("static base URL is always valid");
    url
      .path_segments_mut()
      .expect("youtu.be URL supports path segments")
      .push(self.as_str());
    url
  }
}

impl From<String> for YtMediaId {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl From<YtMediaId> for String {
  fn from(value: YtMediaId) -> Self {
    value.0
  }
}

impl From<YtMediaId> for Url {
  fn from(value: YtMediaId) -> Self {
    value.to_url()
  }
}

impl Display for YtMediaId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl TryFrom<&Url> for YtMediaId {
  type Error = YtIdUrlParseError;

  fn try_from(yt_url: &Url) -> Result<Self, Self::Error> {
    let host = yt_url
      .host_str()
      .ok_or(YtIdUrlParseError::MissingHost)?
      .to_ascii_lowercase();

    // https://youtu.be/VIDEO_ID
    if host == "youtu.be" {
      let mut segments = yt_url
        .path_segments()
        .ok_or(YtIdUrlParseError::MissingVideoId)?;
      let id = segments.next().ok_or(YtIdUrlParseError::MissingVideoId)?;
      return validate_id(id);
    }

    // https://www.youtube.com/..., https://music.youtube.com/...
    if host == "youtube.com" || host.ends_with(".youtube.com") {
      // https://www.youtube.com/watch?v=VIDEO_ID
      // https://music.youtube.com/watch?v=VIDEO_ID
      if let Some((_, v)) = yt_url.query_pairs().find(|(k, _)| k.as_ref() == "v") {
        if v.is_empty() {
          return Err(YtIdUrlParseError::MissingVideoId);
        }

        return Ok(YtMediaId::from(v.into_owned()));
      }

      // https://www.youtube.com/embed/VIDEO_ID
      // https://www.youtube.com/v/VIDEO_ID
      // https://www.youtube.com/shorts/VIDEO_ID
      // https://www.youtube.com/live/VIDEO_ID
      let mut segments = yt_url
        .path_segments()
        .ok_or(YtIdUrlParseError::MissingVideoId)?;
      if let Some(kind) = segments.next() {
        if matches!(kind, "embed" | "v" | "shorts" | "live") {
          if let Some(id) = segments.next() {
            return validate_id(id);
          }
        }
      }

      return Err(YtIdUrlParseError::MissingVideoId);
    }

    // https://www.youtube-nocookie.com/embed/VIDEO_ID
    if host == "youtube-nocookie.com" || host.ends_with(".youtube-nocookie.com") {
      let mut segments = yt_url
        .path_segments()
        .ok_or(YtIdUrlParseError::MissingVideoId)?;
      if segments.next() != Some("embed") {
        return Err(YtIdUrlParseError::MissingVideoId);
      }
      let id = segments.next().ok_or(YtIdUrlParseError::MissingVideoId)?;
      return validate_id(id);
    }

    Err(YtIdUrlParseError::UnsupportedHost { host })
  }
}

fn validate_id(id: &str) -> Result<YtMediaId, YtIdUrlParseError> {
  if id.is_empty() {
    return Err(YtIdUrlParseError::MissingVideoId);
  }
  Ok(YtMediaId::from(id.to_owned()))
}
