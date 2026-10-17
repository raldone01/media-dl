use std::fmt::Display;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum YtPlaylistIdUrlParseError {
  #[error("URL has no host")]
  MissingHost,

  #[error("unsupported YouTube URL host: {host}")]
  UnsupportedHost { host: String },

  #[error("missing YouTube playlist ID")]
  MissingPlaylistId,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone, PartialOrd, Ord)]
#[serde(transparent)]
pub struct YtPlaylistId(String);

impl YtPlaylistId {
  #[must_use]
  pub fn as_str(&self) -> &str {
    &self.0
  }

  #[must_use]
  pub fn to_url(&self) -> Url {
    // Canonical playlist form: https://www.youtube.com/playlist?list=PLAYLIST_ID
    let mut url =
      Url::parse("https://www.youtube.com/playlist").expect("static base URL is always valid");
    url.query_pairs_mut().append_pair("list", self.as_str());
    url
  }
}

impl From<String> for YtPlaylistId {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl From<YtPlaylistId> for String {
  fn from(value: YtPlaylistId) -> Self {
    value.0
  }
}

impl From<YtPlaylistId> for Url {
  fn from(value: YtPlaylistId) -> Self {
    value.to_url()
  }
}

impl Display for YtPlaylistId {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl TryFrom<&Url> for YtPlaylistId {
  type Error = YtPlaylistIdUrlParseError;

  fn try_from(yt_url: &Url) -> Result<Self, Self::Error> {
    let host = yt_url
      .host_str()
      .ok_or(YtPlaylistIdUrlParseError::MissingHost)?
      .to_ascii_lowercase();

    if !is_youtube_host(&host) {
      return Err(YtPlaylistIdUrlParseError::UnsupportedHost { host });
    }

    // The `list` query parameter carries the playlist ID in every relevant URL form:
    //
    // https://www.youtube.com/playlist?list=PLAYLIST_ID
    // https://www.youtube.com/watch?v=VIDEO_ID&list=PLAYLIST_ID
    // https://music.youtube.com/playlist?list=PLAYLIST_ID
    // https://youtu.be/VIDEO_ID?list=PLAYLIST_ID
    // https://www.youtube.com/embed/videoseries?list=PLAYLIST_ID
    if let Some((_, list)) = yt_url.query_pairs().find(|(k, _)| k.as_ref() == "list") {
      return validate_playlist_id(list.as_ref());
    }

    Err(YtPlaylistIdUrlParseError::MissingPlaylistId)
  }
}

fn validate_playlist_id(id: &str) -> Result<YtPlaylistId, YtPlaylistIdUrlParseError> {
  if id.is_empty() {
    return Err(YtPlaylistIdUrlParseError::MissingPlaylistId);
  }
  Ok(YtPlaylistId::from(id.to_owned()))
}

#[must_use]
pub fn is_youtube_host(host: &str) -> bool {
  host == "youtu.be"
    || host == "youtube.com"
    || host.ends_with(".youtube.com")
    || host == "youtube-nocookie.com"
    || host.ends_with(".youtube-nocookie.com")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse(url: &str) -> Result<YtPlaylistId, YtPlaylistIdUrlParseError> {
    let url = Url::parse(url).expect("test URLs are valid");
    YtPlaylistId::try_from(&url)
  }

  #[test]
  fn parses_playlist_query_param_in_all_url_forms() {
    let expected = YtPlaylistId::from("PL1234567890abcdef-_".to_owned());
    for url in [
      "https://www.youtube.com/playlist?list=PL1234567890abcdef-_",
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1234567890abcdef-_",
      "https://music.youtube.com/playlist?list=PL1234567890abcdef-_",
      "https://youtu.be/dQw4w9WgXcQ?list=PL1234567890abcdef-_",
      "https://www.youtube-nocookie.com/embed/videoseries?list=PL1234567890abcdef-_",
    ] {
      assert_eq!(parse(url).unwrap(), expected);
    }
  }

  #[test]
  fn parses_mixes_and_special_lists() {
    for list in [
      "RDdQw4w9WgXcQ",
      "RDMM",
      "OLAK5uy_kExyI6SiHfFm6arUiuphPfB5Yui2TlyC",
      "WL",
      "LL",
    ] {
      let url = format!("https://www.youtube.com/playlist?list={list}");
      assert!(parse(&url).is_ok());
    }
  }

  #[test]
  fn rejects_missing_invalid_and_unsupported() {
    assert!(matches!(
      parse("https://www.youtube.com/playlist"),
      Err(YtPlaylistIdUrlParseError::MissingPlaylistId)
    ));
    assert!(matches!(
      parse("https://www.youtube.com/playlist?list="),
      Err(YtPlaylistIdUrlParseError::MissingPlaylistId)
    ));
    assert!(matches!(
      parse("https://example.com/playlist?list=PL1234567890abcdef-_"),
      Err(YtPlaylistIdUrlParseError::UnsupportedHost { .. })
    ));
  }

  #[test]
  fn to_url_round_trips() {
    let id = YtPlaylistId::from("PL1234567890abcdef-_".to_owned());
    assert_eq!(
      id.to_url().as_str(),
      "https://www.youtube.com/playlist?list=PL1234567890abcdef-_"
    );
    assert_eq!(parse(id.to_url().as_str()).unwrap(), id);
  }
}
