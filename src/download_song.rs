use anyhow::{Context, Result, bail};
use std::{
  fs,
  path::{Path, PathBuf},
  process::Command,
};

use tempfile::TempDir;
use url::Url;

pub fn download_song(output_dir: &Path, url: &Url, format: &str) -> Result<PathBuf> {
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
      // Print only the final output path after all post-processing.
      "--print",
      "after_move:filepath",
      // Avoid normal yt-dlp progress/output so stdout contains the path.
      "--quiet",
      "--no-warnings",
    ])
    .arg(url.as_str())
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

  // Path reported by yt-dlp, still located inside the temp dir.
  let temp_path = String::from_utf8(output.stdout)
    .context("yt-dlp produced invalid UTF-8 in its output")?
    .lines()
    .last()
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(PathBuf::from)
    .context("yt-dlp did not report an output file")?;

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
