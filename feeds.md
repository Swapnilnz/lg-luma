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

Above the rows, a full-width banner cycles through the titles of the `hero` rows every nine
seconds: backdrop, logo, caption and synopsis, and a Resume / Play button. With focus on it,
Left and Right turn the slides by hand and OK opens the title. Settings > Interface > Top Shelf
turns it off. It needs a TMDB key for the artwork.

## Title pages

**Menu** on any poster (in a row, the Top Shelf, Search, or a title page's More Like This)
opens the title's page: logo, year, length, genres and classification (Australia's first,
else the US one), TMDB's score and, with an MDBList key, IMDb, Rotten Tomatoes, Metacritic and
Letterboxd scores, the synopsis and director or creators. Below that:

- Buttons: Play or Resume (opens Nuvio), Trailer, and with Trakt: Watchlist, Mark watched,
  Rate, and for shows Hide from Up Next.
- For shows, season pills and that season's episodes with stills; watched ones are ticked.
  The page opens on the season of the card's episode, or of the next one to watch. OK on an
  episode opens the show in Nuvio (Nuvio's links open a show, not one episode); holding OK on
  an episode marks it watched on Trakt.
- Cast, and More Like This, whose posters open their own page. Back returns to the page
  before, then to Home.

## Quick actions

**Holding OK** on a poster opens its quick actions: Details (its page), Play trailer, and with
Trakt: Add to / Remove from Watchlist, Mark as watched (for shows: the next episode, e.g.
"Mark S2, E4 watched"), Rate (Loved it 10 · Liked it 8 · It was OK 6 · Didn't like it 4 ·
Hated it 2) and Hide from Up Next. Trakt rows reload after each change. **Play** still plays
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
