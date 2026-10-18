//! TODO: keep dirty content flags for only rewriting playlists we changed

use core::fmt;
use std::io::BufReader;
use std::path::Path;
use std::{
  borrow::Cow,
  collections::{BTreeSet, HashMap},
  fs,
  path::PathBuf,
  sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context as _, anyhow, bail};
use jwalk::WalkDir;
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use tempfile::TempDir;
use thiserror::Error;
use tracing::{error, info, trace};
use url::Url;

use crate::CRATE_NAME;
use crate::lyrics::LrclibClient;
use crate::media_library::AudioFile;
use crate::media_library::m3u8::{M3U8Playlist, MediaSource};
use crate::media_library::resource_id::ResourceId;
use crate::utils::{
  file_ext, file_name_as_str, filesystem_safe_file_name, mark_dir_as_cache_dir, safe_overwrite,
  smart_rename,
};
use crate::yt::{YtPlaylistMeta, download_yt_audio};

const AUDIO_FILE_EXTENSIONS: &[&str] = &["mp3", "m4a", "flac", "opus", "ogg", "vorbis", "wav"];

#[derive(Debug)]
struct Cancelled;

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
  Song(AudioFile),
}

pub struct MediaLibrary<'a> {
  library_path: PathBuf,
  /// collects existing music files
  songs: Vec<AudioFile>,
  /// collects existing m3u8 playlists
  playlists: Vec<PlaylistFile>,
  /// collects existing m3u8 playlists for marking untracked media files
  untracked_playlists: Vec<PlaylistFile>,
  lrclib: &'a LrclibClient,
  abort_token: &'a AtomicBool,
  temp_dir: TempDir,
}

impl<'a> MediaLibrary<'a> {
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
      Ok(Some(ExistingFileEntry::Song(AudioFile::new_from_disk(
        file_path,
      )?)))
    } else {
      Ok(None)
    }
  }

  #[must_use]
  pub fn new(
    library_path: PathBuf,
    lrclib: &'a LrclibClient,
    abort_token: &'a AtomicBool,
  ) -> anyhow::Result<Self> {
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
    let mut songs: Vec<AudioFile> = Vec::new();
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

    let temp_dir = TempDir::new_in(&library_path)?;
    mark_dir_as_cache_dir(temp_dir.path(), Some(CRATE_NAME))?;

    let mut selv = Self {
      library_path,
      playlists,
      songs,
      untracked_playlists,
      lrclib,
      abort_token,
      temp_dir,
    };

    selv.startup_playlist_consistency()?;

    Ok(selv)
  }

  fn startup_playlist_consistency(&mut self) -> anyhow::Result<()> {
    for playlist in self.playlists.iter_mut() {
      for media_item in playlist.m3u8_playlist.entries.iter_mut() {
        let media_entry_pointed_resource_id = match &media_item.media_source {
          MediaSource::File(file_path) => self
            .songs
            .iter()
            .find(|song| song.final_file_path() == file_path)
            .map(|song| Cow::Borrowed(song.resource_id())),
          MediaSource::Url(remote_url) => Some(Cow::Owned(ResourceId::from(remote_url))),
        };
        let remote_resource_id = media_item.remote_resource_id();
        match (media_entry_pointed_resource_id, remote_resource_id) {
          (Some(media_entry_pointed_resource_id), Some(remote_resource_id)) => {
            if *media_entry_pointed_resource_id != remote_resource_id {
              bail!(
                "Inconsistency in playlist {playlist}: entry pointed resource id {}, entry remote resource id tag {}",
                media_entry_pointed_resource_id.to_url().to_string(),
                remote_resource_id.to_url().to_string()
              )
            }
          },
          (Some(media_entry_pointed_resource_id), None) => {
            media_item.set_remote_resource_id(&media_entry_pointed_resource_id);
          },
          (None, Some(_remote_resource_id)) => {
            // fine, just means we didn't download a remote media entry yet.
          },
          (None, None) => {
            // broken path, checked later for the final report
          },
        }
      }
    }
    Ok(())
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

  fn song_file_by_resource_id(&self, resource_id: &ResourceId) -> Option<&AudioFile> {
    let song = self
      .songs
      .iter()
      .find(|song| song.resource_id() == resource_id);
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
          let mut file_path = self
            .library_path
            .join(filesystem_safe_file_name(yt_playlist_id.as_str()));
          file_path.set_extension("m3u8");
          let mut playlist = PlaylistFile::new(file_path);
          playlist
            .m3u8_playlist
            .set_playlist_resource_id(resource_id.clone())?;
          self.playlists.push(playlist);
        } else {
          info!(
            "Skipped adding youtube playlist {url_info_string} as it is already part of the media library!"
          )
        }
      },
      _ => {
        bail!(
          "Failed to add {} to the music library!",
          resource_id.to_url().to_string()
        );
      },
    }
    Ok(())
  }

  pub fn add_urls<'b>(
    &mut self,
    urls: impl IntoIterator<Item = &'b Url>,
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

  fn update_playlist_from_remote(
    playlist: &mut PlaylistFile,
    format: &str,
    abort_token: &AtomicBool,
  ) -> anyhow::Result<()> {
    let resource_id = playlist.m3u8_playlist.resource_id();
    match resource_id {
      Some(ResourceId::YtPlaylistId(ref yt_playlist_id)) => {
        // query playlist information from yt
        let yt_playlist_info = YtPlaylistMeta::download_playlist_meta(yt_playlist_id, abort_token)?;
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

  fn download_media_into_library(
    &mut self,
    resource_id: &ResourceId,
    format: &str,
  ) -> anyhow::Result<()> {
    match resource_id {
      ResourceId::YtMediaId(yt_media_id) => {
        info!("Downloading {}", resource_id.to_url().to_string());
        let mut audio_file = download_yt_audio(
          &self.library_path,
          yt_media_id,
          &format,
          self.abort_token,
          &self.temp_dir.path(),
        )?;
        audio_file.rename_if_needed()?;
        if let Err(error) =
          audio_file.download_and_embed_lyrics_if_needed(&self.lrclib, &self.temp_dir.path())
        {
          error!("Failed to fetch lyrics for {audio_file}: {error:#}");
        }
        self.songs.push(audio_file);
      },
      ResourceId::LocalFile(..) => {
        // no-op
      },
      ResourceId::YtPlaylistId(..) => {
        bail!(
          "Playlists are not accepted when downloading media items: {}",
          resource_id.to_url().to_string()
        )
      },
      _ => {
        bail!(
          "Don't know how to download media item: {}",
          resource_id.to_url().to_string()
        )
      },
    }
    Ok(())
  }

  pub fn download(&mut self, format: &str) -> anyhow::Result<()> {
    // update the playlists from the remote
    for playlist in &mut self.playlists {
      if let Err(error) = Self::update_playlist_from_remote(playlist, format, self.abort_token) {
        error!("Failed to download {playlist}: {error:#}");
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
      let removed = media_to_download.remove(existing_media.resource_id());
      if removed {
        info!("Skipping {}", existing_media.resource_id().to_url());
        media_to_check.insert(existing_media.resource_id().clone());
      }
    }

    // download all remaining items (if desired)
    for resource_id in media_to_download {
      if let Err(error) = self.download_media_into_library(&resource_id, &format) {
        error!(
          "Failed to download {}: {error:#}",
          resource_id.to_url().to_string()
        );
      }
    }
    // check metadata of all downloaded items (if desired)
    for resource_id in media_to_check {
      let song_file = self
        .songs
        .iter_mut()
        .find(|song| *song.resource_id() == resource_id)
        .expect("media to check must exist on disk");
      if let Err(error) = song_file.check_media_meta(self.lrclib, &self.temp_dir.path()) {
        error!("Failed to update the metadata of {song_file}: {error:#}")
      }
    }

    Ok(())
  }

  fn fix_playlist_media_entry_paths(&mut self) {
    for playlist in self
      .playlists
      .iter_mut()
      .chain(self.untracked_playlists.iter_mut())
    {
      for media_entry in &mut playlist.m3u8_playlist.entries {
        let Some(song_file) = self
          .songs
          .iter_mut()
          .find(|song| *song.resource_id() == media_entry.resource_id())
        else {
          continue;
        };
        let file_path = song_file.final_file_path();
        media_entry.media_source = MediaSource::File(file_path.clone());
      }
    }
  }

  pub fn write(&mut self) -> anyhow::Result<()> {
    self.fix_playlist_media_entry_paths();

    for playlist in self
      .playlists
      .iter_mut()
      .chain(self.untracked_playlists.iter_mut())
    {
      playlist.write(self.temp_dir.path())?;
    }
    for song in &mut self.songs {
      song.write()?;
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
