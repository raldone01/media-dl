use std::process::{Command, Stdio};

#[must_use]
pub fn has_ytdlp() -> bool {
  Command::new("yt-dlp")
    .arg("--version")
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}

#[must_use]
pub fn has_ffprobe() -> bool {
  Command::new("ffprobe")
    .arg("-version")
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}

#[must_use]
pub fn has_ffmpeg() -> bool {
  Command::new("ffmpeg")
    .arg("-version")
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .is_ok_and(|s| s.success())
}
