use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::{fmt::Display, path::PathBuf, process::Command};
use url::Url;

use crate::{fs_utils::filesystem_safe_file_name, yt_id::YtMediaId, yt_playlist_id::YtPlaylistId};

#[derive(Debug, Clone, Deserialize)]
pub struct YtPlaylistInfo {
  pub id: YtMediaId,
  pub title: String,
  pub entries: Vec<YtSongInfo>,
  pub playlist_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct YtSongInfo {
  pub title: String,
  pub artist: Option<String>,
  pub uploader: String,
  pub album: Option<String>,
  pub duration: usize,
  pub id: YtMediaId,
  pub url: Url,
}

impl YtPlaylistInfo {
  #[must_use]
  pub fn download_from_yt(yt_playlist_id: &YtPlaylistId) -> Result<Self> {
    let output = Command::new("yt-dlp")
      .args([
        "--flat-playlist",
        // Prefer the playlist if the url is for a video in a playlist.
        "--yes-playlist",
        "-J",
        yt_playlist_id.to_url().as_str(),
      ])
      .output()?;

    if !output.status.success() {
      return Err(anyhow!(
        "yt-dlp failed: {}",
        String::from_utf8_lossy(&output.stderr)
      ));
    }

    //let value_debug_str = String::from_utf8(output.stdout.clone()).unwrap();

    let playlist_info = serde_json::from_slice::<Self>(&output.stdout)?;
    Ok(playlist_info)
  }

  #[must_use]
  pub fn filesystem_safe_file_name_without_ext(&self) -> PathBuf {
    let name = filesystem_safe_file_name(&self.title);

    PathBuf::from(if name.is_empty() {
      self.id.to_string()
    } else {
      name
    })
  }
}

impl Display for YtPlaylistInfo {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Playlist \"{}\" ({}) https://youtube.com/playlist?list={}",
      self.title,
      self.entries.len(),
      self.id
    )
  }
}
