use core::fmt;
use std::{
  borrow::Cow,
  ffi::OsStr,
  path::{Path, PathBuf},
  time::Duration,
};

use anyhow::{Context as _, bail};
use lofty::tag::Accessor as _;

use crate::{
  lyrics::{LrcLibError, LrclibClient, Lyrics, LyricsPresence, TrackQuery, embed_lyrics_tag},
  media_library::resource_id::ResourceId,
  utils::{
    AbsoluteFilePath, TaggedFileWithPath, file_ext, filesystem_safe_file_name, smart_rename,
  },
  yt::yt_id_from_tag,
};

pub enum MediaFile {
  Audio(AudioFile),
}

pub struct AudioFile {
  original_file_path: AbsoluteFilePath,
  renamed_file_path: Option<AbsoluteFilePath>,
  resource_id: ResourceId,
  lyrics_presence: LyricsPresence,
  title: Option<String>,
  artist: Option<String>,
  album: Option<String>,
  duration: Duration,
}

impl AudioFile {
  pub fn new_from_disk(file_path: PathBuf) -> anyhow::Result<Self> {
    let tagged_file = TaggedFileWithPath::open(&file_path)?;
    Self::new_from_tagged_file(tagged_file)
  }

  pub fn new_from_tagged_file(tagged_file: TaggedFileWithPath) -> anyhow::Result<Self> {
    let file_path = tagged_file.file_path();
    let Some(tag) = tagged_file.tag() else {
      bail!("No tag found in the file")
    };
    let resource_id = yt_id_from_tag(tag).map(|yt_id| ResourceId::YtMediaId(yt_id));
    let resource_id = match resource_id {
      Some(resource_id) => resource_id,
      None => {
        let Some(absolute_file_path) = AbsoluteFilePath::new(&file_path) else {
          bail!("{} is not a absolute path", file_path.display());
        };
        ResourceId::LocalFile(absolute_file_path)
      },
    };
    let lyrics_presence = LyricsPresence::from_tag(tag);
    let title = tag.title().map(Cow::into_owned);
    let artist = tag.artist().map(Cow::into_owned);
    let album = tag.album().map(Cow::into_owned);
    let duration = tagged_file.duration();

    let absolute_path = AbsoluteFilePath::new_with_cwd(file_path)?;

    Ok(Self {
      original_file_path: absolute_path,
      renamed_file_path: None,
      resource_id,
      lyrics_presence,
      title,
      artist,
      album,
      duration,
    })
  }

  #[must_use]
  fn pretty_name(&self) -> String {
    let title = self
      .title
      .as_ref()
      .map(|title| Cow::Borrowed(title.as_str()))
      .or_else(|| {
        self
          .original_file_path
          .file_name()
          .map(OsStr::to_string_lossy)
      })
      .unwrap_or_else(|| self.original_file_path.to_string_lossy());
    let artist = self
      .artist
      .as_deref()
      .map(|artist| format!(" - {artist}"))
      .unwrap_or_default();

    format!("{title}{artist}")
  }

  pub fn rename_if_needed(&mut self) -> anyhow::Result<()> {
    let file_ext = file_ext(&self.original_file_path)?;
    let pretty_name = self.pretty_name();
    let file_name = filesystem_safe_file_name(&format!("{pretty_name}.{file_ext}"));
    if file_name.is_empty() {
      return Ok(());
    }
    let new_file_path = self
      .original_file_path
      .parent()
      .expect("File must have a parent")
      .join(PathBuf::from(file_name));
    if new_file_path != *self.original_file_path {
      self.renamed_file_path =
        Some(AbsoluteFilePath::new(new_file_path).expect("We came from an absolute path"));
    }
    Ok(())
  }

  fn fetch_lyrics(&self, lrclib: &LrclibClient) -> Result<Option<Lyrics>, LrcLibError> {
    // get lyrics from lrclib
    let Some(title) = &self.title else {
      return Ok(None);
    };
    let mut query = TrackQuery::new(title);
    if let Some(artist) = &self.artist {
      query = query.artist(artist);
    }
    if let Some(album) = &self.album {
      query = query.album(album);
    }
    query = query.duration(self.duration.as_secs_f64());

    let lyrics = match lrclib.get(&query) {
      Ok(lyrics) => lyrics,
      Err(LrcLibError::NotFound) => {
        let results = lrclib.search(&query)?;

        let Some(lyrics) = results.into_iter().next() else {
          return Ok(None);
        };

        lyrics
      },
      Err(error) => return Err(error),
    };

    Ok(Some(lyrics))
  }

  pub fn download_and_embed_lyrics_if_needed(
    &mut self,
    lrclib: &LrclibClient,
    temp_dir: &Path,
  ) -> anyhow::Result<()> {
    // check if we need the lyrics
    if self.lyrics_presence.has_synced_lyrics {
      return Ok(());
    }

    // fetch new lyrics
    let Some(new_lyrics) = self
      .fetch_lyrics(lrclib)
      .context(format!("Failed to get lyrics for {self}"))?
    else {
      return Ok(());
    };

    // embed the lyrics into the file
    let mut tagged_file = TaggedFileWithPath::open(&self.original_file_path)?;
    tagged_file.patch_tag(embed_lyrics_tag(
      new_lyrics.plain_lyrics.as_ref().map(String::as_str),
      new_lyrics.synced_lyrics.as_ref().map(String::as_str),
    ))?;
    tagged_file.overwrite(&temp_dir)?;

    Ok(())
  }

  pub fn check_media_meta(&mut self, lrclib: &LrclibClient, temp_dir: &Path) -> anyhow::Result<()> {
    self.rename_if_needed()?;
    self.download_and_embed_lyrics_if_needed(lrclib, temp_dir)?;
    // TODO: hit the apis to get updated meta data
    Ok(())
  }

  #[must_use]
  pub fn final_file_path(&self) -> &AbsoluteFilePath {
    &self
      .renamed_file_path
      .as_ref()
      .unwrap_or(&self.original_file_path)
  }

  #[must_use]
  pub fn resource_id(&self) -> &ResourceId {
    &self.resource_id
  }

  pub fn write(&mut self) -> anyhow::Result<()> {
    self.original_file_path = AbsoluteFilePath::new(smart_rename(
      Some(self.original_file_path.as_path()),
      self.renamed_file_path.as_ref().map(|p| p.as_path()),
    )?)
    .expect("Was a absolute file path");
    self.renamed_file_path = None;
    Ok(())
  }
}

impl fmt::Display for AudioFile {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let pretty_name = self.pretty_name();

    write!(
      f,
      "Song \"{pretty_name}\" {}",
      self.resource_id.to_url().to_string(),
    )
  }
}
