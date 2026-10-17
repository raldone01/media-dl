use anyhow::{Context, Result, bail};
use std::{
  fs,
  path::{Path, PathBuf},
  process::Command,
};

use tempfile::TempDir;
use url::Url;

use crate::{utils::TaggedFileWithPath, yt::embed_yt_id_tag};

/// TODO: take a mediaid and return a SongFile
pub fn download_yt_audio(output_dir: &Path, yt_url: &Url, format: &str) -> Result<PathBuf> {
  let temp_dir = TempDir::new_in(output_dir).context("failed to create temp dir")?;
  let output_template = temp_dir
    .path()
    .join("%(title)s - %(artist,uploader)s.%(ext)s");

  let output = Command::new("yt-dlp")
    .args([
      // Extract the best available audio.
      "-x",
      "-f",
      "bestaudio/best",
      "--audio-quality",
      "0",
      "--audio-format",
      format,
      // Metadata and thumbnail.
      "--embed-metadata",
      "--embed-thumbnail",
      // Convert the thumbnail to PNG and crop it to a square.
      "--convert-thumbnail",
      "png",
      "--ppa",
      r#"ThumbnailsConvertor:-vf crop="'if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'""#,
      // Prefer the video itself when the URL points into a playlist.
      "--no-playlist",
      // Print the video id (line 1) and the final output path (line 2).
      "--print",
      "after_move:%(id)s",
      "--print",
      "after_move:filepath",
      // Avoid normal yt-dlp progress/output so stdout contains exactly those two lines.
      "--quiet",
      "--no-warnings",
    ])
    .arg(yt_url.as_str())
    .arg("-o")
    .arg(&output_template)
    .output()
    .context("failed to execute yt-dlp")?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
      "yt-dlp failed with status {}: {}",
      output.status,
      stderr.trim()
    );
  }

  let stdout =
    String::from_utf8(output.stdout).context("yt-dlp produced invalid UTF-8 in its output")?;
  let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());

  let yt_id = lines
    .next()
    .context("yt-dlp did not report a video id")?
    .to_owned();

  let temp_path: PathBuf = lines
    .next()
    .context("yt-dlp did not report an output file")?
    .into();

  // add a ytid tag
  let mut tagged_file = TaggedFileWithPath::open(&temp_path)?;
  tagged_file.patch_tag(embed_yt_id_tag(&yt_id))?;
  tagged_file.overwrite()?;

  // TODO: we can also read the tags here since it is open anyways

  let file_name = temp_path
    .file_name()
    .context("yt-dlp output path has no file name")?;
  let final_path = output_dir.join(file_name);

  fs::rename(&temp_path, &final_path).with_context(|| {
    format!(
      "failed to move {} to {}",
      temp_path.display(),
      final_path.display()
    )
  })?;

  Ok(final_path)
}
