use std::{
  collections::BTreeSet,
  path::PathBuf,
  sync::atomic::{self, AtomicBool},
};

use anyhow::Context as _;
use url::Url;

use crate::{lyrics::LrclibClient, media_library::MediaLibrary};

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

pub struct MediaDl {
  output_folder: PathBuf,
  urls: BTreeSet<Url>,
  cookies: PathBuf,
  download_format: String,
  no_network: bool,
  should_exit: AtomicBool,
}

impl MediaDl {
  #[must_use]
  pub fn new(
    output_folder: PathBuf,
    urls: BTreeSet<Url>,
    cookies: PathBuf,
    download_format: String,
    no_network: bool,
  ) -> Self {
    Self {
      output_folder,
      urls,
      cookies,
      download_format,
      no_network,
      should_exit: AtomicBool::new(false),
    }
  }

  pub fn main_loop(&self) -> anyhow::Result<()> {
    let lrclib = LrclibClient::new(USER_AGENT).context("Failed to initialize LRCLIB client")?;
    let mut media_library =
      MediaLibrary::new(self.output_folder.clone(), &lrclib, &self.should_exit)?;
    media_library.add_urls(self.urls.iter(), &self.download_format)?;
    media_library.download(&self.download_format)?;
    media_library.write()?;
    Ok(())
  }

  pub fn set_abort_token(&self) {
    self.should_exit.store(true, atomic::Ordering::Relaxed);
  }
}
