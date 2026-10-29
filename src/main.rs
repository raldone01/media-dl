//! `media-dl` - a media downloading CLI focusing on youtube music.
mod logging;
mod lyrics;
mod media_dl;
mod media_library;
mod utils;
mod yt;
mod yt_dlp;

use anyhow::bail;
use clap::ArgAction::SetFalse;
use clap::builder::PossibleValuesParser;
use clap::{Arg, command, value_parser};
use std::{path::PathBuf, process::exit, str::FromStr as _, sync::Arc};
use tracing::{Level, error};
use url::Url;

use crate::logging::setup_logging;
use crate::media_dl::MediaDl;
use crate::media_library::{AudioSubtype, MimeType, VideoSubtype};
use crate::utils::has_ffmpeg;
use crate::utils::has_ffprobe;
use crate::utils::has_ytdlp;

const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

const AUDIO_DOWNLOAD_FORMATS: &[&str] = &["mp3", "m4a", "flac", "opus", "wav"];
const VIDEO_DOWNLOAD_FORMATS: &[&str] = &["mkv"];

fn https_url(value: &str) -> anyhow::Result<Url> {
  let url = Url::parse(value)?;

  if url.scheme() != "https" {
    bail!("URL {url} must use HTTPS");
  }

  Ok(url)
}

fn convert_format_to_mime_type(format_str: &str) -> anyhow::Result<MimeType> {
  Ok(match format_str {
    "mp3" => MimeType::Audio(AudioSubtype::Mpeg),
    "m4a" => MimeType::Audio(AudioSubtype::Mp4),
    "flac" => MimeType::Audio(AudioSubtype::Flac),
    "opus" => MimeType::Audio(AudioSubtype::Opus),
    "wav" => MimeType::Audio(AudioSubtype::Wav),
    "mkv" => MimeType::Video(VideoSubtype::Mkv),
    other => bail!("Failed to convert {format_str} to a MIME type."),
  })
}

#[must_use]
fn new_argparser() -> clap::Command {
  command!()
    .about(format!(
      "`{CRATE_NAME}` - a media downloading CLI focusing on youtube music."
    ))
    .arg(
      Arg::new("flagged_audio_urls")
        .long("audio-urls")
        .help("Audio urls to download")
        .num_args(1..)
        .value_name("audio urls")
        .value_parser(https_url),
    )
    .arg(
      Arg::new("positional_audio_urls")
        .help("Audio urls to download")
        .num_args(1..)
        .value_name("audio urls")
        .value_parser(https_url),
    )
    .arg(
      Arg::new("default_audio_format")
        .long("default-audio-format")
        .help("Default audio format")
        .value_name("default-audio-format")
        .value_parser(PossibleValuesParser::new(AUDIO_DOWNLOAD_FORMATS))
        .default_value("opus"),
    )
    .arg(
      Arg::new("video_urls")
        .long("video-urls")
        .help("Video urls to download")
        .num_args(1..)
        .value_name("video urls")
        .value_parser(https_url),
    )
    .arg(
      Arg::new("default_video_format")
        .long("default-video-format")
        .help("Default video format")
        .value_name("default-video-format")
        .value_parser(PossibleValuesParser::new(VIDEO_DOWNLOAD_FORMATS))
        .default_value("mkv"),
    )
    .arg(
      Arg::new("media_library_dir")
        .long("media-library")
        .help("Media library directory used to store output files.")
        .num_args(1..)
        .value_name("media library directory")
        .value_parser(value_parser!(PathBuf))
        .required(true),
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
    .arg(
      Arg::new("no_network")
        .long("no-network")
        .help("Local only operation - no network")
        .action(SetFalse),
    )
}

fn main() {
  if let Err(err) = run() {
    eprintln!("{:#}", err);
    exit(1);
  }
}

fn run() -> anyhow::Result<()> {
  let matches = new_argparser().get_matches();

  // set the correct log level
  let log_level = matches
    .get_one::<String>("log_level")
    .and_then(|level| Level::from_str(level).ok());
  setup_logging(log_level);

  let flagged_audio_urls = matches
    .get_many::<Url>("flagged_audio_urls")
    .unwrap_or_default();
  let positional_audio_urls = matches
    .get_many::<Url>("positional_audio_urls")
    .unwrap_or_default();
  let audio_urls = flagged_audio_urls
    .chain(positional_audio_urls)
    .cloned()
    .collect();
  let default_audio_download_format = convert_format_to_mime_type(
    matches
      .get_one::<String>("default_audio_format")
      .expect("Has a default value"),
  )?;

  let video_urls = matches
    .get_many::<Url>("video_urls")
    .unwrap_or_default()
    .cloned()
    .collect();
  let default_video_download_format = convert_format_to_mime_type(
    matches
      .get_one::<String>("default_video_format")
      .expect("Has a default value"),
  )?;

  let media_library_dir = matches
    .get_one::<PathBuf>("media_library_dir")
    .expect("Required argument")
    .clone();

  let cookies_txt_file = matches
    .get_one::<PathBuf>("cookies")
    .expect("Required argument")
    .clone();

  let no_network = matches.get_flag("no_network");

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

  let media_dl = Arc::new(MediaDl::new(
    audio_urls,
    default_audio_download_format,
    video_urls,
    default_video_download_format,
    media_library_dir,
    cookies_txt_file,
    no_network,
  ));

  let ctrlc_process_state = media_dl.clone();
  ctrlc::set_handler(move || {
    println!("\nReceived Ctrl+C! Exiting...");
    ctrlc_process_state.set_abort_token();
  })
  .expect("Error setting Ctrl+C handler");

  media_dl.main_loop()?;

  Ok(())
}
