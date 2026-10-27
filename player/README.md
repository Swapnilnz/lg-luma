# Luma Player

A webOS web app (`com.swapnil.lumaplayer`), Rust compiled to WebAssembly, that Luma launches
when a poster is opened. It finds streams on AIOStreams, plays them on LG's hardware pipeline
and keeps Trakt in sync. Pure logic lives in `crates/luma-core`, shared with Luma; this crate
is the browser side.

## Setup

```sh
cp player/config.example.json player/config.local.json   # then fill it in (gitignored)
```

| Key | Meaning |
|-----|---------|
| `aiostreams.manifest_url` | Your AIOStreams manifest URL. It is a credential: never publish the IPK |
| `subtitle_addons` | Stremio subtitle addon manifest URLs |
| `tmdb.api_key` | v3 key or v4 token: artwork, IMDb lookups, episode names, Next Up |
| `trakt.client_id` | The same Trakt app as Luma's (`client_secret` only for old apps) |
| `defaults` | Languages, subtitles on/off, autoplay, Next Up timing; overridden by Settings on the TV |
| `dev.log_url` | Dev builds post their log here (`node tools/devlog.mjs`) |
| `introdb` | Skip Intro / Recap from [IntroDB](https://introdb.app/docs): on unless `"enabled": false`; `base_url` overrides the API |

Luma side: add the `player` block to `feeds.json` (see `feeds.md`, "Luma Player").

## Dev loops

**0. Logic tests** (repo root, any shell with cargo):

```sh
cargo test -p luma-core
```

**1. Laptop** (Chrome, device toolbar at 1920 x 1080):

```sh
nix develop .#player
cd player && trunk serve --address 0.0.0.0 --port 8080 --features dev
```

Open `http://localhost:8080/?params=` plus URL-encoded launch JSON, for example:

- Movie: `?params=%7B%22type%22%3A%22movie%22%2C%22imdb%22%3A%22tt0133093%22%7D`
- Episode: `?params=%7B%22type%22%3A%22series%22%2C%22imdb%22%3A%22tt0944947%22%2C%22season%22%3A1%2C%22episode%22%3A2%7D`
- A test file (dev builds): add `"url":"https://.../file.mp4"`; `&t=3600` starts at an hour.

Keys: arrows, Enter = OK, Backspace/Escape = Back, Space = Play/Pause, `[` `]` = rewind and
fast-forward, `1`–`4` = Red, Green, Yellow, Blue. Desktop Chrome can't decode most real streams
(HEVC, DV, MKV); use an H.264 MP4 here.

**2. TV live** (every save reloads the player on the TV):

```sh
scripts/player-dev-shell.sh <laptop-ip>        # once: installs com.swapnil.lumaplayer.dev
node tools/devlog.mjs                          # the TV's log, in this terminal
ssh root@192.168.6.184 'luna-send -n 1 luna://com.webos.applicationManager/launch '\''{"id":"com.swapnil.lumaplayer.dev","params":{"type":"movie","imdb":"tt0133093","tmdb":603}}'\'''
```

Set `"player": {"app_id": "com.swapnil.lumaplayer.dev"}` in Luma's `feeds.json` to open your
live code from posters.

**3. Packaged** (file://, the real install):

```sh
nix develop .#player -c scripts/player-deploy.sh
# TV before webOS 24 (Chromium < 96): nix develop .#player-mvp -c scripts/player-deploy.sh
```

## How it behaves

The controls are an Apple TV-style bar: the scrubber, and below it Play/Pause, back and
forward 10 s, then Episodes and Next Episode (shows), Audio & Subtitles, Playback Speed,
Streams, Info and Settings. Each opens as a popover above the bar; Episodes opens a shelf of
season tabs and episode stills along the bottom.

| Key | Controls hidden | On the scrubber | On the buttons | In a panel |
|-----|-----------------|-----------------|----------------|------------|
| OK | Show the controls | Play/pause | Press the button | Select |
| Up | Show the controls | | Back to the scrubber | Move |
| Down | Show the buttons | Down to the buttons | | Move |
| Left / Right | Seek 10 s | Seek 10 s (presses add up, land after 0.6 s) | Move between buttons | Adjust a setting; shelf: seasons or episodes |
| Back | Exit to Luma | Hide the controls (exit when paused) | Hide the controls | Close |
| Rewind / FF | Seek 60 s | Seek 60 s | Seek 60 s | |
| Play, Pause, Stop | As labelled; Stop exits | | | |
| Red / Green / Yellow / Blue | Info / Streams / Audio & Subtitles / Settings | | | Switch panel |

Paused for 10 s, the controls give way to a pause screen: the title or logo, the episode,
its synopsis, where you paused and how long is left, and the time of day top right. Any key
brings the controls back; OK or Play resumes.

- **Start:** TMDB backdrop and logo while AIOStreams answers (15 s, one automatic retry). The
  top stream plays, or the one in the same `bingeGroup` as the last episode.
- **Resume:** the newer of Trakt's playback point and the local one (saved every 15 s). Past
  2 minutes and before 95% it asks "Resume from 1:12:30" or "Start over"; the stream buffers
  meanwhile. An autoplayed next episode resumes without asking.
- **Fallback:** an error, no picture in 12 s, or a 20 s stall in the first minute moves to the
  next stream with a toast; after 5 the picker opens with each failure shown. Later stalls of
  15 s reload the same stream at the position once, then fall back. A cached list (10 min) is
  fetched again on its first failure, since debrid links expire.
- **Trakt:** device sign-in with code and QR on first launch ("Not now" waits a week). Start,
  pause and stop scrobbles; at most one call per 10 s; exit before 80% pauses, at 80% or later
  stops (watched). Failed calls queue locally and replay on the next launch.
- **Next Up:** at 90% the next episode (TMDB seasons, specials skipped, unaired withheld) and
  its streams are fetched; when the credits start (IntroDB) or 45 s before the end, a card
  counts down 10 s, holding while paused. OK plays now, Back hides it. A season finale moves
  to the next season; a series finale shows no card. The card waits until the episode has
  played a minute and its duration has held steady for 30 s, and autoplay never resumes an
  old watch past 80%, so it can't chain into a loop.
- **Tracks:** audio and subtitles picked in the panel are remembered per show (or film), as
  Netflix does; new titles start from the preferred languages in Settings. Subtitles come
  from the file (if the TV lists them) or addons, rendered by the player with size,
  background and 250 ms timing steps. Switching streams asks the addons again for that
  release and picks the same language.
- **Skip Intro / Skip Recap:** IntroDB's community timestamps, where an episode has them. The
  button shows for the segment; with the controls down OK skips and Back hides it. Settings
  can skip automatically. IntroDB's credits start, when known, is when Next Up appears.
- **Episodes shelf:** watched episodes get a tick and partly watched ones a progress bar, from
  Trakt (and the player's own resume points).
- **On a laptop**, a stream whose audio Chrome can't decode (Dolby, DTS) plays silent; after
  6 s a toast says so. The TV decodes those in hardware.

Choices made where the plan left it open, as Netflix and Apple TV do it:

- OK shows the controls and a second OK pauses: the Magic Remote has no Play/Pause buttons.
- Picking another episode on the shelf asks to resume it if it has a saved place; Next
  Episode starts it from the top (or resumes silently, as autoplay does).
- Playback speed resets to Normal for each new title.
- A show card with no episode and no Trakt history starts at S1 E1.
- Back from the Resume question, a message or the picker (nothing playing) exits to Luma.
- After the last episode or a movie ends, the player closes back to Luma.

## Files

| File | Owns |
|------|------|
| `src/app.rs` | State, launch flow, fallback, scrobbling, resume, Next Up, keys |
| `src/app/controls.rs` | Control-bar buttons, Episodes shelf, Next Episode, speed, pause screen |
| `src/ui.rs` | HTML for each layer: stages, controls, Next Up, panels and their rows |
| `src/video.rs` | `<video>`, audio and text tracks through Reflect |
| `src/net.rs` | `fetch` with timeouts, `keepalive` for exit scrobbles |
| `src/store.rs` | localStorage: token, resume points, settings, scrobble queue |
| `src/config.rs`, `build.rs` | `config.local.json` baked in at build time |
| `src/devlog.rs` | Console and laptop log, URLs cut to their host, panics with file and line |
| `style.css` | Layout at 1920 x 1080; avoids `inset` and flex `gap` for older webOS Chromium |

The UI is plain `web-sys` (state rendered to HTML strings, one key handler) rather than
Leptos: it type-checks without a browser, keeps the wasm small, and has no framework
assumptions about the TV's Chromium.
