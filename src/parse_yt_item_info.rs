use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::{fmt::Display, path::PathBuf, process::Command};
use url::Url;

use crate::{utils::filesystem_safe_file_name, yt_id::YtId};

#[derive(Clone)]
pub enum YtItem {
  PlaylistInfo(PlaylistInfo),
  SongInfo(SongInfo),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistInfo {
  pub id: YtId,
  pub title: String,
  pub entries: Vec<SongInfo>,
  pub playlist_count: usize,
}

impl PlaylistInfo {
  #[must_use]
  pub fn filesystem_safe_file_name(&self) -> PathBuf {
    let name = filesystem_safe_file_name(&self.title);

    PathBuf::from(if name.is_empty() {
      self.id.to_string()
    } else {
      name
    })
  }
}

impl Display for PlaylistInfo {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Playlist \"{}\" ({}) https://youtu.be/{}",
      self.title,
      self.entries.len(),
      self.id
    )
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SongInfo {
  pub title: String,
  pub artist: String,
  pub album: String,
  pub duration: usize,
  pub id: YtId,
  // Playlist entries carry `url`; a single-video dump carries `webpage_url`.
  #[serde(alias = "webpage_url")]
  pub url: Url,
}

impl Display for SongInfo {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Song \"{} - {}\" ({:02}:{:02}) https://youtu.be/{}",
      self.title,
      self.artist,
      self.duration / 60,
      self.duration % 60,
      self.id
    )
  }
}

pub fn download_yt_item_info(url: &Url) -> Result<YtItem> {
  let output = Command::new("yt-dlp")
    .args([
      "--flat-playlist",
      // Prefer the playlist if the url is for a video in a playlist.
      "--yes-playlist",
      "-J",
      url.as_str(),
    ])
    .output()?;

  if !output.status.success() {
    return Err(anyhow!(
      "yt-dlp failed: {}",
      String::from_utf8_lossy(&output.stderr)
    ));
  }

  // Parse as a generic value first so we can sniff the item kind.
  let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;

  // Playlists are emitted with `"_type": "playlist"`; single videos don't
  // have that marker at the top level (they have `"_type": "video"` in
  // nested fields only). Fall back to song parsing for anything else.
  match value.get("_type").and_then(|v| v.as_str()) {
    Some("playlist") => {
      let info: PlaylistInfo = serde_json::from_value(value)?;
      Ok(YtItem::PlaylistInfo(info))
    },
    _ => {
      let info: SongInfo = serde_json::from_value(value)?;
      Ok(YtItem::SongInfo(info))
    },
  }
}
