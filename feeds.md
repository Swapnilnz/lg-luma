# Home feed rows

Rows of titles from Trakt and MDBList appear on Home between Favorites and Inputs. Selecting a
card launches Nuvio (or `app_id`) with `{"contentTarget": "nuvio://detail/<type>/<imdb id>"}`,
which opens that title's detail screen. That needs a Nuvio build with webOS deep links
(NuvioTVSmart PR #1151); older builds open on their Home screen.

## Configuration

Copy `feeds.example.json` to `feeds.json` in Luma's preferences directory on the TV:

```sh
scp -O feeds.json root@TV_IP:/media/developer/com.ecklf.luma/feeds.json
```

Luma logs where it looks (`Luma feeds: ...`). The directory is the one holding
`preferences.json`; on some TVs it is `/media/developer/temp/com.ecklf.luma` or
`/media/internal/.com.ecklf.luma`. Restart Luma after changing the file. Without `feeds.json`
nothing changes and no requests are made.

| Key                     | Meaning                                                    |
| ----------------------- | ---------------------------------------------------------- |
| `app_id`                | App each card opens. Default `space.nuvio.webos`.          |
| `limit`                 | Cards per row, up to 40. Default 15.                       |
| `refresh_minutes`       | How often MDBList rows reload. Default 360.                |
| `trakt_refresh_minutes` | How often Trakt rows reload while Home stays open. Default 30; returning to Home also reloads them (at most every 2 minutes). |
| `trakt`                 | `client_id` of a Trakt API app. Apps created before October 2026 may also add their `client_secret`. |
| `mdblist`               | `api_key` from mdblist.com preferences.                    |
| `tmdb`                  | `api_key`: a TMDB v3 key or v4 read access token. Optional; without it cards show titles instead of artwork. |
| `search_keys`           | Remote keys that open Search, by the `keycode` or `scancode` Luma logs (`Luma key: ...` in `/tmp/luma-home.log`). Default none; the search bar above the rows always works. |
| `hero`                  | Row sources whose titles take turns in the Top Shelf banner, e.g. `["trakt:continue", "trakt:watchlist"]`. Default: the `trakt:continue`, `trakt:playback`, `trakt:up_next` and `trakt:watchlist` rows. |
| `rows`                  | Up to 60 rows, each a `title` and a `source`. Only rows near the screen keep artwork in memory. |

Sources:

- `trakt:continue`: paused movies and episodes, plus the next episode of recently watched shows.
- `trakt:playback`: paused movies and episodes only.
- `trakt:up_next`: next episodes of the 12 most recently watched shows.
- `trakt:watchlist`: your Trakt watchlist.
- `trakt:calendar`: episodes of your shows airing in the next seven days, soonest first, with
  a Today / Tonight / Tomorrow / weekday badge on the poster.
- `smart:because`: TMDB's recommendations for the title you watched most recently on Trakt,
  leaving out what you have watched. `smart:because:2` uses the second most recent, and so on.
  Put `{title}` in the row's title (`"Because you watched {title}"`) and it becomes that
  title's name; until it is known the row reads "Recommended for you".
- `smart:new_season`: shows you have finished whose next season is out. Checks up to 15 of
  your most recently watched shows each refresh.
- `mdblist:USER/LIST-SLUG` or `mdblist:LIST_ID`: an MDBList list.

`trakt:` rows reload every `trakt_refresh_minutes` and on returning to Home; `smart:` rows
need the Trakt sign-in too but reload with `refresh_minutes`, as lists do.

## Top Shelf

With the Top Shelf on, Home is two screens. The greeting, favourites and search bar fill the
first; pressing Down brings in the banner, which fills the whole second screen (the header
steps aside). Below it come the remaining rows.

- Its titles come from the `hero` rows (default: Continue Watching, Up Next, Watchlist), and
  those rows are no longer shown as rows. Slides change every nine seconds; Left and Right turn
  them, OK opens the title, Menu opens its page.
- Every slide's poster runs along the bottom right; the current one is lit.
- Above them, the `shelf` rows (default `trakt:calendar` and `smart:new_season`) show as two
  small posters, the first of each (or two from This Week), with an expand button that opens
  all of them in a grid. Right from the last slide reaches the calendar button; Down from the
  banner goes to the first row. These rows are not shown as rows either.
- Pressing Home again while on Home closes whatever is open and goes back to the very top
  (the Home key on a keyboard does the same in the desktop preview).
- Settings > Interface > Top Shelf turns it off: every row is then a row again.

| `feeds.json` key | Meaning | Default |
| --- | --- | --- |
| `hero` | Row sources whose titles take turns in the banner | Trakt continue, playback, up_next, watchlist |
| `shelf` | Row sources shown as small posters on the banner | `trakt:calendar`, `smart:new_season` |

## Wide posters

A focused poster in a row widens to the title's 16:9 backdrop with its logo once that artwork
has loaded, pushing the posters after it along, as Netflix and Nuvio do. Settings > Interface >
Wide poster on focus turns it off.

## Soundbar keep-awake

Some soundbars (Denon's auto standby) power down after minutes without sound. With Settings >
Interface > Keep soundbar awake on (the default), Luma plays a 2-second, 40 Hz tone about 48 dB
down every two minutes: too low to hear, enough to count as signal. It opens the TV's audio
output only for each tone. It keeps going while another app is in front, which also covers a
paused video, if the TV mixes a background app's sound; not every TV does. If the soundbar
still sleeps, raise the level (in Luma's environment, e.g. in `home.sh`):
`LUMA_KEEP_AWAKE_DB=-40`; `LUMA_KEEP_AWAKE_HZ` and `LUMA_KEEP_AWAKE_SECONDS` change the pitch
and the interval. Luma logs `Luma keep-awake:` lines if the TV gives it no audio output.

## Title pages

**OK** on a poster in a row, Search or the This Week sheet opens the title's page too
(Settings > Interface > Selecting a poster: Opens its page, the default, or Plays it); the
Top Shelf's OK still plays. **Menu** on any poster (in a row, the Top Shelf, Search, or a
title page's More Like This) opens the title's page: logo, year, length, genres and classification (Australia's first,
else the US one), TMDB's score and, with an MDBList key, IMDb, Rotten Tomatoes, Metacritic and
Letterboxd scores, the synopsis and director or creators. Below that:

- Buttons: Play or Resume (opens Nuvio), Start Over when part-watched, Trailer, and with
  Trakt: Watchlist, Mark watched, Rate, and for shows Hide from Up Next.
- For shows, season pills and that season's episodes with stills; watched ones are ticked.
  With Settings > Interface > Hide spoilers (on by default), episodes not watched yet show
  the show's backdrop, dimmed, instead of their still. Episodes not out yet are dimmed and
  marked with their date ("Out 13 Nov"), or "Not out yet" when TMDB has none; OK on one
  says so instead of playing.
  The page opens on the season of the card's episode, or of the next one to watch. OK on an
  episode opens the show in Nuvio (Nuvio's links open a show, not one episode); holding OK on
  an episode marks it watched on Trakt.
- Cast, and More Like This, whose posters open their own page. Back returns to the page
  before, then to Home.

## Quick actions

**Holding OK** on a poster opens its quick actions: Details (its page), Play trailer, and with
Trakt: Add to / Remove from Watchlist, Mark as watched (for shows: the next episode, e.g.
"Mark S2, E4 watched"), Rate (Loved it 10 · Liked it 8 · It was OK 6 · Didn't like it 4 ·
Hated it 2) and Hide from Up Next. A part-watched card's Play resumes straight away; Start
Over (here and on its page) plays from the beginning. On Continue Watching cards, Remove from Continue Watching
clears the title's paused playback on Trakt (and, for a show, hides it from Up Next, which
trakt.tv can undo). Trakt rows reload after each change. **Play** still plays
the trailer straight away. Title pages, Details and trailers need the TMDB key.

Continue Watching cards show how long is left ("S1, E4 · 23m left") beside the progress bar.

## Screensaver

After 10 minutes without a key press (Settings > Interface > Screensaver: Off, 5, 10, 15 or
30 minutes), Luma shows your rows' titles full screen, one every 12 seconds: each backdrop
drifts slowly, with the title's logo, the clock, the date and the weather of your weather
location. The clock shifts a little each minute so nothing stays in one place on an OLED
panel. OK opens the title on screen; any other key or moving the pointer returns to where you
were.

While the screensaver is on, Luma holds off LG's own screen saver so it does not cut in first.
After an hour of Luma's screensaver it lets LG's take over again, along with LG's power
saving.

## Luma Player

With a `player` block in `feeds.json`, OK on a card plays the title in Luma Player
(`player/`, its own webOS app) instead of opening it in Nuvio. Without the block, cards open
Nuvio as before.

```json
"player": { "app_id": "com.swapnil.lumaplayer", "play_with": "player", "nuvio_keys": [] }
```

- `play_with`: `player` (default) or `nuvio`, until **Settings > Play with** is changed on the TV.
- Cards pass their IMDb or TMDB ID, title and episode (Continue Watching, Up Next and an episode
  picked on a title page play that episode; a show card plays the next unwatched one).
- Holding OK on a card offers **Open in Nuvio** this once. `nuvio_keys` maps remote keys (as
  `Luma key:` logs them, e.g. the Blue button) to the same.
- Returning to Home from the player reloads the Trakt rows at once, skipping the two-minute
  limit, so Continue Watching shows what was just watched.

The player's own setup, dev loop and keys are in `player/README.md`.

## Trakt sign-in

Create an API app at <https://trakt.tv/oauth/applications> with the redirect URI
`urn:ietf:wg:oauth:2.0:oob` (Trakt warns that it is not https; save it anyway). New apps get
only a Client ID ("signs users in with PKCE"); that is all Luma needs. On first start, the Trakt rows show a code: open
trakt.tv/activate on a phone and enter it. The sign-in is kept in `feeds-state.json` next to
`feeds.json`; delete that file to sign out.

## Search

With a TMDB key, Home shows a search bar above the feed rows, with a microphone button beside
it. The bar opens Search with the TV keyboard; the microphone listens through the remote using
LG's own speech recognition (`com.webos.service.voiceconductor/recognizeVoice`, via Homebrew
Channel's root service unless Luma runs as Home) and searches what it heard. Results are TMDB
movies and shows as posters; selecting one opens it in Nuvio. Back closes Search.

LG keeps the remote's mic button for its own assistant, so Luma never receives it. LG Input
Hook can take it over instead: it runs a script that asks Luma to listen. This replaces the
mic button's normal action in every app, not just Luma. On the TV:

```sh
mkdir -p /home/root/.config/lginputhook
cat > /home/root/.config/lginputhook/luma-mic.sh << 'EOF'
#!/bin/sh
# LG Input Hook passes 1 for a press, 0 for a release.
[ "$1" = "1" ] || exit 0
touch /tmp/luma-voice-search
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}'
EOF
chmod +x /home/root/.config/lginputhook/luma-mic.sh
```

Then map key 428 (the mic button) to it in `/home/root/.config/lginputhook/keybinds.json`:

```json
{ "428": { "action": "exec", "command": "/home/root/.config/lginputhook/luma-mic.sh" } }
```

Input Hook reloads its keybinds within a few seconds. Remove the entry to give the mic button
back to LG.

Other buttons can open Search too: Luma logs remote keys it does not use (`Luma key: ...` in
`/tmp/luma-home.log`); put a `keycode` or `scancode` in `"search_keys"`.
