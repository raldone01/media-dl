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
Library contains <NUM> songs and <NUM> playlists.
Downloaded <NUM> songs, <NUM> lyrics and <NUM> playlists.

* Untracked media:
       Add them to "Untracked Media.m3u8" or "Untracked Media.m3u8.media-dl".
       * "<PATH>" "<TITLE>" - "<ARTIST>" (<DURATION>): Is not part of any playlist.
       Run `<binary_name> --no-network --untrack <PATH>` to suppress and mark it as a free media file.
       * "<URL>" "<TITLE>" - "<ARTIST>" (<DURATION>): Is unavailable.
       Run `<binary_name> --no-network --skip-remote-download <URL>` to suppress.
       * <URL> "<TITLE>" - "<ARTIST>" (<DURATION>) (song): Is unavailable but downloaded as "<PATH>".
       Run `<binary_name> --no-network --skip-remote-download <PATH>` to suppress.
       * <URL> "<TITLE>" - "<ARTIST>" (<DURATION>): Failed to download with:\
              * <ERROR>
       Run `<binary_name> --no-network --skip-remote-download <PATH>` to suppress.
* Untracked playlists:
       * "<PATH>" "<TITLE>": Does not track a remote playlist.
       Run `<binary_name> --no-network --untrack <PATH>` to suppress and mark it as local playlist.
* Tracked playlists
       * <URL> "<TITLE>" (playlist): Is unavailable.
       Run `<binary_name> --no-network --skip-remote-download <URL>` to suppress.
       * <URL> "<TITLE>" "<PATH>" (playlist): Is unavailable but downloaded locally.
       It has <NUM_ENTRIES> entries.
       Run `<binary_name> --no-network --skip-remote-download <PATH>` to suppress.
       It is missing the following files:
              * <PATH>
       Media status:
              * <SAME AS UNTRACKED MEDIA BUT FOR THE MEDIA ITEMS OF THE PLAYLIST>
SECTION_END: took <HUMAN_TIME>
```
