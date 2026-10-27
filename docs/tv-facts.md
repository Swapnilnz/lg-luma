# TV facts (Phase 0)

What the TV itself says, answered with the dev shell before trusting the player. Fill in each
answer; they decide stream filtering, audio tracks and subtitles.

| # | Question | How | Answer |
|---|----------|-----|--------|
| 1 | webOS and Chromium version | `luna-send -n 1 luna://com.webos.service.tv.systemproperty/getSystemInfo '{"keys":["sdkVersion","modelName"]}'` and the player log's user agent | |
| 2 | Which AIOStreams results play in `<video>` | Launch with `"url"` for: 4K HEVC MKV, DV profile 5/7/8, HDR10, TrueHD, DTS, E-AC3, 1080p H.264 | |
| 3 | `video.audioTracks` lists and switches | Multi-audio MKV, Yellow > Audio | |
| 4 | Embedded MKV subtitles in `video.textTracks` | MKV with SRT and PGS, Yellow > Subtitles | |
| 5 | AIOStreams, Trakt, TMDB answer from `file://` | Packaged build (loop 3), watch the log for "network error" | |
| 6 | Range seeks on debrid URLs are fast | FF to 80% on a 60 GB remux, time to first frame | |
| 7 | Keycodes for Play, Pause, Stop, FF, RW, colours | Dev build logs "unmapped keyCode" for any key `luma-core/src/remote.rs` misses | |
| 8 | Screen saver stays off; HDR mode switches | 20 min HDR file untouched | |
| 9 | Release wasm loads from the packaged app | Loop 3 shows anything other than a blank screen | |
| 10 | IntroDB answers from `file://` | Packaged build, an episode with a known intro: the log shows "Luma Player IntroDB: intro true" | |

Notes:

- Stable Rust (1.82+) ships a `std` built with WebAssembly reference types, and wasm-bindgen
  then uses externref, which Chromium supports from 96 (webOS 24). If Q1 says Chromium is
  older, build from `nix develop .#player-mvp` (nightly, `build-std`, `target-cpu=mvp`).
