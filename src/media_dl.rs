use std::{
  collections::BTreeSet,
  path::PathBuf,
  sync::atomic::{self, AtomicBool},
};

use anyhow::Context as _;
use url::Url;

use crate::{
  lyrics::LrclibClient,
  media_library::{MediaLibrary, MimeType},
};

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

pub struct MediaDl {
  audio_urls: BTreeSet<Url>,
  default_audio_download_format: MimeType,

  video_urls: BTreeSet<Url>,
  default_video_download_format: MimeType,

  media_library_dir: PathBuf,
  cookies: PathBuf,
  no_network: bool,
  should_exit: AtomicBool,
}

impl MediaDl {
  #[must_use]
  pub fn new(
    audio_urls: BTreeSet<Url>,
    default_audio_download_format: MimeType,
    video_urls: BTreeSet<Url>,
    default_video_download_format: MimeType,
    media_library_dir: PathBuf,
    cookies: PathBuf,
    no_network: bool,
  ) -> Self {
    Self {
      audio_urls,
      default_audio_download_format,
      video_urls,
      default_video_download_format,
      media_library_dir,
      cookies,
      no_network,
      should_exit: AtomicBool::new(false),
    }
  }

  pub fn main_loop(&self) -> anyhow::Result<()> {
    let lrclib = LrclibClient::new(USER_AGENT).context("Failed to initialize LRCLIB client")?;
    let mut media_library =
      MediaLibrary::new(self.media_library_dir.clone(), &lrclib, &self.should_exit)?;
    media_library.add_urls(self.audio_urls.iter(), self.default_audio_download_format)?;
    if !self.no_network {
      media_library.download(self.default_audio_download_format)?;
    }
    media_library.write()?;
    Ok(())
  }

  pub fn set_abort_token(&self) {
    self.should_exit.store(true, atomic::Ordering::Relaxed);
  }
}
