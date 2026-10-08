use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::{path::PathBuf, process::Command};
use url::Url;

pub enum YtItem {
  PlaylistInfo(PlaylistInfo),
  SongInfo(SongInfo),
}

#[derive(Debug, Deserialize)]
pub struct PlaylistInfo {
  pub id: String,
  pub title: String,
  pub entries: Vec<SongInfo>,
  pub playlist_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SongInfo {
  pub title: String,
  pub artist: String,
  pub album: String,
  pub duration: usize,
  pub id: String,
  // Playlist entries carry `url`; a single-video dump carries `webpage_url`.
  #[serde(alias = "webpage_url")]
  pub url: Url,
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
