//! Yt Music Downloader

use crate::download_song::download_song;
use crate::embedded_lyrics::embed_lyrics;
use crate::embedded_yt_id::get_yt_id_from_tag;
use crate::lofty_utils::TaggedFileWithPath;
use crate::lrclib_net::{LrcLibError, Lrclib, Lyrics, TrackQuery};
use crate::parse_yt_item_info::{PlaylistInfo, SongInfo, YtItem, download_yt_item_info};
use crate::utils::file_ext;
use crate::yt_dlp::{has_ffmpeg, has_ffprobe};
use crate::{logging::setup_logging, yt_dlp::has_ytdlp};
use anyhow::{Context, anyhow};
use anyhow::{Error, bail};
use clap::builder::PossibleValuesParser;
use clap::{Arg, command, value_parser};
use jwalk::WalkDir;
use rayon::iter::{ParallelBridge, ParallelIterator};
use std::{
  collections::BTreeSet,
  fs,
  path::PathBuf,
  process::exit,
  str::FromStr as _,
  sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
  },
};
use tracing::{Level, error, info, trace};
use url::Url;

mod download_song;
mod embedded_lyrics;
mod embedded_yt_id;
mod lofty_utils;
mod logging;
mod lrclib_net;
mod parse_id_from_yt_url;
mod parse_yt_item_info;
mod utils;
mod yt_dlp;

const FILE_FORMATS: &[&str] = &["mp3", "m4a", "flac", "opus"];

pub const NAME: &str = env!("CARGO_PKG_NAME");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
struct Cancelled;

struct ExistingSongFile {
  file_path: PathBuf,
  yt_id: String,
  has_unsynced_lyrics: bool,
  has_synced_lyrics: bool,
}

enum ExistingFileEntry {
  Playlist(PathBuf),
  Song(ExistingSongFile),
}

struct ProcessState {
  output_folder: PathBuf,
  urls: BTreeSet<Url>,
  cookies: PathBuf,
  download_format: String,
  should_exit: AtomicBool,
}

impl ProcessState {
  #[must_use]
  fn new(
    output_folder: PathBuf,
    urls: BTreeSet<Url>,
    cookies: PathBuf,
    download_format: String,
  ) -> Self {
    Self {
      output_folder,
      urls,
      cookies,
      download_format,
      should_exit: AtomicBool::new(false),
    }
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

    if ext == "m3u8" {
      Ok(Some(ExistingFileEntry::Playlist(file_path)))
    } else if FILE_FORMATS.contains(&ext.as_str()) {
      let tagged_file = TaggedFileWithPath::open(&file_path)?;
      let Some(tag) = tagged_file.tag() else {
        bail!("No tag found in the file")
      };
      let Some(yt_id) = get_yt_id_from_tag(tag) else {
        bail!("Couldn't extract a yt id from the file")
      };
      // TODO: call into lyrics code
      Ok(Some(ExistingFileEntry::Song(ExistingSongFile {
        file_path,
        yt_id,
        has_unsynced_lyrics: false,
        has_synced_lyrics: false,
      })))
    } else {
      Ok(None)
    }
  }

  fn download_urls(&self) -> anyhow::Result<()> {
    let lrclib = Lrclib::new(USER_AGENT).context("Failed to initialize LRCLIB client")?;

    fs::create_dir_all(&self.output_folder).context("Failed to create output folder")?;

    let file_entries = WalkDir::new(&self.output_folder).max_depth(1).into_iter();

    let exiting_files: Vec<ExistingFileEntry> = match file_entries
      .par_bridge()
      .map(
        |entry_result| -> Result<Option<ExistingFileEntry>, Cancelled> {
          if self.should_exit.load(Ordering::Relaxed) {
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
    let mut existing_songs: Vec<ExistingSongFile> = Vec::new();
    // collects existing m3u8 playlists
    let mut existing_playlists: Vec<PathBuf> = Vec::new();

    for entry in exiting_files {
      match entry {
        ExistingFileEntry::Playlist(p) => existing_playlists.push(p),
        ExistingFileEntry::Song(s) => existing_songs.push(s),
      }
    }

    let mut failed_urls = Vec::new();

    let mut tracked_playlists: Vec<PlaylistInfo> = Vec::new();
    let mut tracked_songs: Vec<SongInfo> = Vec::new();

    for url in &self.urls {
      let item_info = match download_yt_item_info(url) {
        Ok(item) => item,
        Err(error) => {
          failed_urls.push((url, error));
          continue;
        },
      };

      match item_info {
        YtItem::PlaylistInfo(playlist_info) => {
          for song_info in &playlist_info.entries {
            tracked_songs.push(song_info.clone());
          }
          tracked_playlists.push(playlist_info);
        },
        YtItem::SongInfo(song_info) => {
          tracked_songs.push(song_info);
        },
      }
    }

    // process all songs
    for song_info in &tracked_songs {
      let url = &song_info.url;
      let song_path = match download_song(&self.output_folder, url, &self.download_format) {
        Ok(song_path) => song_path,
        Err(error) => {
          failed_urls.push((url, error));
          continue;
        },
      };

      match get_lyrics_for_song_info(&lrclib, &song_info) {
        Ok(Some(lyrics)) => {
          match embed_lyrics(
            &song_path,
            lyrics.plain_lyrics.as_ref().map(|s| s.as_str()),
            lyrics.synced_lyrics.as_ref().map(|s| s.as_str()),
          ) {
            Ok(embedded_result) => {},
            Err(error) => failed_urls.push((url, error)),
          }
        },
        Ok(None) => info!("No lyrics found for {}", song_info.title),
        Err(error) => failed_urls.push((url, error)),
      }
    }

    for (url, error) in &failed_urls {
      error!("{url} failed with {error}")
    }

    Ok(())
  }
}

fn get_lyrics_for_song_info(
  lrclib: &Lrclib,
  song_info: &SongInfo,
) -> anyhow::Result<Option<Lyrics>> {
  let mut query = TrackQuery::new(&song_info.title);
  query = query.artist(&song_info.artist);
  query = query.album(&song_info.album);
  query = query.duration(song_info.duration as f64);

  match lrclib.get(&query) {
    Ok(lyrics) => return Ok(Some(lyrics)),
    Err(LrcLibError::NotFound) => {
      let results = lrclib.search(&query).context("LRCLIB search failed")?;

      if let Some(result) = results.into_iter().next() {
        return Ok(Some(result));
      } else {
        info!("No lyrics found for {}", song_info.title);
      }

      Ok(None)
    },
    Err(error) => Err(error).context("LRCLIB get failed"),
  }
}

fn https_url(value: &str) -> Result<Url, Error> {
  let url = Url::parse(value)?;

  if url.scheme() != "https" {
    return Err(anyhow!("URL must use HTTPS"));
  }

  Ok(url)
}

#[must_use]
fn new_argparser() -> clap::Command {
  command!()
    .about("Downloads songs from youtube music.")
    .arg(
      Arg::new("flagged_urls")
        .long("urls")
        .help("Urls to download")
        .num_args(1..)
        .value_name("urls")
        .value_parser(https_url),
    )
    .arg(
      Arg::new("positional_urls")
        .help("Urls to download")
        .num_args(1..)
        .value_name("urls")
        .value_parser(https_url),
    )
    .arg(
      Arg::new("output_folder")
        .long("output-folder")
        .help("Output folder")
        .num_args(1..)
        .value_name("output folder")
        .value_parser(value_parser!(PathBuf))
        .required(true),
    )
    .arg(
      Arg::new("format")
        .long("format")
        .help("Format")
        .value_name("format")
        .value_parser(PossibleValuesParser::new(FILE_FORMATS))
        .default_value("opus"),
    )
    .arg(
      Arg::new("log_level")
        .long("log-level")
        .help("Log level")
        .value_name("log level")
        .value_parser(["TRACE", "DEBUG", "INFO", "WARNING", "ERROR"]),
    )
    .arg(
      Arg::new("cookies")
        .long("cookies")
        .help("Youtube cookies.txt")
        .value_name("cookies.txt")
        .value_parser(value_parser!(PathBuf))
        .required(true),
    )
}

fn main() -> anyhow::Result<()> {
  let matches = new_argparser().get_matches();

  // set the correct log level
  let log_level = matches
    .get_one::<String>("log_level")
    .and_then(|level| Level::from_str(level).ok());
  setup_logging(log_level);

  let flagged_urls = matches.get_many::<Url>("flagged_urls").unwrap_or_default();
  let positional_urls = matches
    .get_many::<Url>("positional_urls")
    .unwrap_or_default();
  let urls = flagged_urls.chain(positional_urls).cloned().collect();

  let output_folder = matches
    .get_one::<PathBuf>("output_folder")
    .expect("Required argument")
    .clone();

  let cookies_txt_file = matches
    .get_one::<PathBuf>("cookies")
    .expect("Required argument")
    .clone();

  let download_format = matches
    .get_one::<String>("format")
    .expect("Has a default value")
    .clone();

  if !has_ytdlp() {
    error!("yt-dlp was not found. Make sure it is installed and in your PATH.");
    exit(1);
  }

  if !has_ffmpeg() {
    error!("ffmpeg was not found. Make sure it is installed and in your PATH.")
  }

  if !has_ffprobe() {
    error!("ffprobe was not found. Make sure it is installed and in your PATH.")
  }

  let process_state = Arc::new(ProcessState::new(
    output_folder,
    urls,
    cookies_txt_file,
    download_format,
  ));

  let ctrlc_process_state = process_state.clone();
  ctrlc::set_handler(move || {
    println!("\nReceived Ctrl+C! Exiting...");
    ctrlc_process_state
      .should_exit
      .store(true, Ordering::Relaxed);
  })
  .expect("Error setting Ctrl+C handler");

  process_state.download_urls()?;

  Ok(())
}
