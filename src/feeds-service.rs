//! The feed worker: Trakt (device sign-in, continue watching, watchlist), MDBList lists and
//! TMDB artwork. One thread does every request, decode and cache write; the SDL thread only
//! receives finished rows and decoded images.
//!
//! Configuration is `feeds.json` next to the preferences file. Without it nothing starts.
use crate::app_artwork::Artwork;
use crate::feeds::{Item, Row, Status};
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
    /// `mdblist:USER/LIST` or `mdblist:LIST_ID`.
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

fn is_trakt(source: &str) -> bool {
    source.starts_with("trakt:")
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
                title: row.title.clone(),
                pinned: is_trakt(&row.source),
                status: Status::Ready,
                items: saved.items.clone(),
            },
            None => Row {
                title: row.title.clone(),
                pinned: is_trakt(&row.source),
                status: Status::Loading,
                items: vec![],
            },
        })
        .collect()
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
    Near(Vec<usize>),
}

pub enum Response {
    Rows(Vec<Row>),
    Image { url: String, image: Arc<Artwork> },
}

pub struct Service {
    sender: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Response>,
}

impl Service {
    /// Starts the worker when there is a configuration with rows.
    pub fn new(config: Option<Config>, saved: Saved, path: PathBuf) -> Self {
        let (sender, commands) = mpsc::channel();
        let (responses, receiver) = mpsc::channel();
        if let Some(config) = config.filter(|config| !config.rows.is_empty()) {
            thread::spawn(move || Worker::new(config, saved, path, responses, commands).run());
        }
        Self { sender, receiver }
    }

    pub fn near(&self, rows: Vec<usize>) {
        let _ = self.sender.send(Command::Near(rows));
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
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    // The card's own size: 216 x 324 for a 2:3 poster.
    let image = if image.width() > 216 || image.height() > 324 {
        image.thumbnail(216, 324)
    } else {
        image
    };
    Some(Artwork {
        key: format!("feed:{url}"),
        pixels: image.into_rgba8(),
        color: [20, 20, 20],
    })
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
}

impl Worker {
    fn new(
        config: Config,
        saved: Saved,
        path: PathBuf,
        responses: mpsc::Sender<Response>,
        commands: mpsc::Receiver<Command>,
    ) -> Self {
        let now = Instant::now();
        let age_now = now_secs();
        let mut status = Vec::new();
        let mut items = Vec::new();
        let mut next_due = Vec::new();
        for row in &config.rows {
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
            Command::Near(rows) => {
                let near: HashSet<usize> = rows.into_iter().collect();
                if near != self.near {
                    self.near = near;
                    self.near_changed = true;
                    // Home dropped the artwork of rows outside every set it sends; forget it
                    // here too, even when a later set brings those rows back, so it is resent.
                    let keep: HashSet<String> = self
                        .config
                        .rows
                        .iter()
                        .zip(&self.items)
                        .enumerate()
                        .filter(|(index, (row, _))| {
                            self.near.contains(index) || is_trakt(&row.source)
                        })
                        .flat_map(|(_, (_, items))| items)
                        .filter_map(|item| item.image.clone())
                        .collect();
                    self.images.retain(|url| keep.contains(url));
                }
            }
        }
    }

    /// Takes every waiting command. False once Home has gone away.
    fn handle_commands(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.handle(command),
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn uses_trakt(&self) -> bool {
        self.config.rows.iter().any(|row| is_trakt(&row.source))
    }

    fn signed_in(&self) -> bool {
        self.saved.token.is_some()
    }

    /// One pass: sign-in, due rows, then artwork. False once Home has gone away.
    fn step(&mut self) -> bool {
        let refocus = std::mem::take(&mut self.refocus);
        if self.uses_trakt() && self.trakt_sign_in() && !self.publish() {
            return false;
        }
        let mut saved = false;
        for index in 0..self.config.rows.len() {
            let source = self.config.rows[index].source.clone();
            let trakt = is_trakt(&source);
            if trakt && !self.signed_in() {
                continue;
            }
            let now = Instant::now();
            if !(trakt && refocus) && now < self.next_due[index] {
                continue;
            }
            match self.fetch(&source) {
                Ok(items) => {
                    self.saved.rows.insert(
                        source.clone(),
                        SavedRow {
                            updated_at: now_secs(),
                            items: items.clone(),
                        },
                    );
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
            if !is_trakt(&row.source) || self.signed_in() {
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
                let status = if is_trakt(&row.source) && !self.signed_in() {
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
                let items = if is_trakt(&row.source) && !self.signed_in() {
                    vec![]
                } else {
                    self.items[index].clone()
                };
                Row {
                    title: row.title.clone(),
                    pinned: is_trakt(&row.source),
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
    fn load_images(&mut self) -> bool {
        // Only what Home is shown: signed-out Trakt rows are published without items, and Home
        // drops their artwork, so it is fetched again after sign-in.
        self.near_changed = false;
        let signed_in = self.signed_in();
        let wanted: Vec<String> = self
            .config
            .rows
            .iter()
            .zip(&self.items)
            .enumerate()
            .filter(|(index, (row, _))| {
                // Trakt rows (Continue Watching) keep their posters wherever Home is scrolled.
                (self.near.contains(index) || is_trakt(&row.source))
                    && (signed_in || !is_trakt(&row.source))
            })
            .flat_map(|(_, (_, items))| items)
            .filter_map(|item| item.image.clone())
            .collect();
        let current: HashSet<&String> = wanted.iter().collect();
        self.images.retain(|url| current.contains(url));
        self.image_retry.retain(|url, _| current.contains(url));
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

    fn fetch(&mut self, source: &str) -> Result<Vec<Item>, String> {
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
            });
        }
        Ok(items)
    }

    // ---- TMDB artwork ----

    fn tmdb(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let key = self
            .config
            .tmdb
            .as_ref()
            .map(|tmdb| tmdb.api_key.trim().to_owned())
            .ok_or("No TMDB key")?;
        let url = format!("{TMDB}{path}");
        // A v4 read access token is a JWT; a v3 key is a short hex string.
        let reply = if key.starts_with("eyJ") {
            let bearer = format!("Bearer {key}");
            self.http.get(
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
            query.push(("api_key", key.as_str()));
            self.http
                .get(&url, &query, &[("Accept", "application/json")], JSON_LIMIT)?
        };
        if !reply.ok() {
            return Err(format!("TMDB answered HTTP {}", reply.status));
        }
        reply.json()
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
                Some(Entry {
                    kind,
                    title: text(item, "title"),
                    subtitle: String::new(),
                    imdb,
                    tmdb,
                    trakt: None,
                    progress: None,
                    at: String::new(),
                })
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
                            if is_trakt(&row.source) {
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
        let value = self.trakt_get("/sync/playback", &[])?;
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
                    let ids = ids(movie);
                    entries.push(Entry {
                        kind: Kind::Movie,
                        title: text(movie, "title"),
                        subtitle: String::new(),
                        imdb: ids.imdb,
                        tmdb: ids.tmdb,
                        trakt: ids.trakt,
                        progress: Some(progress as f32 / 100.),
                        at,
                    });
                }
                Some("episode") => {
                    let (Some(show), Some(episode)) = (item.get("show"), item.get("episode"))
                    else {
                        continue;
                    };
                    let ids = ids(show);
                    entries.push(Entry {
                        kind: Kind::Series,
                        title: text(show, "title"),
                        subtitle: episode_code(episode),
                        imdb: ids.imdb,
                        tmdb: ids.tmdb,
                        trakt: ids.trakt,
                        progress: Some(progress as f32 / 100.),
                        at,
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
                kind: Kind::Series,
                title: text(&show, "title"),
                subtitle: format!("Next: {}", episode_code(next)),
                imdb: ids.imdb,
                tmdb: ids.tmdb,
                trakt: Some(trakt),
                progress: None,
                at,
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
            let ids = ids(media);
            entries.push(Entry {
                kind,
                title: text(media, "title"),
                subtitle: String::new(),
                imdb: ids.imdb,
                tmdb: ids.tmdb,
                trakt: ids.trakt,
                progress: None,
                at: text(item, "listed_at"),
            });
        }
        Ok(entries)
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
            kind: Kind::Series,
            title: "Show".into(),
            subtitle: subtitle.into(),
            imdb: None,
            tmdb: None,
            trakt: Some(1),
            progress: None,
            at: String::new(),
        };
        let kept = deduplicate(vec![entry("paused"), entry("next")]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].subtitle, "paused");
    }
}
