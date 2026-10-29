//! TODO: handle rename collisions
//! TODO: keep dirty content flags for only rewriting playlists we changed

use std::collections::BTreeMap;
use std::fmt;
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
use crate::media_library::m3u8::MediaSource;
use crate::media_library::mime_type::MimeType;
use crate::media_library::resource_id::ResourceId;
use crate::media_library::{AudioFile, MediaFile, PlaylistFile};
use crate::utils::{
  CacheDirState, OperationCancelled, dir_is_cache_dir, file_ext, file_name_as_str,
  filesystem_safe_file_name, mark_dir_as_cache_dir,
};
use crate::yt::{YtPlaylistMeta, download_yt_audio};

const AUDIO_FILE_EXTENSIONS: &[&str] = &["mp3", "m4a", "flac", "opus", "ogg", "vorbis", "wav"];

enum ScannedFile {
  Playlist(PlaylistFile),
  StandalonePlaylist(PlaylistFile),
  MediaFile(MediaFile),
}

pub struct MediaLibrary<'a> {
  /// Keeps track of all media files that exist on disk.
  media_files: BTreeMap<ResourceId, BTreeMap<MimeType, MediaFile>>,
  /// Keeps track of playlists.
  playlists: Vec<PlaylistFile>,
  /// Keeps track of standalone media files.
  standalone_playlists: Vec<PlaylistFile>,

  /// This is the directory where all our media and playlists are stored.
  media_library_path: PathBuf,
  /// Client for lrclib to download lyrics.
  lrclib_client: &'a LrclibClient,
  /// Used to cancel long running operations pre-emptively.
  abort_token: &'a AtomicBool,
  /// A temp dir inside the music library folder.
  temp_dir: TempDir,
}

impl<'a> MediaLibrary<'a> {
  #[must_use]
  pub fn new(
    media_library_path: PathBuf,
    lrclib_client: &'a LrclibClient,
    abort_token: &'a AtomicBool,
  ) -> anyhow::Result<Self> {
    fs::create_dir_all(&media_library_path).context("Failed to create media library folder")?;

    let media_library_file_entries = WalkDir::new(&media_library_path)
      .max_depth(1)
      .skip_hidden(false)
      .try_into_iter()
      .with_context(|| {
        format!(
          "Failed to open media library dir: {}",
          media_library_path.display()
        )
      })?;

    let scanned_files: Vec<ScannedFile> = media_library_file_entries
      .par_bridge()
      .map(|entry| Self::scan_entry(entry, &abort_token))
      .filter_map(Result::transpose)
      .collect::<Result<_, OperationCancelled>>()
      .map_err(|OperationCancelled| anyhow!("Library scan cancelled"))?;

    // collects existing media files
    let mut media_files: BTreeMap<ResourceId, BTreeMap<MimeType, MediaFile>> = Default::default();
    // collects existing playlists
    let mut playlists: Vec<PlaylistFile> = Default::default();
    let mut standalone_playlists: Vec<PlaylistFile> = Default::default();

    for entry in scanned_files {
      match entry {
        ScannedFile::Playlist(p) => playlists.push(p),
        ScannedFile::StandalonePlaylist(p) => standalone_playlists.push(p),
        ScannedFile::MediaFile(m) => {
          let media_file_entry = media_files.entry(m.resource_id().clone()).or_default();
          let media_type_entry = media_file_entry.entry(m.mime_type());
          match media_type_entry {
            std::collections::btree_map::Entry::Occupied(occupied_entry) => {
              let existing_media_file = occupied_entry.get();
              bail!(
                "Duplicate media file {} of same MIME type: {} {}",
                m.resource_id().to_string(),
                m.original_file_path().display(),
                existing_media_file.original_file_path().display()
              )
            },
            std::collections::btree_map::Entry::Vacant(vacant_entry) => {
              vacant_entry.insert(m);
            },
          }
        },
      }
    }

    let temp_dir = TempDir::new_in(&media_library_path)?;
    mark_dir_as_cache_dir(temp_dir.path(), Some(CRATE_NAME))?;

    let mut selv = Self {
      media_library_path,
      media_files,
      playlists,
      standalone_playlists,
      lrclib_client,
      abort_token,
      temp_dir,
    };

    selv.startup_playlist_consistency()?;

    Ok(selv)
  }

  fn scan_entry(
    entry_result: jwalk::Result<jwalk::DirEntry<((), ())>>,
    abort_token: &AtomicBool,
  ) -> Result<Option<ScannedFile>, OperationCancelled> {
    if abort_token.load(Ordering::Relaxed) {
      return Err(OperationCancelled);
    }

    Ok(Self::scan_file(entry_result).unwrap_or_else(|error| {
      error!(source = %error, "Failed to read file entry while processing directory");
      None
    }))
  }

  fn scan_file(
    entry_result: Result<jwalk::DirEntry<((), ())>, jwalk::Error>,
  ) -> anyhow::Result<Option<ScannedFile>> {
    let entry = match entry_result {
      Ok(entry) => entry,
      Err(e) => {
        let file_path = e
          .path()
          .map_or_else(|| "Unknown path".to_string(), |p| p.display().to_string());
        bail!("Failed to read file entry while processing directory: {file_path}",);
      },
    };

    if entry.file_type().is_dir()
      && matches!(
        dir_is_cache_dir(&entry.path(), Some(CRATE_NAME)),
        Ok(CacheDirState::IsCacheDir)
      )
    {
      // dir is old cache dir from us
      trash::delete(entry.path())?;
    }

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
      if file_name_str.starts_with("standalone media")
        || file_name_str.starts_with("standalone_media")
      {
        return Ok(Some(ScannedFile::StandalonePlaylist(
          PlaylistFile::new_from_disk(file_path)?,
        )));
      }
      Ok(Some(ScannedFile::Playlist(PlaylistFile::new_from_disk(
        file_path,
      )?)))
    } else if file_name_str.ends_with(".m3u8") {
      Ok(Some(ScannedFile::Playlist(PlaylistFile::new_from_disk(
        file_path,
      )?)))
    } else if AUDIO_FILE_EXTENSIONS.contains(&ext.as_str()) {
      Ok(Some(ScannedFile::MediaFile(MediaFile::Audio(
        AudioFile::new_from_disk(file_path)?,
      ))))
    } else {
      Ok(None)
    }
  }

  fn startup_playlist_consistency(&mut self) -> anyhow::Result<()> {
    for playlist in self.playlists.iter_mut() {
      for media_item in playlist.m3u8_playlist.entries.iter_mut() {
        let media_entry_pointed_resource_id = match &media_item.media_source {
          MediaSource::File(file_path) => self
            .media_files
            .iter()
            .find_map(|(_, media_files_for_resource)| {
              media_files_for_resource
                .iter()
                .find(|(_, media_file)| media_file.original_file_path() == file_path)
            })
            .map(|(_, media_file)| Cow::Borrowed(media_file.resource_id())),
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

  fn standalone_playlist_mut(&mut self) -> &mut PlaylistFile {
    if self.standalone_playlists.is_empty() {
      let file_path = self
        .media_library_path
        .join("Standalone Media.m3u8.media-dl");
      self.standalone_playlists.push(PlaylistFile::new(file_path));
    }
    self
      .standalone_playlists
      .get_mut(0)
      .expect("pushed at least one item onto the vec")
  }

  fn add_url(&mut self, url: &Url, format: MimeType) -> anyhow::Result<()> {
    let resource_id = ResourceId::from(url);
    let url = resource_id.to_url();
    let existing_media_file = self
      .media_files
      .get(&resource_id)
      .and_then(|files_by_resource: &BTreeMap<MimeType, MediaFile>| files_by_resource.get(&format));
    let media_file_info_string = existing_media_file
      .map(|existing_media_file| format!("{existing_media_file}"))
      .unwrap_or_else(|| url.to_string());
    match resource_id {
      ResourceId::YtMediaId(_) => {
        // add the song to standalone_playlists if not part of them already
        let standalone_playlist_entry = self
          .standalone_playlists
          .iter()
          .find(|item| item.m3u8_playlist.find(&resource_id).is_some());

        if standalone_playlist_entry.is_some() {
          // already listed on an standalone playlist
          info!("Skipped adding {media_file_info_string} as it is already standalone media!")
        } else {
          // not yet on the standalone playlists so we add it
          let standalone_playlist = self.standalone_playlist_mut();
          standalone_playlist
            .m3u8_playlist
            .add_media_entry(url.clone(), format);
          info!("Added {media_file_info_string} as standalone media!")
        }
      },
      ResourceId::YtPlaylistId(ref yt_playlist_id) => {
        // add a new playlist to playlists
        let playlist_is_already_part_of_library = self
          .playlists
          .iter()
          .find(|playlist| playlist.m3u8_playlist.resource_id().as_ref() == Some(&resource_id))
          .is_some();

        if !playlist_is_already_part_of_library {
          let mut file_path = self
            .media_library_path
            .join(filesystem_safe_file_name(yt_playlist_id.as_str()));
          file_path.set_extension("m3u8");
          let mut playlist = PlaylistFile::new(file_path);
          playlist
            .m3u8_playlist
            .set_playlist_resource_id(resource_id.clone())?;
          playlist.m3u8_playlist.set_remote_default_format(format);
          self.playlists.push(playlist);
        } else {
          info!(
            "Skipped adding youtube playlist {media_file_info_string} as it is already part of the media library!"
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
    format: MimeType,
  ) -> Result<(), AddUrlsError> {
    let mut url_errors = HashMap::new();
    for url in urls.into_iter() {
      if let Err(error) = self.add_url(url, format) {
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
    default_format: MimeType,
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
          default_format,
        );
        Ok(())
      },
      Some(resource_id) => {
        // we have resource ids which we don't know how to download
        bail!(
          "The playlist {} is tracking an invalid playlist remote resource {}",
          playlist.final_file_path().display(),
          resource_id.to_url().to_string()
        )
      },
      None => {
        // The playlist doesn't track a remote: no-op
        Ok(())
      },
    }
  }

  fn download_media(
    media_library_path: &Path,
    abort_token: &AtomicBool,
    temp_dir: &TempDir,
    lrclib_client: &LrclibClient,
    resource_id: ResourceId,
    format: MimeType,
  ) -> anyhow::Result<MediaFile> {
    let media_file = match resource_id {
      ResourceId::YtMediaId(ref yt_media_id) => {
        info!("Downloading {}", resource_id.to_url().to_string());
        let mut audio_file = download_yt_audio(
          &media_library_path,
          yt_media_id,
          format,
          abort_token,
          temp_dir.path(),
        )?;
        audio_file.rename_if_needed()?;
        if let Err(error) =
          audio_file.download_and_embed_lyrics_if_needed(lrclib_client, temp_dir.path())
        {
          error!("Failed to fetch lyrics for {audio_file}: {error:#}");
        }
        MediaFile::Audio(audio_file)
      },
      ResourceId::LocalFile(..) => {
        bail!("Can't download a local file")
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
    };
    Ok(media_file)
  }

  pub fn download(&mut self, default_format: MimeType) -> anyhow::Result<()> {
    // update the playlists from the remote
    for playlist in &mut self.playlists {
      if playlist.m3u8_playlist.remote_skip_download() {
        continue;
      }
      if let Err(error) =
        Self::update_playlist_from_remote(playlist, default_format, self.abort_token)
      {
        error!("Failed to download {playlist}: {error:#}");
      }
    }

    let mut all_wanted_media_and_formats = BTreeMap::<ResourceId, BTreeSet<MimeType>>::new();
    // collect all media items listed on playlists and the standalone lists
    for playlist in self
      .playlists
      .iter()
      .chain(self.standalone_playlists.iter())
    {
      for media_entry in playlist.m3u8_playlist.entries.iter() {
        if media_entry.remote_skip_download() {
          continue;
        }
        let desired_media_format = media_entry.remote_format().unwrap_or_else(|| {
          playlist
            .m3u8_playlist
            .remote_default_format()
            .unwrap_or_else(|| default_format)
        });
        let media_to_download_by_resource_id = all_wanted_media_and_formats
          .entry(media_entry.resource_id())
          .or_default();
        media_to_download_by_resource_id.insert(desired_media_format);
      }
    }

    for (resource_id, wanted_formats) in all_wanted_media_and_formats {
      let existing_formats = self.media_files.get(&resource_id);

      let missing_formats: Vec<_> = wanted_formats
        .into_iter()
        .filter(|wanted_format| {
          existing_formats.map_or(true, |existing_formats| {
            !existing_formats.contains_key(wanted_format)
          })
        })
        .collect();

      // 1. download all missing formats, keep the meta data for later
      // TODO: source aware downloader so it can store its meta data
      for media_format_to_download in missing_formats {
        match Self::download_media(
          &self.media_library_path,
          self.abort_token,
          &self.temp_dir,
          self.lrclib_client,
          resource_id.clone(),
          media_format_to_download,
        ) {
          Ok(media_file) => {
            let existing_formats = self.media_files.entry(resource_id.clone());
            existing_formats
              .or_default()
              .insert(media_format_to_download, media_file);
          },
          Err(error) => {
            error!(
              "Failed to download {}: {error:#}",
              resource_id.to_url().to_string()
            );
          },
        }
      }

      // 2. fetch meta data if not yet acquired before
      // TODO: impl

      // 3. update all already downloaded formats with the stored meta data
      if let Some(existing_formats) = self.media_files.get_mut(&resource_id) {
        for (_, existing_media_file) in existing_formats {
          if let Err(error) =
            existing_media_file.check_media_meta(self.lrclib_client, &self.temp_dir.path())
          {
            error!("Failed to update the metadata of {existing_media_file}: {error:#}")
          }
        }
      }
    }

    Ok(())
  }

  fn fix_playlist_media_entry_paths(&mut self) {
    for playlist in self
      .playlists
      .iter_mut()
      .chain(self.standalone_playlists.iter_mut())
    {
      let playlist_default_format = playlist.m3u8_playlist.remote_default_format();
      for playlist_media_entry in &mut playlist.m3u8_playlist.entries {
        let Some(media_files_for_format) =
          self.media_files.get(&playlist_media_entry.resource_id())
        else {
          // error??
          continue;
        };
        let Some(desired_format_for_playlist) = playlist_media_entry
          .remote_format()
          .or(playlist_default_format)
        else {
          // error??
          continue;
        };
        let Some(file_path) = media_files_for_format
          .get(&desired_format_for_playlist)
          // TODO: rework how renames work because we would need the final file path here
          .map(|media_file| media_file.original_file_path())
        else {
          // error??
          continue;
        };
        playlist_media_entry.media_source = MediaSource::File(file_path.clone());
      }
    }
  }

  pub fn write(&mut self) -> anyhow::Result<()> {
    self.fix_playlist_media_entry_paths();

    for playlist in self
      .playlists
      .iter_mut()
      .chain(self.standalone_playlists.iter_mut())
    {
      playlist.write(self.temp_dir.path())?;
    }
    for media_file in &mut self
      .media_files
      .values_mut()
      .map(|v| v.values_mut())
      .flatten()
    {
      media_file.write()?;
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
