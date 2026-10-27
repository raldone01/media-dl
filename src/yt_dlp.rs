//! Wrapper around the yt-dlp downloader binary.

use std::{
  collections::VecDeque,
  ffi::OsStr,
  io::{self, BufRead, BufReader, Read},
  process::{Command, ExitStatus, Stdio},
  sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError, Sender},
  },
  thread::{self, JoinHandle},
  time::Duration,
};

use process_wrap::std::*;
use serde::{Deserialize, de::DeserializeOwned};

use crate::utils::tie_command_to_thread;

const DL_PREFIX: &str = "[ytdlp-dl]";
const PP_PREFIX: &str = "[ytdlp-pp]";
const STDERR_TAIL_LINES: usize = 50;

/// How quickly an abort is noticed.
const ABORT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum YtDlpError {
  #[error("yt-dlp was aborted")]
  Aborted,
  #[error("yt-dlp is not installed or not in PATH")]
  NotInstalled,
  #[error("failed to run yt-dlp: {0}")]
  Io(#[from] io::Error),
  #[error("rate limited by YouTube: {message}")]
  RateLimited { message: String },
  #[error("YouTube requires sign-in / bot check: {message}")]
  BotCheck { message: String },
  #[error("media unavailable: {message}")]
  Unavailable { message: String },
  #[error("yt-dlp exited with {status}: {message}")]
  Failed {
    status: ExitStatus,
    message: String,
    stderr: String,
  },
  #[error("failed to parse yt-dlp JSON: {0}")]
  Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub enum YtDlpEvent {
  Download(DownloadProgress),
  PostProcess(PostProcessProgress),
  Warning(String),
}

/// Subset of yt-dlp's progress dict. Numbers can be int or float, so use f64.
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadProgress {
  /// "downloading" | "finished" | "error"
  pub status: String,
  pub downloaded_bytes: Option<f64>,
  pub total_bytes: Option<f64>,
  pub total_bytes_estimate: Option<f64>,
  pub speed: Option<f64>,
  pub eta: Option<f64>,
  pub fragment_index: Option<f64>,
  pub fragment_count: Option<f64>,
}

impl DownloadProgress {
  /// 0.0..=1.0 if the total size is known.
  pub fn fraction(&self) -> Option<f64> {
    let total = self.total_bytes.or(self.total_bytes_estimate)?;
    let done = self.downloaded_bytes?;
    (total > 0.0).then(|| (done / total).clamp(0.0, 1.0))
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostProcessProgress {
  /// "started" | "processing" | "finished"
  pub status: String,
  /// e.g. "ExtractAudio", "EmbedThumbnail", "FFmpegMetadata"
  pub postprocessor: Option<String>,
}

#[derive(Debug, Default)]
pub struct YtDlpOutput {
  /// Non-progress stdout lines (output of `--print`, `-J`, ...).
  pub stdout_lines: Vec<String>,
  pub warnings: Vec<String>,
}

pub struct YtDlp<'a> {
  cmd: Command,
  abort_token: Option<&'a AtomicBool>,
}

impl<'a> YtDlp<'a> {
  pub fn new() -> Self {
    let mut cmd = Command::new("yt-dlp");
    // ensure the process dies with us on unix
    tie_command_to_thread(&mut cmd);
    cmd
      .args(["--newline", "--progress"])
      .arg("--progress-template")
      .arg(format!("download:{DL_PREFIX}%(progress)j"))
      .arg("--progress-template")
      .arg(format!("postprocess:{PP_PREFIX}%(progress)j"))
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::piped());

    Self {
      cmd,
      abort_token: None,
    }
  }

  /// Abort the process (and its children) as soon as this flag becomes true.
  pub fn abort_token(mut self, abort_token: &'a AtomicBool) -> Self {
    self.abort_token = Some(abort_token);
    self
  }

  pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
    self.cmd.arg(arg);
    self
  }

  pub fn args<I, S>(mut self, args: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
  {
    self.cmd.args(args);
    self
  }

  /// Run without caring about progress.
  pub fn run_quiet(self) -> Result<YtDlpOutput, YtDlpError> {
    self.run(|_| {})
  }

  /// Run and parse stdout as JSON (for `-J` / `--dump-single-json`).
  pub fn run_json<T: DeserializeOwned>(self) -> Result<T, YtDlpError> {
    let out = self.run_quiet()?;
    Ok(serde_json::from_str(&out.stdout_lines.join("\n"))?)
  }

  pub fn run(mut self, mut on_event: impl FnMut(YtDlpEvent)) -> Result<YtDlpOutput, YtDlpError> {
    let token = self.abort_token;
    let aborted = move || token.is_some_and(|t| t.load(Ordering::Relaxed));

    if aborted() {
      return Err(YtDlpError::Aborted);
    }

    let mut wrapped_cmd = CommandWrap::from(self.cmd);

    #[cfg(windows)]
    {
      use windows::Win32::System::Threading::*;
      wrapped_cmd
        .wrap(CreationFlags(CREATE_NO_WINDOW | DETACHED_PROCESS))
        .wrap(JobObject);
    }
    #[cfg(unix)]
    wrapped_cmd.wrap(ProcessGroup::leader());

    let mut child = wrapped_cmd.spawn().map_err(|e| match e.kind() {
      io::ErrorKind::NotFound => YtDlpError::NotInstalled,
      _ => YtDlpError::Io(e),
    })?;

    let (tx, rx) = mpsc::channel();
    let readers = [
      spawn_reader(
        child.stdout().take().expect("piped"),
        Stream::Stdout,
        tx.clone(),
      ),
      spawn_reader(child.stderr().take().expect("piped"), Stream::Stderr, tx),
    ];

    let mut collector = Collector::default();

    loop {
      if aborted() {
        let _ = child.kill();
        let _ = child.wait();
        // Don't join the readers. They exit on their own once the pipes
        // close, and we don't want an abort to ever block.
        drop(readers);
        return Err(YtDlpError::Aborted);
      }
      match rx.recv_timeout(ABORT_POLL_INTERVAL) {
        Ok((stream, line)) => collector.handle(stream, line, &mut on_event),
        Err(RecvTimeoutError::Timeout) => {},
        // Both readers hit EOF: the process closed its output.
        Err(RecvTimeoutError::Disconnected) => break,
      }
    }

    for r in readers {
      let _ = r.join();
    }
    let status = child.wait()?;

    if !status.success() {
      // If the user aborted at the very end, report that instead of a confusing failure.
      if aborted() {
        return Err(YtDlpError::Aborted);
      }
      return Err(collector.into_error(status));
    }
    Ok(collector.out)
  }
}

impl Default for YtDlp<'_> {
  fn default() -> Self {
    Self::new()
  }
}

/// Accumulates output while the process runs.
#[derive(Default)]
struct Collector {
  out: YtDlpOutput,
  errors: Vec<String>,
  stderr_tail: VecDeque<String>,
}

impl Collector {
  fn handle(&mut self, stream: Stream, line: String, on_event: &mut impl FnMut(YtDlpEvent)) {
    if let Some(json) = line.strip_prefix(DL_PREFIX) {
      if let Ok(p) = serde_json::from_str(json) {
        on_event(YtDlpEvent::Download(p));
      }
      return;
    }
    if let Some(json) = line.strip_prefix(PP_PREFIX) {
      if let Ok(p) = serde_json::from_str(json) {
        on_event(YtDlpEvent::PostProcess(p));
      }
      return;
    }

    match stream {
      Stream::Stdout => self.out.stdout_lines.push(line),
      Stream::Stderr => {
        if let Some(msg) = line.strip_prefix("ERROR: ") {
          self.errors.push(msg.to_owned());
        } else if let Some(msg) = line.strip_prefix("WARNING: ") {
          self.out.warnings.push(msg.to_owned());
          on_event(YtDlpEvent::Warning(msg.to_owned()));
        }
        if self.stderr_tail.len() == STDERR_TAIL_LINES {
          self.stderr_tail.pop_front();
        }
        self.stderr_tail.push_back(line);
      },
    }
  }

  fn into_error(self, status: ExitStatus) -> YtDlpError {
    let stderr = Vec::from(self.stderr_tail).join("\n");
    classify(status, &self.errors, stderr)
  }
}

#[derive(Clone, Copy)]
enum Stream {
  Stdout,
  Stderr,
}

fn spawn_reader(
  r: impl Read + Send + 'static,
  stream: Stream,
  tx: Sender<(Stream, String)>,
) -> JoinHandle<()> {
  thread::spawn(move || {
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
      buf.clear();
      match r.read_until(b'\n', &mut buf) {
        Ok(0) | Err(_) => break,
        Ok(_) => {
          let line = String::from_utf8_lossy(&buf)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
          if tx.send((stream, line)).is_err() {
            break;
          }
        },
      }
    }
  })
}

fn classify(status: ExitStatus, errors: &[String], stderr: String) -> YtDlpError {
  let message = if errors.is_empty() {
    stderr.lines().last().unwrap_or("unknown error").to_owned()
  } else {
    errors.join("\n")
  };
  let m = message.to_lowercase();

  if m.contains("http error 429") || m.contains("too many requests") || m.contains("rate-limited") {
    YtDlpError::RateLimited { message }
  } else if m.contains("not a bot") || m.contains("sign in to confirm") {
    YtDlpError::BotCheck { message }
  } else if m.contains("video unavailable")
    || m.contains("private video")
    || m.contains("has been removed")
    || m.contains("does not exist")
  {
    YtDlpError::Unavailable { message }
  } else {
    YtDlpError::Failed {
      status,
      message,
      stderr,
    }
  }
}
