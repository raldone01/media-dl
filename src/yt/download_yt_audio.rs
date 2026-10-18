use std::{
  path::{Path, PathBuf},
  sync::atomic::AtomicBool,
};

use anyhow::{Context, Result};

use crate::{
  media_library::AudioFile,
  utils::{TaggedFileWithPath, smart_rename},
  yt::{YtMediaId, embed_yt_id_tag},
  yt_dlp::YtDlp,
};

pub fn download_yt_audio(
  output_dir: &Path,
  yt_media_id: &YtMediaId,
  format: &str,
  abort_token: &AtomicBool,
  temp_dir: &Path,
) -> Result<AudioFile> {
  let output_template = temp_dir.join("%(title)s - %(artist,uploader)s.%(ext)s");
  let yt_url = yt_media_id.to_url();

  let out = YtDlp::new()
    .abort_token(abort_token)
    // Extract the best available audio.
    .args([
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
      "--convert-thumbnail",
      "png",
      // Convert the thumbnail to PNG and crop it to a square.
      "--ppa",
      r#"ThumbnailsConvertor:-vf crop="'if(gt(ih,iw),iw,ih)':'if(gt(iw,ih),ih,iw)'""#,
      // Prefer the video itself when the URL points into a playlist.
      "--no-playlist",
      "--quiet",
      "--print",
      "after_move:filepath",
      // output template
      "-o",
    ])
    .arg(&output_template)
    .arg(yt_url.as_str())
    .run_quiet()?;

  let mut lines = out
    .stdout_lines
    .iter()
    .map(|l| l.trim())
    .filter(|l| !l.is_empty());
  let temp_path: PathBuf = lines
    .next()
    .context("yt-dlp did not report an output file")?
    .into();

  let file_name = temp_path
    .file_name()
    .context("yt-dlp output path has no file name")?;
  let target_path = output_dir.join(file_name);

  let final_path = smart_rename(Some(&temp_path), Some(&target_path))?;

  // add a yt id tag
  let mut tagged_file = TaggedFileWithPath::open(&final_path)?;
  tagged_file.patch_tag(embed_yt_id_tag(&yt_media_id))?;
  tagged_file.overwrite(temp_dir)?;

  AudioFile::new_from_tagged_file(tagged_file)
}
