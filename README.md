# media-dl

This tool should take a list of playlist files.
Then it downloads them all flat into a folder and lyrics. (opus files)
Then it lists all files that are not in any playlist.
It also stores the yt id.
It can verify if yt music files got taken down.

It will generate .m3u8 files for each playlist pointing to the music folder.

Implement cooldowns and retries (exponential bounded backoff).
Implement watching a cookies.txt until it is replaced with new ones.
Ask the user to change them when it fails.

Wanted log output:
```
SECTION_START: Reading existing files
SECTION_END: took <HUMAN_TIME>, Read <NUM> existing files: <NUM_SONGS> songs, <NUM_PLAYLISTS> playlists
SECTION_START: Fetching url info.
Fetching "<URL>".
Fetched "<URL>": playlist "<TITLE>" with <LEN> items.
Fetched "<URL>": song "<TITLE>" - "<ARTIST>" (<DURATION>).
SECTION_END: took <HUMAN_TIME>, Fetched <NUM> song info, <NUM> playlist info
SECTION_START: Downloading
Downloading "<URL>": "<TITLE>" - "<ARTIST>" (<DURATION>)
Skipping "<URL>": "<TITLE>" - "<ARTIST>" (<DURATION>) already exists
Fetching lyrics for "<URL>" "<TITLE>" - "<ARTIST>" (<DURATION>)
Fetched lyrics for "<URL>" "<TITLE>" - "<ARTIST>" (<DURATION>)
SECTION_END: took <HUMAN_TIME>, Downloaded <NUM> songs of <EXISTING_NUM>, Downloaded <NUM> lyrics, <LYRICS_PERCENT> of songs now have lyrics
SECTION_START: Updating playlists
SECTION_END: took <HUMAN_TIME>
SECTION_START: Final report
Downloaded <NUM> songs and <NUM> playlists.
Provided URL breakdown:
* <URL> "<TITLE>" - "<ARTIST>" (<DURATION>) (song): Is unavailable.
* <URL> "<TITLE>" - "<ARTIST>" (<DURATION>) (song): Is unavailable but downloaded as "<PATH>".
* <URL> "<TITLE>" (playlist): Is unavailable.
* <URL> "<TITLE>" "<PATH>" (playlist): Is unavailable but downloaded locally.
       It has <NUM_ENTRIES> entries.
       It is missing the following files:
              * <PATH>
* <URL> "<TITLE>" (<NUM_ITEMS>) "<PATH>" (playlist):
       Untracked media files:
       Add `#EXTUNTRACKEDMEDIA` before to suppress.
              * <PATH> "<TITLE>" - "<ARTIST>" (<DURATION>) (<URL if available>)
       Errors:
       Add `#EXTSKIPREMOTE` to suppress.
              * <URL> "<TITLE>" - "<ARTIST>" (<DURATION>): Is unavailable.
              * <URL> "<TITLE>" - "<ARTIST>" (<DURATION>): Failed to download with: <ERRORS>
* Untracked playlists:
       Add `#EXTUNTRACKEDPLAYLIST` to suppress or `#EXTTRACKEDPLAYLIST <URL>` to track.
       * <PATH> "<TITLE>"
* Untracked media:
       Add them to "Untracked Media.m3u8" or "Untracked Media.m3u8.media-dl".
       * <PATH> "<TITLE>" - "<ARTIST>" (<DURATION>) (song)
SECTION_END: took <HUMAN_TIME>
```
