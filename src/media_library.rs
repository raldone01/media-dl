//! TODO: keep dirty content flags for only rewriting playlists we changed

use core::fmt;
use std::io::BufReader;
use std::{
  borrow::Cow,
  collections::{BTreeSet, HashMap},
  ffi::OsStr,
  fs,
  path::PathBuf,
  sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context as _, anyhow, bail};
use jwalk::WalkDir;
use lofty::tag::Accessor;
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use thiserror::Error;
use tracing::{error, info, trace};
use url::Url;

use crate::absolute_file_path::AbsoluteFilePath;
use crate::fs_utils::write_or_rename;
use crate::{
  AUDIO_FILE_EXTENSIONS,
  download_playlist_info::YtPlaylistInfo,
  embedded_lyrics::LyricsPresence,
  embedded_yt_id::yt_id_from_tag,
  fs_utils::{file_ext, file_name_as_str, filesystem_safe_file_name},
  lofty_utils::TaggedFileWithPath,
  m3u8::M3U8Playlist,
  resource_id::ResourceId,
};

#[derive(Debug)]
struct Cancelled;

struct SongFile {
  original_file_path: PathBuf,
  renamed_file_path: Option<PathBuf>,
  resource_id: ResourceId,
  lyrics_presence: LyricsPresence,
  title: Option<String>,
  artist: Option<String>,
  album: Option<String>,
}

impl SongFile {
  fn new(file_path: PathBuf) -> anyhow::Result<Self> {
    let tagged_file = TaggedFileWithPath::open(&file_path)?;
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
    Ok(Self {
      original_file_path: file_path,
      renamed_file_path: None,
      resource_id,
      lyrics_presence,
      title,
      artist,
      album,
    })
  }
}

impl fmt::Display for SongFile {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    write!(
      f,
      "Song \"{title}{artist}\" {}",
      self.resource_id.to_url().to_string(),
    )
  }
}

struct PlaylistFile {
  original_file_path: Option<PathBuf>,
  renamed_file_path: Option<PathBuf>,
  m3u8_playlist: M3U8Playlist,
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
      && let Some(resource_id) = self.m3u8_playlist.resource_id()
    {
      let resource_id_str = String::from(&resource_id);
      file_name = filesystem_safe_file_name(&resource_id_str);
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

  pub fn write(&self) -> anyhow::Result<()> {
    write_or_rename(
      self.original_file_path.as_ref(),
      self.renamed_file_path.as_ref(),
      true,
      |mut writer, file_path| {
        Ok(
          self.m3u8_playlist.serialize(
            &mut writer,
            &file_path
              .parent()
              .expect("Playlist file must have a parent"),
          )?,
        )
      },
    )?;
    Ok(())
  }
}

impl fmt::Display for PlaylistFile {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let file_path = self
      .original_file_path
      .as_ref()
      .or(self.renamed_file_path.as_ref())
      .map(|path| path.to_string_lossy())
      .unwrap_or(Cow::Borrowed("Unknown playlist file path"));
    let resource_id = self.m3u8_playlist.resource_id();
    write!(
      f,
      "Playlist \"{file_path}\" {}",
      resource_id
        .as_ref()
        .map(ResourceId::to_url)
        .as_ref()
        .map(Url::to_string)
        .unwrap_or_else(|| format!("{:?}", resource_id)),
    )
  }
}

enum ExistingFileEntry {
  Playlist(PlaylistFile),
  UntrackedPlaylist(PlaylistFile),
  Song(SongFile),
}

pub struct MediaLibrary {
  library_path: PathBuf,
  /// collects existing music files
  songs: Vec<SongFile>,
  /// collects existing m3u8 playlists
  playlists: Vec<PlaylistFile>,
  /// collects existing m3u8 playlists for marking untracked media files
  untracked_playlists: Vec<PlaylistFile>,
}

impl MediaLibrary {
  fn ingest_existing_file(
    entry_result: Result<jwalk::DirEntry<((), ())>, jwalk::Error>,
  ) -> anyhow::Result<Option<ExistingFileEntry>> {
    let entry = match entry_result {
      Ok(entry) => entry,
      Err(e) => {
        let file_path = e
          .path()
          .map_or_else(|| "Unknown path".to_string(), |p| p.display().to_string());
        bail!("Failed to read file entry while processing directory: {file_path}",);
      },
    };

    if !entry.file_type().is_file() {
      return Ok(None);
    }

    let file_path = entry.path();
    let file_name_str = file_name_as_str(&file_path)?;
    let ext = match file_ext(&file_path) {
      Ok(ext) => ext,
      Err(error) => {
        trace!(
            file_path = %file_path.display(),
            source = %error,
            "Ignoring file without file extension."
        );
        return Ok(None);
      },
    };

    if file_name_str.ends_with(".m3u8.media-dl") {
      if file_name_str.starts_with("untracked media")
        || file_name_str.starts_with("untracked_media")
      {
        return Ok(Some(ExistingFileEntry::UntrackedPlaylist(
          PlaylistFile::new_from_disk(file_path)?,
        )));
      }
      Ok(Some(ExistingFileEntry::Playlist(
        PlaylistFile::new_from_disk(file_path)?,
      )))
    } else if file_name_str.ends_with(".m3u8") {
      Ok(Some(ExistingFileEntry::Playlist(
        PlaylistFile::new_from_disk(file_path)?,
      )))
    } else if AUDIO_FILE_EXTENSIONS.contains(&ext.as_str()) {
      Ok(Some(ExistingFileEntry::Song(SongFile::new(file_path)?)))
    } else {
      Ok(None)
    }
  }

  #[must_use]
  pub fn new(library_path: PathBuf, abort_token: &AtomicBool) -> anyhow::Result<Self> {
    fs::create_dir_all(&library_path).context("Failed to create output folder")?;

    let file_entries = WalkDir::new(&library_path).max_depth(1).into_iter();

    let exiting_files: Vec<ExistingFileEntry> = match file_entries
      .par_bridge()
      .map(
        |entry_result| -> Result<Option<ExistingFileEntry>, Cancelled> {
          if abort_token.load(Ordering::Relaxed) {
            return Err(Cancelled);
          }

          match Self::ingest_existing_file(entry_result) {
            Ok(existing_file_entry) => Ok(existing_file_entry),
            Err(error) => {
              // TODO: collect errors
              error!(
                  source = %error,
                  "Failed to read file entry while processing directory",
              );
              Ok(None)
            },
          }
        },
      )
      // Short-circuits on the first Err; Rayon stops dispatching
      // new items to workers and the outstanding map() calls are dropped.
      .collect::<Result<Vec<_>, Cancelled>>()
    {
      Ok(entries) => entries.into_iter().flatten().collect(),
      Err(Cancelled) => Vec::new(), // or propagate / handle however you want
    };

    // collects existing music files
    let mut songs: Vec<SongFile> = Vec::new();
    // collects existing m3u8 playlists
    let mut playlists: Vec<PlaylistFile> = Vec::new();
    let mut untracked_playlists: Vec<PlaylistFile> = Vec::new();

    for entry in exiting_files {
      match entry {
        ExistingFileEntry::Playlist(p) => playlists.push(p),
        ExistingFileEntry::UntrackedPlaylist(p) => untracked_playlists.push(p),
        ExistingFileEntry::Song(s) => songs.push(s),
      }
    }

    // TODO: run reconcile_files_to_m3u8
    // This should copy the resource ids back into the m3u8 playlists from the files
    // and correct playlist file names
    // we should crash on inconsistencies

    Ok(Self {
      library_path,
      playlists,
      songs,
      untracked_playlists,
    })
  }

  fn untracked_playlist_mut(&mut self) -> &mut PlaylistFile {
    if self.untracked_playlists.is_empty() {
      let file_path = self.library_path.join("Untracked Media.m3u8.media-dl");
      self.untracked_playlists.push(PlaylistFile::new(file_path));
    }
    self
      .untracked_playlists
      .get_mut(0)
      .expect("pushed at least one item onto the vec")
  }

  fn song_file_by_resource_id(&self, resource_id: &ResourceId) -> Option<&SongFile> {
    let song = self
      .songs
      .iter()
      .find(|song| song.resource_id == *resource_id);
    song
  }

  fn add_url(&mut self, url: &Url, format: String) -> anyhow::Result<()> {
    let resource_id = ResourceId::from(url);
    let url = resource_id.to_url();
    let existing_song = self.song_file_by_resource_id(&resource_id);
    let url_info_string = existing_song
      .map(|existing_song| format!("{existing_song}"))
      .unwrap_or_else(|| url.to_string());
    match resource_id {
      ResourceId::YtMediaId(_) => {
        // add the song to untracked_playlists if not part of them already
        let untracked_playlist_entry = self
          .untracked_playlists
          .iter()
          .find(|item| item.m3u8_playlist.find(&resource_id).is_some());

        if untracked_playlist_entry.is_some() {
          // already listed on an untracked playlist
          info!("Skipped adding {url_info_string} as it is already untracked media!")
        } else {
          // not yet on the untracked playlists so we add it
          let untracked_playlist = self.untracked_playlist_mut();
          untracked_playlist
            .m3u8_playlist
            .add_media_entry(url.clone(), format);
          info!("Added {url_info_string} as untracked media!")
        }
      },
      ResourceId::YtPlaylistId(ref yt_playlist_id) => {
        // add a new playlist to playlists
        let playlist_is_already_part_of_playlist = self
          .playlists
          .iter()
          .find(|playlist| playlist.m3u8_playlist.resource_id().as_ref() == Some(&resource_id))
          .is_some();

        if !playlist_is_already_part_of_playlist {
          let mut m3u8_playlist = M3U8Playlist::default();
          m3u8_playlist.set_playlist_resource_id(resource_id.clone())?;

          let mut file_path = self
            .library_path
            .join(filesystem_safe_file_name(yt_playlist_id.as_str()));
          file_path.set_extension("m3u8");
          self.playlists.push(PlaylistFile::new(file_path));
        } else {
          info!(
            "Skipped adding youtube playlist {url_info_string} as it is already part of the media library!"
          )
        }
      },
      other_resource_id => {
        bail!("Failed to add {other_resource_id:?} to the music library!");
      },
    }
    Ok(())
  }

  pub fn add_urls<'a>(
    &mut self,
    urls: impl IntoIterator<Item = &'a Url>,
    format: &str,
  ) -> Result<(), AddUrlsError> {
    let mut url_errors = HashMap::new();
    for url in urls.into_iter() {
      if let Err(error) = self.add_url(url, format.to_string()) {
        url_errors.insert(url.to_owned(), error);
      }
    }
    if !url_errors.is_empty() {
      return Err(AddUrlsError { url_errors });
    }
    Ok(())
  }

  fn update_playlist_from_remote(playlist: &mut PlaylistFile, format: &str) -> anyhow::Result<()> {
    let resource_id = playlist.m3u8_playlist.resource_id();
    match resource_id {
      Some(ResourceId::YtPlaylistId(ref yt_playlist_id)) => {
        // query playlist information from yt
        let yt_playlist_info = YtPlaylistInfo::download_from_yt(yt_playlist_id)?;
        playlist.queue_rename(&yt_playlist_info.title)?;
        playlist.m3u8_playlist.update_playlist_from_remote(
          yt_playlist_info
            .entries
            .into_iter()
            .map(|item| ResourceId::YtMediaId(item.id)),
          &format,
        );
        Ok(())
      },
      Some(resource_id) => {
        // we have resource ids which we don't know how to download
        bail!(
          "The playlist {} is tracking an invalid playlist remote resource {}",
          playlist
            .original_file_path
            .as_ref()
            .or(playlist.renamed_file_path.as_ref())
            .expect("At least one fs path must be set.")
            .display(),
          resource_id.to_url().to_string()
        )
      },
      None => {
        // The playlist doesn't track a remote: no-op
        Ok(())
      },
    }
  }

  fn download_media_into_library(&mut self, resource_id: &ResourceId) -> anyhow::Result<()> {
    Ok(())
  }

  pub fn download(&mut self, format: &str) -> anyhow::Result<()> {
    // update the playlists from the remote
    for playlist in &mut self.playlists {
      if let Err(error) = Self::update_playlist_from_remote(playlist, format) {
        error!("Failed to download {playlist}: {error}");
      }
    }

    let mut media_to_download = BTreeSet::<ResourceId>::new();
    // collect all media items listed on playlists and the untracked lists
    for media_entry in self
      .playlists
      .iter()
      .chain(self.untracked_playlists.iter())
      .map(|playlist| playlist.m3u8_playlist.entries.iter())
      .flatten()
    {
      media_to_download.insert(media_entry.resource_id());
    }
    let total_media = media_to_download.len();
    // subtract the existing files from media_to_download and add them to media_to_check
    let mut media_to_check = BTreeSet::<ResourceId>::new();
    for existing_media in &self.songs {
      let removed = media_to_download.remove(&existing_media.resource_id);
      if removed {
        media_to_check.insert(existing_media.resource_id.clone());
      }
    }

    // download all remaining items (if desired)
    for resource_id in media_to_download {
      if let Err(error) = self.download_media_into_library(&resource_id) {
        error!(
          "Failed to download {}: {error}",
          resource_id.to_url().to_string()
        );
      }
    }
    // check metadata of all downloaded items (if desired)
    // run reconcile?
    Ok(())
  }

  pub fn write(&self) -> anyhow::Result<()> {
    for playlist in &self.playlists {
      playlist.write()?;
    }
    for untracked_playlist in &self.untracked_playlists {
      untracked_playlist.write()?;
    }
    Ok(())
  }
}

#[derive(Error, Debug)]
pub struct AddUrlsError {
  url_errors: HashMap<Url, anyhow::Error>,
}

impl fmt::Display for AddUrlsError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "failed to add {} URL(s)", self.url_errors.len())?;

    for (url, err) in &self.url_errors {
      write!(f, "\n  {url}: {err}")?;
    }

    Ok(())
  }
}
