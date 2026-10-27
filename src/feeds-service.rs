//! The feed worker: Trakt (device sign-in, continue watching, watchlist), MDBList lists and
//! TMDB artwork. One thread does every request, decode and cache write; the SDL thread only
//! receives finished rows and decoded images.
//!
//! Configuration is `feeds.json` next to the preferences file. Without it nothing starts.
use crate::app_artwork::Artwork;
use crate::feeds::{
    Details, Episode, Item, Person, Row, Season, Status, TitleInfo, TraktAction, TraktTitle,
};
use image::{ImageReader, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const JSON_LIMIT: usize = 4 * 1024 * 1024;
const IMAGE_LIMIT: usize = 1024 * 1024;
const STATE_LIMIT: usize = 4 * 1024 * 1024;
const TRAKT: &str = "https://api.trakt.tv";
const MDBLIST: &str = "https://api.mdblist.com";
const TMDB: &str = "https://api.themoviedb.org/3";
/// Portrait posters, which carry the title artwork.
const TMDB_IMAGES: &str = "https://image.tmdb.org/t/p/w342";
/// Saved state from before portrait posters holds backdrop URLs and old captions; it is
/// dropped once.
const STATE_VERSION: u32 = 3;
const USER_AGENT: &str = "LumaNative-Feeds/1.0";
const DEFAULT_APP: &str = "space.nuvio.webos";
/// Recently watched shows checked for a next episode.
const UP_NEXT_SHOWS: usize = 12;
/// A failed row is tried again after this long.
const RETRY: Duration = Duration::from_secs(5 * 60);
/// Returning to Home refreshes Trakt rows at most this often.
const TRAKT_REFOCUS: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// App every card launches with `{"contentTarget": link}`.
    pub app_id: String,
    /// Cards per row.
    pub limit: usize,
    /// MDBList rows: minutes between refreshes.
    pub refresh_minutes: u64,
    /// Trakt rows: minutes between refreshes while Home stays open.
    pub trakt_refresh_minutes: u64,
    pub trakt: Option<TraktConfig>,
    pub mdblist: Option<KeyConfig>,
    pub tmdb: Option<KeyConfig>,
    pub rows: Vec<RowConfig>,
    /// Remote keys (SDL keycodes or scancodes, as Luma logs them) that open Search.
    pub search_keys: Vec<i32>,
    /// Row sources whose titles take turns in the Top Shelf banner. Default: the Trakt
    /// Continue Watching, Up Next and Watchlist rows. With the Top Shelf on, these rows are
    /// shown in it instead of as rows.
    pub hero: Vec<String>,
    /// Row sources shown as small posters on the Top Shelf instead of as rows. Default:
    /// `trakt:calendar` and `smart:new_season`.
    pub shelf: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TraktConfig {
    pub client_id: String,
    /// Only older Trakt apps have one: apps created since October 2026 are issued none, and
    /// the device sign-in then needs only the client ID.
    #[serde(default)]
    pub client_secret: Option<String>,
}

impl TraktConfig {
    /// Adds the client secret to an OAuth body when this app has one.
    fn with_secret(&self, mut body: Value) -> Value {
        if let Some(secret) = self
            .client_secret
            .as_deref()
            .map(str::trim)
            .filter(|secret| !secret.is_empty() && !secret.starts_with("YOUR_"))
            && let Some(fields) = body.as_object_mut()
        {
            fields.insert("client_secret".into(), Value::String(secret.to_owned()));
        }
        body
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyConfig {
    pub api_key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RowConfig {
    pub title: String,
    /// `trakt:continue`, `trakt:playback`, `trakt:up_next`, `trakt:watchlist`,
    /// `trakt:calendar`, `smart:because[:N]`, `smart:new_season`, `mdblist:USER/LIST` or
    /// `mdblist:LIST_ID`.
    pub source: String,
}

impl Config {
    fn normalized(mut self) -> Self {
        if self.app_id.trim().is_empty() {
            self.app_id = DEFAULT_APP.into();
        }
        if self.limit == 0 {
            self.limit = 15;
        }
        self.limit = self.limit.min(40);
        if self.refresh_minutes == 0 {
            self.refresh_minutes = 360;
        }
        if self.trakt_refresh_minutes == 0 {
            self.trakt_refresh_minutes = 30;
        }
        self.rows.retain(|row| !row.source.trim().is_empty());
        self.rows.truncate(60);
        self
    }

    fn interval(&self, source: &str) -> Duration {
        let minutes = if is_trakt(source) {
            self.trakt_refresh_minutes
        } else {
            self.refresh_minutes
        };
        Duration::from_secs(minutes.max(1) * 60)
    }
}

/// Trakt rows: they reload on returning to Home and keep their posters in memory.
fn is_trakt(source: &str) -> bool {
    source.starts_with("trakt:")
}

/// Rows that need the Trakt sign-in: Trakt's own, and the smart rows built from its history.
fn needs_trakt(source: &str) -> bool {
    is_trakt(source) || source.starts_with("smart:")
}

impl Config {
    /// Whether a row's titles also appear in the Top Shelf.
    fn hero_source(&self, source: &str) -> bool {
        if self.hero.is_empty() {
            matches!(
                source,
                "trakt:continue"
                    | "trakt:playback"
                    | "trakt:up_next"
                    | "trakt:upnext"
                    | "trakt:watchlist"
            )
        } else {
            self.hero.iter().any(|hero| hero == source)
        }
    }

    /// Whether a row's titles appear as small posters on the Top Shelf.
    fn shelf_source(&self, source: &str) -> bool {
        if self.shelf.is_empty() {
            matches!(
                source,
                "trakt:calendar" | "smart:new_season" | "smart:new_seasons"
            )
        } else {
            self.shelf.iter().any(|shelf| shelf == source)
        }
    }

    /// Rows whose posters stay loaded wherever Home is scrolled: Trakt's, and those the Top
    /// Shelf shows (it shows them all at once).
    fn always_loaded(&self, source: &str) -> bool {
        is_trakt(source) || self.hero_source(source) || self.shelf_source(source)
    }

    pub fn trakt_configured(&self) -> bool {
        self.trakt
            .as_ref()
            .is_some_and(|trakt| !trakt.client_id.trim().is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Token {
    access_token: String,
    refresh_token: String,
    created_at: u64,
    expires_in: u64,
}

impl Token {
    fn from_reply(value: &Value) -> Option<Self> {
        Some(Self {
            access_token: value.get("access_token")?.as_str()?.to_owned(),
            refresh_token: value.get("refresh_token")?.as_str()?.to_owned(),
            created_at: value
                .get("created_at")
                .and_then(Value::as_u64)
                .unwrap_or_else(now_secs),
            expires_in: value
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(24 * 60 * 60),
        })
    }

    fn expiring(&self) -> bool {
        self.created_at.saturating_add(self.expires_in) < now_secs() + 60 * 60
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedRow {
    updated_at: u64,
    items: Vec<Item>,
    /// A row's own title, when its source names it ("Because you watched ...").
    #[serde(default)]
    title: Option<String>,
}

/// What survives a restart: the Trakt sign-in, each row's last items and the TMDB artwork
/// already looked up.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    version: u32,
    token: Option<Token>,
    rows: HashMap<String, SavedRow>,
    artwork: HashMap<String, Option<String>>,
}

pub fn config_path(preferences: &Path) -> PathBuf {
    preferences.with_file_name("feeds.json")
}

pub fn state_path(preferences: &Path) -> PathBuf {
    preferences.with_file_name("feeds-state.json")
}

fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() > limit {
        return Err(format!("{} is too large", path.display()));
    }
    Ok(Some(bytes))
}

/// `Ok(None)` when there is no feeds.json: the feature stays off.
pub fn load_config(path: &Path) -> Result<Option<Config>, String> {
    let Some(bytes) = read_bounded(path, 64 * 1024)? else {
        return Ok(None);
    };
    let config: Config = serde_json::from_slice(&bytes)
        .map_err(|error| format!("feeds.json could not be read: {error}"))?;
    Ok(Some(config.normalized()))
}

pub fn load_state(path: &Path) -> Saved {
    read_bounded(path, STATE_LIMIT)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
        .map(|mut saved| {
            // Older rows and lookups point at landscape artwork: load them again as posters,
            // keeping the Trakt sign-in.
            if saved.version != STATE_VERSION {
                saved.rows.clear();
                saved.artwork.clear();
                saved.version = STATE_VERSION;
            }
            saved
        })
        .unwrap_or(Saved {
            version: STATE_VERSION,
            ..Saved::default()
        })
}

fn save_state(path: &Path, saved: &Saved) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(saved).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

/// What Home shows before the worker answers: each row's saved items, or a loading row.
pub fn initial_rows(config: &Config, saved: &Saved) -> Vec<Row> {
    config
        .rows
        .iter()
        .map(|row| match saved.rows.get(&row.source) {
            Some(saved) => Row {
                title: saved
                    .title
                    .clone()
                    .unwrap_or_else(|| static_title(&row.title)),
                pinned: config.always_loaded(&row.source),
                hero: config.hero_source(&row.source),
                extra: config.shelf_source(&row.source),
                status: Status::Ready,
                items: saved.items.clone(),
            },
            None => Row {
                title: static_title(&row.title),
                pinned: config.always_loaded(&row.source),
                hero: config.hero_source(&row.source),
                extra: config.shelf_source(&row.source),
                status: Status::Loading,
                items: vec![],
            },
        })
        .collect()
}

/// A row's heading before its source has named it: a "Because you watched {title}" row
/// reads "Recommended for you" until it knows the title.
fn static_title(title: &str) -> String {
    if title.contains("{title}") {
        "Recommended for you".into()
    } else {
        title.to_owned()
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub enum Command {
    /// Home came back on screen.
    Refresh,
    /// The rows on or near screen: only their artwork is downloaded and kept.
    /// With `keep`, posters already sent stay with Home wherever it scrolls.
    Near { rows: Vec<usize>, keep: bool },
    /// What Trakt knows about a title: Watchlist, watched episodes, next episode.
    TitleState(String),
    /// A change on Trakt.
    Action(String, TraktAction),
}

/// Requests for the title worker.
pub enum TitleCommand {
    Open(String),
    Season(String, u32),
}

pub enum Response {
    Rows(Vec<Row>),
    /// A title's preview; its backdrop and logo follow as `Image` responses.
    Details {
        link: String,
        result: Result<Details, String>,
    },
    /// TMDB results for `query`; their posters follow as `Image` responses.
    Search {
        query: String,
        result: Result<Vec<Item>, String>,
    },
    Image {
        url: String,
        image: Arc<Artwork>,
    },
    /// A title page's data; its backdrop, logo, cast and posters follow as `Image`s.
    Title {
        link: String,
        result: Result<TitleInfo, String>,
    },
    /// One season of a show's page; its stills follow as `Image`s.
    Season {
        link: String,
        season: u32,
        result: Result<Vec<Episode>, String>,
    },
    TraktState {
        link: String,
        result: Result<TraktTitle, String>,
    },
    /// A Trakt change finished: what to tell the viewer.
    Action {
        link: String,
        result: Result<String, String>,
    },
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Response>,
    /// Queries for the search worker; None without a TMDB key.
    search: Option<mpsc::Sender<String>>,
    /// Title links for the details worker (previews and trailers); None without a TMDB key.
    details: Option<mpsc::Sender<String>>,
    /// Title pages; None without a TMDB key.
    titles: Option<mpsc::Sender<TitleCommand>>,
    /// Titles whose artwork Home wants ahead of focus (slides, cards near the screen), each
    /// with whether its full-screen backdrop is wanted too; None without a TMDB key.
    slides: Option<mpsc::Sender<Vec<(String, bool)>>>,
    trakt: bool,
}

impl Service {
    /// Starts the worker when there is a configuration with rows.
    pub fn new(
        config: Option<Config>,
        saved: Saved,
        path: PathBuf,
        offset: time::UtcOffset,
    ) -> Self {
        let (sender, commands) = mpsc::channel();
        let (responses, receiver) = mpsc::channel();
        // Search runs on its own worker, so a long row load never holds up results.
        let search = config.as_ref().and_then(tmdb_key).map(|key| {
            let (queries, requests) = mpsc::channel();
            let responses = responses.clone();
            thread::spawn(move || search_worker(&key, requests, responses));
            queries
        });
        let details = config.as_ref().and_then(tmdb_key).map(|key| {
            let (links, requests) = mpsc::channel();
            let responses = responses.clone();
            thread::spawn(move || details_worker(&key, requests, responses));
            links
        });
        let titles = config.as_ref().and_then(tmdb_key).map(|key| {
            let mdblist = config.as_ref().and_then(mdblist_key);
            let (sender, requests) = mpsc::channel();
            let responses = responses.clone();
            thread::spawn(move || title_worker(&key, mdblist.as_deref(), requests, responses));
            sender
        });
        let slides = config.as_ref().and_then(tmdb_key).map(|key| {
            let (sender, requests) = mpsc::channel();
            let responses = responses.clone();
            thread::spawn(move || slides_worker(&key, requests, responses));
            sender
        });
        // Trakt changes run on the row worker, which holds the sign-in.
        let trakt = config
            .as_ref()
            .is_some_and(|config| config.trakt_configured() && !config.rows.is_empty());
        if let Some(config) = config.filter(|config| !config.rows.is_empty()) {
            thread::spawn(move || {
                Worker::new(config, saved, path, responses, commands, offset).run()
            });
        }
        Self {
            sender,
            receiver,
            search,
            details,
            titles,
            slides,
            trakt,
        }
    }

    /// Whether Trakt is configured: cards then offer Watchlist, Mark watched and Rate.
    pub fn trakt(&self) -> bool {
        self.trakt
    }

    pub fn title(&self, command: TitleCommand) {
        if let Some(titles) = &self.titles {
            let _ = titles.send(command);
        }
    }

    /// Details, logos and landscape art for these titles, and backdrops for those marked.
    pub fn slides(&self, links: Vec<(String, bool)>) {
        if let Some(slides) = &self.slides
            && !links.is_empty()
        {
            let _ = slides.send(links);
        }
    }

    pub fn trakt_state(&self, link: String) {
        let _ = self.sender.send(Command::TitleState(link));
    }

    pub fn trakt_action(&self, link: String, action: TraktAction) {
        let _ = self.sender.send(Command::Action(link, action));
    }

    /// Whether Search is available: it needs a TMDB key.
    pub fn can_search(&self) -> bool {
        self.search.is_some()
    }

    /// Fetches a title's preview: logo, backdrop, synopsis and trailer.
    pub fn details(&self, link: String) {
        if let Some(details) = &self.details {
            let _ = details.send(link);
        }
    }

    pub fn search(&self, query: String) {
        if let Some(search) = &self.search {
            let _ = search.send(query);
        }
    }

    pub fn near(&self, rows: Vec<usize>, keep: bool) {
        let _ = self.sender.send(Command::Near { rows, keep });
    }

    pub fn refresh(&self) {
        let _ = self.sender.send(Command::Refresh);
    }
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn json(&self) -> Result<Value, String> {
        serde_json::from_slice(&self.body)
            .map_err(|_| "The service returned unreadable data".into())
    }
}

struct Http {
    agent: ureq::Agent,
}

impl Http {
    fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .max_redirects(3)
            .proxy(None)
            .build()
            .new_agent();
        Self { agent }
    }

    fn get(
        &self,
        url: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        limit: usize,
    ) -> Result<Reply, String> {
        let mut request = self.agent.get(url).header("User-Agent", USER_AGENT);
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let response = request
            .call()
            .map_err(|error| connection_error(url, error))?;
        read(response, limit)
    }

    fn post(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
        limit: usize,
    ) -> Result<Reply, String> {
        let mut request = self
            .agent
            .post(url)
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/json");
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        let payload = body.to_string().into_bytes();
        let response = request
            .send(&payload[..])
            .map_err(|error| connection_error(url, error))?;
        read(response, limit)
    }
}

fn connection_error(url: &str, error: ureq::Error) -> String {
    let host = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url);
    eprintln!("Luma feeds: {host}: {error}");
    format!("{host} is unavailable. Trying again later.")
}

fn read(mut response: ureq::http::Response<ureq::Body>, limit: usize) -> Result<Reply, String> {
    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| "The response could not be read".to_string())?;
    if body.len() > limit {
        return Err("The response is too large".into());
    }
    Ok(Reply { status, body })
}

/// Percent-encodes one URL path segment.
fn segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Movie,
    Series,
}

impl Kind {
    fn from_text(value: &str) -> Option<Self> {
        match value {
            "movie" | "movies" => Some(Self::Movie),
            "show" | "shows" | "series" | "tv" => Some(Self::Series),
            _ => None,
        }
    }
    fn tmdb(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "tv",
        }
    }
    fn nuvio(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }
    fn trakt(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "show",
        }
    }
    fn trakt_group(self) -> &'static str {
        match self {
            Self::Movie => "movies",
            Self::Series => "shows",
        }
    }
}

/// A title before its artwork is resolved.
struct Entry {
    kind: Kind,
    title: String,
    subtitle: String,
    imdb: Option<String>,
    tmdb: Option<u64>,
    trakt: Option<u64>,
    progress: Option<f32>,
    /// RFC 3339 time of the activity, for newest-first ordering.
    at: String,
    episode: Option<[u32; 2]>,
    badge: Option<String>,
}

impl Entry {
    fn new(kind: Kind, title: String, ids: Ids) -> Self {
        Self {
            kind,
            title,
            subtitle: String::new(),
            imdb: ids.imdb,
            tmdb: ids.tmdb,
            trakt: ids.trakt,
            progress: None,
            at: String::new(),
            episode: None,
            badge: None,
        }
    }
}

fn episode_numbers(episode: &Value) -> Option<[u32; 2]> {
    let number = |key: &str| episode.get(key).and_then(Value::as_u64).map(|n| n as u32);
    Some([number("season")?, number("number")?])
}

/// "1h 5m left", "23m left".
fn time_left(runtime: Option<u64>, progress: f64) -> Option<String> {
    let minutes = (runtime? as f64 * (1. - progress / 100.)).round() as u64;
    (minutes > 0).then(|| match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m left"),
        (h, 0) => format!("{h}h left"),
        (h, m) => format!("{h}h {m}m left"),
    })
}

struct Ids {
    imdb: Option<String>,
    tmdb: Option<u64>,
    trakt: Option<u64>,
}

fn ids(media: &Value) -> Ids {
    let ids = media.get("ids");
    let field = |key: &str| ids.and_then(|ids| ids.get(key));
    Ids {
        imdb: field("imdb")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("tt"))
            .map(str::to_owned),
        tmdb: field("tmdb").and_then(Value::as_u64),
        trakt: field("trakt").and_then(Value::as_u64),
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn episode_code(episode: &Value) -> String {
    let number = |key: &str| episode.get(key).and_then(Value::as_u64).unwrap_or(0);
    format!("S{}, E{}", number("season"), number("number"))
}

fn link(kind: Kind, imdb: Option<&str>, tmdb: Option<u64>) -> Option<String> {
    if let Some(imdb) = imdb {
        return Some(format!("nuvio://detail/{}/{imdb}", kind.nuvio()));
    }
    tmdb.map(|id| format!("nuvio://tmdb/{}/{id}", kind.nuvio()))
}

/// TMDB's poster (with the title on it), else its landscape artwork.
fn image_path(value: &Value) -> Option<String> {
    ["poster_path", "backdrop_path"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|path| path.starts_with('/'))
        .map(str::to_owned)
}

fn decode(url: &str, bytes: &[u8]) -> Option<Artwork> {
    // A little over the card's 216 x 324, so the focused poster stays sharp as it grows.
    decode_as(format!("feed:{url}"), bytes, 240, 360)
}

/// Decodes artwork no larger than `width` x `height`; the key's prefix tells the renderer how
/// to fit it (`feed:` posters fill their card, `backdrop:` the screen, anything else contains).
fn decode_as(key: String, bytes: &[u8], width: u32, height: u32) -> Option<Artwork> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width() > width || image.height() > height {
        image.thumbnail(width, height)
    } else {
        image
    };
    Some(Artwork {
        key,
        pixels: image.into_rgba8(),
        color: [20, 20, 20],
    })
}

fn tmdb_key(config: &Config) -> Option<String> {
    config
        .tmdb
        .as_ref()
        .map(|tmdb| tmdb.api_key.trim().to_owned())
        .filter(|key| !key.is_empty() && !key.starts_with("YOUR_"))
}

fn tmdb_request(
    http: &Http,
    key: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Value, String> {
    let url = format!("{TMDB}{path}");
    // A v4 read access token is a JWT; a v3 key is a short hex string.
    let reply = if key.starts_with("eyJ") {
        let bearer = format!("Bearer {key}");
        http.get(
            &url,
            query,
            &[
                ("Authorization", bearer.as_str()),
                ("Accept", "application/json"),
            ],
            JSON_LIMIT,
        )?
    } else {
        let mut query = query.to_vec();
        query.push(("api_key", key));
        http.get(&url, &query, &[("Accept", "application/json")], JSON_LIMIT)?
    };
    if !reply.ok() {
        return Err(format!("TMDB answered HTTP {}", reply.status));
    }
    reply.json()
}

/// TMDB's movies and shows for `query`, as cards that open in the feeds app.
fn search(http: &Http, key: &str, query: &str) -> Result<Vec<Item>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(vec![]);
    }
    let value = tmdb_request(
        http,
        key,
        "/search/multi",
        &[("query", query), ("include_adult", "false"), ("page", "1")],
    )?;
    Ok(value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let kind = match result.get("media_type").and_then(Value::as_str)? {
                "movie" => Kind::Movie,
                "tv" => Kind::Series,
                // People are not titles Nuvio can open.
                _ => return None,
            };
            let id = result.get("id").and_then(Value::as_u64)?;
            let title = ["title", "name"]
                .iter()
                .find_map(|key| result.get(*key).and_then(Value::as_str))
                .unwrap_or("")
                .to_owned();
            let image = result
                .get("poster_path")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with('/'))
                .map(|path| format!("{TMDB_IMAGES}{path}"));
            Some(Item {
                title,
                subtitle: String::new(),
                link: link(kind, None, Some(id))?,
                image,
                ..Item::default()
            })
        })
        .take(28)
        .collect())
}

/// The TMDB movie or show behind a card's deep link: `nuvio://detail/<type>/<imdb id>` or
/// `nuvio://tmdb/<type>/<tmdb id>`.
fn tmdb_title(http: &Http, key: &str, link: &str) -> Result<(Kind, u64), String> {
    let mut parts = link
        .strip_prefix("nuvio://")
        .ok_or("Not a title link")?
        .split('/');
    let (source, kind, id) = (parts.next(), parts.next(), parts.next());
    let kind = kind.and_then(Kind::from_text).ok_or("Unknown title type")?;
    let id = id.ok_or("Missing title id")?;
    match source {
        Some("tmdb") => id
            .strip_prefix("tmdb:")
            .unwrap_or(id)
            .parse()
            .map(|id| (kind, id))
            .map_err(|_| "Invalid TMDB id".into()),
        Some("detail") => {
            let value = tmdb_request(
                http,
                key,
                &format!("/find/{}", segment(id)),
                &[("external_source", "imdb_id")],
            )?;
            let results = match kind {
                Kind::Movie => "movie_results",
                Kind::Series => "tv_results",
            };
            value
                .get(results)
                .and_then(Value::as_array)
                .and_then(|list| list.first())
                .and_then(|first| first.get("id"))
                .and_then(Value::as_u64)
                .map(|id| (kind, id))
                .ok_or_else(|| "TMDB does not know this title".into())
        }
        _ => Err("Not a title link".into()),
    }
}

/// The first of `keys` that holds text.
fn first_text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// Whether a TMDB video is of `kind` (and official, when asked).
fn video_is(video: &Value, kind: &str, official: bool) -> bool {
    video.get("type").and_then(Value::as_str) == Some(kind)
        && (!official || video.get("official").and_then(Value::as_bool) == Some(true))
}

/// Everything a preview shows, from one TMDB request with images and videos appended.
fn details(http: &Http, key: &str, link: &str) -> Result<Details, String> {
    let (kind, id) = tmdb_title(http, key, link)?;
    let value = tmdb_request(
        http,
        key,
        &format!("/{}/{id}", kind.tmdb()),
        &[
            ("append_to_response", "images,videos"),
            ("include_image_language", "en,null"),
        ],
    )?;
    Ok(details_from(kind, &value))
}

/// A preview's details from TMDB's movie or show, with `images` and `videos` appended.
fn details_from(kind: Kind, value: &Value) -> Details {
    let date = first_text(value, &["release_date", "first_air_date"]);
    let extent = match kind {
        Kind::Movie => value
            .get("runtime")
            .and_then(Value::as_u64)
            .filter(|minutes| *minutes > 0)
            .map(|minutes| match (minutes / 60, minutes % 60) {
                (0, m) => format!("{m}m"),
                (h, 0) => format!("{h}h"),
                (h, m) => format!("{h}h {m}m"),
            }),
        Kind::Series => value
            .get("number_of_seasons")
            .and_then(Value::as_u64)
            .filter(|seasons| *seasons > 0)
            .map(|seasons| format!("{seasons} Season{}", if seasons == 1 { "" } else { "s" })),
    };
    let rating = value
        .get("vote_average")
        .and_then(Value::as_f64)
        .filter(|_| value.get("vote_count").and_then(Value::as_u64).unwrap_or(0) >= 20)
        .filter(|rating| *rating > 0.)
        .map(|rating| rating as f32);
    let genres = value
        .get("genres")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|genre| genre.get("name").and_then(Value::as_str))
        .take(2)
        .map(str::to_owned)
        .collect();
    let path = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|path| path.starts_with('/'))
            .map(str::to_owned)
    };
    // An English logo first, then one without language (text-free marks).
    let logos: Vec<&Value> = value
        .get("images")
        .and_then(|images| images.get("logos"))
        .and_then(Value::as_array)
        .map(|logos| logos.iter().collect())
        .unwrap_or_default();
    let logo = ["en", ""]
        .iter()
        .find_map(|language| {
            logos.iter().find(|logo| {
                logo.get("iso_639_1").and_then(Value::as_str).unwrap_or("") == *language
            })
        })
        .and_then(|logo| path(logo.get("file_path")))
        .map(|path| format!("https://image.tmdb.org/t/p/w500{path}"));
    let videos: Vec<&Value> = value
        .get("videos")
        .and_then(|videos| videos.get("results"))
        .and_then(Value::as_array)
        .map(|videos| {
            videos
                .iter()
                .filter(|video| video.get("site").and_then(Value::as_str) == Some("YouTube"))
                .collect()
        })
        .unwrap_or_default();
    let trailer = videos
        .iter()
        .find(|video| video_is(video, "Trailer", true))
        .or_else(|| {
            videos
                .iter()
                .find(|video| video_is(video, "Trailer", false))
        })
        .or_else(|| videos.iter().find(|video| video_is(video, "Teaser", false)))
        .and_then(|video| video.get("key").and_then(Value::as_str))
        .filter(|key| {
            !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
        .map(str::to_owned);
    Details {
        title: first_text(value, &["title", "name"]),
        overview: first_text(value, &["overview"]),
        year: date.chars().take(4).collect(),
        rating,
        extent,
        genres,
        backdrop: path(value.get("backdrop_path"))
            .map(|path| format!("https://image.tmdb.org/t/p/w1280{path}")),
        landscape: path(value.get("backdrop_path"))
            .map(|path| format!("https://image.tmdb.org/t/p/w780{path}")),
        logo,
        trailer,
    }
}

/// Answers the newest title only, then fetches its backdrop and logo.
fn details_worker(key: &str, links: mpsc::Receiver<String>, responses: mpsc::Sender<Response>) {
    let http = Http::new();
    let mut next: Option<String> = None;
    loop {
        let mut link = match next.take() {
            Some(link) => link,
            None => match links.recv() {
                Ok(link) => link,
                Err(_) => return,
            },
        };
        while let Ok(newer) = links.try_recv() {
            link = newer;
        }
        let result = details(&http, key, &link);
        let images: Vec<(String, String, u32, u32)> = result
            .as_ref()
            .map(|details| {
                // The wide card's artwork first: focus shows it as soon as it lands.
                let landscape = details
                    .landscape
                    .clone()
                    .map(|url| (format!("feed:{url}"), url, 640, 360));
                let backdrop = details
                    .backdrop
                    .clone()
                    .map(|url| (format!("backdrop:{url}"), url, 1280, 720));
                let logo = details
                    .logo
                    .clone()
                    .map(|url| (format!("logo:{url}"), url, 640, 260));
                landscape.into_iter().chain(logo).chain(backdrop).collect()
            })
            .unwrap_or_default();
        if let Err(error) = &result {
            eprintln!("Luma feeds: preview {link}: {error}");
        }
        if responses.send(Response::Details { link, result }).is_err() {
            return;
        }
        for (image_key, url, width, height) in images {
            // Focus moved on: that title's artwork, not this one's.
            if let Ok(newer) = links.try_recv() {
                next = Some(newer);
                break;
            }
            let Ok(reply) = http.get(&url, &[], &[], 4 * IMAGE_LIMIT) else {
                continue;
            };
            if !reply.ok() {
                continue;
            }
            if let Some(image) = decode_as(image_key, &reply.body, width, height) {
                let response = Response::Image {
                    url,
                    image: Arc::new(image),
                };
                if responses.send(response).is_err() {
                    return;
                }
            }
        }
    }
}

fn mdblist_key(config: &Config) -> Option<String> {
    config
        .mdblist
        .as_ref()
        .map(|mdblist| mdblist.api_key.trim().to_owned())
        .filter(|key| !key.is_empty() && !key.starts_with("YOUR_"))
}

/// Profile photos and episode stills, at TMDB's small sizes.
const TMDB_PROFILES: &str = "https://image.tmdb.org/t/p/w185";
const TMDB_STILLS: &str = "https://image.tmdb.org/t/p/w300";

/// A title's classification: Australia's first, else the US one.
fn certification(kind: Kind, value: &Value) -> String {
    let results: Vec<&Value> = match kind {
        Kind::Movie => value.get("release_dates"),
        Kind::Series => value.get("content_ratings"),
    }
    .and_then(|ratings| ratings.get("results"))
    .and_then(Value::as_array)
    .map(|results| results.iter().collect())
    .unwrap_or_default();
    ["AU", "US"]
        .iter()
        .find_map(|country| {
            let result = results.iter().find(|result| {
                result.get("iso_3166_1").and_then(Value::as_str) == Some(*country)
            })?;
            let rating = match kind {
                Kind::Movie => result
                    .get("release_dates")
                    .and_then(Value::as_array)?
                    .iter()
                    .filter_map(|date| date.get("certification").and_then(Value::as_str))
                    .find(|rating| !rating.trim().is_empty())?,
                Kind::Series => result.get("rating").and_then(Value::as_str)?,
            };
            let rating = rating.trim();
            (!rating.is_empty()).then(|| rating.to_owned())
        })
        .unwrap_or_default()
}

/// IMDb, Rotten Tomatoes, Metacritic and Letterboxd scores from MDBList. Best effort: a
/// failure only leaves them out.
fn mdblist_ratings(http: &Http, key: &str, kind: Kind, id: u64) -> Vec<String> {
    let kind = match kind {
        Kind::Movie => "movie",
        Kind::Series => "show",
    };
    let reply = match http.get(
        &format!("{MDBLIST}/tmdb/{kind}/{id}"),
        &[("apikey", key)],
        &[("Accept", "application/json")],
        JSON_LIMIT,
    ) {
        Ok(reply) if reply.ok() => reply,
        Ok(reply) => {
            eprintln!("Luma feeds: MDBList ratings HTTP {}", reply.status);
            return vec![];
        }
        Err(_) => return vec![],
    };
    let Ok(value) = reply.json() else {
        return vec![];
    };
    let ratings: Vec<(&str, f64)> = value
        .get("ratings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rating| {
            let source = rating.get("source").and_then(Value::as_str)?;
            let value = rating.get("value").and_then(Value::as_f64)?;
            (value > 0.).then_some((source, value))
        })
        .collect();
    let find = |source: &str| {
        ratings
            .iter()
            .find(|(candidate, _)| *candidate == source)
            .map(|(_, value)| *value)
    };
    let mut out = Vec::new();
    if let Some(value) = find("imdb") {
        out.push(format!("IMDb {value:.1}"));
    }
    if let Some(value) = find("tomatoes") {
        out.push(format!("RT {value:.0}%"));
    }
    if let Some(value) = find("metacritic") {
        out.push(format!("Metacritic {value:.0}"));
    }
    if let Some(value) = find("letterboxd") {
        out.push(format!("Letterboxd {value:.1}"));
    }
    out
}

/// Everything a title page shows, from one TMDB request (and MDBList's ratings).
fn title_info(
    http: &Http,
    key: &str,
    mdblist: Option<&str>,
    title_link: &str,
) -> Result<(Kind, u64, TitleInfo), String> {
    let (kind, id) = tmdb_title(http, key, title_link)?;
    let extra = match kind {
        Kind::Movie => "images,videos,credits,recommendations,release_dates",
        Kind::Series => "images,videos,credits,recommendations,content_ratings",
    };
    let value = tmdb_request(
        http,
        key,
        &format!("/{}/{id}", kind.tmdb()),
        &[
            ("append_to_response", extra),
            ("include_image_language", "en,null"),
        ],
    )?;
    let details = details_from(kind, &value);
    let mut ratings: Vec<String> = details
        .rating
        .map(|rating| format!("TMDB {rating:.1}"))
        .into_iter()
        .collect();
    if let Some(mdblist) = mdblist {
        ratings.extend(mdblist_ratings(http, mdblist, kind, id));
    }
    let names = |list: Option<&Value>, filter: &dyn Fn(&Value) -> bool| -> Vec<String> {
        list.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|person| filter(person))
            .filter_map(|person| person.get("name").and_then(Value::as_str))
            .take(2)
            .map(str::to_owned)
            .collect()
    };
    let credit = match kind {
        Kind::Movie => {
            let directors = names(
                value.get("credits").and_then(|credits| credits.get("crew")),
                &|person: &Value| person.get("job").and_then(Value::as_str) == Some("Director"),
            );
            if directors.is_empty() {
                String::new()
            } else {
                format!("Directed by {}", directors.join(" and "))
            }
        }
        Kind::Series => {
            let creators = names(value.get("created_by"), &|_: &Value| true);
            if creators.is_empty() {
                String::new()
            } else {
                format!("Created by {}", creators.join(" and "))
            }
        }
    };
    let cast = value
        .get("credits")
        .and_then(|credits| credits.get("cast"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(15)
        .filter_map(|person| {
            Some(Person {
                name: person.get("name").and_then(Value::as_str)?.to_owned(),
                role: first_text(person, &["character"]),
                image: person
                    .get("profile_path")
                    .and_then(Value::as_str)
                    .filter(|path| path.starts_with('/'))
                    .map(|path| format!("{TMDB_PROFILES}{path}")),
            })
        })
        .collect();
    let seasons = match kind {
        Kind::Movie => vec![],
        Kind::Series => value
            .get("seasons")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|season| {
                let number = season.get("season_number").and_then(Value::as_u64)? as u32;
                let episodes = season.get("episode_count").and_then(Value::as_u64)? as u32;
                (number > 0 && episodes > 0).then(|| Season {
                    number,
                    name: first_text(season, &["name"]),
                })
            })
            .collect(),
    };
    let more = value
        .get("recommendations")
        .and_then(|recommendations| recommendations.get("results"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let kind = result
                .get("media_type")
                .and_then(Value::as_str)
                .and_then(Kind::from_text)
                .unwrap_or(kind);
            let id = result.get("id").and_then(Value::as_u64)?;
            Some(Item {
                title: first_text(result, &["title", "name"]),
                link: link(kind, None, Some(id))?,
                image: result
                    .get("poster_path")
                    .and_then(Value::as_str)
                    .filter(|path| path.starts_with('/'))
                    .map(|path| format!("{TMDB_IMAGES}{path}")),
                ..Item::default()
            })
        })
        .take(20)
        .collect();
    Ok((
        kind,
        id,
        TitleInfo {
            details,
            series: kind == Kind::Series,
            certification: certification(kind, &value),
            ratings,
            credit,
            cast,
            seasons,
            more,
        },
    ))
}

/// One season's episodes, from TMDB.
fn season_episodes(http: &Http, key: &str, id: u64, season: u32) -> Result<Vec<Episode>, String> {
    let value = tmdb_request(http, key, &format!("/tv/{id}/season/{season}"), &[])?;
    Ok(value
        .get("episodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|episode| {
            Some(Episode {
                season,
                number: episode.get("episode_number").and_then(Value::as_u64)? as u32,
                name: first_text(episode, &["name"]),
                runtime: episode
                    .get("runtime")
                    .and_then(Value::as_u64)
                    .filter(|minutes| *minutes > 0)
                    .map(|minutes| minutes as u32),
                still: episode
                    .get("still_path")
                    .and_then(Value::as_str)
                    .filter(|path| path.starts_with('/'))
                    .map(|path| format!("{TMDB_STILLS}{path}")),
                aired: first_text(episode, &["air_date"]),
            })
        })
        .collect())
}

/// Downloads and decodes artwork, then sends it to Home. False once Home has gone away.
fn send_image(
    http: &Http,
    responses: &mpsc::Sender<Response>,
    key: String,
    url: String,
    size: (u32, u32),
    limit: usize,
) -> bool {
    let Ok(reply) = http.get(&url, &[], &[], limit) else {
        return true;
    };
    if !reply.ok() {
        return true;
    }
    match decode_as(key, &reply.body, size.0, size.1) {
        Some(image) => responses
            .send(Response::Image {
                url,
                image: Arc::new(image),
            })
            .is_ok(),
        None => true,
    }
}

/// A title's backdrop and logo, as previews key them.
fn title_artwork(details: &Details) -> Vec<(String, String, (u32, u32), usize)> {
    let backdrop = details
        .backdrop
        .clone()
        .map(|url| (format!("backdrop:{url}"), url, (1280, 720), 4 * IMAGE_LIMIT));
    let logo = details
        .logo
        .clone()
        .map(|url| (format!("logo:{url}"), url, (640, 260), 4 * IMAGE_LIMIT));
    // The landscape cut too, so a card of this title widens as soon as it is focused.
    let landscape = details
        .landscape
        .clone()
        .map(|url| (format!("feed:{url}"), url, (640, 360), IMAGE_LIMIT));
    backdrop.into_iter().chain(logo).chain(landscape).collect()
}

/// Title pages: a title's data, then its artwork, cast photos and posters; and a season's
/// episodes, then their stills. A newer request cuts short the artwork of an older one.
fn title_worker(
    key: &str,
    mdblist: Option<&str>,
    commands: mpsc::Receiver<TitleCommand>,
    responses: mpsc::Sender<Response>,
) {
    let http = Http::new();
    // Titles already looked up, so Back and returning to a page are instant.
    let mut known: HashMap<String, (Kind, u64, TitleInfo)> = HashMap::new();
    let mut next: Option<TitleCommand> = None;
    loop {
        let command = match next.take() {
            Some(command) => command,
            None => match commands.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        let images: Vec<(String, String, (u32, u32), usize)> = match command {
            TitleCommand::Open(link) => {
                let result = match known.get(&link) {
                    Some(found) => Ok(found.clone()),
                    None => title_info(&http, key, mdblist, &link),
                };
                let images = match &result {
                    Ok((_, _, info)) => {
                        let mut images = title_artwork(&info.details);
                        images.extend(info.cast.iter().filter_map(|person| {
                            let url = person.image.clone()?;
                            Some((format!("feed:{url}"), url, (180, 270), IMAGE_LIMIT))
                        }));
                        images.extend(info.more.iter().filter_map(|item| {
                            let url = item.image.clone()?;
                            Some((format!("feed:{url}"), url, (240, 360), IMAGE_LIMIT))
                        }));
                        images
                    }
                    Err(_) => vec![],
                };
                if let Ok(found) = &result {
                    if known.len() > 30 {
                        known.clear();
                    }
                    known.insert(link.clone(), found.clone());
                }
                let result = result.map(|(_, _, info)| info);
                if responses.send(Response::Title { link, result }).is_err() {
                    return;
                }
                images
            }
            TitleCommand::Season(link, season) => {
                let id = match known.get(&link) {
                    Some((_, id, _)) => Ok(*id),
                    None => tmdb_title(&http, key, &link).map(|(_, id)| id),
                };
                let result = id.and_then(|id| season_episodes(&http, key, id, season));
                let images = match &result {
                    Ok(episodes) => episodes
                        .iter()
                        .filter_map(|episode| {
                            let url = episode.still.clone()?;
                            Some((format!("feed:{url}"), url, (400, 225), IMAGE_LIMIT))
                        })
                        .collect(),
                    Err(_) => vec![],
                };
                let response = Response::Season {
                    link,
                    season,
                    result,
                };
                if responses.send(response).is_err() {
                    return;
                }
                images
            }
        };
        for (image_key, url, size, limit) in images {
            if let Ok(newer) = commands.try_recv() {
                next = Some(newer);
                break;
            }
            if !send_image(&http, &responses, image_key, url, size, limit) {
                return;
            }
        }
    }
}

/// Artwork ahead of focus: each title's details, then its logo and landscape art, and its
/// full-screen backdrop when marked. A newer list replaces the rest of an older one.
fn slides_worker(
    key: &str,
    requests: mpsc::Receiver<Vec<(String, bool)>>,
    responses: mpsc::Sender<Response>,
) {
    let http = Http::new();
    let mut known: HashMap<String, Details> = HashMap::new();
    let mut next: Option<Vec<(String, bool)>> = None;
    loop {
        let mut links = match next.take() {
            Some(links) => links,
            None => match requests.recv() {
                Ok(links) => links,
                Err(_) => return,
            },
        };
        while let Ok(newer) = requests.try_recv() {
            links = newer;
        }
        'links: for (link, full) in links {
            let result = match known.get(&link) {
                Some(found) => Ok(found.clone()),
                None => details(&http, key, &link),
            };
            let Ok(found) = result else {
                continue;
            };
            if known.len() > 400 {
                known.clear();
            }
            known.insert(link.clone(), found.clone());
            let images: Vec<_> = title_artwork(&found)
                .into_iter()
                .filter(|(image_key, ..)| full || !image_key.starts_with("backdrop:"))
                .collect();
            let response = Response::Details {
                link,
                result: Ok(found),
            };
            if responses.send(response).is_err() {
                return;
            }
            for (image_key, url, size, limit) in images {
                if let Ok(newer) = requests.try_recv() {
                    next = Some(newer);
                    break 'links;
                }
                if !send_image(&http, &responses, image_key, url, size, limit) {
                    return;
                }
            }
        }
    }
}

/// Answers the newest query only, then fetches its posters until a newer query arrives.
fn search_worker(key: &str, queries: mpsc::Receiver<String>, responses: mpsc::Sender<Response>) {
    let http = Http::new();
    let mut next: Option<String> = None;
    loop {
        let mut query = match next.take() {
            Some(query) => query,
            None => match queries.recv() {
                Ok(query) => query,
                Err(_) => return,
            },
        };
        while let Ok(newer) = queries.try_recv() {
            query = newer;
        }
        let result = search(&http, key, &query);
        let images: Vec<String> = result
            .as_ref()
            .map(|items| items.iter().filter_map(|item| item.image.clone()).collect())
            .unwrap_or_default();
        if responses.send(Response::Search { query, result }).is_err() {
            return;
        }
        for url in images {
            if let Ok(newer) = queries.try_recv() {
                next = Some(newer);
                break;
            }
            let Ok(reply) = http.get(&url, &[], &[], IMAGE_LIMIT) else {
                continue;
            };
            if !reply.ok() {
                continue;
            }
            if let Some(image) = decode(&url, &reply.body) {
                let response = Response::Image {
                    url,
                    image: Arc::new(image),
                };
                if responses.send(response).is_err() {
                    return;
                }
            }
        }
    }
}

struct Device {
    device_code: String,
    user_code: String,
    url: String,
    interval: Duration,
    expires: Instant,
    next_poll: Instant,
}

struct Worker {
    config: Config,
    saved: Saved,
    path: PathBuf,
    http: Http,
    responses: mpsc::Sender<Response>,
    status: Vec<Status>,
    items: Vec<Vec<Item>>,
    next_due: Vec<Instant>,
    device: Option<Device>,
    auth_error: Option<String>,
    auth_retry: Option<Instant>,
    images: HashSet<String>,
    /// Artwork that failed to download, and when to try it again.
    image_retry: HashMap<String, Instant>,
    last_refocus: Option<Instant>,
    commands: mpsc::Receiver<Command>,
    /// A Refresh arrived: Trakt rows load on the next pass.
    refocus: bool,
    /// Rows whose artwork Home wants.
    near: HashSet<usize>,
    near_changed: bool,
    /// "Keep loaded posters": Home never drops a poster, so neither does this worker.
    keep: bool,
    /// Each row's own title, when its source names it ("Because you watched ...").
    titles: Vec<Option<String>>,
    /// Trakt lookups and changes from menus and title pages, run between row loads.
    actions: Vec<Command>,
    /// Trakt IDs on the Watchlist, by kind, and when they were read.
    watchlist: HashMap<Kind, (Instant, HashSet<u64>)>,
    /// The TV's time zone, for the calendar's "Tonight" and weekdays.
    offset: time::UtcOffset,
}

/// A fetched row: its cards, and its own title when the source names it.
struct Fetched {
    items: Vec<Item>,
    title: Option<String>,
}

impl Worker {
    fn new(
        config: Config,
        saved: Saved,
        path: PathBuf,
        responses: mpsc::Sender<Response>,
        commands: mpsc::Receiver<Command>,
        offset: time::UtcOffset,
    ) -> Self {
        let now = Instant::now();
        let age_now = now_secs();
        let mut status = Vec::new();
        let mut items = Vec::new();
        let mut next_due = Vec::new();
        let mut titles = Vec::new();
        for row in &config.rows {
            titles.push(
                saved
                    .rows
                    .get(&row.source)
                    .and_then(|cached| cached.title.clone()),
            );
            match saved.rows.get(&row.source) {
                Some(cached) => {
                    status.push(Status::Ready);
                    items.push(cached.items.clone());
                    // Trakt rows are fresh at every start; lists keep their cache until due.
                    let age = Duration::from_secs(age_now.saturating_sub(cached.updated_at));
                    next_due.push(if is_trakt(&row.source) {
                        now
                    } else {
                        now + config.interval(&row.source).saturating_sub(age)
                    });
                }
                None => {
                    status.push(Status::Loading);
                    items.push(vec![]);
                    next_due.push(now);
                }
            }
        }
        Self {
            config,
            saved,
            path,
            http: Http::new(),
            responses,
            status,
            items,
            next_due,
            device: None,
            auth_error: None,
            auth_retry: None,
            images: HashSet::new(),
            image_retry: HashMap::new(),
            // The first pass loads every row already; Home's first Refresh adds nothing.
            last_refocus: Some(now),
            commands,
            refocus: false,
            // Until Home says which rows it shows: the top few.
            near: (0..4).collect(),
            near_changed: false,
            keep: false,
            titles,
            actions: Vec::new(),
            watchlist: HashMap::new(),
            offset,
        }
    }

    fn run(mut self) {
        loop {
            if !self.step() {
                break;
            }
            // Something arrived during the pass: act on it now, not at the next wake.
            if self.refocus || self.near_changed {
                continue;
            }
            match self.commands.recv_timeout(self.next_wake()) {
                Ok(command) => self.handle(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !self.handle_commands() {
                break;
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Refresh => {
                if self
                    .last_refocus
                    .is_none_or(|last| last.elapsed() >= TRAKT_REFOCUS)
                {
                    self.last_refocus = Some(Instant::now());
                    self.refocus = true;
                }
            }
            Command::Near { rows, keep } => {
                let near: HashSet<usize> = rows.into_iter().collect();
                if near != self.near || keep != self.keep {
                    self.near = near;
                    self.keep = keep;
                    self.near_changed = true;
                    // Home dropped the artwork of rows outside every set it sends; forget it
                    // here too, even when a later set brings those rows back, so it is resent.
                    let held: HashSet<String> = self.row_images(true).into_iter().collect();
                    self.images.retain(|url| held.contains(url));
                }
            }
            command @ (Command::TitleState(_) | Command::Action(..)) => {
                // A burst of lookups for the same title needs one answer.
                if !self
                    .actions
                    .iter()
                    .any(|queued| same_command(queued, &command))
                {
                    self.actions.push(command);
                }
            }
        }
    }

    /// Runs the Trakt lookups and changes waiting. False once Home has gone away.
    fn run_actions(&mut self) -> bool {
        while !self.actions.is_empty() {
            let command = self.actions.remove(0);
            let response = match command {
                Command::TitleState(link) => {
                    let result = self.title_state(&link);
                    Response::TraktState { link, result }
                }
                Command::Action(link, action) => {
                    let result = self.trakt_change(&link, &action);
                    if result.is_ok() {
                        // Rows show the change at once: Continue Watching, Watchlist, Up Next.
                        self.refocus = true;
                        self.last_refocus = Some(Instant::now());
                    }
                    Response::Action { link, result }
                }
                Command::Refresh | Command::Near { .. } => continue,
            };
            if self.responses.send(response).is_err() {
                return false;
            }
        }
        true
    }

    /// Takes every waiting command, and runs Trakt lookups and changes at once, so a long
    /// first load does not hold them up. False once Home has gone away.
    fn handle_commands(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.handle(command),
                Err(mpsc::TryRecvError::Empty) => return self.run_actions(),
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn uses_trakt(&self) -> bool {
        self.config.rows.iter().any(|row| needs_trakt(&row.source))
    }

    fn signed_in(&self) -> bool {
        self.saved.token.is_some()
    }

    /// One pass: sign-in, due rows, then artwork. False once Home has gone away.
    fn step(&mut self) -> bool {
        if !self.run_actions() {
            return false;
        }
        let refocus = std::mem::take(&mut self.refocus);
        if self.uses_trakt() && self.trakt_sign_in() && !self.publish() {
            return false;
        }
        let mut saved = false;
        for index in 0..self.config.rows.len() {
            let source = self.config.rows[index].source.clone();
            if needs_trakt(&source) && !self.signed_in() {
                continue;
            }
            let now = Instant::now();
            if !(is_trakt(&source) && refocus) && now < self.next_due[index] {
                continue;
            }
            match self.fetch(&source) {
                Ok(Fetched { items, title }) => {
                    self.saved.rows.insert(
                        source.clone(),
                        SavedRow {
                            updated_at: now_secs(),
                            items: items.clone(),
                            title: title.clone(),
                        },
                    );
                    self.titles[index] = title;
                    self.items[index] = items;
                    self.status[index] = Status::Ready;
                    self.next_due[index] = Instant::now() + self.config.interval(&source);
                    saved = true;
                }
                Err(error) => {
                    eprintln!("Luma feeds: {source}: {error}");
                    // Keep showing older items rather than replacing them with an error.
                    if self.items[index].is_empty() {
                        self.status[index] = Status::Error(error);
                    }
                    self.next_due[index] = Instant::now() + RETRY;
                }
            }
            // Artwork for rows on screen comes before the rest of a long first load.
            if !self.publish() || !self.handle_commands() || !self.load_images() {
                return false;
            }
        }
        if saved {
            if self.saved.artwork.len() > 3000 {
                self.saved.artwork.clear();
            }
            let sources: HashSet<&str> = self
                .config
                .rows
                .iter()
                .map(|row| row.source.as_str())
                .collect();
            self.saved
                .rows
                .retain(|source, _| sources.contains(source.as_str()));
            self.save();
        }
        self.load_images()
    }

    fn next_wake(&self) -> Duration {
        let now = Instant::now();
        let mut wake = now + Duration::from_secs(300);
        if let Some(device) = &self.device {
            wake = wake.min(device.next_poll);
        }
        if let Some(retry) = self.auth_retry {
            wake = wake.min(retry);
        }
        for (index, row) in self.config.rows.iter().enumerate() {
            if !needs_trakt(&row.source) || self.signed_in() {
                wake = wake.min(self.next_due[index]);
            }
        }
        wake.saturating_duration_since(now)
            .max(Duration::from_secs(1))
    }

    fn save(&self) {
        if let Err(error) = save_state(&self.path, &self.saved) {
            eprintln!("Luma feeds: saving {}: {error}", self.path.display());
        }
    }

    fn rows(&self) -> Vec<Row> {
        self.config
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let status = if needs_trakt(&row.source) && !self.signed_in() {
                    if let Some(device) = &self.device {
                        Status::SignIn {
                            url: device.url.clone(),
                            code: device.user_code.clone(),
                        }
                    } else if let Some(error) = &self.auth_error {
                        Status::Error(error.clone())
                    } else {
                        Status::Loading
                    }
                } else {
                    self.status[index].clone()
                };
                let items = if needs_trakt(&row.source) && !self.signed_in() {
                    vec![]
                } else {
                    self.items[index].clone()
                };
                Row {
                    title: self.titles[index]
                        .clone()
                        .unwrap_or_else(|| static_title(&row.title)),
                    pinned: self.config.always_loaded(&row.source),
                    hero: self.config.hero_source(&row.source),
                    extra: self.config.shelf_source(&row.source),
                    status,
                    items,
                }
            })
            .collect()
    }

    fn publish(&self) -> bool {
        self.responses.send(Response::Rows(self.rows())).is_ok()
    }

    /// Downloads artwork no earlier pass has fetched, newest rows first.
    /// Artwork URLs of the rows near the screen and of Trakt rows, which are downloaded; with
    /// `held`, also every row's when Home keeps loaded posters, which are not forgotten.
    /// Signed-out Trakt rows are published without items and Home drops their artwork, so it
    /// is never counted and is fetched again after sign-in.
    fn row_images(&self, held: bool) -> Vec<String> {
        let signed_in = self.signed_in();
        self.config
            .rows
            .iter()
            .zip(&self.items)
            .enumerate()
            .filter(|(index, (row, _))| {
                // Trakt rows (Continue Watching) keep their posters wherever Home is scrolled.
                (self.near.contains(index)
                    || self.config.always_loaded(&row.source)
                    || (held && self.keep))
                    && (signed_in || !needs_trakt(&row.source))
            })
            .flat_map(|(_, (_, items))| items)
            .filter_map(|item| item.image.clone())
            .collect()
    }

    fn load_images(&mut self) -> bool {
        self.near_changed = false;
        let wanted = self.row_images(false);
        let held: HashSet<String> = self.row_images(true).into_iter().collect();
        self.images.retain(|url| held.contains(url));
        self.image_retry.retain(|url, _| held.contains(url));
        let now = Instant::now();
        for url in &wanted {
            if self.image_retry.get(url).is_some_and(|retry| now < *retry) {
                continue;
            }
            if !self.images.insert(url.clone()) {
                continue;
            }
            let image = match self.http.get(url, &[], &[], IMAGE_LIMIT) {
                Ok(reply) if reply.ok() => decode(url, &reply.body),
                Ok(reply) => {
                    eprintln!("Luma feeds: artwork HTTP {}", reply.status);
                    None
                }
                Err(_) => None,
            };
            if let Some(image) = image {
                let response = Response::Image {
                    url: url.clone(),
                    image: Arc::new(image),
                };
                if self.responses.send(response).is_err() {
                    return false;
                }
                self.image_retry.remove(url);
            } else {
                // Tried again on a later pass, not every second.
                self.images.remove(url);
                self.image_retry
                    .insert(url.clone(), Instant::now() + Duration::from_secs(10 * 60));
            }
            // Scrolled elsewhere meanwhile: start over with the rows now on screen.
            if !self.handle_commands() {
                return false;
            }
            if self.near_changed {
                return true;
            }
        }
        true
    }

    fn fetch(&mut self, source: &str) -> Result<Fetched, String> {
        let mut title = None;
        let mut entries = match source {
            "trakt:continue" => {
                let mut entries = self.trakt_playback()?;
                entries.extend(self.trakt_up_next()?);
                newest_first(&mut entries);
                deduplicate(entries)
            }
            "trakt:playback" => {
                let mut entries = self.trakt_playback()?;
                newest_first(&mut entries);
                deduplicate(entries)
            }
            "trakt:up_next" | "trakt:upnext" => {
                let mut entries = self.trakt_up_next()?;
                newest_first(&mut entries);
                entries
            }
            "trakt:watchlist" => self.trakt_watchlist()?,
            "trakt:calendar" => self.trakt_calendar()?,
            "smart:new_season" | "smart:new_seasons" => self.new_seasons()?,
            _ if source == "smart:because" || source.starts_with("smart:because:") => {
                // smart:because:2 is built from the second most recent title watched.
                let nth = source
                    .strip_prefix("smart:because:")
                    .and_then(|n| n.trim().parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let (entries, seed) = self.because(nth - 1)?;
                let index = self.config.rows.iter().position(|row| row.source == source);
                let pattern = index
                    .map(|index| self.config.rows[index].title.clone())
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| "Because you watched {title}".into());
                title = seed.map(|seed| {
                    if pattern.contains("{title}") {
                        pattern.replace("{title}", &seed)
                    } else {
                        pattern.clone()
                    }
                });
                entries
            }
            _ => match source.strip_prefix("mdblist:") {
                Some(list) => self.mdblist(list.trim())?,
                None => return Err(format!("Unknown row source {source}")),
            },
        };
        entries.truncate(self.config.limit);
        let mut items = Vec::new();
        for entry in entries {
            let Some(link) = link(entry.kind, entry.imdb.as_deref(), entry.tmdb) else {
                continue;
            };
            let image = self.artwork(entry.kind, entry.tmdb, entry.imdb.as_deref());
            items.push(Item {
                title: entry.title,
                subtitle: entry.subtitle,
                link,
                image,
                progress: entry.progress,
                episode: entry.episode,
                badge: entry.badge,
            });
        }
        Ok(Fetched { items, title })
    }

    // ---- TMDB artwork ----

    fn tmdb(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let key = tmdb_key(&self.config).ok_or("No TMDB key")?;
        tmdb_request(&self.http, &key, path, query)
    }

    fn artwork(&mut self, kind: Kind, tmdb: Option<u64>, imdb: Option<&str>) -> Option<String> {
        self.config.tmdb.as_ref()?;
        let key = match (tmdb, imdb) {
            (Some(id), _) => format!("{}:{id}", kind.tmdb()),
            (None, Some(imdb)) => format!("imdb:{imdb}"),
            (None, None) => return None,
        };
        if let Some(cached) = self.saved.artwork.get(&key) {
            return cached.clone();
        }
        let result = match (tmdb, imdb) {
            (Some(id), _) => self
                .tmdb(&format!("/{}/{id}", kind.tmdb()), &[])
                .map(|value| image_path(&value)),
            (None, Some(imdb)) => self
                .tmdb(
                    &format!("/find/{}", segment(imdb)),
                    &[("external_source", "imdb_id")],
                )
                .map(|value| {
                    let results = match kind {
                        Kind::Movie => ["movie_results", "tv_results"],
                        Kind::Series => ["tv_results", "movie_results"],
                    };
                    results.iter().find_map(|key| {
                        value
                            .get(*key)
                            .and_then(Value::as_array)
                            .and_then(|list| list.first())
                            .and_then(image_path)
                    })
                }),
            (None, None) => return None,
        };
        match result {
            Ok(path) => {
                let url = path.map(|path| format!("{TMDB_IMAGES}{path}"));
                self.saved.artwork.insert(key, url.clone());
                url
            }
            Err(error) => {
                eprintln!("Luma feeds: TMDB {key}: {error}");
                None
            }
        }
    }

    // ---- MDBList ----

    fn mdblist(&self, list: &str) -> Result<Vec<Entry>, String> {
        let key = self
            .config
            .mdblist
            .as_ref()
            .map(|mdblist| mdblist.api_key.trim().to_owned())
            .ok_or("Add an mdblist api_key to feeds.json")?;
        let path = if !list.is_empty() && list.bytes().all(|byte| byte.is_ascii_digit()) {
            format!("/lists/{list}/items")
        } else {
            let (user, name) = list
                .split_once('/')
                .ok_or("MDBList rows need mdblist:USER/LIST or mdblist:LIST_ID")?;
            format!("/lists/{}/{}/items", segment(user), segment(name))
        };
        let limit = self.config.limit.to_string();
        let reply = self.http.get(
            &format!("{MDBLIST}{path}"),
            &[("apikey", key.as_str()), ("limit", limit.as_str())],
            &[("Accept", "application/json")],
            JSON_LIMIT,
        )?;
        if !reply.ok() {
            return Err(format!("MDBList answered HTTP {}", reply.status));
        }
        let value = reply.json()?;
        let mut items: Vec<Value> = match value.as_array() {
            Some(list) => list.clone(),
            None => ["movies", "shows"]
                .iter()
                .flat_map(|key| {
                    value
                        .get(*key)
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect(),
        };
        // Lists split into movies and shows keep their combined rank order.
        items.sort_by_key(|item| item.get("rank").and_then(Value::as_u64).unwrap_or(u64::MAX));
        Ok(items
            .iter()
            .filter_map(|item| {
                let kind = Kind::from_text(&text(item, "mediatype"))
                    .or_else(|| Kind::from_text(&text(item, "type")))
                    .unwrap_or(Kind::Movie);
                let nested = ids(item);
                let imdb = item
                    .get("imdb_id")
                    .and_then(Value::as_str)
                    .filter(|id| id.starts_with("tt"))
                    .map(str::to_owned)
                    .or(nested.imdb);
                let tmdb = nested
                    .tmdb
                    .or_else(|| item.get("tmdb_id").and_then(Value::as_u64))
                    .or_else(|| item.get("tmdbid").and_then(Value::as_u64))
                    // MDBList's list items name the TMDB id plainly "id".
                    .or_else(|| item.get("id").and_then(Value::as_u64));
                if imdb.is_none() && tmdb.is_none() {
                    return None;
                }
                Some(Entry::new(
                    kind,
                    text(item, "title"),
                    Ids {
                        imdb,
                        tmdb,
                        trakt: None,
                    },
                ))
            })
            .collect())
    }

    // ---- Trakt ----

    fn trakt_headers<'a>(client_id: &'a str, bearer: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
        let mut headers = vec![
            ("trakt-api-version", "2"),
            ("trakt-api-key", client_id),
            ("Accept", "application/json"),
        ];
        if let Some(bearer) = bearer {
            headers.push(("Authorization", bearer));
        }
        headers
    }

    fn sign_in_failed(&mut self, error: String, wait: Duration) {
        eprintln!("Luma feeds: Trakt sign-in: {error}");
        self.device = None;
        self.auth_error = Some(error);
        self.auth_retry = Some(Instant::now() + wait);
    }

    /// Advances the device sign-in by at most one request. True when the rows changed.
    fn trakt_sign_in(&mut self) -> bool {
        if self.signed_in() {
            return false;
        }
        let Some(trakt) = self.config.trakt.clone() else {
            let changed = self.auth_error.is_none();
            self.auth_error = Some("Add your Trakt client_id to feeds.json".into());
            return changed;
        };
        let now = Instant::now();
        if self.auth_retry.is_some_and(|retry| now < retry) {
            return false;
        }
        if self
            .device
            .as_ref()
            .is_some_and(|device| now >= device.expires)
        {
            self.device = None;
        }
        let headers = Self::trakt_headers(&trakt.client_id, None);
        if self.device.is_none() {
            let reply = self.http.post(
                &format!("{TRAKT}/oauth/device/code"),
                &headers,
                &json!({"client_id": trakt.client_id}),
                JSON_LIMIT,
            );
            match reply {
                Ok(reply) if reply.ok() => {
                    let value = reply.json().unwrap_or(Value::Null);
                    let field =
                        |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
                    let seconds = |key: &str, default: u64| {
                        value.get(key).and_then(Value::as_u64).unwrap_or(default)
                    };
                    match (field("device_code"), field("user_code")) {
                        (Some(device_code), Some(user_code)) => {
                            let interval = Duration::from_secs(seconds("interval", 5).max(1));
                            self.device = Some(Device {
                                device_code,
                                user_code,
                                url: field("verification_url")
                                    .unwrap_or_else(|| "https://trakt.tv/activate".into()),
                                interval,
                                expires: now + Duration::from_secs(seconds("expires_in", 600)),
                                next_poll: now + interval,
                            });
                            self.auth_error = None;
                            self.auth_retry = None;
                        }
                        _ => self.sign_in_failed(
                            "Trakt returned no sign-in code".into(),
                            Duration::from_secs(60),
                        ),
                    }
                }
                Ok(reply) => self.sign_in_failed(
                    format!(
                        "Trakt refused sign-in (HTTP {}). Check client_id in feeds.json.",
                        reply.status
                    ),
                    Duration::from_secs(5 * 60),
                ),
                Err(error) => self.sign_in_failed(error, Duration::from_secs(60)),
            }
            return true;
        }
        let Some(device) = &self.device else {
            return false;
        };
        if now < device.next_poll {
            return false;
        }
        let device_code = device.device_code.clone();
        let interval = device.interval;
        let reply = self.http.post(
            &format!("{TRAKT}/oauth/device/token"),
            &headers,
            &trakt.with_secret(json!({
                "code": device_code,
                "client_id": trakt.client_id,
            })),
            JSON_LIMIT,
        );
        match reply {
            Ok(reply) if reply.ok() => {
                match reply.json().ok().as_ref().and_then(Token::from_reply) {
                    Some(token) => {
                        self.saved.token = Some(token);
                        self.device = None;
                        self.auth_error = None;
                        self.auth_retry = None;
                        self.save();
                        // Every Trakt row loads right away.
                        let now = Instant::now();
                        for (index, row) in self.config.rows.iter().enumerate() {
                            if needs_trakt(&row.source) {
                                self.next_due[index] = now;
                                self.status[index] = Status::Loading;
                            }
                        }
                    }
                    None => self.sign_in_failed(
                        "Trakt returned no access token".into(),
                        Duration::from_secs(60),
                    ),
                }
                true
            }
            // 400: not entered yet. 429: polling too fast.
            Ok(reply) if reply.status == 400 || reply.status == 429 => {
                let wait = if reply.status == 429 {
                    interval * 2
                } else {
                    interval
                };
                if let Some(device) = &mut self.device {
                    device.next_poll = Instant::now() + wait;
                }
                false
            }
            // 404 invalid, 409 already used, 410 expired: start over with a new code.
            Ok(reply) if matches!(reply.status, 404 | 409 | 410) => {
                self.device = None;
                // Wake right away for the new code instead of the idle 5 minutes.
                self.auth_retry = Some(Instant::now());
                true
            }
            Ok(reply) if reply.status == 418 => {
                self.sign_in_failed(
                    "Trakt sign-in was denied".into(),
                    Duration::from_secs(5 * 60),
                );
                true
            }
            Ok(reply) => {
                self.sign_in_failed(
                    format!(
                        "Trakt sign-in failed (HTTP {}). Check client_id in feeds.json.",
                        reply.status
                    ),
                    Duration::from_secs(5 * 60),
                );
                true
            }
            Err(_) => {
                if let Some(device) = &mut self.device {
                    device.next_poll = Instant::now() + interval * 2;
                }
                false
            }
        }
    }

    fn refresh_token(&mut self) -> Result<(), String> {
        let trakt = self.config.trakt.clone().ok_or("Trakt is not configured")?;
        let refresh = self
            .saved
            .token
            .as_ref()
            .map(|token| token.refresh_token.clone())
            .ok_or("Sign in to Trakt")?;
        let reply = self.http.post(
            &format!("{TRAKT}/oauth/token"),
            &Self::trakt_headers(&trakt.client_id, None),
            &trakt.with_secret(json!({
                "refresh_token": refresh,
                "client_id": trakt.client_id,
                "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
                "grant_type": "refresh_token",
            })),
            JSON_LIMIT,
        )?;
        let token = reply
            .ok()
            .then(|| reply.json().ok())
            .flatten()
            .as_ref()
            .and_then(Token::from_reply);
        match token {
            Some(token) => {
                self.saved.token = Some(token);
                self.save();
                Ok(())
            }
            // Only a definite refusal signs out; anything else is retried later.
            None if matches!(reply.status, 400 | 401 | 403) => {
                self.saved.token = None;
                self.save();
                self.auth_retry = Some(Instant::now());
                Err("Trakt sign-in expired. Sign in again.".into())
            }
            None => Err(format!("Trakt answered HTTP {}", reply.status)),
        }
    }

    fn trakt_get(&mut self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let trakt = self.config.trakt.clone().ok_or("Trakt is not configured")?;
        if self.saved.token.as_ref().is_some_and(Token::expiring) {
            self.refresh_token()?;
        }
        for attempt in 0..2 {
            let token = self
                .saved
                .token
                .as_ref()
                .map(|token| token.access_token.clone())
                .ok_or("Sign in to Trakt")?;
            let bearer = format!("Bearer {token}");
            let reply = self.http.get(
                &format!("{TRAKT}{path}"),
                query,
                &Self::trakt_headers(&trakt.client_id, Some(bearer.as_str())),
                JSON_LIMIT,
            )?;
            if reply.ok() {
                return reply.json();
            }
            if reply.status == 401 && attempt == 0 {
                self.refresh_token()?;
                continue;
            }
            return Err(format!("Trakt answered HTTP {}", reply.status));
        }
        Err("Trakt sign-in failed".into())
    }

    fn trakt_playback(&mut self) -> Result<Vec<Entry>, String> {
        let value = self.trakt_get("/sync/playback", &[("extended", "full")])?;
        let mut entries = Vec::new();
        for item in value.as_array().into_iter().flatten() {
            let progress = item
                .get("progress")
                .and_then(Value::as_f64)
                .unwrap_or(0.)
                .clamp(0., 100.);
            if progress >= 95. {
                continue;
            }
            let at = text(item, "paused_at");
            match item.get("type").and_then(Value::as_str) {
                Some("movie") => {
                    let Some(movie) = item.get("movie") else {
                        continue;
                    };
                    let runtime = movie.get("runtime").and_then(Value::as_u64);
                    entries.push(Entry {
                        subtitle: time_left(runtime, progress).unwrap_or_default(),
                        progress: Some(progress as f32 / 100.),
                        at,
                        ..Entry::new(Kind::Movie, text(movie, "title"), ids(movie))
                    });
                }
                Some("episode") => {
                    let (Some(show), Some(episode)) = (item.get("show"), item.get("episode"))
                    else {
                        continue;
                    };
                    let runtime = episode.get("runtime").and_then(Value::as_u64);
                    let subtitle = match time_left(runtime, progress) {
                        Some(left) => format!("{} · {left}", episode_code(episode)),
                        None => episode_code(episode),
                    };
                    entries.push(Entry {
                        subtitle,
                        progress: Some(progress as f32 / 100.),
                        at,
                        episode: episode_numbers(episode),
                        ..Entry::new(Kind::Series, text(show, "title"), ids(show))
                    });
                }
                _ => {}
            }
        }
        Ok(entries)
    }

    fn trakt_up_next(&mut self) -> Result<Vec<Entry>, String> {
        // Shows hidden from progress on Trakt stay out of Up Next. Best effort.
        let hidden: HashSet<u64> = self
            .trakt_get(
                "/users/hidden/progress_watched",
                &[("type", "show"), ("limit", "250")],
            )
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|item| ids(item.get("show")?).trakt)
            .collect();
        let watched = self.trakt_get("/sync/watched/shows", &[("extended", "noseasons")])?;
        let mut shows: Vec<(String, Value)> = watched
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| Some((text(item, "last_watched_at"), item.get("show")?.clone())))
            .filter(|(_, show)| ids(show).trakt.is_some_and(|id| !hidden.contains(&id)))
            .collect();
        shows.sort_by(|a, b| b.0.cmp(&a.0));
        shows.truncate(UP_NEXT_SHOWS);
        let mut entries = Vec::new();
        for (at, show) in shows {
            let ids = ids(&show);
            let Some(trakt) = ids.trakt else {
                continue;
            };
            let progress = match self.trakt_get(
                &format!("/shows/{trakt}/progress/watched"),
                &[
                    ("hidden", "false"),
                    ("specials", "false"),
                    ("count_specials", "false"),
                ],
            ) {
                Ok(progress) => progress,
                Err(error) => {
                    eprintln!("Luma feeds: Trakt show {trakt}: {error}");
                    continue;
                }
            };
            let Some(next) = progress.get("next_episode").filter(|next| next.is_object()) else {
                continue;
            };
            entries.push(Entry {
                subtitle: format!("Next: {}", episode_code(next)),
                at,
                episode: episode_numbers(next),
                ..Entry::new(
                    Kind::Series,
                    text(&show, "title"),
                    Ids {
                        trakt: Some(trakt),
                        ..ids
                    },
                )
            });
        }
        Ok(entries)
    }

    fn trakt_watchlist(&mut self) -> Result<Vec<Entry>, String> {
        let value = self.trakt_get("/sync/watchlist", &[])?;
        let mut entries = Vec::new();
        for item in value.as_array().into_iter().flatten() {
            let (kind, media) = match item.get("type").and_then(Value::as_str) {
                Some("movie") => (Kind::Movie, item.get("movie")),
                Some("show") => (Kind::Series, item.get("show")),
                _ => continue,
            };
            let Some(media) = media else {
                continue;
            };
            entries.push(Entry {
                at: text(item, "listed_at"),
                ..Entry::new(kind, text(media, "title"), ids(media))
            });
        }
        Ok(entries)
    }
    // ---- Smart rows ----

    /// Shows hidden from progress on Trakt. Best effort.
    fn hidden_shows(&mut self) -> HashSet<u64> {
        self.trakt_get(
            "/users/hidden/progress_watched",
            &[("type", "show"), ("limit", "250")],
        )
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|item| ids(item.get("show")?).trakt)
        .collect()
    }

    /// Episodes of your shows airing in the next week, soonest first, with a day badge.
    fn trakt_calendar(&mut self) -> Result<Vec<Entry>, String> {
        use time::format_description::well_known::Rfc3339;
        let now = time::OffsetDateTime::now_utc().to_offset(self.offset);
        let today = now.date();
        let start = format!(
            "{:04}-{:02}-{:02}",
            today.year(),
            u8::from(today.month()),
            today.day()
        );
        let value = self.trakt_get(&format!("/calendars/my/shows/{start}/7"), &[])?;
        let mut entries = Vec::new();
        for item in value.as_array().into_iter().flatten() {
            let (Some(show), Some(episode)) = (item.get("show"), item.get("episode")) else {
                continue;
            };
            let first_aired = text(item, "first_aired");
            let badge = time::OffsetDateTime::parse(&first_aired, &Rfc3339)
                .ok()
                .map(|aired| day_label(aired.to_offset(self.offset), now));
            entries.push(Entry {
                subtitle: episode_code(episode),
                at: first_aired,
                episode: episode_numbers(episode),
                badge,
                ..Entry::new(Kind::Series, text(show, "title"), ids(show))
            });
        }
        entries.sort_by(|a, b| a.at.cmp(&b.at));
        Ok(entries)
    }

    /// TMDB's recommendations for the `nth` most recent title watched on Trakt, leaving out
    /// what has been watched. Also returns that title's name for the row's heading.
    fn because(&mut self, nth: usize) -> Result<(Vec<Entry>, Option<String>), String> {
        let history = self.trakt_get("/sync/history", &[("limit", "60")])?;
        let mut seeds: Vec<(Kind, u64, String)> = Vec::new();
        let mut watched: HashSet<(Kind, u64)> = HashSet::new();
        for item in history.as_array().into_iter().flatten() {
            let (kind, media) = match item.get("type").and_then(Value::as_str) {
                Some("movie") => (Kind::Movie, item.get("movie")),
                Some("episode") => (Kind::Series, item.get("show")),
                _ => continue,
            };
            let Some(media) = media else {
                continue;
            };
            let Some(tmdb) = ids(media).tmdb else {
                continue;
            };
            if watched.insert((kind, tmdb)) {
                seeds.push((kind, tmdb, text(media, "title")));
            }
        }
        let Some((kind, id, title)) = seeds.get(nth).cloned() else {
            return Ok((vec![], None));
        };
        let value = self.tmdb(&format!("/{}/{id}/recommendations", kind.tmdb()), &[])?;
        let mut entries = Vec::new();
        for result in value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let kind = result
                .get("media_type")
                .and_then(Value::as_str)
                .and_then(Kind::from_text)
                .unwrap_or(kind);
            let Some(tmdb) = result.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if watched.contains(&(kind, tmdb)) {
                continue;
            }
            // The poster is already here: no artwork lookup per card.
            if let Some(path) = image_path(result) {
                self.saved.artwork.insert(
                    format!("{}:{tmdb}", kind.tmdb()),
                    Some(format!("{TMDB_IMAGES}{path}")),
                );
            }
            entries.push(Entry::new(
                kind,
                first_text(result, &["title", "name"]),
                Ids {
                    imdb: None,
                    tmdb: Some(tmdb),
                    trakt: None,
                },
            ));
        }
        Ok((entries, Some(title)))
    }

    /// Shows you finished whose next season is out: the next episode to watch opens a season
    /// after every earlier one is complete.
    fn new_seasons(&mut self) -> Result<Vec<Entry>, String> {
        let hidden = self.hidden_shows();
        let watched = self.trakt_get("/sync/watched/shows", &[("extended", "full")])?;
        let mut candidates: Vec<(String, Value)> = watched
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                let show = item.get("show")?;
                let aired = show.get("aired_episodes").and_then(Value::as_u64)?;
                let seen: usize = item
                    .get("seasons")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|season| season.get("number").and_then(Value::as_u64) != Some(0))
                    .filter_map(|season| season.get("episodes").and_then(Value::as_array))
                    .map(Vec::len)
                    .sum();
                let trakt = ids(show).trakt?;
                (aired > seen as u64 && !hidden.contains(&trakt))
                    .then(|| (text(item, "last_watched_at"), show.clone()))
            })
            .collect();
        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        candidates.truncate(15);
        let mut entries = Vec::new();
        for (at, show) in candidates {
            let Some(trakt) = ids(&show).trakt else {
                continue;
            };
            let Ok(progress) = self.trakt_get(
                &format!("/shows/{trakt}/progress/watched"),
                &[
                    ("hidden", "false"),
                    ("specials", "false"),
                    ("count_specials", "false"),
                ],
            ) else {
                continue;
            };
            let Some([season, number]) = progress
                .get("next_episode")
                .filter(|next| next.is_object())
                .and_then(episode_numbers)
            else {
                continue;
            };
            let count = |season: &Value, key: &str| season.get(key).and_then(Value::as_u64);
            let earlier_done = progress
                .get("seasons")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|candidate| {
                    count(candidate, "number").is_some_and(|n| n > 0 && n < u64::from(season))
                })
                .all(|candidate| count(candidate, "completed") >= count(candidate, "aired"));
            if number != 1 || season < 2 || !earlier_done {
                continue;
            }
            entries.push(Entry {
                subtitle: format!("Season {season} is here"),
                at,
                episode: Some([season, 1]),
                badge: Some("New season".into()),
                ..Entry::new(Kind::Series, text(&show, "title"), ids(&show))
            });
        }
        Ok(entries)
    }

    // ---- Title pages and quick actions ----

    fn trakt_post(&mut self, path: &str, body: &Value) -> Result<Value, String> {
        let trakt = self.config.trakt.clone().ok_or("Trakt is not configured")?;
        if self.saved.token.as_ref().is_some_and(Token::expiring) {
            self.refresh_token()?;
        }
        for attempt in 0..2 {
            let token = self
                .saved
                .token
                .as_ref()
                .map(|token| token.access_token.clone())
                .ok_or("Sign in to Trakt first")?;
            let bearer = format!("Bearer {token}");
            let reply = self.http.post(
                &format!("{TRAKT}{path}"),
                &Self::trakt_headers(&trakt.client_id, Some(bearer.as_str())),
                body,
                JSON_LIMIT,
            )?;
            if reply.ok() {
                return Ok(reply.json().unwrap_or(Value::Null));
            }
            if reply.status == 401 && attempt == 0 {
                self.refresh_token()?;
                continue;
            }
            return Err(format!("Trakt answered HTTP {}", reply.status));
        }
        Err("Trakt sign-in failed".into())
    }

    /// Trakt's own ID for a title, found by its IMDb or TMDB ID.
    fn trakt_id(&mut self, kind: Kind, title_ids: &Value) -> Result<u64, String> {
        let (source, id) = match title_ids.get("imdb").and_then(Value::as_str) {
            Some(imdb) => ("imdb", imdb.to_owned()),
            None => (
                "tmdb",
                title_ids
                    .get("tmdb")
                    .and_then(Value::as_u64)
                    .ok_or("No title id")?
                    .to_string(),
            ),
        };
        let value = self.trakt_get(
            &format!("/search/{source}/{}", segment(&id)),
            &[("type", kind.trakt())],
        )?;
        value
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|result| ids(result.get(kind.trakt())?).trakt)
            .ok_or_else(|| "Trakt does not know this title".into())
    }

    /// Trakt IDs on the Watchlist, read again at most every two minutes.
    fn watchlist_ids(&mut self, kind: Kind) -> Result<HashSet<u64>, String> {
        if let Some((at, known)) = self.watchlist.get(&kind)
            && at.elapsed() < Duration::from_secs(120)
        {
            return Ok(known.clone());
        }
        let value = self.trakt_get(&format!("/sync/watchlist/{}", kind.trakt_group()), &[])?;
        let listed: HashSet<u64> = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| ids(item.get(kind.trakt())?).trakt)
            .collect();
        self.watchlist
            .insert(kind, (Instant::now(), listed.clone()));
        Ok(listed)
    }

    /// Whether a title is on the Watchlist and watched, and a show's watched episodes.
    fn title_state(&mut self, link: &str) -> Result<TraktTitle, String> {
        if !self.signed_in() {
            return Err("Sign in to Trakt first".into());
        }
        let (kind, title_ids) = link_ids(link).ok_or("Not a title link")?;
        let trakt = self.trakt_id(kind, &title_ids)?;
        let mut state = TraktTitle {
            in_watchlist: self.watchlist_ids(kind)?.contains(&trakt),
            ..TraktTitle::default()
        };
        match kind {
            Kind::Movie => {
                let history =
                    self.trakt_get(&format!("/sync/history/movies/{trakt}"), &[("limit", "1")])?;
                state.watched = history.as_array().is_some_and(|list| !list.is_empty());
            }
            Kind::Series => {
                let progress = self.trakt_get(
                    &format!("/shows/{trakt}/progress/watched"),
                    &[
                        ("hidden", "false"),
                        ("specials", "false"),
                        ("count_specials", "false"),
                    ],
                )?;
                for season in progress
                    .get("seasons")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(number) = season.get("number").and_then(Value::as_u64) else {
                        continue;
                    };
                    for episode in season
                        .get("episodes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if episode.get("completed").and_then(Value::as_bool) == Some(true)
                            && let Some(episode) = episode.get("number").and_then(Value::as_u64)
                        {
                            state.episodes.insert([number as u32, episode as u32]);
                        }
                    }
                }
                state.next = progress
                    .get("next_episode")
                    .filter(|next| next.is_object())
                    .and_then(episode_numbers);
                let count = |key: &str| progress.get(key).and_then(Value::as_u64).unwrap_or(0);
                state.watched = count("aired") > 0 && count("completed") >= count("aired");
            }
        }
        Ok(state)
    }

    /// Makes a change on Trakt. Returns what to tell the viewer.
    fn trakt_change(&mut self, link: &str, action: &TraktAction) -> Result<String, String> {
        if !self.signed_in() {
            return Err("Sign in to Trakt first".into());
        }
        let (kind, title_ids) = link_ids(link).ok_or("Not a title link")?;
        let group = |entry: Value, group: &str| {
            let mut body = serde_json::Map::new();
            body.insert(group.to_owned(), Value::Array(vec![entry]));
            Value::Object(body)
        };
        let media = json!({ "ids": title_ids.clone() });
        let (path, body, message) = match action {
            TraktAction::WatchlistAdd => (
                "/sync/watchlist",
                group(media, kind.trakt_group()),
                "Added to your Trakt Watchlist".to_owned(),
            ),
            TraktAction::WatchlistRemove => (
                "/sync/watchlist/remove",
                group(media, kind.trakt_group()),
                "Removed from your Watchlist".to_owned(),
            ),
            TraktAction::Watched => (
                "/sync/history",
                group(media, kind.trakt_group()),
                "Marked as watched".to_owned(),
            ),
            TraktAction::Episode(season, number) => (
                "/sync/history",
                group(
                    json!({
                        "ids": title_ids,
                        "seasons": [{ "number": season, "episodes": [{ "number": number }] }],
                    }),
                    "shows",
                ),
                format!(
                    "Marked {} watched",
                    crate::feeds::episode_label([*season, *number])
                ),
            ),
            TraktAction::Rate(rating) => (
                "/sync/ratings",
                group(
                    json!({ "ids": title_ids, "rating": rating }),
                    kind.trakt_group(),
                ),
                format!("Rated {rating}/10 on Trakt"),
            ),
            TraktAction::HideUpNext => (
                "/users/hidden/progress_watched",
                group(media, "shows"),
                "Hidden from Up Next".to_owned(),
            ),
        };
        let reply = self.trakt_post(path, &body)?;
        // Trakt answers 201 even for an ID it could not match: it lists those under not_found.
        let missing = reply
            .get("not_found")
            .and_then(Value::as_object)
            .is_some_and(|groups| {
                groups
                    .values()
                    .any(|list| list.as_array().is_some_and(|list| !list.is_empty()))
            });
        if missing {
            return Err("Trakt could not match this title".into());
        }
        if matches!(
            action,
            TraktAction::WatchlistAdd | TraktAction::WatchlistRemove
        ) {
            self.watchlist.remove(&kind);
        }
        Ok(message)
    }
}

/// Whether two queued Trakt requests are the same, so only one is sent.
fn same_command(a: &Command, b: &Command) -> bool {
    match (a, b) {
        (Command::TitleState(a), Command::TitleState(b)) => a == b,
        (Command::Action(a, x), Command::Action(b, y)) => a == b && x == y,
        _ => false,
    }
}

/// A card link's kind and IDs for Trakt: `{"imdb": "tt..."}` or `{"tmdb": 603}`.
fn link_ids(link: &str) -> Option<(Kind, Value)> {
    let mut parts = link.strip_prefix("nuvio://")?.split('/');
    let (source, kind, id) = (parts.next()?, parts.next()?, parts.next()?);
    let kind = Kind::from_text(kind)?;
    let title_ids = match source {
        "detail" if id.starts_with("tt") => json!({ "imdb": id }),
        "tmdb" => json!({ "tmdb": id.strip_prefix("tmdb:").unwrap_or(id).parse::<u64>().ok()? }),
        _ => return None,
    };
    Some((kind, title_ids))
}

/// The calendar's badge: "Tonight", "Tomorrow", a weekday, or "Out now".
fn day_label(aired: time::OffsetDateTime, now: time::OffsetDateTime) -> String {
    let days = (aired.date() - now.date()).whole_days();
    match days {
        ..=-1 => "Out now".into(),
        0 if aired.hour() >= 17 => "Tonight".into(),
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ => match aired.weekday() {
            time::Weekday::Monday => "Mon",
            time::Weekday::Tuesday => "Tue",
            time::Weekday::Wednesday => "Wed",
            time::Weekday::Thursday => "Thu",
            time::Weekday::Friday => "Fri",
            time::Weekday::Saturday => "Sat",
            time::Weekday::Sunday => "Sun",
        }
        .into(),
    }
}

fn newest_first(entries: &mut [Entry]) {
    entries.sort_by(|a, b| b.at.cmp(&a.at));
}

/// One card per title: a show paused mid-episode and also in Up Next shows once, as paused.
fn deduplicate(entries: Vec<Entry>) -> Vec<Entry> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter(|entry| {
            let key = match (entry.trakt, &entry.imdb, entry.tmdb) {
                (Some(id), _, _) => format!("trakt:{id}"),
                (None, Some(imdb), _) => format!("imdb:{imdb}"),
                (None, None, Some(id)) => format!("tmdb:{id}"),
                (None, None, None) => format!("title:{}", entry.title),
            };
            seen.insert((entry.kind, key))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_prefer_imdb_and_fall_back_to_tmdb() {
        assert_eq!(
            link(Kind::Series, Some("tt0944947"), Some(1399)).as_deref(),
            Some("nuvio://detail/series/tt0944947")
        );
        assert_eq!(
            link(Kind::Movie, None, Some(603)).as_deref(),
            Some("nuvio://tmdb/movie/603")
        );
        assert_eq!(link(Kind::Movie, None, None), None);
    }

    #[test]
    fn paths_are_percent_encoded() {
        assert_eq!(
            segment("garycrawfordgc/latest-tv"),
            "garycrawfordgc%2Flatest-tv"
        );
        assert_eq!(segment("a b"), "a%20b");
    }

    #[test]
    fn config_defaults_apply() {
        let config: Config =
            serde_json::from_str(r#"{"rows":[{"title":"Popular","source":"mdblist:a/b"}]}"#)
                .unwrap();
        let config = config.normalized();
        assert_eq!(config.app_id, DEFAULT_APP);
        assert_eq!(config.limit, 15);
        assert_eq!(config.rows.len(), 1);
    }

    #[test]
    fn duplicates_keep_the_first_entry() {
        let entry = |subtitle: &str| Entry {
            subtitle: subtitle.into(),
            ..Entry::new(
                Kind::Series,
                "Show".into(),
                Ids {
                    imdb: None,
                    tmdb: None,
                    trakt: Some(1),
                },
            )
        };
        let kept = deduplicate(vec![entry("paused"), entry("next")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].subtitle, "paused");
    }
}
