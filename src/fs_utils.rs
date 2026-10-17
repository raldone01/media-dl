use std::{
  fs::{self, File},
  io::{BufWriter, Write},
  path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};

pub fn file_name_as_str(file: &Path) -> Result<String> {
  file
    .file_name()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .context("failed to convert path to str")
}

/// Returns the file extension without the dot.
pub fn file_ext(file: &Path) -> Result<String> {
  file
    .extension()
    .and_then(|s| s.to_str())
    .map(str::to_ascii_lowercase)
    .context("file has no extension")
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

fn unique_path(path: &Path) -> PathBuf {
  let mut index = 0;
  let mut new_path = path.to_path_buf();

  while new_path.exists() {
    index += 1;
    let file_name = path
      .file_name()
      .map(|name| name.to_string_lossy())
      .unwrap_or_default();
    new_path = path.with_file_name(format!("{file_name}_{index}"));
  }

  new_path
}

/// This function renames the file from the `original_file_path` to the `new_file_path`.
/// If `new_file_path` already exists the path will be made unique by appending `_<number>`.
/// The final file path will be returned.
/// If both `original_file_path` and `new_file_path` are not provided this function errors.
/// If both `original_file_path` and `new_file_path` are the same, this function is a no-op if `dirty_contents` is false.
///
/// If `dirty_contents` is true, `contents_fn` will be invoked and written to the output file.
pub fn write_or_rename(
  original_file_path: Option<impl AsRef<Path>>,
  new_file_path: Option<impl AsRef<Path>>,
  dirty_contents: bool,
  contents_fn: impl FnOnce(&mut dyn Write, &Path) -> Result<()>,
) -> Result<PathBuf> {
  let original = original_file_path.as_ref().map(|path| path.as_ref());
  let new = new_file_path.as_ref().map(|path| path.as_ref());

  if original.is_none() && new.is_none() {
    bail!("Both the original_file_path and the new_file_path are None!")
  }

  let output = new.or(original).expect("checked above");

  if !dirty_contents {
    if let Some(original) = original {
      if original == output {
        return Ok(output.to_path_buf());
      }

      let final_path = if output.exists() {
        unique_path(output)
      } else {
        output.to_path_buf()
      };

      fs::rename(original, &final_path).with_context(|| {
        format!(
          "Failed to rename file {} to {}",
          original.display(),
          final_path.display()
        )
      })?;

      return Ok(final_path);
    }

    return Ok(output.to_path_buf());
  }

  let final_path = if let Some(original) = original {
    if original != output && output.exists() {
      unique_path(output)
    } else {
      output.to_path_buf()
    }
  } else if output.exists() {
    unique_path(output)
  } else {
    output.to_path_buf()
  };

  {
    let context = || format!("Failed to write file {}", final_path.display());
    let file = File::create(&final_path).with_context(context)?;
    let mut writer = BufWriter::new(file);

    contents_fn(&mut writer, &final_path).with_context(context)?;

    // BufWriter only flushes on drop or on explicit `flush`. Flush
    // explicitly so we can propagate errors (a dropped BufWriter
    // swallows I/O failures).
    writer.flush().with_context(context)?;
  }

  if let Some(original) = original {
    if original != final_path.as_path() {
      fs::remove_file(original)
        .with_context(|| format!("Failed to remove original file {}", original.display()))?;
    }
  }

  Ok(final_path)
}
