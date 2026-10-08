//! Minimal LRCLIB API client.

use std::{
  sync::{Arc, Mutex},
  thread,
  time::{Duration, Instant},
};

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use serde::Deserialize;
use thiserror::Error;

const DEFAULT_BASE_URL: &str = "https://lrclib.net";
const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_MAX_RETRIES: usize = 3;

#[derive(Clone)]
pub struct Lrclib {
  client: Client,
  base_url: String,
  user_agent: String,
  limiter: Arc<Mutex<RateLimiter>>,
  max_retries: usize,
}

struct RateLimiter {
  next_request: Instant,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
  pub id: u64,
  pub track_name: String,
  pub artist_name: String,
  pub album_name: String,
  pub duration: Option<f64>,
  pub instrumental: bool,
  pub plain_lyrics: Option<String>,
  pub synced_lyrics: Option<String>,
}

#[derive(Debug, Error)]
pub enum LrcLibError {
  #[error("LRCLIB request failed: {0}")]
  Request(#[from] reqwest::Error),

  #[error("LRCLIB returned HTTP {status}{}", match .retry_after {
    Some(delay) => format!(", retry after {delay:?}"),
    None => String::new(),
  })]
  Http {
    status: StatusCode,
    retry_after: Option<Duration>,
    body: String,
  },

  #[error("LRCLIB returned an invalid Retry-After header")]
  InvalidRetryAfter,

  #[error("lyrics not found")]
  NotFound,
}

#[derive(Debug, Clone, Default)]
pub struct TrackQuery<'a> {
  pub artist_name: Option<&'a str>,
  pub track_name: Option<&'a str>,
  pub album_name: Option<&'a str>,
  pub duration: Option<f64>,
}

impl<'a> TrackQuery<'a> {
  pub fn new(track_name: &'a str) -> Self {
    Self {
      track_name: Some(track_name),
      ..Default::default()
    }
  }

  pub fn artist(mut self, artist_name: &'a str) -> Self {
    self.artist_name = Some(artist_name);
    self
  }

  pub fn album(mut self, album_name: &'a str) -> Self {
    self.album_name = Some(album_name);
    self
  }

  pub fn duration(mut self, duration: f64) -> Self {
    self.duration = Some(duration);
    self
  }
}

impl Lrclib {
  /// Create a client using LRCLIB's public API.
  ///
  /// `application` should identify the application and ideally include a
  /// version, for example `"my-player/1.2.3"`.
  pub fn new(application: impl Into<String>) -> Result<Self, LrcLibError> {
    let user_agent = application.into();

    let client = Client::builder().user_agent(user_agent.clone()).build()?;

    Ok(Self {
      client,
      base_url: DEFAULT_BASE_URL.to_owned(),
      user_agent,
      limiter: Arc::new(Mutex::new(RateLimiter {
        next_request: Instant::now(),
      })),
      max_retries: DEFAULT_MAX_RETRIES,
    })
  }

  /// Override the API endpoint, primarily useful for testing.
  pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
    self.base_url = base_url.into().trim_end_matches('/').to_owned();
    self
  }

  /// Set the maximum number of retries after HTTP 429 responses.
  pub fn with_max_retries(mut self, max_retries: usize) -> Self {
    self.max_retries = max_retries;
    self
  }

  /// Look up lyrics for a track.
  ///
  /// LRCLIB's `/api/get` endpoint returns a single result or HTTP 404.
  pub fn get(&self, query: &TrackQuery<'_>) -> Result<Lyrics, LrcLibError> {
    let url = format!("{}/api/get", self.base_url);
    let request = self.build_query(self.client.get(url), query);
    self.execute(request)
  }

  /// Look up lyrics by their LRCLIB database ID.
  pub fn get_by_id(&self, id: u64) -> Result<Lyrics, LrcLibError> {
    let url = format!("{}/api/get/{}", self.base_url, id);
    let request = self.client.get(url);

    self.execute(request)
  }

  /// Search LRCLIB.
  ///
  /// Unlike `/api/get`, this endpoint returns zero or more matches.
  pub fn search(&self, query: &TrackQuery<'_>) -> Result<Vec<Lyrics>, LrcLibError> {
    let url = format!("{}/api/search", self.base_url);
    let request = self.build_query(self.client.get(url), query);
    self.execute(request)
  }

  fn build_query<'a>(&self, mut request: RequestBuilder, query: &TrackQuery<'a>) -> RequestBuilder {
    if let Some(value) = query.artist_name {
      request = request.query(&[("artist_name", value)]);
    }

    if let Some(value) = query.track_name {
      request = request.query(&[("track_name", value)]);
    }

    if let Some(value) = query.album_name {
      request = request.query(&[("album_name", value)]);
    }

    if let Some(value) = query.duration {
      request = request.query(&[("duration", value)]);
    }

    request
  }

  fn execute<T>(&self, request: RequestBuilder) -> Result<T, LrcLibError>
  where
    T: serde::de::DeserializeOwned,
  {
    for attempt in 0..=self.max_retries {
      self.wait_for_rate_limit();

      let response = request
        .try_clone()
        .expect("LRCLIB request must be clonable")
        .send()?;

      if response.status() == StatusCode::NOT_FOUND {
        return Err(LrcLibError::NotFound);
      }

      if response.status() == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = parse_retry_after(response.headers())?;
        let delay = retry_after.unwrap_or_else(|| {
          // Exponential backoff with a small upper bound.
          let seconds = 2_u64.saturating_pow((attempt as u32).min(4));
          Duration::from_secs(seconds)
        });

        if attempt == self.max_retries {
          let body = response.text().unwrap_or_default();

          return Err(LrcLibError::Http {
            status: StatusCode::TOO_MANY_REQUESTS,
            retry_after: Some(delay),
            body,
          });
        }

        thread::sleep(delay);
        continue;
      }

      let status = response.status();

      if !status.is_success() {
        let body = response.text().unwrap_or_default();

        return Err(LrcLibError::Http {
          status,
          retry_after: None,
          body,
        });
      }

      return Ok(response.json::<T>()?);
    }

    unreachable!("retry loop always returns")
  }

  fn wait_for_rate_limit(&self) {
    let sleep_duration = {
      let mut limiter = self
        .limiter
        .lock()
        .expect("LRCLIB rate limiter mutex was poisoned");

      let now = Instant::now();
      let sleep_duration = limiter.next_request.saturating_duration_since(now);

      // Reserve the next request slot before releasing the mutex so
      // concurrent callers cannot bypass the global client-side rate limit.
      // The slot is scheduled relative to when this request is allowed to
      // start, matching the previous async behavior.
      let request_start = now + sleep_duration;
      limiter.next_request = request_start + DEFAULT_INTERVAL;

      sleep_duration
    };

    if !sleep_duration.is_zero() {
      thread::sleep(sleep_duration);
    }
  }

  /// Returns the User-Agent that is sent to LRCLIB.
  pub fn user_agent(&self) -> &str {
    &self.user_agent
  }
}

fn parse_retry_after(
  headers: &reqwest::header::HeaderMap,
) -> Result<Option<Duration>, LrcLibError> {
  let Some(value) = headers.get(reqwest::header::RETRY_AFTER) else {
    return Ok(None);
  };

  let value = value
    .to_str()
    .map_err(|_| LrcLibError::InvalidRetryAfter)?
    .trim();

  // LRCLIB normally uses the delta-seconds form of Retry-After.
  // An HTTP-date form is intentionally not guessed at because doing so
  // incorrectly could cause another request too early.
  let seconds = value
    .parse::<u64>()
    .map_err(|_| LrcLibError::InvalidRetryAfter)?;

  Ok(Some(Duration::from_secs(seconds)))
}
