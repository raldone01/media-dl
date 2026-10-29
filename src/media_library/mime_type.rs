use anyhow::bail;
use lofty::file::FileType;

/// Lowercases the MIME type and drops parameters such as `;codecs=opus`.
fn normalize(value: &str) -> String {
  value
    .split(';')
    .next()
    .unwrap_or_default()
    .trim()
    .to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AudioSubtype {
  Aac,
  Aiff,
  Ape,
  Flac,
  /// Also known as MP3
  Mpeg,
  /// Also known as M4A
  Mp4,
  Mpc,
  Opus,
  Vorbis,
  Speex,
  Wav,
  WavPack,
}

impl AudioSubtype {
  #[must_use]
  pub fn as_str(&self) -> &'static str {
    match self {
      AudioSubtype::Aac => "audio/aac",
      AudioSubtype::Aiff => "audio/aiff",
      AudioSubtype::Ape => "audio/ape",
      AudioSubtype::Flac => "audio/flac",
      AudioSubtype::Mpeg => "audio/mpeg",
      AudioSubtype::Mp4 => "audio/mp4",
      AudioSubtype::Mpc => "audio/musepack",
      AudioSubtype::Opus => "audio/opus",
      AudioSubtype::Vorbis => "audio/vorbis",
      AudioSubtype::Speex => "audio/speex",
      AudioSubtype::Wav => "audio/wav",
      AudioSubtype::WavPack => "audio/wavpack",
    }
  }
}

impl TryFrom<FileType> for AudioSubtype {
  type Error = anyhow::Error;

  fn try_from(value: FileType) -> Result<Self, Self::Error> {
    Ok(match value {
      FileType::Aac => Self::Aac,
      FileType::Aiff => Self::Aiff,
      FileType::Ape => Self::Ape,
      FileType::Flac => Self::Flac,
      FileType::Mp4 => Self::Mp4,
      FileType::Mpc => Self::Mpc,
      FileType::Mpeg => Self::Mpeg,
      FileType::Opus => Self::Opus,
      FileType::Speex => Self::Speex,
      FileType::Vorbis => Self::Vorbis,
      FileType::Wav => Self::Wav,
      FileType::WavPack => Self::WavPack,
      FileType::Custom(custom) => bail!("Can not convert custom lofty file type {custom}"),
      other => bail!("Lofty added new file type {other:?}"),
    })
  }
}

impl TryFrom<&str> for AudioSubtype {
  type Error = anyhow::Error;

  fn try_from(value: &str) -> Result<Self, Self::Error> {
    let value = normalize(value);
    let Some(value) = value.strip_prefix("audio/") else {
      bail!("Audio MIME type doesn't start with 'audio/'.")
    };
    let value = value.strip_prefix("x-").unwrap_or(value);
    Ok(match value {
      "aac" | "aacp" => AudioSubtype::Aac,
      "aiff" | "aif" => AudioSubtype::Aiff,
      "ape" | "monkeys-audio" => AudioSubtype::Ape,
      "flac" => AudioSubtype::Flac,
      "mpeg" | "mp3" | "mpeg3" | "mpg" => AudioSubtype::Mpeg,
      "mp4" | "m4a" | "mp4a-latm" => AudioSubtype::Mp4,
      "musepack" | "mpc" => AudioSubtype::Mpc,
      "opus" => AudioSubtype::Opus,
      // `audio/ogg` is a container; Vorbis is by far its most common content.
      "vorbis" | "ogg" => AudioSubtype::Vorbis,
      "speex" => AudioSubtype::Speex,
      "wav" | "wave" | "vnd.wave" => AudioSubtype::Wav,
      "wavpack" => AudioSubtype::WavPack,
      _ => bail!("Can not infer MIME type from {value}"),
    })
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VideoSubtype {
  /// Also known as matroska.
  Mkv,
  Mp4,
}

impl VideoSubtype {
  #[must_use]
  pub fn as_str(&self) -> &'static str {
    match self {
      VideoSubtype::Mkv => "video/matroska",
      VideoSubtype::Mp4 => "video/mp4",
    }
  }
}

impl TryFrom<&str> for VideoSubtype {
  type Error = anyhow::Error;

  fn try_from(value: &str) -> Result<Self, Self::Error> {
    let value = normalize(value);
    let Some(value) = value.strip_prefix("video/") else {
      bail!("Video MIME type doesn't start with 'video/'.")
    };
    let value = value.strip_prefix("x-").unwrap_or(value);
    Ok(match value {
      "matroska" => VideoSubtype::Mkv,
      "mp4" => VideoSubtype::Mp4,
      _ => bail!("Can not infer MIME type from {value}"),
    })
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MimeType {
  Audio(AudioSubtype),
  Video(VideoSubtype),
}

impl MimeType {
  #[must_use]
  pub fn as_str(&self) -> &'static str {
    match self {
      MimeType::Audio(audio_format) => audio_format.as_str(),
      MimeType::Video(video_format) => video_format.as_str(),
    }
  }
}

impl TryFrom<&str> for MimeType {
  type Error = anyhow::Error;

  fn try_from(value: &str) -> Result<Self, Self::Error> {
    let normalized = normalize(value);
    if normalized.starts_with("audio/") {
      Ok(MimeType::Audio(AudioSubtype::try_from(
        normalized.as_str(),
      )?))
    } else if normalized.starts_with("video/") {
      Ok(MimeType::Video(VideoSubtype::try_from(
        normalized.as_str(),
      )?))
    } else {
      bail!("Unsupported MIME type '{value}': expected 'audio/*' or 'video/*'.")
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn audio_round_trip() {
    use AudioSubtype::*;
    for s in [
      Aac, Aiff, Ape, Flac, Mpeg, Mp4, Mpc, Opus, Vorbis, Speex, Wav, WavPack,
    ] {
      assert_eq!(AudioSubtype::try_from(s.as_str()).unwrap(), s);
      assert_eq!(MimeType::try_from(s.as_str()).unwrap(), MimeType::Audio(s));
    }
  }

  #[test]
  fn aliases_and_params() {
    assert_eq!(
      AudioSubtype::try_from("audio/x-m4a").unwrap(),
      AudioSubtype::Mp4
    );
    assert_eq!(
      AudioSubtype::try_from("audio/mp3").unwrap(),
      AudioSubtype::Mpeg
    );
    assert_eq!(
      AudioSubtype::try_from("Audio/Opus; codecs=opus").unwrap(),
      AudioSubtype::Opus
    );
  }

  #[test]
  fn mime_dispatch() {
    assert_eq!(
      MimeType::try_from("video/x-matroska").unwrap(),
      MimeType::Video(VideoSubtype::Mkv)
    );
    assert!(MimeType::try_from("image/png").is_err());
    assert!(MimeType::try_from("audio/unknown").is_err());
  }
}
