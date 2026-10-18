use std::{
  ffi::OsStr,
  fs::{self, File, OpenOptions},
  io::{self, Write},
  path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, anyhow};
use tempfile::NamedTempFile;

pub fn file_name_as_str(file: &Path) -> Result<String> {
  file
    .file_name()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .context("failed to convert path to str")
}

/// Returns the file extension without the dot.
pub fn file_ext(file_path: &Path) -> Result<String> {
  file_path
    .extension()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .with_context(|| format!("File {} has no extension", file_path.display()))
}

#[must_use]
pub fn filesystem_safe_file_name(name: &str) -> String {
  name
    .chars()
    .map(|c| match c {
      '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
      c if c.is_control() => '_',
      c => c,
    })
    .collect::<String>()
    .trim()
    .trim_matches('.')
    .to_owned()
}

/// Atomically finds an unused path based on `file_path`, trying `file_path`, then
/// `file_path` with `_1`, `_2`, ... appended before the extension, and creates
/// an empty file there to *reserve* it. (race free)
fn create_unique_file(file_path: &Path) -> io::Result<(PathBuf, File)> {
  let mut candidate = file_path.to_path_buf();
  let mut n: u64 = 0;

  loop {
    match OpenOptions::new()
      .write(true)
      .create_new(true) // O_EXCL on Unix, CREATE_NEW on Windows: atomic test-and-create
      .open(&candidate)
    {
      Ok(file) => return Ok((candidate, file)),
      Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
        n += 1;
        candidate = numbered_path(file_path, n);
      },
      Err(e) => return Err(e),
    }
  }
}

/// Builds `<stem>_<n><.ext>` next to `file_path`.
#[must_use]
fn numbered_path(file_path: &Path, n: u64) -> PathBuf {
  let stem = file_path
    .file_stem()
    .and_then(OsStr::to_str)
    .unwrap_or("file");
  let name = match file_path.extension().and_then(OsStr::to_str) {
    Some(ext) => format!("{stem}_{n}.{ext}"),
    None => format!("{stem}_{n}"),
  };
  file_path.with_file_name(name)
}

/// Renames `original_file_path` to `renamed_file_path`. If a file already
/// exists at `renamed_file_path`, a numeric suffix (`_1`, `_2`, ...) is
/// appended until a free name is found. (race free)
///
/// - If both paths are given, `original_file_path` is moved onto the
///   resolved unique path.
/// - If `original_file_path` is `None`, there is nothing to move; the
///   unique name is still reserved on disk (as an empty file) and
///   returned, so the caller gets the same TOCTOU-free guarantee when
///   they go on to create that file themselves.
/// - If `renamed_file_path` is `None`, there's no rename target: the
///   original path (if any) is returned unchanged.
/// - If neither is given, an error is returned.
pub fn smart_rename(
  original_file_path: Option<impl AsRef<Path>>,
  renamed_file_path: Option<impl AsRef<Path>>,
) -> Result<PathBuf> {
  let renamed_file_path = match renamed_file_path {
    Some(p) => p,
    None => {
      return original_file_path
        .map(|p| p.as_ref().to_path_buf())
        .ok_or_else(|| {
          anyhow!(
            "smart_rename: at least one of original_file_path or renamed_file_path must be set"
          )
        });
    },
  };
  let renamed_file_path = renamed_file_path.as_ref();

  let original_file_path = match original_file_path {
    Some(p) => p,
    None => {
      // Nothing to move - just reserve a unique name at/near renamed_file_path.
      let (reserved, _file) = create_unique_file(renamed_file_path)
        .with_context(|| format!("failed to reserve a unique path near {renamed_file_path:?}"))?;
      return Ok(reserved);
    },
  };
  let original_file_path = original_file_path.as_ref();

  if original_file_path == renamed_file_path {
    return Ok(original_file_path.to_path_buf());
  }

  // Atomically reserve a free slot at/near renamed_file_path...
  let (target, placeholder) = create_unique_file(renamed_file_path)
    .with_context(|| format!("failed to reserve a unique path near {renamed_file_path:?}"))?;
  drop(placeholder); // close it before we overwrite it via rename

  // ...then move the real file onto it. fs::rename replaces the
  // (empty, ours) placeholder atomically on both Unix and Windows.
  fs::rename(original_file_path, &target)
    .with_context(|| format!("failed to rename {original_file_path:?} to {target:?}"))?;

  Ok(target)
}

/// Safely overwrites `file_path`: A reader can never observe a partially-written `file_path`.
///
/// `temp_dir` must exist and be on the same filesystem as `file_path`.
pub fn safe_overwrite(
  file_path: &Path,
  contents_fn: impl FnOnce(&mut dyn Write) -> Result<()>,
  temp_dir: &Path,
) -> Result<()> {
  let mut temp_file = NamedTempFile::new_in(temp_dir)
    .with_context(|| format!("failed to create temp file in {temp_dir:?}"))?;

  contents_fn(&mut temp_file as &mut dyn Write)
    .with_context(|| format!("failed to write contents to {:?}", temp_file.path()))?;
  temp_file
    .as_file()
    .sync_all()
    .with_context(|| format!("failed to fsync {:?}", temp_file.path()))?;

  temp_file.persist(file_path).map_err(|e| {
    anyhow!(e.error).context(format!("failed to persist temp file to {file_path:?}"))
  })?;

  Ok(())
}
