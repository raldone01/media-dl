# gytm-down

This tool should take a list of playlist files.
Then it downloads them all flat into a folder and lyrics. (opus files)
Then it lists all files that are not in any playlist.
It also stores the yt id.
It can verify if yt music files got taken down.

It will generate .m3u8 files for each playlist pointing to the music folder.

Implement cooldowns and retries (exponential bounded backoff).
Implement watching a cookies.txt until it is replaced with new ones.
Ask the user to change them when it fails.

# download only the playlist if the link refers to a video and playlist
yt-dlp --yes-playlist

# we want to create an archive file (temp file) upon app start
# and update it as we download more
--download-archive FILE
            Download  only videos not listed in the archive file.  Record the IDs of all
            downloaded videos in i

add has_ffprobe and has_ffmpeg (needed for --extract-audio)

--audio-quality 0 #best audio quality

--audio-format [aac, alac, flac, m4a, mp3, opus, vorbis, wav]

# maybe interesting
--convert-subs FORMAT
            Convert the subtitles to another format (currently supported: ass, lrc, srt,
            vtt).   Use  "--convert-subs  none"  to disable conversion (default) (Alias:
            --convert- subtitles)

   Preset Aliases:
     Predefined aliases for convenience and ease of use.  Note that future  versions  of
     yt-dlp may add or adjust presets, but the existing preset names will not be changed
     or removed

     -t mp3
            -f 'ba[acodec^=mp3]/ba/b' -x --audio-format mp3

     -t aac
            -f 'ba[acodec^=aac]/ba[acodec^=mp4a.40.]/ba/b' -x --audio-format aac

     -t mp4
            --merge-output-format   mp4   --remux-video  mp4  -S  vcodec:h264,lang,qual‐
            ity,res,fps,hdr:12,a codec:aac

     -t mkv
            --merge-output-format mkv --remux-video mkv

     -t sleep
            --sleep-subtitles 5 --sleep-requests 0.75 --sleep-interval  10  --max-sleep-
            interval 20

LRC API
https://github.com/elizatowinston-dev/elizato-dl/blob/main/elizato_dl/core.py
thumbnail fetching

extract_playlist_metadata

"--format",             "bestaudio/best",
"--add-metadata",
"--output",             str(output_dir / "%(artist,uploader)s - %(title)s.%(ext)s"), # want featuring and shit here too
"--extractor-args",     "youtube:player_client=tv,android_vr;player_skip=web_music",

first we start with single file downloads

Your program should avoid parsing the normal stdout since they may change in future
     versions.  Instead, they should use options such as  -J,  --print,  --progress-tem‐
     plate,  --exec  etc  to  create  console output that you can reliably reproduce and
     parse.
--get-description                --print description
 --get-description                --print description
            --get-duration                   --print duration_string
            --get-filename                   --print filename
            --get-format                     --print format
            --get-id                         --print id
            --get-thumbnail                  --print thumbnail
            -e, --get-title                  --print title
            -g, --get-url                    --print urls
            --match-title REGEX              --match-filters "title ~= (?i)REGEX"
            --reject-title REGEX             --match-filters "title !~= (?i)REGEX"
            --min-views COUNT                --match-filters "view_count >=? COUNT"
            --max-views COUNT                --match-filters "view_count <=? COUNT"
            --break-on-reject                Use --break-match-filters
            --user-agent UA                  --add-headers "User-Agent:UA"
            --referer URL                    --add-headers "Referer:URL"
            --playlist-start NUMBER          -I NUMBER:
            --playlist-end NUMBER            -I :NUMBER
            --playlist-reverse               -I ::-1
            --no-playlist-reverse            Default
            --no-colors                      --color no_color

https://github.com/yt-dlp/yt-dlp/wiki
