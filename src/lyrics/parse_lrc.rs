use std::sync::OnceLock;

/// Parse LRC text into `(timestamp_ms, text)` pairs, sorted by time.
///
/// Handles the common variants: `[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]`,
/// `[mm:ss:xx]`, and multiple timestamps on the same line
/// (e.g. `[00:01.00][00:10.00]Chorus`). Unknown metadata lines such as
/// `[ar:Artist]` are skipped.
#[must_use]
pub fn parse_lrc(lrc: &str) -> Vec<(u32, String)> {
  use regex::Regex;

  static RE: OnceLock<Regex> = OnceLock::new();
  let re = RE.get_or_init(|| {
    // Anchored: the whole bracketed tag must be a timestamp.
    Regex::new(r"^\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]$").unwrap()
  });

  let mut entries: Vec<(u32, String)> = Vec::new();

  for line in lrc.lines() {
    let mut pos = 0usize;
    let mut timestamps: Vec<u32> = Vec::new();

    // Consume the leading run of `[...]` tags.
    // Timestamp tags add timestamps; metadata/other tags are ignored.
    while line[pos..].starts_with('[') {
      let rest = &line[pos..];
      let Some(close) = rest.find(']') else {
        break;
      };

      let tag = &rest[..=close];

      if let Some(cap) = re.captures(tag) {
        let minutes: u32 = cap[1].parse().unwrap_or(0);
        let seconds: u32 = cap[2].parse().unwrap_or(0);

        let millis: u32 = match cap.get(3) {
          Some(g) => {
            let s = g.as_str();
            let n: u32 = s.parse().unwrap_or(0);

            // Normalize 1/2/3-digit fractions to milliseconds.
            match s.len() {
              1 => n * 100,
              2 => n * 10,
              _ => n,
            }
          },
          None => 0,
        };

        timestamps.push(minutes * 60_000 + seconds * 1_000 + millis);
      }

      pos += close + 1;
    }

    if timestamps.is_empty() {
      // Metadata-only line, blank line, or invalid leading tag.
      continue;
    }

    let text = line[pos..].trim().to_string();

    for ts in timestamps {
      entries.push((ts, text.clone()));
    }
  }

  entries.sort_by_key(|(ts, _)| *ts);
  entries
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn empty_input_yields_no_entries() {
    assert!(parse_lrc("").is_empty());
    assert!(parse_lrc("\n\n\n").is_empty());
    assert!(parse_lrc("   \n\t\n").is_empty());
  }

  #[test]
  fn basic_timestamp_mm_ss() {
    let got = parse_lrc("[00:12]hello");
    assert_eq!(got, vec![(12_000, "hello".to_string())]);
  }

  #[test]
  fn fraction_one_digit_is_deciseconds() {
    let got = parse_lrc("[00:01.5]a");
    assert_eq!(got, vec![(1_500, "a".to_string())]);
  }

  #[test]
  fn fraction_two_digits_is_centiseconds() {
    let got = parse_lrc("[00:01.05]a");
    assert_eq!(got, vec![(1_050, "a".to_string())]);
  }

  #[test]
  fn fraction_three_digits_is_milliseconds() {
    let got = parse_lrc("[00:01.005]a");
    assert_eq!(got, vec![(1_005, "a".to_string())]);
  }

  #[test]
  fn colon_separated_fraction_is_accepted() {
    // `[mm:ss:xx]` - some editors emit this instead of `[mm:ss.xx]`.
    let got = parse_lrc("[00:01:05]a");
    assert_eq!(got, vec![(1_050, "a".to_string())]);
  }

  #[test]
  fn minutes_can_have_up_to_three_digits() {
    let got = parse_lrc("[123:45.678]a");
    assert_eq!(got, vec![(123 * 60_000 + 45_000 + 678, "a".to_string())]);
  }

  #[test]
  fn multiple_timestamps_on_one_line_expand() {
    let got = parse_lrc("[00:01.00][00:10.00]Chorus");
    assert_eq!(
      got,
      vec![
        (1_000, "Chorus".to_string()),
        (10_000, "Chorus".to_string()),
      ]
    );
  }

  #[test]
  fn output_is_sorted_by_timestamp() {
    // Out-of-order input; the parser must sort.
    let got = parse_lrc("[00:10.00]b\n[00:01.00]a\n[00:05.00][00:20.00]c");
    assert_eq!(
      got,
      vec![
        (1_000, "a".to_string()),
        (5_000, "c".to_string()),
        (10_000, "b".to_string()),
        (20_000, "c".to_string()),
      ]
    );
  }

  #[test]
  fn metadata_only_lines_are_skipped() {
    let lrc = "[ar:Artist]\n[ti:Title]\n[offset:500]\n";
    assert!(parse_lrc(lrc).is_empty());
  }

  #[test]
  fn mixed_leading_metadata_is_discarded() {
    // A metadata tag *after* a timestamp, still in the leading run,
    // must be dropped without being absorbed into the text.
    let got = parse_lrc("[ar:Artist][00:01.00][ti:Song]hello");
    assert_eq!(got, vec![(1_000, "hello".to_string())]);
  }

  #[test]
  fn timestamp_line_with_trailing_metadata_like_brackets_is_preserved() {
    // Once a leading run has ended, further `[...]` are part of the
    // text; the parser must not strip them.
    let got = parse_lrc("[00:01.00]foo [bar] baz");
    assert_eq!(got, vec![(1_000, "foo [bar] baz".to_string())]);
  }

  #[test]
  fn leading_bracket_that_is_not_a_timestamp_stops_the_run() {
    // `[chorus]` is not a metadata tag -- it looks like free text --
    // but the parser cannot tell the difference, so it treats it as
    // a leading tag and drops it. Everything after it becomes text.
    let got = parse_lrc("[00:01.00][chorus]sing");
    assert_eq!(got, vec![(1_000, "sing".to_string())]);
  }

  #[test]
  fn unterminated_bracket_is_not_a_tag() {
    // No closing `]`: the line has no tags at all, so it is skipped.
    assert!(parse_lrc("[00:01.00unterminated").is_empty());
  }

  #[test]
  fn text_is_trimmed() {
    let got = parse_lrc("[00:01.00]   padded   ");
    assert_eq!(got, vec![(1_000, "padded".to_string())]);
  }

  #[test]
  fn empty_text_after_timestamp_is_allowed() {
    // An LRC line with a timestamp and no text is a legitimate "gap"
    // marker.
    let got = parse_lrc("[00:01.00]");
    assert_eq!(got, vec![(1_000, "".to_string())]);
  }

  #[test]
  fn a_full_realistic_lrc_document() {
    let lrc = "\
[ar:Some Artist]
[ti:Some Song]
[length:03:45]

[00:00.00]Intro line
[00:05.25]First verse line
[00:10.00][01:00.00]Repeated chorus
[00:15.50]Second verse line
[01:05.00]Outro
";
    let got = parse_lrc(lrc);
    assert_eq!(
      got,
      vec![
        (0, "Intro line".to_string()),
        (5_250, "First verse line".to_string()),
        (10_000, "Repeated chorus".to_string()),
        (15_500, "Second verse line".to_string()),
        (60_000, "Repeated chorus".to_string()),
        (65_000, "Outro".to_string()),
      ]
    );
  }

  #[test]
  fn invalid_leading_fraction_falls_back_to_zero() {
    // A group like `[00:01.abc]` is not a valid timestamp and is
    // therefore treated as a leading metadata tag: discarded. If
    // nothing else tags the line, it is skipped.
    assert!(parse_lrc("[00:01.abc]text").is_empty());
  }
}
