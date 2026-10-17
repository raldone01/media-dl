//! MediaDl media downloading CLI focusing on youtube music.
mod logging;
mod lyrics;
mod media_dl;
mod media_library;
mod utils;
mod yt;

use anyhow::Error;
use anyhow::anyhow;
use clap::ArgAction::SetFalse;
use clap::builder::PossibleValuesParser;
use clap::{Arg, command, value_parser};
use std::{path::PathBuf, process::exit, str::FromStr as _, sync::Arc};
use tracing::{Level, error};
use url::Url;

use crate::logging::setup_logging;
use crate::media_dl::MediaDl;
use crate::utils::has_ffmpeg;
use crate::utils::has_ffprobe;
use crate::utils::has_ytdlp;

const DOWNLOAD_FORMATS: &[&str] = &["mp3", "m4a", "flac", "opus"];

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
        .value_parser(PossibleValuesParser::new(DOWNLOAD_FORMATS))
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
    .arg(
      Arg::new("no_network")
        .long("no-network")
        .help("Local only operation - no network")
        .action(SetFalse),
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

  let download_format = matches
    .get_one::<String>("format")
    .expect("Has a default value")
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
    output_folder,
    urls,
    cookies_txt_file,
    download_format,
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
