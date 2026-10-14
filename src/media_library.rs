//! TODO: keep dirty paths for only rewriting playlists we changed

use core::fmt;
use std::{
  borrow::Cow,
  collections::HashMap,
  ffi::OsStr,
  fmt::Display,
  fs,
  path::PathBuf,
  sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Context as _, bail};
use jwalk::WalkDir;
use lofty::tag::Accessor;
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use thiserror::Error;
use tracing::{error, info, trace};
use url::Url;

use crate::{
  AUDIO_FILE_EXTENSIONS,
  embedded_lyrics::LyricsPresence,
  embedded_yt_id::yt_id_from_tag,
  lofty_utils::TaggedFileWithPath,
  m3u8::M3U8Playlist,
  resource_id::ResourceId,
  url_downloader::UrlDownloader,
  utils::{file_ext, file_name_as_str, filesystem_safe_file_name},
};

#[derive(Debug)]
struct Cancelled;

struct SongFile {
  file_path: PathBuf,
  resource_id: ResourceId,
  lyrics_presence: LyricsPresence,
  title: Option<String>,
  artist: Option<String>,
  album: Option<String>,
}

impl SongFile {
  fn new(file_path: PathBuf) -> Result<Self, anyhow::Error> {
    let tagged_file = TaggedFileWithPath::open(&file_path)?;
    let Some(tag) = tagged_file.tag() else {
      bail!("No tag found in the file")
    };
    let resource_id = yt_id_from_tag(tag)
      .map(|yt_id| ResourceId::YtMediaId(yt_id))
      .unwrap_or_else(|| ResourceId::LocalFile(file_path.clone()));
    let lyrics_presence = LyricsPresence::from_tag(tag);
    let title = tag.title().map(Cow::into_owned);
    let artist = tag.artist().map(Cow::into_owned);
    let album = tag.album().map(Cow::into_owned);
    Ok(Self {
      file_path,
      resource_id,
      lyrics_presence,
      title,
      artist,
      album,
    })
  }
}

impl Display for SongFile {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let title = self
      .title
      .as_ref()
      .map(|title| Cow::Borrowed(title.as_str()))
      .or_else(|| self.file_path.file_name().map(OsStr::to_string_lossy))
      .unwrap_or_else(|| self.file_path.to_string_lossy());
    let artist = self
      .artist
      .as_deref()
      .map(|artist| format!(" - {artist}"))
      .unwrap_or_default();

    write!(
      f,
      "Song \"{title}{artist}\" {}",
      self
        .resource_id
        .to_url()
        .as_ref()
        .map(Url::to_string)
        .unwrap_or_else(|_| format!("{:?}", self.resource_id)),
    )
  }
}

struct PlaylistFile {
  file_path: PathBuf,
  m3u8_playlist: M3U8Playlist,
}

impl PlaylistFile {
  pub fn overwrite(&self) -> Result<(), anyhow::Error> {
    fs::write(&self.file_path, self.m3u8_playlist.serialize())
      .context(format!("Error while writing {}", self.file_path.display()))?;
    Ok(())
  }
}

enum ExistingFileEntry {
  Playlist(PlaylistFile),
  UntrackedPlaylist(PlaylistFile),
  Song(SongFile),
}

pub struct MediaLibrary {
  library_path: PathBuf,
  url_downloader: UrlDownloader,
  /// collects existing music files
  songs: Vec<SongFile>,
  /// collects existing m3u8 playlists
  playlists: Vec<PlaylistFile>,
  /// collects existing m3u8 playlists for marking untracked media files
  untracked_playlists: Vec<PlaylistFile>,
}

impl MediaLibrary {
  fn ingest_playlist_file(file_path: PathBuf) -> Result<PlaylistFile, anyhow::Error> {
    let file_content = fs::read_to_string(&file_path)?;
    let m3u8_playlist = M3U8Playlist::deserialize(&file_content)?;
    Ok(PlaylistFile {
      file_path,
      m3u8_playlist,
    })
  }

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
          Self::ingest_playlist_file(file_path)?,
        )));
      }
      Ok(Some(ExistingFileEntry::Playlist(
        Self::ingest_playlist_file(file_path)?,
      )))
    } else if file_name_str.ends_with(".m3u8") {
      Ok(Some(ExistingFileEntry::Playlist(
        Self::ingest_playlist_file(file_path)?,
      )))
    } else if AUDIO_FILE_EXTENSIONS.contains(&ext.as_str()) {
      Ok(Some(ExistingFileEntry::Song(SongFile::new(
        file_path.to_owned(),
      )?)))
    } else {
      Ok(None)
    }
  }

  #[must_use]
  pub fn new(library_path: PathBuf, abort_token: &AtomicBool) -> Result<Self, anyhow::Error> {
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
      url_downloader: Default::default(),
      playlists,
      songs,
      untracked_playlists,
    })
  }

  fn untracked_playlist_mut(&mut self) -> &mut PlaylistFile {
    if self.untracked_playlists.is_empty() {
      let file_path = self.library_path.join("Untracked Media.m3u8.media-dl");
      self.untracked_playlists.push(PlaylistFile {
        file_path,
        m3u8_playlist: M3U8Playlist::default(),
      });
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

  fn add_url(&mut self, url: &Url, format: String) -> Result<(), anyhow::Error> {
    let resource_id = ResourceId::from(url);
    let url = resource_id
      .to_url()
      .expect("We came from a url so we must be able to construct one again.");
    let existing_song = self.song_file_by_resource_id(&resource_id);
    let url_info_string = existing_song
      .map(|existing_song| format!("{existing_song}"))
      .unwrap_or_else(|| url.to_string());
    match resource_id {
      ResourceId::YtMediaId(ref yt_media_id) => {
        // add the song to untracked_playlists if not part of them already
        let untracked_playlist_entry = self
          .untracked_playlists
          .iter()
          .find(|item| item.m3u8_playlist.find(&resource_id).is_some());

        if let Some(existing_untracked_entry) = untracked_playlist_entry {
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
          self.playlists.push(PlaylistFile {
            // TODO: handle duplicate file name another playlist with that name might already exist on disk
            file_path,
            m3u8_playlist,
          });
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

  pub fn overwrite(&self) -> Result<(), anyhow::Error> {
    for playlist in &self.playlists {
      playlist.overwrite()?;
    }
    for untracked_playlist in &self.untracked_playlists {
      untracked_playlist.overwrite()?;
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
