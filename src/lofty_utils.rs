use std::{
  error::Error,
  path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};
use lofty::{
  config::WriteOptions,
  file::{AudioFile as _, TaggedFile, TaggedFileExt as _},
  probe::Probe,
  tag::Tag,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatchTagError<E: Error> {
  #[error("No primary tag to patch")]
  NoPrimaryTag,
  #[error("Error while patching: {0}")]
  PatchError(#[from] E),
}

pub struct TaggedFileWithPath {
  file_path: Option<PathBuf>,
  // BoundTaggedFile exists but we can't get access to file_path for logging errors and warnings so we roll our own.
  tagged_file: TaggedFile,
}

impl TaggedFileWithPath {
  #[must_use]
  pub fn open(file_path: &Path) -> Result<Self> {
    let tagged_file = Probe::open(file_path)
      .with_context(|| format!("failed to open {}", file_path.display()))?
      .read()
      .with_context(|| format!("failed to read tags from {}", file_path.display()))?;
    Ok(Self {
      file_path: Some(file_path.to_owned()),
      tagged_file,
    })
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

  pub fn tag(&self) -> Option<&Tag> {
    self.tagged_file.primary_tag()
  }

  pub fn overwrite(&self) -> Result<()> {
    let Some(file_path) = self.file_path.as_deref() else {
      bail!("No file_path associated with the TaggedFile.");
    };
    self
      .tagged_file
      .save_to_path(file_path, WriteOptions::default())
      .with_context(|| format!("failed to write tags to {}", file_path.display()))
  }
}
