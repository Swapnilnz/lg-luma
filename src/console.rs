//! The console tile: wakes the gaming PC, asks the console agent on it to get ready (TV-only
//! display, TV audio, Hydra Big Picture full screen), then switches the TV to the PC's HDMI
//! input. Configured by the optional `console` block in preferences.json; without one,
//! nothing here shows.
//!
//! The PC side is a small HTTP agent (`GET /status`, `POST /console`) on the LAN. Requests
//! go only to the configured IPv4 host, without proxies or redirects, with short timeouts and
//! bounded replies, on a worker thread.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// The synthetic app the tile is drawn and navigated as (first in the favourites).
pub const APP_ID: &str = "luma.console";
/// Without a ready PC by then, the transition offers Switch anyway / Cancel.
pub const READY_TIMEOUT: Duration = Duration::from_secs(25);
/// The card lifting off its row, then growing to fill the screen.
pub const LIFT: Duration = Duration::from_millis(160);
pub const GROW: Duration = Duration::from_millis(700);
/// "Ready" shows at least this long after the tile was picked, so the transition reads.
pub const MIN_SHOW: Duration = Duration::from_millis(1400);
/// "Ready" holds this long (the shockwave and chime) before the fade begins.
pub const READY_HOLD: Duration = Duration::from_millis(700);
/// The fade to black before the input switches.
pub const FADE: Duration = Duration::from_millis(420);
/// The waiting pulse's period (the halo breathes with it).
pub const PULSE: Duration = Duration::from_millis(2400);
/// After the switch, the black screen stays this long (Luma goes to the background).
const LINGER: Duration = Duration::from_secs(4);
/// How often Home asks whether the PC is awake, for the tile's status dot.
pub const PROBE_EVERY: Duration = Duration::from_secs(30);

const WORKER_DEADLINE: Duration = Duration::from_secs(40);
const POLL: Duration = Duration::from_millis(750);
const WAKE_EVERY: Duration = Duration::from_secs(5);
const REPLY_LIMIT: u64 = 1024;
/// The library reply (twelve games and their details) and one image.
const LIBRARY_LIMIT: u64 = 256 * 1024;
const ART_LIMIT: u64 = 8 * 1024 * 1024;
const PHOTO_LIMIT: u64 = 4 * 1024 * 1024;
/// At most this many games are kept from the library.
pub const GAMES: usize = 12;
/// Screenshots per game, for the Play page's moving backdrop.
pub const SHOTS: u8 = 5;

/// One of a game's images on the PC agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtKind {
    /// Portrait box art.
    Cover,
    /// Wide library artwork.
    Hero,
    /// The title logo, transparent.
    Logo,
    Shot(u8),
}

impl ArtKind {
    pub fn name(self) -> String {
        match self {
            Self::Cover => "cover".into(),
            Self::Hero => "hero".into(),
            Self::Logo => "logo".into(),
            Self::Shot(index) => format!("shot{index}"),
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "cover" => Some(Self::Cover),
            "hero" => Some(Self::Hero),
            "logo" => Some(Self::Logo),
            _ => name
                .strip_prefix("shot")
                .and_then(|index| index.parse::<u8>().ok())
                .filter(|index| *index < SHOTS && name.len() == 5)
                .map(Self::Shot),
        }
    }

    /// The largest size it is decoded at: covers for the rail and the details sheet, the
    /// rest for a full-screen backdrop or a logo.
    pub fn bounds(self) -> (u32, u32) {
        match self {
            Self::Cover => (360, 540),
            Self::Hero | Self::Shot(_) => (1280, 720),
            Self::Logo => (640, 240),
        }
    }

    /// The renderer's key prefix: `backdrop:` fills its rect (cover-cropped); logos keep
    /// their shape (contain).
    fn key_prefix(self) -> &'static str {
        match self {
            Self::Logo => "console-logo:",
            _ => "backdrop:console:",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Achievements {
    pub unlocked: u32,
    pub total: u32,
}

/// A game on the PC, as the agent's `/library` lists it (most recently played first).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Game {
    /// `shop:objectId`, what `/play` and `/art` take.
    pub key: String,
    pub title: String,
    pub play_time_ms: f64,
    /// RFC 3339.
    pub last_played: Option<String>,
    pub running: bool,
    pub favorite: bool,
    pub achievements: Achievements,
    pub installed_bytes: Option<f64>,
    pub description: String,
    pub developer: String,
    pub publisher: String,
    pub genres: Vec<String>,
    pub release_date: String,
    /// Steam's controller support: "full", "partial" or empty.
    pub controller: String,
    /// The art kinds the agent has for it.
    pub art: Vec<String>,
}

impl Game {
    pub fn has(&self, kind: ArtKind) -> bool {
        self.art
            .iter()
            .any(|name| ArtKind::parse(name) == Some(kind))
    }

    pub fn shots(&self) -> Vec<ArtKind> {
        (0..SHOTS)
            .map(ArtKind::Shot)
            .filter(|kind| self.has(*kind))
            .collect()
    }

    /// The renderer key for one of its images.
    pub fn art_key(&self, kind: ArtKind) -> String {
        format!("{}{}:{}", kind.key_prefix(), self.key, kind.name())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Library {
    pub games: Vec<Game>,
}

/// What a game key may contain (`steam:1245620`, `custom:…`): it goes into URLs and file names.
pub fn valid_key(key: &str) -> bool {
    (1..=128).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

fn clean(text: &str, limit: usize) -> String {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        text
    } else {
        let cut: String = text.chars().take(limit - 1).collect();
        format!("{}\u{2026}", cut.trim_end())
    }
}

impl Library {
    /// Bounds everything from the PC: valid keys, short strings, finite numbers, known art.
    pub fn sanitize(mut self) -> Self {
        self.games.retain(|game| valid_key(&game.key));
        let mut seen = std::collections::HashSet::new();
        self.games.retain(|game| seen.insert(game.key.clone()));
        self.games.truncate(GAMES);
        for game in &mut self.games {
            game.title = clean(&game.title, 80);
            if game.title.is_empty() {
                game.title = game.key.clone();
            }
            game.description = clean(&game.description, 600);
            game.developer = clean(&game.developer, 60);
            game.publisher = clean(&game.publisher, 60);
            game.release_date = clean(&game.release_date, 30);
            game.controller = clean(&game.controller, 12);
            game.genres = game
                .genres
                .iter()
                .map(|genre| clean(genre, 24))
                .filter(|genre| !genre.is_empty())
                .take(3)
                .collect();
            if !game.play_time_ms.is_finite() || game.play_time_ms < 0. {
                game.play_time_ms = 0.;
            }
            game.installed_bytes = game
                .installed_bytes
                .filter(|bytes| bytes.is_finite() && *bytes > 0.);
            game.last_played = game
                .last_played
                .take()
                .filter(|at| at.len() <= 40 && parse_time(at).is_some());
            game.achievements.unlocked = game.achievements.unlocked.min(game.achievements.total);
            game.art.retain(|name| ArtKind::parse(name).is_some());
        }
        self
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice::<Self>(bytes)
            .ok()
            .map(Self::sanitize)
    }
}

pub fn parse_time(text: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339).ok()
}

/// The desktop preview's library: no PC, no art (the posters are generated).
pub fn demo_library() -> Library {
    let game = |key: &str, title: &str, hours: f64, days: i64, unlocked: u32, total: u32| Game {
        key: key.into(),
        title: title.into(),
        play_time_ms: hours * 3_600_000.,
        last_played: Some(
            (time::OffsetDateTime::now_utc() - time::Duration::days(days))
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
        ),
        achievements: Achievements { unlocked, total },
        installed_bytes: Some(hours.mul_add(1.1e9, 8e9)),
        description: format!(
            "{title} is a demo entry for the desktop preview. On the TV this shows the store \
             description from your Hydra library."
        ),
        developer: "Demo Studio".into(),
        publisher: "Demo Publishing".into(),
        genres: vec!["Action".into(), "RPG".into()],
        release_date: "24 Feb, 2022".into(),
        controller: "full".into(),
        ..Game::default()
    };
    Library {
        games: vec![
            game("demo:1", "Starfall Odyssey", 41.3, 2, 37, 42),
            game("demo:2", "Neon Drift", 12.8, 5, 9, 30),
            game("demo:3", "Ironwood", 88.0, 9, 51, 51),
            game("demo:4", "Hollow Tide", 3.4, 21, 2, 24),
            game("demo:5", "Ember Protocol", 26.1, 40, 18, 40),
            game("demo:6", "Skybound", 0.5, 70, 0, 0),
        ],
    }
}

/// Where the library and its art are kept on the TV, so the Play page shows them while the
/// PC sleeps.
pub fn library_file(cache: &std::path::Path) -> std::path::PathBuf {
    cache.join("library.json")
}

fn art_file(cache: &std::path::Path, key: &str, kind: ArtKind) -> std::path::PathBuf {
    cache.join(format!(
        "{}-{}.img",
        key.replace([':', '.'], "_"),
        kind.name()
    ))
}

fn read_limited(path: &std::path::Path, limit: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

pub fn load_library(cache: &std::path::Path) -> Option<Library> {
    Library::parse(&read_limited(&library_file(cache), LIBRARY_LIMIT)?)
}

fn store(path: &std::path::Path, bytes: &[u8]) {
    let temporary = path.with_extension("tmp");
    let result = std::fs::create_dir_all(path.parent().unwrap_or(path))
        .and_then(|()| std::fs::write(&temporary, bytes))
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = result {
        eprintln!("Luma console: cache {}: {error}", path.display());
    }
}

fn get(agent: &ureq::Agent, config: &Config, path: &str, limit: u64) -> Option<Vec<u8>> {
    let mut response = agent
        .get(format!("{}{path}", config.base()))
        .header("X-Console-Token", &config.token)
        .call()
        .ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .ok()?;
    (body.len() as u64 <= limit).then_some(body)
}

/// One random photo from the PC's photo folder (JPEG, at most 1920x1080). Never wakes the PC.
pub fn photo(config: &Config) -> Option<Vec<u8>> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .build()
        .new_agent();
    get(&agent, config, "/photo", PHOTO_LIMIT)
}

/// The PC's library, saved for next time. None when the PC does not answer.
pub fn library(config: &Config, cache: Option<&std::path::Path>) -> Option<Library> {
    let bytes = get(&agent(), config, "/library", LIBRARY_LIMIT)?;
    let library = Library::parse(&bytes)?;
    if let (Some(cache), Ok(text)) = (cache, serde_json::to_vec(&library)) {
        store(&library_file(cache), &text);
    }
    Some(library)
}

/// One image: from the cache, else (when the PC is awake) from the agent, which may have to
/// download it first, so this waits longer than the other requests.
pub fn art(
    config: &Config,
    cache: Option<&std::path::Path>,
    game: &Game,
    kind: ArtKind,
    online: bool,
) -> Option<crate::app_artwork::Artwork> {
    let file = cache.map(|cache| art_file(cache, &game.key, kind));
    let cached = file
        .as_deref()
        .and_then(|file| read_limited(file, ART_LIMIT));
    let bytes = match cached {
        Some(bytes) => bytes,
        None if online && valid_key(&game.key) => {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(25)))
                .http_status_as_error(false)
                .max_redirects(0)
                .proxy(None)
                .build()
                .new_agent();
            let bytes = get(
                &agent,
                config,
                &format!("/art?key={}&kind={}", game.key, kind.name()),
                ART_LIMIT,
            )?;
            if let Some(file) = &file {
                store(file, &bytes);
            }
            bytes
        }
        None => return None,
    };
    let (width, height) = kind.bounds();
    crate::feeds_service::decode_as(game.art_key(kind), &bytes, width, height)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// The PC's HDMI input, `com.webos.app.hdmiN`.
    pub input: String,
    /// The tile's title; "Hydra" when empty.
    pub title: String,
    /// The PC's IPv4 address and the agent's port.
    pub host: String,
    pub port: u16,
    /// The PC's wired MAC address, for Wake-on-LAN (`aa:bb:cc:dd:ee:ff` or with dashes).
    pub mac: String,
    /// Where the wake packet goes: the LAN's broadcast address.
    pub broadcast: String,
    /// The agent's shared secret (`X-Console-Token`).
    pub token: String,
}

impl Config {
    pub fn valid(&self) -> bool {
        self.input
            .strip_prefix("com.webos.app.hdmi")
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| n > 0 && n < 100 && self.input == format!("com.webos.app.hdmi{n}"))
            && self.title.chars().count() <= 40
            && !self.title.chars().any(char::is_control)
            && self.host.parse::<Ipv4Addr>().is_ok()
            && self.broadcast.parse::<Ipv4Addr>().is_ok()
            && self.port != 0
            && parse_mac(&self.mac).is_some()
            && !self.token.is_empty()
            && self.token.len() <= 128
            && self.token.bytes().all(|b| b.is_ascii_graphic())
    }

    pub fn title(&self) -> &str {
        let title = self.title.trim();
        if title.is_empty() { "Hydra" } else { title }
    }

    fn base(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let parts: Vec<_> = text.trim().split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0; 6];
    for (byte, part) in mac.iter_mut().zip(parts) {
        if part.len() != 2 {
            return None;
        }
        *byte = u8::from_str_radix(part, 16).ok()?;
    }
    Some(mac)
}

/// Wake-on-LAN: six 0xFF bytes, then the MAC sixteen times.
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [0xff; 102];
    for chunk in packet[6..].chunks_exact_mut(6) {
        chunk.copy_from_slice(&mac);
    }
    packet
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Wake packets sent; waiting for the agent to answer.
    Waking,
    /// The agent is switching the display and audio and opening Hydra.
    Starting,
    Ready,
    /// No answer in time: Switch anyway or Cancel.
    Failed,
}

/// One press of the tile, from the growing card to the input switch.
#[derive(Clone, Debug)]
pub struct Session {
    pub generation: u64,
    pub started: Instant,
    pub phase: Phase,
    /// When the fade to black begins (Ready, held until `MIN_SHOW`).
    pub ready_at: Option<Instant>,
    pub launched_at: Option<Instant>,
    /// Focus to return to when the transition is cancelled.
    pub restore: crate::state::Target,
    /// The control the card grows from (a cover, the band, the tile).
    pub source: crate::state::Target,
    /// The game to launch; None opens Hydra.
    pub game: Option<Game>,
    pub cancel: Arc<AtomicBool>,
    /// Sound cues already played: the last waiting pulse, the chime, the whoosh.
    pulsed: Option<Instant>,
    chimed: bool,
    whooshed: bool,
}

impl Session {
    pub fn new(
        generation: u64,
        restore: crate::state::Target,
        source: crate::state::Target,
        game: Option<Game>,
        now: Instant,
    ) -> Self {
        Self {
            generation,
            started: now,
            phase: Phase::Waking,
            ready_at: None,
            launched_at: None,
            restore,
            source,
            game,
            cancel: Arc::new(AtomicBool::new(false)),
            pulsed: None,
            chimed: false,
            whooshed: false,
        }
    }

    /// The title shown: the game's, else the configured one ("Hydra").
    pub fn title<'a>(&'a self, config: Option<&'a Config>) -> &'a str {
        self.game
            .as_ref()
            .map(|game| game.title.as_str())
            .unwrap_or_else(|| config.map_or("Hydra", Config::title))
    }

    /// "Ready" is on screen (the shockwave and chime).
    pub fn ready(&self, now: Instant) -> bool {
        self.ready_at.is_some_and(|at| now >= at)
    }

    /// 0 to 1 over the moment of Ready, for the shockwave.
    pub fn burst(&self, now: Instant) -> Option<f32> {
        let at = self.ready_at.filter(|at| now >= *at)?;
        Some((now.duration_since(at).as_secs_f32() / READY_HOLD.as_secs_f32()).min(1.))
    }

    /// The sound for this frame, once each: a pulse every PULSE while waiting (after the
    /// launch swell), the chime at Ready (with the whoosh into black in it), or the whoosh
    /// alone after Switch anyway.
    pub fn cue(&mut self, now: Instant) -> Option<crate::state::Sound> {
        use crate::state::Sound;
        if self.launched_at.is_some() || self.phase == Phase::Failed {
            return None;
        }
        if let Some(at) = self.ready_at {
            if !self.whooshed && now >= at + READY_HOLD {
                self.whooshed = true;
                self.chimed = true;
                return Some(Sound::Whoosh);
            }
            if !self.chimed && now >= at {
                // The chime carries the whoosh, timed to the fade.
                self.chimed = true;
                self.whooshed = true;
                return Some(Sound::Ready);
            }
            return None;
        }
        let due = self
            .pulsed
            .map_or(self.started + PULSE / 2, |at| at + PULSE);
        if now >= due {
            self.pulsed = Some(now);
            return Some(Sound::Waiting);
        }
        None
    }

    /// A phase from the worker. Ready is held until the transition has had its moment.
    pub fn update(&mut self, phase: Phase, now: Instant) {
        if self.phase == Phase::Failed || self.ready_at.is_some() {
            return;
        }
        self.phase = phase;
        if phase == Phase::Ready {
            self.ready_at = Some(now.max(self.started + MIN_SHOW));
        }
    }

    /// Switch anyway: fade out and switch now, whatever the PC said (no Ready moment).
    pub fn force(&mut self, now: Instant) {
        self.phase = Phase::Ready;
        self.ready_at = Some(now.checked_sub(READY_HOLD).unwrap_or(now));
        self.chimed = true;
    }

    /// Times the wait out. True when the phase changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.ready_at.is_none()
            && self.phase != Phase::Failed
            && now.duration_since(self.started) >= READY_TIMEOUT
        {
            self.phase = Phase::Failed;
            return true;
        }
        false
    }

    pub fn launch_due(&self, now: Instant) -> bool {
        self.launched_at.is_none()
            && self
                .ready_at
                .is_some_and(|at| now >= at + READY_HOLD + FADE)
    }

    /// The black screen has stayed long enough after the switch.
    pub fn finished(&self, now: Instant) -> bool {
        self.launched_at
            .is_some_and(|at| now.duration_since(at) >= LINGER)
    }

    /// 0 to 1: the card lifting off its row (before it grows).
    pub fn lift(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.started).as_secs_f32() / LIFT.as_secs_f32()).min(1.);
        1. - (1. - t).powi(3)
    }

    /// 0 to 1: the card growing from its place to the whole screen, after the lift (eased).
    pub fn grow(&self, now: Instant) -> f32 {
        let elapsed = now.duration_since(self.started).as_secs_f32() - LIFT.as_secs_f32();
        let t = (elapsed / (GROW - LIFT).as_secs_f32()).clamp(0., 1.);
        // Ease-out expo: fast off the row, then a long settle.
        if t >= 1. {
            1.
        } else {
            1. - 2f32.powf(-10. * t)
        }
    }

    /// 0 to 1: the fade to black.
    pub fn fade(&self, now: Instant) -> f32 {
        match self.ready_at.map(|at| at + READY_HOLD) {
            Some(at) if now >= at => {
                (now.duration_since(at).as_secs_f32() / FADE.as_secs_f32()).min(1.)
            }
            _ => 0.,
        }
    }

    /// Something on screen moves: the card, the pulsing ring or the fade.
    pub fn animating(&self) -> bool {
        self.phase != Phase::Failed && self.launched_at.is_none()
    }

    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(1500)))
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .build()
        .new_agent()
}

/// The agent's phase ("idle", "starting", "ready", "desk"), or None when it does not answer.
fn status(agent: &ureq::Agent, config: &Config) -> Option<String> {
    let mut response = agent
        .get(format!("{}/status", config.base()))
        .header("X-Console-Token", &config.token)
        .call()
        .ok()?;
    if response.status().as_u16() != 200 {
        eprintln!("Luma console: status {}", response.status().as_u16());
        return None;
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(REPLY_LIMIT)
        .read_to_end(&mut body)
        .ok()?;
    let reply: serde_json::Value = serde_json::from_slice(&body).ok()?;
    reply
        .get("phase")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// The request that readies the PC: `/play?key=` for one game, else `/console` (Hydra).
pub fn start_path(game: Option<&str>) -> String {
    match game.filter(|key| valid_key(key)) {
        Some(key) => format!("/play?key={key}"),
        None => "/console".into(),
    }
}

fn start(agent: &ureq::Agent, config: &Config, game: Option<&str>) -> bool {
    agent
        .post(format!("{}{}", config.base(), start_path(game)))
        .header("X-Console-Token", &config.token)
        .send_empty()
        .is_ok_and(|response| response.status().is_success())
}

fn wake(config: &Config) {
    let (Some(mac), Ok(broadcast)) = (parse_mac(&config.mac), config.broadcast.parse::<Ipv4Addr>())
    else {
        return;
    };
    let packet = magic_packet(mac);
    let result = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|socket| {
        socket.set_broadcast(true)?;
        for _ in 0..3 {
            socket.send_to(&packet, (broadcast, 9))?;
        }
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("Luma console: wake packet: {error}");
    }
}

/// Is the PC's agent answering? For the tile's status dot.
pub fn probe(config: &Config) -> bool {
    status(&agent(), config).is_some()
}

/// Wakes the PC and readies it, reporting each phase. Stops quietly when cancelled.
pub fn run(
    config: &Config,
    game: Option<&str>,
    webos: bool,
    cancel: &AtomicBool,
    mut report: impl FnMut(Phase),
) {
    let cancelled = || cancel.load(Ordering::Relaxed);
    if !webos {
        // Desktop preview: the phases on timers; nothing is sent.
        for (phase, wait) in [
            (Phase::Waking, 1600),
            (Phase::Starting, 1600),
            (Phase::Ready, 0),
        ] {
            if cancelled() {
                return;
            }
            report(phase);
            std::thread::sleep(Duration::from_millis(wait));
        }
        return;
    }
    let agent = agent();
    let deadline = Instant::now() + WORKER_DEADLINE;
    report(Phase::Waking);
    let mut woken: Option<Instant> = None;
    // Wake until the agent answers.
    loop {
        if cancelled() {
            return;
        }
        if Instant::now() > deadline {
            eprintln!("Luma console: the PC did not wake");
            report(Phase::Failed);
            return;
        }
        if woken.is_none_or(|at| at.elapsed() >= WAKE_EVERY) {
            wake(config);
            woken = Some(Instant::now());
        }
        if status(&agent, config).is_some() {
            break;
        }
        std::thread::sleep(POLL);
    }
    report(Phase::Starting);
    // Always ask: the PC may have been put back at the desk since it last said "ready".
    let mut asked = false;
    loop {
        if cancelled() {
            return;
        }
        if Instant::now() > deadline {
            eprintln!("Luma console: the PC did not get ready");
            report(Phase::Failed);
            return;
        }
        if !asked {
            asked = start(&agent, config, game);
        } else if status(&agent, config).as_deref() == Some("ready") {
            eprintln!("Luma console: ready");
            report(Phase::Ready);
            return;
        }
        std::thread::sleep(POLL);
    }
}
