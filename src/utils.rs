use std::path::Path;

use anyhow::{Context as _, Result};

pub fn file_name_as_str(file: &Path) -> Result<String> {
  file
    .file_name()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .context("failed to convert path to str")
}

pub fn file_ext(file: &Path) -> Result<String> {
  file
    .extension()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .context("audio file has no extension")
}
