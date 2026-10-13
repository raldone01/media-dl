use url::Url;

#[must_use]
pub fn yt_id_from_url(yt_url: &Url) -> Option<String> {
  let host = yt_url.host_str()?.to_ascii_lowercase();

  // https://youtu.be/VIDEO_ID
  if host == "youtu.be" {
    let mut segments = yt_url.path_segments()?;
    return segments
      .next()
      .filter(|id| is_youtube_id(id))
      .map(str::to_owned);
  }

  // https://www.youtube.com/..., https://music.youtube.com/...
  if host == "youtube.com" || host.ends_with(".youtube.com") {
    // https://www.youtube.com/watch?v=VIDEO_ID
    // https://music.youtube.com/watch?v=VIDEO_ID
    if let Some((_, v)) = yt_url.query_pairs().find(|(k, _)| k.as_ref() == "v") {
      if is_youtube_id(v.as_ref()) {
        return Some(v.into_owned());
      }
    }

    // https://www.youtube.com/embed/VIDEO_ID
    // https://www.youtube.com/v/VIDEO_ID
    // https://www.youtube.com/shorts/VIDEO_ID
    // https://www.youtube.com/live/VIDEO_ID
    let mut segments = yt_url.path_segments()?;
    if let Some(kind) = segments.next() {
      if matches!(kind, "embed" | "v" | "shorts" | "live") {
        if let Some(id) = segments.next() {
          if is_youtube_id(id) {
            return Some(id.to_owned());
          }
        }
      }
    }
  }

  // https://www.youtube-nocookie.com/embed/VIDEO_ID
  if host == "youtube-nocookie.com" || host.ends_with(".youtube-nocookie.com") {
    let mut segments = yt_url.path_segments()?;
    if segments.next()? == "embed" {
      if let Some(id) = segments.next() {
        if is_youtube_id(id) {
          return Some(id.to_owned());
        }
      }
    }
  }

  None
}

#[must_use]
pub fn yt_id_from_url_str(yt_url_str: &str) -> Option<String> {
  let yt_url = Url::parse(yt_url_str).ok()?;
  yt_id_from_url(&yt_url)
}

#[must_use]
fn is_youtube_id(s: &str) -> bool {
  s.len() == 11
    && s
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
