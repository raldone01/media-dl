use std::{
  error::Error,
  fs,
  path::{Path, PathBuf},
  time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use lofty::{
  config::WriteOptions,
  file::{AudioFile as _, FileType, TaggedFile, TaggedFileExt as _},
  read_from_path,
  tag::Tag,
};
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatchTagError<E: Error> {
  #[error("No primary tag to patch")]
  NoPrimaryTag,
  #[error("Error while patching: {0}")]
  PatchError(#[from] E),
}

pub struct TaggedFileWithPath {
  file_path: PathBuf,
  // BoundTaggedFile exists but we can't get access to file_path for logging errors and warnings so we roll our own.
  tagged_file: TaggedFile,
}

impl TaggedFileWithPath {
  #[must_use]
  pub fn open(file_path: &Path) -> Result<Self> {
    let tagged_file = read_from_path(file_path)
      .with_context(|| format!("failed to open {}", file_path.display()))?;
    Ok(Self {
      file_path: file_path.to_owned(),
      tagged_file,
    })
  }

  #[must_use]
  pub fn file_path(&self) -> &Path {
    &self.file_path
  }

  pub fn patch_tag<E: Error>(
    &mut self,
    patch_fn: impl FnOnce(Tag) -> Result<Tag, E>,
  ) -> Result<(), PatchTagError<E>> {
    let tagged_file = &mut self.tagged_file;
    // Take the primary tag out so we can round-trip it through the
    // format-specific type to add the custom field.
    let original_tag = match tagged_file.primary_tag() {
      Some(tag) => tag,
      None => {
        return Err(PatchTagError::NoPrimaryTag);
      },
    };
    let updated_tag = patch_fn(original_tag.clone())?;
    tagged_file.insert_tag(updated_tag.clone());
    Ok(())
  }

  #[must_use]
  pub fn tag(&self) -> Option<&Tag> {
    self.tagged_file.primary_tag()
  }

  #[must_use]
  pub fn duration(&self) -> Duration {
    self.tagged_file.properties().duration()
  }

  #[must_use]
  pub fn file_type(&self) -> FileType {
    self.tagged_file.file_type()
  }

  pub fn overwrite(&self, temp_dir: &Path) -> Result<()> {
    let file_path = &self.file_path;
    let temp_file = NamedTempFile::new_in(temp_dir)
      .with_context(|| format!("failed to create temp file in {temp_dir:?}"))?;

    fs::copy(file_path, &temp_file)?;

    self
      .tagged_file
      .save_to_path(&temp_file, WriteOptions::default())
      .with_context(|| {
        format!(
          "failed to write tags to tempfile {}",
          temp_file.path().display()
        )
      })?;

    temp_file
      .as_file()
      .sync_all()
      .with_context(|| format!("failed to fsync {:?}", temp_file.path()))?;

    temp_file.persist(file_path).map_err(|e| {
      anyhow!(e.error).context(format!("failed to persist temp file to {file_path:?}"))
    })?;

    Ok(())
  }
}
