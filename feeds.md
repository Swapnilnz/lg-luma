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
| `search_keys`           | Remote keys that open Search, by the `keycode` or `scancode` Luma logs (`Luma key: ...` in `/tmp/luma-home.log`). Default none; the header's Search button always works. |
| `rows`                  | Up to 60 rows, each a `title` and a `source`. Only rows near the screen keep artwork in memory. |

Sources:

- `trakt:continue`: paused movies and episodes, plus the next episode of recently watched shows.
- `trakt:playback`: paused movies and episodes only.
- `trakt:up_next`: next episodes of the 12 most recently watched shows.
- `trakt:watchlist`: your Trakt watchlist.
- `mdblist:USER/LIST-SLUG` or `mdblist:LIST_ID`: an MDBList list.

## Trakt sign-in

Create an API app at <https://trakt.tv/oauth/applications> with the redirect URI
`urn:ietf:wg:oauth:2.0:oob` (Trakt warns that it is not https; save it anyway). New apps get
only a Client ID ("signs users in with PKCE"); that is all Luma needs. On first start, the Trakt rows show a code: open
trakt.tv/activate on a phone and enter it. The sign-in is kept in `feeds-state.json` next to
`feeds.json`; delete that file to sign out.

## Search

With a TMDB key, a Search button appears in the header next to TV Settings. It opens a query field
with the TV keyboard (use its microphone to dictate) and shows matching movies and shows as
posters; selecting one opens it in Nuvio. Back closes Search.

To open Search from a remote button, press it on Home while Luma runs as Home, then read the
key Luma logged:

```sh
grep "Luma key" /tmp/luma-home.log
```

Put its `keycode` (or `scancode`) in `"search_keys"`, e.g. `"search_keys": [1073742106]`, and
restart Luma. If LG's own assistant still opens on that button, it cannot be taken over.
