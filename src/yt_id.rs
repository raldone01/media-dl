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

  #[error("invalid YouTube video ID: {id:?}")]
  InvalidVideoId { id: String },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[serde(transparent)]
pub struct YtId(String);

impl YtId {
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

impl From<String> for YtId {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl From<YtId> for String {
  fn from(value: YtId) -> Self {
    value.0
  }
}

impl From<YtId> for Url {
  fn from(value: YtId) -> Self {
    value.to_url()
  }
}

impl Display for YtId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl TryFrom<&Url> for YtId {
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
      let mut invalid_query_id = None;

      // https://www.youtube.com/watch?v=VIDEO_ID
      // https://music.youtube.com/watch?v=VIDEO_ID
      if let Some((_, v)) = yt_url.query_pairs().find(|(k, _)| k.as_ref() == "v") {
        if is_youtube_id(v.as_ref()) {
          return Ok(YtId::from(v.into_owned()));
        }
        if !v.is_empty() {
          invalid_query_id = Some(v.into_owned());
        }
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

      return match invalid_query_id {
        Some(id) => Err(YtIdUrlParseError::InvalidVideoId { id }),
        None => Err(YtIdUrlParseError::MissingVideoId),
      };
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

fn validate_id(id: &str) -> Result<YtId, YtIdUrlParseError> {
  if id.is_empty() {
    Err(YtIdUrlParseError::MissingVideoId)
  } else if is_youtube_id(id) {
    Ok(YtId::from(id.to_owned()))
  } else {
    Err(YtIdUrlParseError::InvalidVideoId { id: id.to_owned() })
  }
}

#[must_use]
fn is_youtube_id(s: &str) -> bool {
  s.len() == 11
    && s
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
