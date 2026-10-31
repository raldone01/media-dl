use std::{
  borrow::Cow,
  fmt, fs,
  io::BufReader,
  path::{Path, PathBuf},
};

use anyhow::{Context as _, anyhow, bail};
use url::Url;

use crate::{
  media_library::{m3u8::M3U8Playlist, resource_id::PlaylistResourceId},
  utils::{file_ext, filesystem_safe_file_name, safe_overwrite, smart_rename},
};

pub struct PlaylistFile {
  original_file_path: Option<PathBuf>,
  renamed_file_path: Option<PathBuf>,
  pub m3u8_playlist: M3U8Playlist,
}

impl PlaylistFile {
  pub fn new_from_disk(file_path: PathBuf) -> anyhow::Result<Self> {
    let file = fs::File::open(&file_path)
      .with_context(|| format!("Failed to open: {}", file_path.display()))?;
    let mut buf_read = BufReader::new(&file);
    let playlist_dir = file_path
      .parent()
      .ok_or_else(|| anyhow!("File {} must have a parent", file_path.display()))?;
    let m3u8_playlist = M3U8Playlist::parse(&mut buf_read, &playlist_dir)?;
    Ok(Self {
      original_file_path: Some(file_path),
      renamed_file_path: None,
      m3u8_playlist,
    })
  }

  #[must_use]
  pub fn new(new_file_path: PathBuf) -> Self {
    Self {
      original_file_path: None,
      renamed_file_path: Some(new_file_path),
      m3u8_playlist: Default::default(),
    }
  }

  pub fn queue_rename(&mut self, new_playlist_name: &str) -> anyhow::Result<()> {
    let mut file_name = filesystem_safe_file_name(new_playlist_name);
    if file_name.is_empty()
      && let Some(playlist_id) = self.m3u8_playlist.playlist_id()
    {
      let playlist_id_str = playlist_id.to_string();
      file_name = filesystem_safe_file_name(&playlist_id_str);
    }
    if file_name.is_empty() {
      bail!("Failed to compute a file name for {}", self);
    }

    let mut current_file_path = self
      .renamed_file_path
      .as_ref()
      .or(self.original_file_path.as_ref())
      .expect("At least one file path must be set")
      .clone();
    let ext = file_ext(&current_file_path)?;
    current_file_path.set_file_name(format!("{file_name}.{ext}"));
    self.renamed_file_path = Some(current_file_path);
    Ok(())
  }

  #[must_use]
  pub fn final_file_path(&self) -> &Path {
    &self
      .renamed_file_path
      .as_ref()
      .or(self.original_file_path.as_ref())
      .expect("At least renamed_file_path or original_file_path must be Some")
  }

  pub fn write(&mut self, temp_dir: &Path) -> anyhow::Result<()> {
    self.original_file_path = Some(smart_rename(
      self.original_file_path.as_ref(),
      self.renamed_file_path.as_ref().map(|p| p.as_path()),
    )?);
    self.renamed_file_path = None;

    safe_overwrite(
      self.final_file_path(),
      |mut writer| {
        Ok(
          self.m3u8_playlist.serialize(
            &mut writer,
            &self
              .final_file_path()
              .parent()
              .expect("Playlist file must have a parent"),
          )?,
        )
      },
      temp_dir,
    )?;
    Ok(())
  }
}

impl fmt::Display for PlaylistFile {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let file_path = self
      .original_file_path
      .as_ref()
      .or(self.renamed_file_path.as_ref())
      .map(|path| path.to_string_lossy())
      .unwrap_or(Cow::Borrowed("Unknown playlist file path"));
    let resource_id = self.m3u8_playlist.playlist_id();
    write!(
      f,
      "Playlist \"{file_path}\" {}",
      resource_id
        .as_ref()
        .map(PlaylistResourceId::to_url)
        .as_ref()
        .map(Url::to_string)
        .unwrap_or_else(|| format!("{:?}", resource_id)),
    )
  }
}
