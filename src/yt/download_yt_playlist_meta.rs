use anyhow::Result;
use serde::Deserialize;
use std::sync::atomic::AtomicBool;

use crate::{
  yt::{YtMediaId, YtPlaylistId},
  yt_dlp::YtDlp,
};

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
  pub fn download_playlist_meta(
    yt_playlist_id: &YtPlaylistId,
    abort_token: &AtomicBool,
  ) -> Result<Self> {
    let playlist_meta = YtDlp::new()
      .abort_token(abort_token)
      .args(["--flat-playlist", "--yes-playlist", "-J"])
      .arg(yt_playlist_id.to_url().as_str())
      .run_json()?;
    Ok(playlist_meta)
  }
}
