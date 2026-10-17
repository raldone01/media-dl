use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::process::Command;

use crate::{yt::YtMediaId, yt::YtPlaylistId};

#[derive(Debug, Clone, Deserialize)]
pub struct YtPlaylistMeta {
  pub title: String,
  pub uploader: String,
  pub entries: Vec<YtPlaylistEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct YtPlaylistEntry {
  pub id: YtMediaId,
}

impl YtPlaylistMeta {
  #[must_use]
  pub fn download_playlist_meta(yt_playlist_id: &YtPlaylistId) -> Result<Self> {
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
}
