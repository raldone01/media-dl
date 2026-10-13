use std::collections::{HashMap, hash_map};

use url::Url;

use crate::parse_yt_item_info::{YtItem, download_yt_item_info};

#[derive(Clone)]
pub enum UrlInfo {
  Yt(YtItem),
}

#[derive(Default)]
pub struct UrlDownloader {
  info_cache: HashMap<Url, UrlInfo>,
}

impl UrlDownloader {
  #[must_use]
  pub fn new() -> Self {
    Default::default()
  }

  pub fn download_info(&mut self, url: &Url) -> Result<&UrlInfo, anyhow::Error> {
    match self.info_cache.entry(url.clone()) {
      hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
      hash_map::Entry::Vacant(entry) => {
        let item_info = UrlInfo::Yt(download_yt_item_info(url)?);
        Ok(entry.insert(item_info))
      },
    }
  }
}
