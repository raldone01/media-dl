use std::path::Path;

use anyhow::{Context as _, Result};

pub fn file_ext(file: &Path) -> Result<String> {
  Ok(
    file
      .extension()
      .and_then(|s| s.to_str())
      .map(str::to_ascii_lowercase)
      .context("audio file has no extension")?,
  )
}
