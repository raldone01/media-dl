use std::{fs, path::Path};

use anyhow::Context as _;

const CACHEDIR_TAG_FILE_NAME: &str = "CACHEDIR.TAG";
const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

pub enum CachedirState {
  IsCacheDir,
  NormalDir,
}

/// Checks if the given directory is a `CACHEDIR.TAG` directory.
/// Optionally verifies the app name.
#[must_use]
pub fn dir_is_cache_dir(
  cache_dir_file_path: &Path,
  app_name: Option<&str>,
) -> anyhow::Result<CachedirState> {
  let tag_path = cache_dir_file_path.join(CACHEDIR_TAG_FILE_NAME);

  let contents = match fs::read_to_string(&tag_path) {
    Ok(contents) => contents,
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(CachedirState::NormalDir),
    Err(err) => {
      return Err(err).with_context(|| {
        format!(
          "failed to read cache directory tag at {}",
          tag_path.display()
        )
      });
    },
  };

  let mut lines = contents.lines();
  if lines.next() != Some(CACHEDIR_TAG_SIGNATURE) {
    return Ok(CachedirState::NormalDir);
  }

  if let Some(app_name) = app_name {
    let expected = format!("# This file is a cache directory tag created by {app_name}.");
    if !lines.any(|line| line == expected.as_str()) {
      return Ok(CachedirState::NormalDir);
    }
  }

  Ok(CachedirState::IsCacheDir)
}

/// Marks the given directory as a cache directory by writing a `CACHEDIR.TAG` file.
#[must_use]
pub fn mark_dir_as_cache_dir(
  to_be_cache_dir_file_path: &Path,
  app_name: Option<&str>,
) -> anyhow::Result<()> {
  let tag_path = to_be_cache_dir_file_path.join(CACHEDIR_TAG_FILE_NAME);

  let mut contents = String::new();
  contents.push_str(CACHEDIR_TAG_SIGNATURE);
  contents.push('\n');

  if let Some(app_name) = app_name {
    contents.push_str(&format!(
      "# This file is a cache directory tag created by {app_name}.\n"
    ));
  }

  contents.push_str("# For information about cache directory tags, see:\n");
  contents.push_str("#\thttps://bford.info/cachedir/\n");

  fs::write(&tag_path, contents).with_context(|| {
    format!(
      "failed to write cache directory tag at {}",
      tag_path.display()
    )
  })?;

  Ok(())
}
