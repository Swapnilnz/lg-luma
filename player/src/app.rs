//! The player: one `App` behind a thread-local, driven by keys, video events, a 250 ms tick
//! and async tasks. Tasks carry the launch's generation and drop their result when a newer
//! launch (Next Up, a relaunch) has taken over.

use crate::config::Config;
use crate::dom::{self, now_ms};
use crate::log;
use crate::net::{self, Failure};
use crate::store::{self, Settings};
use crate::ui;
use crate::video::Video;
use luma_core::ids::{Kind, media_key, stremio_id};
use luma_core::launch::Launch;
use luma_core::remote::Key;
use luma_core::resume::{self, Offer};
use luma_core::scrobble::{self, Call, Event, Scrobbler};
use luma_core::segments::{Segments, Skip};
use luma_core::selection::{self, Attempts};
use luma_core::stremio::{self, Stream, Subtitle};
use luma_core::subs::{self, Track};
use luma_core::tmdb::{self, Meta};
use luma_core::trakt::{self, Media, Token};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

mod controls;
pub use controls::{OsdButton, OsdFocus, SPEEDS};

const STREAMS_TIMEOUT_MS: i32 = 15_000;
const STREAM_CACHE_MS: f64 = 10. * 60_000.;
const FIRST_FRAME_MS: f64 = 12_000.;
const EARLY_STALL_MS: f64 = 20_000.;
const LATE_STALL_MS: f64 = 15_000.;
const OSD_MS: f64 = 5_000.;
const SAVE_EVERY_MS: f64 = 15_000.;
const SEEK_COMMIT_MS: f64 = 600.;
/// Next Up's streams are fetched from here on.
const PREFETCH_AT: f64 = 0.9;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Stream lists by Stremio ID: (fetched at, streams). Survives relaunches.
    static CACHE: RefCell<HashMap<String, (f64, Vec<Stream>)>> = RefCell::new(HashMap::new());
}

/// Runs `f` on the app. None before start, or if called while the app is already borrowed
/// (an event fired synchronously from inside another handler); that is logged, never a panic.
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => app.as_mut().map(f),
        Err(_) => {
            log!("Luma Player: re-entrant app access skipped");
            None
        }
    })
}

/// Like `with`, but only while launch `epoch` is still the current one.
fn update<R>(epoch: u64, f: impl FnOnce(&mut App) -> R) -> Option<R> {
    with(|app| (app.epoch == epoch).then(|| f(app))).flatten()
}

#[derive(Clone, Debug)]
pub enum Screen {
    Loading(String),
    SignIn {
        code: String,
        url: String,
        qr: String,
    },
    Resume(Offer),
    Playing,
    Message {
        title: String,
        body: String,
        retry: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Picker,
    Tracks,
    Settings,
    Info,
    Speed,
    /// The Episodes shelf along the bottom: season tabs over episode stills.
    Episodes,
}

/// Where a stream starts once its duration is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    Zero,
    At(f64),
    Percent(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubChoice {
    Off,
    Embedded(usize),
    Addon(usize),
}

pub struct NextUp {
    pub at: (u32, u32),
    pub name: Option<String>,
    pub still: Option<String>,
    pub streams: Option<Vec<Stream>>,
    pub shown_at: Option<f64>,
    pub dismissed: bool,
}

pub struct App {
    pub config: Config,
    pub settings: Settings,
    pub video: Video,
    pub token: Option<Token>,
    pub epoch: u64,
    pub launch: Launch,
    pub meta: Option<Meta>,
    pub tmdb_id: Option<u64>,
    pub episode_name: Option<String>,
    pub episode_runtime: Option<f64>,
    pub screen: Screen,
    pub panel: Option<Panel>,
    pub focus: usize,
    pub notice: Option<String>,
    pub osd_until: f64,
    pub toast: Option<(String, f64)>,
    pub streams: Vec<Stream>,
    pub from_cache: bool,
    pub refetched: bool,
    pub current: Option<usize>,
    pub attempts: Attempts,
    pub start: Start,
    pub metadata: bool,
    /// Play was asked for (not just a preload behind the Resume question).
    pub wants_play: bool,
    pub started_ms: f64,
    pub first_frame: bool,
    pub stall_since: Option<f64>,
    pub reloaded: bool,
    pub seeking: bool,
    pub seek_preview: Option<f64>,
    pub seek_commit_ms: f64,
    pub ended: bool,
    pub scrobbler: Scrobbler,
    pub media: Option<Media>,
    pub last_save_ms: f64,
    pub sign_in_skipped: bool,
    pub binge_group: Option<String>,
    pub auto_resume: bool,
    pub next: Option<NextUp>,
    pub next_checked: bool,
    pub addon_subs: Vec<Subtitle>,
    pub subs_for: Option<String>,
    pub sub_choice: SubChoice,
    pub sub_track: Option<Track>,
    /// Where focus sits in the controls: the scrubber, or a button below it.
    pub osd_focus: OsdFocus,
    pub speed: f64,
    /// When playback was paused, for the pause screen 10 s later.
    pub paused_at: Option<f64>,
    pub episode_overview: Option<String>,
    /// Episodes by season from TMDB, for the Episodes shelf.
    pub seasons: HashMap<u32, Vec<tmdb::Episode>>,
    pub seasons_loading: Vec<u32>,
    pub shelf_season: u32,
    /// 0: the season tabs, 1: the episodes.
    pub shelf_row: u8,
    pub shelf_focus: usize,
    /// IntroDB's intro, recap and credits for this episode.
    pub segments: Option<Segments>,
    /// A skip the viewer hid with Back, so it stays hidden for that segment.
    pub skip_hidden: Option<Skip>,
    /// Segments skipped automatically (auto-skip), so seeking back doesn't skip them again.
    pub skipped: Vec<Skip>,
    /// Trakt's watched episodes and this show's paused ones (percent), for the shelf.
    pub watched: HashSet<(u32, u32)>,
    pub partial: HashMap<(u32, u32), f64>,
    /// Audio and subtitle languages picked during this title: kept across stream switches.
    pub audio_wish: Option<String>,
    pub subtitle_choice_wish: Option<Option<String>>,
    pub first_frame_ms: f64,
    pub audio_checked: bool,
    /// The duration last seen and when it changed: Next Up waits for it to settle, since a
    /// stream still being indexed can report a duration that grows as it plays.
    pub duration_seen: f64,
    pub duration_changed_ms: f64,
}

impl App {
    fn new(config: Config, settings: Settings, video: Video) -> Self {
        Self {
            config,
            settings,
            video,
            token: store::token(),
            epoch: 0,
            launch: Launch::default(),
            meta: None,
            tmdb_id: None,
            episode_name: None,
            episode_runtime: None,
            screen: Screen::Loading(String::new()),
            panel: None,
            focus: 0,
            notice: None,
            osd_until: 0.,
            toast: None,
            streams: vec![],
            from_cache: false,
            refetched: false,
            current: None,
            attempts: Attempts::default(),
            start: Start::Zero,
            metadata: false,
            wants_play: false,
            started_ms: 0.,
            first_frame: false,
            stall_since: None,
            reloaded: false,
            seeking: false,
            seek_preview: None,
            seek_commit_ms: 0.,
            ended: false,
            scrobbler: Scrobbler::new(),
            media: None,
            last_save_ms: 0.,
            sign_in_skipped: false,
            binge_group: None,
            auto_resume: false,
            next: None,
            next_checked: false,
            addon_subs: vec![],
            subs_for: None,
            sub_choice: SubChoice::Off,
            sub_track: None,
            osd_focus: OsdFocus::Scrubber,
            speed: 1.,
            paused_at: None,
            episode_overview: None,
            seasons: HashMap::new(),
            seasons_loading: vec![],
            shelf_season: 1,
            shelf_row: 1,
            shelf_focus: 0,
            segments: None,
            skip_hidden: None,
            skipped: vec![],
            watched: HashSet::new(),
            partial: HashMap::new(),
            audio_wish: None,
            subtitle_choice_wish: None,
            first_frame_ms: 0.,
            audio_checked: false,
            duration_seen: 0.,
            duration_changed_ms: 0.,
        }
    }

    pub fn render(&self) {
        ui::render(self);
    }

    pub fn title(&self) -> String {
        self.meta
            .as_ref()
            .map(|m| m.title.clone())
            .or_else(|| self.launch.title.clone())
            .unwrap_or_else(|| "Luma Player".into())
    }

    /// `S1 E2 · Winter Is Coming`, for series.
    pub fn episode_line(&self) -> Option<String> {
        let key = self.launch.episode_key()?;
        let label = luma_core::ids::episode_label(key);
        Some(match &self.episode_name {
            Some(name) if !name.is_empty() => format!("{label} · {name}"),
            _ => label,
        })
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), now_ms() + 4_000.));
    }

    pub fn show_osd(&mut self) {
        self.osd_until = now_ms() + OSD_MS;
    }

    pub fn osd_visible(&self) -> bool {
        matches!(self.screen, Screen::Playing)
            && self.panel != Some(Panel::Episodes)
            && !self.pause_screen()
            && (self.panel.is_some()
                || now_ms() < self.osd_until
                || self.seek_preview.is_some()
                || (self.video.paused() && self.first_frame && !self.ended))
    }

    pub fn buffering(&self) -> bool {
        matches!(self.screen, Screen::Playing)
            && self.current.is_some()
            && self.wants_play
            && (!self.first_frame || self.stall_since.is_some() || self.seeking)
    }

    fn media_key(&self) -> Option<String> {
        Some(media_key(
            self.launch.kind,
            self.launch.imdb.as_deref()?,
            self.launch.episode_key(),
        ))
    }

    fn media_for(&self) -> Option<Media> {
        let ids = trakt::Ids {
            imdb: self.launch.imdb.clone(),
            tmdb: self.tmdb_id.or(self.launch.tmdb),
        };
        if ids.imdb.is_none() && ids.tmdb.is_none() {
            return None;
        }
        Some(match self.launch.episode_key() {
            Some((season, number)) => Media::Episode {
                show: ids,
                season,
                number,
            },
            None => Media::Movie(ids),
        })
    }

    fn progress(&self) -> f64 {
        scrobble::percent(self.video.position(), self.video.duration())
    }

    /// Clears everything about the last title, keeping the show's TMDB details for an episode
    /// of the same show.
    fn reset(&mut self, launch: Launch) {
        let same_show = self.launch.kind == launch.kind
            && ((launch.imdb.is_some() && self.launch.imdb == launch.imdb)
                || (launch.tmdb.is_some() && self.launch.tmdb == launch.tmdb));
        if !same_show {
            self.meta = None;
            self.tmdb_id = None;
            self.binge_group = None;
            self.seasons.clear();
            self.seasons_loading.clear();
            self.watched.clear();
            self.partial.clear();
        }
        self.segments = None;
        self.skip_hidden = None;
        self.skipped.clear();
        self.audio_wish = None;
        self.subtitle_choice_wish = None;
        self.duration_seen = 0.;
        self.duration_changed_ms = now_ms();
        self.osd_focus = OsdFocus::Scrubber;
        self.speed = 1.;
        self.paused_at = None;
        self.episode_overview = None;
        self.epoch += 1;
        self.video.unload();
        self.screen = Screen::Loading(String::new());
        self.panel = None;
        self.notice = None;
        self.launch = launch;
        self.episode_name = None;
        self.episode_runtime = None;
        self.streams.clear();
        self.from_cache = false;
        self.refetched = false;
        self.current = None;
        self.attempts = Attempts::default();
        self.start = Start::Zero;
        self.metadata = false;
        self.wants_play = false;
        self.first_frame = false;
        self.stall_since = None;
        self.reloaded = false;
        self.seeking = false;
        self.seek_preview = None;
        self.ended = false;
        self.scrobbler = Scrobbler::new();
        self.media = None;
        self.last_save_ms = now_ms();
        self.next = None;
        self.next_checked = false;
        self.addon_subs.clear();
        self.subs_for = None;
        self.sub_choice = SubChoice::Off;
        self.sub_track = None;
    }

    // ----- Playback -----

    /// Loads stream `index` and plays it from `start`.
    pub fn play_index(&mut self, index: usize, start: Start) {
        self.load_index(index, start);
        self.wants_play = true;
        self.started_ms = now_ms();
        self.video.play();
        self.render();
    }

    /// Loads without playing, so the stream buffers while the viewer answers Resume.
    fn load_index(&mut self, index: usize, start: Start) {
        let Some(stream) = self.streams.get(index) else {
            return;
        };
        log!(
            "Luma Player: stream {index}: {} {}",
            stream.name,
            stream.url
        );
        let url = stream.url.clone();
        self.current = Some(index);
        self.start = start;
        self.metadata = false;
        self.sub_choice = SubChoice::Off;
        self.sub_track = None;
        self.first_frame = false;
        self.stall_since = None;
        self.reloaded = false;
        self.ended = false;
        self.screen = Screen::Playing;
        self.video.load(&url);
    }

    fn apply_start(&mut self) {
        let duration = self.video.duration();
        match self.start {
            Start::Zero => {}
            Start::At(seconds) => self.video.set_position(seconds),
            Start::Percent(percent) if duration > 0. => {
                self.video.set_position(percent / 100. * duration);
            }
            Start::Percent(_) => return,
        }
        self.start = Start::Zero;
    }

    /// The viewer answered the Resume question.
    fn choose_resume(&mut self, resume: bool) {
        let Screen::Resume(offer) = self.screen.clone() else {
            return;
        };
        self.start = if resume {
            Start::Percent(offer.percent)
        } else {
            Start::Zero
        };
        self.screen = Screen::Playing;
        if self.metadata {
            if resume {
                self.apply_start();
            } else {
                self.video.set_position(0.);
            }
        }
        self.wants_play = true;
        self.started_ms = now_ms();
        self.video.play();
        self.show_osd();
        self.render();
    }

    /// The current stream failed: try the next, or open the picker after five.
    pub fn fail(&mut self, reason: &str) {
        let Some(index) = self.current else {
            return;
        };
        log!("Luma Player: stream {index} failed: {reason}");
        let position = self.video.position();
        let start = if position > 5. {
            Start::At(position)
        } else {
            self.start
        };
        let may_go_on = self.attempts.fail(index, reason);
        // A cached list may hold expired debrid links: fetch it again, once.
        if self.from_cache && !self.refetched {
            self.refetched = true;
            self.video.unload();
            self.current = None;
            self.toast("Refreshing the stream list…");
            self.render();
            let epoch = self.epoch;
            let launch = self.launch.clone();
            let config = self.config.clone();
            spawn_local(async move {
                CACHE.with(|c| c.borrow_mut().remove(&stream_cache_key(&launch)));
                let streams = fetch_streams(&config, &launch, epoch).await;
                update(epoch, move |app| match streams {
                    Ok(streams) if !streams.is_empty() => {
                        app.streams = streams;
                        app.from_cache = false;
                        app.attempts = Attempts::new(app.streams.len());
                        let pick = selection::binge_pick(&app.streams, app.binge_group.as_deref());
                        app.play_index(pick, start);
                    }
                    _ => app.open_picker_after_failures(),
                });
            });
            return;
        }
        match may_go_on.then(|| self.attempts.next_after(index)).flatten() {
            Some(next) => {
                self.toast("That stream didn't play. Trying the next one…");
                self.play_index(next, start);
            }
            None => self.open_picker_after_failures(),
        }
    }

    fn open_picker_after_failures(&mut self) {
        self.video.unload();
        self.current = None;
        self.wants_play = false;
        self.screen = Screen::Playing;
        self.notice = Some("None of these streams played. Pick one to try again.".into());
        self.open_panel(Panel::Picker);
    }

    fn pick_stream(&mut self, index: usize) {
        if index >= self.streams.len() {
            return;
        }
        let position = self.video.position();
        let start = if self.first_frame && position > 5. {
            Start::At(position)
        } else {
            self.start
        };
        if self.first_frame && self.current.is_some() {
            // A deliberate switch: Trakt sees a pause, then a start on the new stream.
            self.scrobble_event(Event::Paused);
        }
        self.attempts.clear(index);
        self.notice = None;
        self.panel = None;
        self.refetched = true;
        self.play_index(index, start);
    }

    // ----- Video events -----

    pub fn on_video(&mut self, event: &str) {
        match event {
            "loadedmetadata" => {
                self.metadata = true;
                self.apply_start();
                self.video.set_rate(self.speed);
            }
            "playing" => {
                if !self.first_frame {
                    self.first_frame = true;
                    self.on_first_frame();
                }
                self.stall_since = None;
                self.seeking = false;
                self.paused_at = None;
                self.scrobble_event(Event::Playing);
            }
            "timeupdate" => {
                if self.first_frame && !self.video.paused() {
                    self.stall_since = None;
                }
            }
            "waiting" | "stalled" => {
                if self.first_frame && self.stall_since.is_none() && !self.video.paused() {
                    self.stall_since = Some(now_ms());
                }
            }
            "seeking" => self.seeking = true,
            "seeked" => self.seeking = false,
            "pause" => {
                if self.first_frame && !self.ended && !self.video.ended() {
                    self.paused_at = Some(now_ms());
                    self.scrobble_event(Event::Paused);
                    self.save_position(true);
                }
            }
            "ended" => self.on_ended(),
            // An unloaded element reports errors too; only a loaded stream's count.
            "error" if self.current.is_some() && !self.video.src().is_empty() => {
                let reason = match self.video.error_code() {
                    2 => "Network error".to_owned(),
                    3 => "The TV couldn't decode it".to_owned(),
                    4 => "Format not supported".to_owned(),
                    code => format!("Playback error {code}"),
                };
                self.fail(&reason);
            }
            _ => {}
        }
        self.render();
    }

    fn on_first_frame(&mut self) {
        let (w, h) = self.video.size();
        log!(
            "Luma Player: first frame after {:.1} s, {w}x{h}",
            (now_ms() - self.started_ms) / 1000.
        );
        if let Some(stream) = self.current.and_then(|i| self.streams.get(i)) {
            self.binge_group = stream.binge_group.clone();
        }
        self.first_frame_ms = now_ms();
        self.audio_checked = false;
        // Audio: this title's pick, else this show's, else the preferred languages.
        let audio = self.video.audio_tracks();
        if audio.len() > 1 {
            let languages: Vec<String> = audio.iter().map(|t| t.language.clone()).collect();
            if let Some(index) = selection::pick_track(&languages, &self.audio_wishes())
                && !audio[index].on
            {
                self.video.set_audio(index);
            }
        }
        // Subtitles: an embedded track in the wanted language, else the addons' (each new
        // stream asks the addons again, for subtitles that match its release).
        self.video.set_text(None);
        if let Some(language) = self.subtitle_wish() {
            let embedded: Vec<String> = self
                .video
                .text_tracks()
                .iter()
                .map(|t| t.language.clone())
                .collect();
            if let Some(index) = selection::pick_track(&embedded, &[language]) {
                self.set_subtitles(SubChoice::Embedded(index));
            }
        }
        self.request_subtitles();
        if self.segments.is_none() {
            spawn_local(fetch_segments(self.epoch));
        }
    }

    /// The show (or film) whose audio and subtitle picks are remembered.
    fn show_key(&self) -> Option<String> {
        self.launch.imdb.clone()
    }

    /// Audio languages in order of preference, "Original" left out.
    fn audio_wishes(&self) -> Vec<String> {
        let remembered = self.audio_wish.clone().or_else(|| {
            self.show_key()
                .and_then(|key| store::show_prefs(&key).audio)
        });
        remembered
            .map(|language| vec![language])
            .unwrap_or_else(|| self.settings.audio_languages.clone())
            .into_iter()
            .filter(|language| language != "original")
            .collect()
    }

    /// The subtitle language wanted, None for off: this title's pick, else this show's, else
    /// the defaults.
    fn subtitle_wish(&self) -> Option<String> {
        if let Some(wish) = &self.subtitle_choice_wish {
            return wish.clone();
        }
        if let Some(remembered) = self
            .show_key()
            .and_then(|key| store::show_prefs(&key).subtitles)
        {
            return remembered;
        }
        self.settings
            .subtitles_on
            .then(|| self.settings.subtitle_languages.first().cloned())
            .flatten()
    }

    /// Addon subtitles in the wanted language, once the list is in, unless some are on.
    fn apply_addon_wish(&mut self) {
        if self.sub_choice != SubChoice::Off {
            return;
        }
        let Some(language) = self.subtitle_wish() else {
            return;
        };
        let languages: Vec<String> = self.addon_subs.iter().map(|s| s.lang.clone()).collect();
        if let Some(index) = selection::pick_track(&languages, &[language]) {
            self.set_subtitles(SubChoice::Addon(index));
        }
    }

    /// Subtitles picked in the Tracks panel: kept for this title and remembered for the show.
    fn choose_subtitles(&mut self, choice: SubChoice) {
        let language = match &choice {
            SubChoice::Off => None,
            SubChoice::Embedded(index) => self
                .video
                .text_tracks()
                .get(*index)
                .map(|t| t.language.clone()),
            SubChoice::Addon(index) => self.addon_subs.get(*index).map(|s| s.lang.clone()),
        };
        let language = language.filter(|l| !l.is_empty());
        self.subtitle_choice_wish = Some(language.clone());
        if let Some(key) = self.show_key() {
            store::save_show_subtitles(&key, language.as_deref());
        }
        self.set_subtitles(choice);
    }

    /// An audio track picked in the Tracks panel: kept and remembered the same way.
    fn choose_audio(&mut self, index: usize) {
        self.video.set_audio(index);
        let language = self
            .video
            .audio_tracks()
            .get(index)
            .map(|t| t.language.clone())
            .filter(|l| !l.is_empty());
        if let Some(language) = language {
            if let Some(key) = self.show_key() {
                store::save_show_audio(&key, &language);
            }
            self.audio_wish = Some(language);
        }
        self.toast("Audio changed");
    }

    fn on_ended(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.scrobble_event(Event::Ended);
        if let Some(key) = self.media_key() {
            store::clear_position(&key);
        }
        self.mark_watched();
        let autoplay = self.settings.autoplay_next;
        let play_now = self
            .next
            .as_ref()
            .is_some_and(|next| autoplay && !next.dismissed);
        if play_now {
            self.play_next();
        } else if let Some(next) = &mut self.next {
            // Autoplay is off (or the card was dismissed): offer it, without a countdown.
            next.dismissed = false;
            next.shown_at = Some(now_ms());
        } else {
            self.exit();
        }
    }

    // ----- The 250 ms tick -----

    pub fn tick(&mut self) {
        let now = now_ms();
        if let Some((_, until)) = &self.toast
            && now > *until
        {
            self.toast = None;
        }
        // Focus returns to the scrubber once the controls fade (not while a panel is up).
        if self.osd_focus != OsdFocus::Scrubber && self.panel.is_none() && !self.osd_visible() {
            self.osd_focus = OsdFocus::Scrubber;
        }
        if matches!(self.screen, Screen::Playing) && self.current.is_some() && self.wants_play {
            self.watchdog(now);
            if self.current.is_none() {
                self.render();
                return;
            }
            let duration = self.video.duration();
            if (duration - self.duration_seen).abs() > 1. {
                self.duration_seen = duration;
                self.duration_changed_ms = now;
            }
            self.auto_skip();
            self.check_audio(now);
            if let Some(target) = self.seek_preview
                && now >= self.seek_commit_ms
            {
                self.seek_preview = None;
                self.video.set_position(target);
            }
            let progress = self.progress();
            if let Some(call) = self.scrobbler.tick(progress, now) {
                self.send_scrobble(call, false);
            }
            if self.first_frame && now - self.last_save_ms > SAVE_EVERY_MS && !self.video.paused() {
                self.save_position(false);
            }
            self.next_up(now);
        }
        self.render();
    }

    fn watchdog(&mut self, now: f64) {
        if !self.first_frame {
            if now - self.started_ms > FIRST_FRAME_MS && !self.video.paused() {
                self.fail("No picture after 12 seconds");
            }
            return;
        }
        let Some(since) = self.stall_since else {
            return;
        };
        let stalled = now - since;
        let position = self.video.position();
        if position < 60. && stalled > EARLY_STALL_MS {
            self.fail("Stalled in the first minute");
        } else if position >= 60. && stalled > LATE_STALL_MS {
            if self.reloaded {
                self.fail("Stalled twice");
            } else if let Some(index) = self.current {
                log!("Luma Player: stalled 15 s at {position:.0} s, reloading");
                self.toast("Reconnecting…");
                self.load_index(index, Start::At(position));
                self.reloaded = true;
                self.first_frame = true;
                self.stall_since = Some(now);
                self.video.play();
            }
        }
    }

    fn next_up(&mut self, now: f64) {
        let duration = self.video.duration();
        if self.launch.kind != Kind::Series || duration <= 0. || !self.first_frame {
            return;
        }
        let position = self.video.position();
        if !self.next_checked && position / duration >= PREFETCH_AT {
            self.next_checked = true;
            let current = self.launch.episode_key();
            let found = self.meta.as_ref().zip(current).and_then(|(meta, current)| {
                luma_core::next::after(current, &meta.seasons, meta.last_aired)
            });
            if let Some(at) = found {
                self.next = Some(NextUp {
                    at,
                    name: None,
                    still: None,
                    streams: None,
                    shown_at: None,
                    dismissed: false,
                });
                spawn_local(prefetch_next(self.epoch, at));
            }
        }
        let left = duration - position;
        // Credits from IntroDB when known, else a fixed time before the end.
        let credits = self
            .segments
            .and_then(|s| s.outro)
            .filter(|outro| outro.start < duration - 5.)
            .is_some_and(|outro| position >= outro.start);
        let near_end = credits || (left <= self.settings.next_card_seconds && left > 1.);
        // Guards against a card that keeps coming back: the duration must have settled for
        // 30 s, and this episode must have played for a minute.
        let settled = now - self.duration_changed_ms > 30_000.
            && now - self.first_frame_ms > 60_000.
            && duration > 300.;
        let countdown = self.settings.countdown_seconds * 1000.;
        let autoplay = self.settings.autoplay_next;
        let paused = self.video.paused();
        let at = self.next.as_ref().map(|n| n.at);
        let mut play = false;
        if let Some(next) = &mut self.next {
            if !next.dismissed && next.shown_at.is_none() && near_end && settled {
                log!(
                    "Luma Player: Next Up for S{}E{} at {position:.0}/{duration:.0} s{}",
                    next.at.0,
                    next.at.1,
                    if credits { " (credits)" } else { "" }
                );
                next.shown_at = Some(now);
            }
            // The countdown holds while paused.
            if paused && let Some(shown) = next.shown_at {
                next.shown_at = Some(shown + 250.);
            }
            if autoplay
                && !paused
                && !next.dismissed
                && next.shown_at.is_some_and(|shown| now - shown >= countdown)
            {
                play = true;
            }
        }
        if play {
            if let Some((season, number)) = at {
                log!("Luma Player: autoplaying S{season}E{number}");
            }
            self.play_next();
        }
    }

    /// Stops this episode and starts the next in the same `<video>`, without relaunching.
    fn play_next(&mut self) {
        let Some(next) = self.next.take() else {
            return;
        };
        self.exit_scrobble(false);
        if self.progress() >= scrobble::WATCHED_PERCENT
            && let Some(key) = self.media_key()
        {
            store::clear_position(&key);
            self.mark_watched();
        } else {
            self.save_position(true);
        }
        let launch = self.launch.with_episode(next.at);
        let streams = next.streams;
        spawn_local(async move { begin(launch, streams, true) });
    }

    // ----- Scrobbling and resume points -----

    fn scrobble_event(&mut self, event: Event) {
        let call = self.scrobbler.event(event, self.progress(), now_ms());
        if let Some(call) = call {
            self.send_scrobble(call, false);
        }
    }

    fn exit_scrobble(&mut self, keepalive: bool) {
        if !self.first_frame {
            return;
        }
        if let Some(call) = self.scrobbler.event(Event::Exit, self.progress(), now_ms()) {
            self.send_scrobble(call, keepalive);
        }
    }

    fn send_scrobble(&self, call: Call, keepalive: bool) {
        let (Some(app), Some(media)) = (self.config.trakt.clone(), self.media.clone()) else {
            return;
        };
        log!(
            "Luma Player: scrobble {} at {:.1}%",
            call.action.path(),
            call.progress
        );
        let path = format!("/scrobble/{}", call.action.path());
        let body = trakt::scrobble_body(&media, call.progress).to_string();
        let Some(token) = self.token.clone() else {
            // Not signed in: nothing to send, nothing to queue.
            return;
        };
        if keepalive {
            net::keepalive(&app.replay(&token.access, &path, &body));
            return;
        }
        spawn_local(async move {
            let reply = trakt_send(|app, token| app.replay(token, &path, &body)).await;
            match reply {
                Some(Ok(reply)) if trakt::settled(reply.status) => {}
                Some(Ok(reply)) if reply.status == 401 || reply.status == 403 => {
                    log!("Luma Player: Trakt refused the scrobble ({})", reply.status);
                }
                _ => store::queue_push(&path, &body),
            }
        });
    }

    fn save_position(&mut self, force: bool) {
        let position = self.video.position();
        let duration = self.video.duration();
        if !self.first_frame || (!force && position < 1.) {
            return;
        }
        if let Some(key) = self.media_key() {
            if scrobble::percent(position, duration) >= resume::MAX_PERCENT {
                store::clear_position(&key);
            } else if position > 0. {
                store::save_position(&key, position, duration, now_ms());
            }
        }
        self.last_save_ms = now_ms();
    }

    /// Back, Stop, or the end with nothing next: tell Trakt, save the place, close.
    pub fn exit(&mut self) {
        self.exit_scrobble(true);
        self.save_position(true);
        crate::devlog::flush();
        self.video.unload();
        self.current = None;
        self.screen = Screen::Message {
            title: self.title(),
            body: "Returning to Luma…".into(),
            retry: false,
        };
        self.render();
        // On the TV this returns to Luma; a desktop tab stays on the message.
        dom::window().close().ok();
    }

    // ----- Subtitles -----

    fn request_subtitles(&mut self) {
        if self.config.subtitle_addons.is_empty() {
            return;
        }
        let Some(imdb) = self.launch.imdb.clone() else {
            return;
        };
        let filename = self
            .current
            .and_then(|i| self.streams.get(i))
            .and_then(|s| s.filename.clone());
        let id = stremio_id(&imdb, self.launch.episode_key());
        let wanted = format!("{id}|{}", filename.clone().unwrap_or_default());
        if self.subs_for.as_deref() == Some(wanted.as_str()) {
            // Same release as before: the list stands, only the pick is made again.
            self.apply_addon_wish();
            return;
        }
        self.addon_subs.clear();
        self.subs_for = Some(wanted);
        let epoch = self.epoch;
        let kind = self.launch.kind;
        let addons = self.config.subtitle_addons.clone();
        spawn_local(async move {
            let mut found: Vec<Subtitle> = vec![];
            for base in addons {
                let url = stremio::subtitles_url(&base, kind, &id, filename.as_deref());
                match net::send(&luma_core::http::Request::get(url), 10_000).await {
                    Ok(reply) if reply.ok() => match stremio::parse_subtitles(&reply.text) {
                        Ok(list) => found.extend(list),
                        Err(error) => log!("Luma Player subtitles: {error}"),
                    },
                    Ok(reply) => log!("Luma Player subtitles: addon answered {}", reply.status),
                    Err(error) => log!("Luma Player subtitles: {error}"),
                }
            }
            log!("Luma Player: {} addon subtitles", found.len());
            update(epoch, move |app| {
                app.addon_subs = found;
                app.apply_addon_wish();
                app.render();
            });
        });
    }

    pub fn set_subtitles(&mut self, choice: SubChoice) {
        self.sub_track = None;
        match &choice {
            SubChoice::Off | SubChoice::Addon(_) => self.video.set_text(None),
            SubChoice::Embedded(index) => self.video.set_text(Some(*index)),
        }
        if let SubChoice::Addon(index) = choice
            && let Some(sub) = self.addon_subs.get(index).cloned()
        {
            let epoch = self.epoch;
            spawn_local(async move {
                let track = match net::bytes(&sub.url, 15_000).await {
                    Ok((status, bytes)) if (200..300).contains(&status) => {
                        let track = Track::new(subs::parse(&subs::decode(&bytes)));
                        (!track.is_empty()).then_some(track)
                    }
                    _ => None,
                };
                update(epoch, move |app| {
                    if app.sub_choice != SubChoice::Addon(index) {
                        return;
                    }
                    match track {
                        Some(track) => {
                            log!("Luma Player: {} subtitle cues", track.len());
                            app.sub_track = Some(track);
                        }
                        None => {
                            app.sub_choice = SubChoice::Off;
                            app.toast("Those subtitles didn't load. Try another.");
                        }
                    }
                    app.render();
                });
            });
        }
        self.sub_choice = choice;
    }

    /// The subtitle HTML for now, with the viewer's offset (positive shows them later).
    pub fn subtitle_html(&self) -> String {
        match (&self.sub_choice, &self.sub_track) {
            (SubChoice::Addon(_), Some(track)) => {
                let at =
                    self.video.position() - f64::from(self.settings.subtitle_offset_ms) / 1000.;
                track.html_at(at)
            }
            _ => String::new(),
        }
    }

    // ----- Panels -----

    pub fn open_panel(&mut self, panel: Panel) {
        self.panel = Some(panel);
        let rows = ui::rows(self, panel);
        // Focus what's current: the playing stream, the chosen track, else the first row.
        self.focus = rows
            .iter()
            .position(|r| r.selected && r.id.is_some())
            .or_else(|| rows.iter().position(|r| r.id.is_some()))
            .unwrap_or(0);
        self.render();
    }

    fn move_focus(&mut self, step: i32) {
        let Some(panel) = self.panel else { return };
        let rows = ui::rows(self, panel);
        let selectable: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].id.is_some()).collect();
        if selectable.is_empty() {
            return;
        }
        let at = selectable
            .iter()
            .position(|&i| i == self.focus)
            .unwrap_or(0) as i32;
        let next = (at + step).clamp(0, selectable.len() as i32 - 1) as usize;
        self.focus = selectable[next];
    }

    fn activate(&mut self, adjust: i32) {
        let Some(panel) = self.panel else { return };
        let rows = ui::rows(self, panel);
        let Some(id) = rows.get(self.focus).and_then(|r| r.id.clone()) else {
            return;
        };
        use ui::RowId as R;
        let cycle = |value: f64, options: &[f64], step: i32| -> f64 {
            let at = options
                .iter()
                .position(|o| (o - value).abs() < 0.01)
                .unwrap_or(0) as i32;
            let n = options.len() as i32;
            let dir = if step < 0 { -1 } else { 1 };
            options[((at + dir + n) % n) as usize]
        };
        let step = if adjust == 0 { 1 } else { adjust };
        match id {
            R::Stream(index) => {
                if adjust == 0 {
                    self.pick_stream(index);
                }
            }
            R::Audio(index) => {
                if adjust == 0 {
                    self.choose_audio(index);
                }
            }
            R::SubOff => {
                if adjust == 0 {
                    self.choose_subtitles(SubChoice::Off);
                }
            }
            R::SubEmbedded(index) => {
                if adjust == 0 {
                    self.choose_subtitles(SubChoice::Embedded(index));
                }
            }
            R::SubAddon(index) => {
                if adjust == 0 {
                    self.choose_subtitles(SubChoice::Addon(index));
                }
            }
            R::AutoSkip => self.settings.auto_skip = !self.settings.auto_skip,
            R::SubSize => {
                self.settings.subtitle_size =
                    ((i32::from(self.settings.subtitle_size) + step + 3) % 3) as u8;
            }
            R::SubBox => self.settings.subtitle_box = !self.settings.subtitle_box,
            R::SubOffset => {
                self.settings.subtitle_offset_ms = if adjust == 0 {
                    0
                } else {
                    (self.settings.subtitle_offset_ms + 250 * adjust).clamp(-30_000, 30_000)
                };
            }
            R::Autoplay => self.settings.autoplay_next = !self.settings.autoplay_next,
            R::NextCard => {
                self.settings.next_card_seconds = cycle(
                    self.settings.next_card_seconds,
                    &[30., 45., 60., 90., 120.],
                    step,
                );
            }
            R::Countdown => {
                self.settings.countdown_seconds =
                    cycle(self.settings.countdown_seconds, &[5., 10., 15., 20.], step);
            }
            R::AudioLanguage => {
                let now = self
                    .settings
                    .audio_languages
                    .first()
                    .cloned()
                    .unwrap_or_default();
                self.settings.audio_languages = vec![ui::cycle_language(&now, step, true)];
            }
            R::SubtitleLanguage => {
                let now = self
                    .settings
                    .subtitle_languages
                    .first()
                    .cloned()
                    .unwrap_or_default();
                self.settings.subtitle_languages = vec![ui::cycle_language(&now, step, false)];
            }
            R::SubtitlesOn => self.settings.subtitles_on = !self.settings.subtitles_on,
            R::Trakt => {
                if adjust == 0 {
                    self.trakt_toggle();
                }
            }
            R::Speed(rate) => {
                if adjust == 0 {
                    self.set_speed(rate);
                }
            }
        }
        self.settings.save();
    }

    fn trakt_toggle(&mut self) {
        match self.token.take() {
            Some(token) => {
                if let Some(app) = &self.config.trakt {
                    net::keepalive(&app.revoke(&token.access));
                }
                store::save_token(None);
                store::skip_sign_in(now_ms());
                self.toast("Signed out of Trakt");
            }
            None => {
                store::clear_skip_sign_in();
                self.toast("Trakt sign-in shows on the next title you open");
            }
        }
    }

    // ----- Keys -----

    /// True when the key was used (its default is then prevented).
    pub fn on_key(&mut self, key: Key) -> bool {
        let used = self.route_key(key);
        self.render();
        used
    }

    fn route_key(&mut self, key: Key) -> bool {
        match self.screen.clone() {
            Screen::Loading(_) => {
                if key == Key::Back || key == Key::Stop {
                    self.exit();
                }
                true
            }
            Screen::SignIn { .. } => {
                if matches!(key, Key::Ok | Key::Back) {
                    self.sign_in_skipped = true;
                    store::skip_sign_in(now_ms());
                    self.screen = Screen::Loading("Finding the best stream…".into());
                }
                true
            }
            Screen::Resume(_) => {
                match key {
                    Key::Up | Key::Left => self.focus = 0,
                    Key::Down | Key::Right => self.focus = 1,
                    Key::Ok | Key::Play | Key::PlayPause => self.choose_resume(self.focus == 0),
                    Key::Back | Key::Stop => self.exit(),
                    _ => {}
                }
                true
            }
            Screen::Message { retry, .. } => {
                match key {
                    Key::Ok if retry => {
                        let launch = self.launch.clone();
                        spawn_local(async move { begin(launch, None, false) });
                    }
                    Key::Ok | Key::Back | Key::Stop => self.exit(),
                    _ => {}
                }
                true
            }
            Screen::Playing => self.playing_key(key),
        }
    }

    fn playing_key(&mut self, key: Key) -> bool {
        if self.pause_screen() {
            self.leave_pause_screen(key);
            return true;
        }
        if self.panel == Some(Panel::Episodes) {
            self.shelf_key(key);
            return true;
        }
        if let Some(panel) = self.panel {
            match key {
                Key::Back => {
                    if self.current.is_none() && panel == Panel::Picker {
                        // Nothing plays: Back from the picker leaves.
                        self.exit();
                    } else {
                        // Back to the controls, on the button that opened the panel.
                        self.panel = None;
                        self.notice = None;
                        self.show_osd();
                    }
                }
                Key::Up => self.move_focus(-1),
                Key::Down => self.move_focus(1),
                Key::Left => self.activate(-1),
                Key::Right => self.activate(1),
                Key::Ok => self.activate(0),
                Key::Red => self.toggle_panel(Panel::Info),
                Key::Green => self.toggle_panel(Panel::Picker),
                Key::Yellow => self.toggle_panel(Panel::Tracks),
                Key::Blue => self.toggle_panel(Panel::Settings),
                Key::Stop => self.exit(),
                _ => self.media_key_pressed(key),
            }
            return true;
        }
        if let OsdFocus::Button(index) = self.osd_focus
            && self.osd_visible()
            && self.button_key(key, index)
        {
            return true;
        }
        // Skip Intro / Skip Recap, while the controls are down: OK skips, Back hides it.
        if let Some(kind) = self.skip_offer().map(|(kind, _)| kind)
            && !self.osd_visible()
        {
            match key {
                Key::Ok => {
                    self.skip_now();
                    return true;
                }
                Key::Back => {
                    self.skip_hidden = Some(kind);
                    return true;
                }
                _ => {}
            }
        }
        // Next Up: OK plays now, Back keeps the credits.
        let card = self
            .next
            .as_ref()
            .is_some_and(|next| next.shown_at.is_some() && !next.dismissed);
        if card {
            match key {
                Key::Ok => {
                    self.play_next();
                    return true;
                }
                Key::Back => {
                    if let Some(next) = &mut self.next {
                        next.dismissed = true;
                    }
                    if self.ended {
                        self.exit();
                    }
                    return true;
                }
                _ => {}
            }
        }
        match key {
            Key::Ok => {
                if self.osd_visible() || self.video.paused() {
                    self.toggle_play();
                } else {
                    self.show_osd();
                }
            }
            Key::Up => self.show_osd(),
            // Down moves into the control bar, at the button it was last on.
            Key::Down => {
                self.show_osd();
                self.osd_focus = OsdFocus::Button(self.default_button());
            }
            Key::Left => self.seek_by(-10.),
            Key::Right => self.seek_by(10.),
            Key::Back => {
                if self.osd_visible() && !self.video.paused() {
                    self.osd_until = 0.;
                } else {
                    self.exit();
                }
            }
            Key::Red => self.open_panel(Panel::Info),
            Key::Green => self.open_panel(Panel::Picker),
            Key::Yellow => self.open_panel(Panel::Tracks),
            Key::Blue => self.open_panel(Panel::Settings),
            other => self.media_key_pressed(other),
        }
        true
    }

    fn toggle_panel(&mut self, panel: Panel) {
        if self.panel == Some(panel) {
            self.panel = None;
        } else {
            self.open_panel(panel);
        }
    }

    fn media_key_pressed(&mut self, key: Key) {
        match key {
            Key::PlayPause => self.toggle_play(),
            Key::Play => {
                if self.video.paused() {
                    self.toggle_play();
                }
            }
            Key::Pause => {
                if !self.video.paused() {
                    self.toggle_play();
                }
            }
            Key::FastForward => self.seek_by(60.),
            Key::Rewind => self.seek_by(-60.),
            Key::Stop => self.exit(),
            _ => {}
        }
    }

    fn toggle_play(&mut self) {
        if self.current.is_none() {
            return;
        }
        if self.video.paused() {
            self.wants_play = true;
            if !self.first_frame {
                self.started_ms = now_ms();
            }
            self.video.play();
        } else {
            self.video.pause();
        }
        self.show_osd();
    }

    /// Seeks add up while the remote is pressed and land 600 ms after the last press.
    fn seek_by(&mut self, seconds: f64) {
        if self.current.is_none() || !self.metadata {
            return;
        }
        let duration = self.video.duration();
        let from = self.seek_preview.unwrap_or_else(|| self.video.position());
        let to = if duration > 0. {
            (from + seconds).clamp(0., (duration - 1.).max(0.))
        } else {
            (from + seconds).max(0.)
        };
        self.seek_preview = Some(to);
        self.seek_commit_ms = now_ms() + SEEK_COMMIT_MS;
        self.show_osd();
    }
}

// ----- Start-up and launches -----

pub fn start() {
    let config = crate::config::load();
    crate::devlog::init(config.log_url.clone());
    let Some(video) = Video::find() else {
        log!("Luma Player: no <video> element");
        return;
    };
    let settings = Settings::load(&config.defaults);
    APP.with(|cell| *cell.borrow_mut() = Some(App::new(config, settings, video.clone())));
    install(&video);
    spawn_local(replay_queue());
    match launch_params() {
        Ok(launch) => begin(launch, None, false),
        Err(error) => {
            log!("Luma Player: {error}");
            with(|app| {
                app.screen = Screen::Message {
                    title: "Nothing to play".into(),
                    body: format!("{error}. Open a title from Luma."),
                    retry: false,
                };
                app.render();
            });
        }
    }
}

fn install(video: &Video) {
    let document = dom::document();
    dom::listen(document.as_ref(), "keydown", |event| {
        let Some(event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
            return;
        };
        let code = event.key_code();
        let Some(key) = luma_core::remote::key(code) else {
            if cfg!(feature = "dev") {
                log!("Luma Player key: unmapped keyCode {code}");
            }
            return;
        };
        if with(|app| app.on_key(key)).unwrap_or(false) {
            event.prevent_default();
        }
    });
    for name in [
        "loadedmetadata",
        "playing",
        "timeupdate",
        "waiting",
        "stalled",
        "seeking",
        "seeked",
        "pause",
        "ended",
        "error",
    ] {
        dom::listen(video.target(), name, move |_| {
            with(|app| app.on_video(name));
        });
    }
    // Home or another app in front: pause, so Trakt and the resume point hold this place.
    dom::listen(document.as_ref(), "visibilitychange", |_| {
        if dom::document().hidden() {
            with(|app| {
                if app.first_frame && !app.video.paused() {
                    app.video.pause();
                }
                app.save_position(true);
                crate::devlog::flush();
            });
        }
    });
    // A poster opened while the player is already open.
    dom::listen(document.as_ref(), "webOSRelaunch", |_| {
        dom::palm_call("activate");
        match launch_params() {
            Ok(launch) => relaunch(launch),
            Err(error) => log!("Luma Player relaunch: {error}"),
        }
    });
    // The Magic Remote pointer: a click on a control-bar button, shelf tab or card, or panel
    // row picks it; anywhere else it acts as OK.
    dom::listen(document.as_ref(), "click", |event| {
        let target = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let number = |attribute: &str| -> Option<usize> {
            let selector = format!("[{attribute}]");
            target
                .as_ref()?
                .closest(&selector)
                .ok()
                .flatten()?
                .get_attribute(attribute)?
                .parse()
                .ok()
        };
        let (button, season, episode, row) = (
            number("data-osd"),
            number("data-season"),
            number("data-ep"),
            number("data-row"),
        );
        let skip = number("data-skip").is_some();
        with(|app| {
            if skip {
                app.skip_now();
            } else if let Some(index) = button {
                app.click_button(index);
            } else if season.is_some() || episode.is_some() {
                app.click_shelf(season.map(|s| s as u32), episode);
            } else {
                if let Some(row) = row {
                    app.focus = row;
                }
                app.on_key(Key::Ok);
                return;
            }
            app.render();
        });
    });
    dom::every(250, || {
        with(App::tick);
    });
    dom::every(100, || {
        if let Some(html) = with(|app| app.subtitle_html()) {
            dom::set_html("subs", &html);
        }
    });
}

/// `PalmSystem.launchParams` on the TV, else `?params=` (the dev shell and the laptop).
fn launch_params() -> Result<Launch, String> {
    let from_palm = dom::palm()
        .map(|palm| dom::get(&palm, "launchParams"))
        .and_then(|params| {
            params.as_string().or_else(|| {
                (!params.is_undefined() && !params.is_null())
                    .then(|| js_sys::JSON::stringify(&params).ok()?.as_string())
                    .flatten()
            })
        })
        .filter(|text| {
            let t = text.trim();
            !t.is_empty() && t != "{}" && t != "null"
        });
    let search = dom::window().location().search().unwrap_or_default();
    let text = from_palm
        .or_else(|| luma_core::http::query_param(&search, "params"))
        .ok_or("No launch params")?;
    let mut launch = Launch::from_json(&text)?;
    if !cfg!(feature = "dev") {
        launch.url = None;
        launch.start = None;
    } else if let Some(t) = luma_core::http::query_param(&search, "t").and_then(|t| t.parse().ok())
    {
        launch.start = Some(t);
    }
    Ok(launch)
}

fn relaunch(launch: Launch) {
    let same = with(|app| {
        let same = app.launch.kind == launch.kind
            && app.launch.imdb.is_some()
            && app.launch.imdb == launch.imdb
            && (launch.episode_key().is_none() || app.launch.episode_key() == launch.episode_key());
        if same {
            app.show_osd();
            app.render();
        } else {
            app.exit_scrobble(false);
            app.save_position(true);
        }
        same
    })
    .unwrap_or(false);
    if !same {
        begin(launch, None, false);
    }
}

/// Starts a title: clears the screen, then fetches everything it needs.
fn begin(launch: Launch, streams: Option<Vec<Stream>>, auto_resume: bool) {
    let epoch = with(|app| {
        app.reset(launch);
        app.auto_resume = auto_resume;
        app.screen = Screen::Loading("Loading…".into());
        app.render();
        app.epoch
    });
    if let Some(epoch) = epoch {
        spawn_local(prepare(epoch, streams));
    }
}

fn status(epoch: u64, text: &str) {
    update(epoch, |app| {
        if matches!(app.screen, Screen::Loading(_)) {
            app.screen = Screen::Loading(text.into());
            app.render();
        }
    });
}

fn message(epoch: u64, title: &str, body: &str, retry: bool) {
    update(epoch, |app| {
        app.screen = Screen::Message {
            title: title.into(),
            body: body.into(),
            retry,
        };
        app.render();
    });
}

async fn prepare(epoch: u64, carried: Option<Vec<Stream>>) {
    let Some((launch, config, have_meta)) = update(epoch, |app| {
        (app.launch.clone(), app.config.clone(), app.meta.is_some())
    }) else {
        return;
    };
    // 1. Artwork and IDs from TMDB, for the loading screen.
    if let Some(key) = &config.tmdb_key
        && !have_meta
    {
        let id = match (launch.tmdb, &launch.imdb) {
            (Some(id), _) => Some(id),
            (None, Some(imdb)) => tmdb_find(key, imdb, launch.kind).await,
            (None, None) => None,
        };
        if let Some(id) = id {
            let meta = tmdb_details(key, launch.kind, id).await;
            update(epoch, |app| {
                app.tmdb_id = Some(id);
                if app.launch.imdb.is_none() {
                    app.launch.imdb = meta.as_ref().and_then(|m| m.imdb.clone());
                }
                app.meta = meta;
                app.render();
            });
        }
    }
    let Some(launch) = update(epoch, |app| app.launch.clone()) else {
        return;
    };
    if launch.imdb.is_none() && launch.url.is_none() {
        let why = if config.tmdb_key.is_none() {
            "This card has only a TMDB ID; add a TMDB key to the player's config to look up its IMDb ID."
        } else {
            "TMDB has no IMDb ID for it, and AIOStreams needs one."
        };
        message(epoch, "Can't play this title", why, false);
        return;
    }
    // 2. Trakt sign-in, the first time.
    ensure_signed_in(epoch).await;
    // 3. A show with no episode: the next unwatched one.
    if launch.kind == Kind::Series && launch.episode_key().is_none() {
        status(epoch, "Finding your next episode…");
        let next = match &launch.imdb {
            Some(imdb) => {
                let imdb = imdb.clone();
                trakt_send(move |app, token| app.progress(token, &imdb))
                    .await
                    .and_then(Result::ok)
                    .filter(|reply| reply.ok())
                    .and_then(|reply| trakt::parse_next_episode(&reply.text))
            }
            None => None,
        };
        update(epoch, |app| {
            let at = next.unwrap_or_else(|| {
                luma_core::next::first(app.meta.as_ref().map_or(&[][..], |m| &m.seasons))
            });
            app.launch = app.launch.with_episode(at);
        });
    }
    let Some(launch) = update(epoch, |app| {
        app.media = app.media_for();
        app.render();
        app.launch.clone()
    }) else {
        return;
    };
    spawn_local(episode_details(epoch));
    if launch.kind == Kind::Series {
        spawn_local(load_show_progress(epoch));
    }
    // 4. The resume point (Trakt and local), looked up while the streams load.
    let resume_cell = std::rc::Rc::new(RefCell::new(None::<Option<resume::Point>>));
    {
        let cell = resume_cell.clone();
        spawn_local(async move {
            let point = resume_point(epoch).await;
            *cell.borrow_mut() = Some(point);
        });
    }
    // 5. Streams.
    status(epoch, "Finding the best stream…");
    let (streams, from_cache) = match carried {
        Some(streams) if !streams.is_empty() => (Ok(streams), true),
        _ => {
            let cached = cached_streams(&launch);
            match cached {
                Some(streams) => (Ok(streams), true),
                None => (fetch_streams(&config, &launch, epoch).await, false),
            }
        }
    };
    let streams = match streams {
        Ok(streams) => streams,
        Err(StreamsError::Slow) => {
            message(
                epoch,
                "AIOStreams is slow",
                "It didn't answer in time. Press OK to try again.",
                true,
            );
            return;
        }
        Err(StreamsError::Other(error)) => {
            message(epoch, "Couldn't find streams", &error, true);
            return;
        }
    };
    // Wait a little for the resume lookup; it is usually done by now.
    for _ in 0..20 {
        if resume_cell.borrow().is_some() {
            break;
        }
        dom::sleep(100).await;
    }
    let point = (*resume_cell.borrow()).flatten();
    update(epoch, move |app| {
        if streams.is_empty() {
            app.screen = Screen::Message {
                title: "No streams found".into(),
                body: format!("AIOStreams has nothing playable for {}.", app.title()),
                retry: false,
            };
            app.render();
            return;
        }
        app.streams = streams;
        app.from_cache = from_cache;
        app.attempts = Attempts::new(app.streams.len());
        let index = selection::binge_pick(&app.streams, app.binge_group.as_deref());
        let runtime = app
            .episode_runtime
            .or_else(|| app.meta.as_ref().and_then(|m| m.runtime))
            .map(|minutes| minutes * 60.);
        let offer = resume::offer(point, runtime);
        match (app.launch.start, offer) {
            (Some(t), _) => app.play_index(index, Start::At(t)),
            (None, Some(offer)) if app.auto_resume && offer.percent < 80. => {
                app.toast(format!("Resuming from {}", ui::offer_label(&offer)));
                app.play_index(index, Start::Percent(offer.percent));
            }
            // Autoplay never lands near the end of an old watch: that loops Next Up.
            (None, Some(offer)) if app.auto_resume => {
                log!(
                    "Luma Player: old resume point at {:.0}% ignored for autoplay",
                    offer.percent
                );
                app.play_index(index, Start::Zero);
            }
            (None, Some(offer)) => {
                app.load_index(index, Start::Zero);
                app.screen = Screen::Resume(offer);
                app.focus = 0;
                app.render();
            }
            (None, None) => app.play_index(index, Start::Zero),
        }
    });
}

// ----- Network tasks -----

pub enum StreamsError {
    Slow,
    Other(String),
}

fn stream_cache_key(launch: &Launch) -> String {
    stremio_id(
        launch.imdb.as_deref().unwrap_or_default(),
        launch.episode_key(),
    )
}

fn cached_streams(launch: &Launch) -> Option<Vec<Stream>> {
    let key = stream_cache_key(launch);
    CACHE.with(|cache| {
        cache
            .borrow()
            .get(&key)
            .filter(|(at, _)| now_ms() - at < STREAM_CACHE_MS)
            .map(|(_, streams)| streams.clone())
    })
}

async fn fetch_streams(
    config: &Config,
    launch: &Launch,
    epoch: u64,
) -> Result<Vec<Stream>, StreamsError> {
    if let Some(url) = &launch.url {
        return Ok(vec![Stream {
            name: "Direct URL".into(),
            description: "Dev launch".into(),
            url: url.clone(),
            ..Stream::default()
        }]);
    }
    let base = config.streams.clone().ok_or_else(|| {
        StreamsError::Other("Add your AIOStreams manifest URL to player/config.local.json.".into())
    })?;
    let imdb = launch
        .imdb
        .clone()
        .ok_or_else(|| StreamsError::Other("No IMDb ID".into()))?;
    let id = stremio_id(&imdb, launch.episode_key());
    let request = luma_core::http::Request::get(stremio::streams_url(&base, launch.kind, &id));
    let started = now_ms();
    let mut reply = net::send(&request, STREAMS_TIMEOUT_MS).await;
    if matches!(reply, Err(Failure::Timeout)) {
        status(epoch, "AIOStreams is slow, trying again…");
        reply = net::send(&request, STREAMS_TIMEOUT_MS).await;
    }
    let reply = match reply {
        Ok(reply) => reply,
        Err(Failure::Timeout) => return Err(StreamsError::Slow),
        Err(error) => return Err(StreamsError::Other(format!("AIOStreams: {error}"))),
    };
    if !reply.ok() {
        return Err(StreamsError::Other(format!(
            "AIOStreams answered {}",
            reply.status
        )));
    }
    let all = stremio::parse_streams(&reply.text).map_err(StreamsError::Other)?;
    let total = all.len();
    let streams = selection::playable(all);
    log!(
        "Luma Player: {} of {total} streams playable in {:.1} s",
        streams.len(),
        (now_ms() - started) / 1000.
    );
    CACHE.with(|cache| cache.borrow_mut().insert(id, (now_ms(), streams.clone())));
    Ok(streams)
}

async fn tmdb_find(key: &str, imdb: &str, kind: Kind) -> Option<u64> {
    let reply = net::send(&tmdb::find(key, imdb), 10_000).await.ok()?;
    tmdb::parse_find(&reply.text, kind)
}

async fn tmdb_details(key: &str, kind: Kind, id: u64) -> Option<Meta> {
    let reply = net::send(&tmdb::details(key, kind, id), 10_000)
        .await
        .ok()?;
    match tmdb::parse_details(&reply.text, kind) {
        Ok(meta) => Some(meta),
        Err(error) => {
            log!("Luma Player TMDB: {error}");
            None
        }
    }
}

/// The episode's name and runtime for the loading screen and the Resume question.
async fn episode_details(epoch: u64) {
    let Some((key, show, Some((season, number)))) = update(epoch, |app| {
        (
            app.config.tmdb_key.clone(),
            app.tmdb_id,
            app.launch.episode_key(),
        )
    }) else {
        return;
    };
    let (Some(key), Some(show)) = (key, show) else {
        return;
    };
    let Ok(reply) = net::send(&tmdb::season(&key, show, season), 10_000).await else {
        return;
    };
    let episodes = tmdb::parse_season(&reply.text);
    let episode = episodes.iter().find(|e| e.number == number).cloned();
    update(epoch, |app| {
        if let Some(episode) = episode {
            app.episode_name = Some(episode.name).filter(|n| !n.is_empty());
            app.episode_overview = Some(episode.overview).filter(|o| !o.is_empty());
            app.episode_runtime = episode.runtime;
        }
        // The Episodes shelf opens on this season without another fetch.
        if !episodes.is_empty() {
            app.seasons.insert(season, episodes);
        }
        app.render();
    });
}

/// Next Up: its streams (to start in under 3 s) and its name and still.
async fn prefetch_next(epoch: u64, at: (u32, u32)) {
    let Some((config, launch, key, show)) = update(epoch, |app| {
        (
            app.config.clone(),
            app.launch.with_episode(at),
            app.config.tmdb_key.clone(),
            app.tmdb_id,
        )
    }) else {
        return;
    };
    if let (Some(key), Some(show)) = (key, show)
        && let Ok(reply) = net::send(&tmdb::season(&key, show, at.0), 10_000).await
    {
        let episode = tmdb::parse_season(&reply.text)
            .into_iter()
            .find(|e| e.number == at.1);
        update(epoch, |app| {
            if let (Some(next), Some(episode)) = (&mut app.next, episode) {
                next.name = Some(episode.name).filter(|n| !n.is_empty());
                next.still = episode.still;
            }
        });
    }
    let streams = match cached_streams(&launch) {
        Some(streams) => Some(streams),
        None => fetch_streams(&config, &launch, epoch).await.ok(),
    };
    update(epoch, |app| {
        if let Some(next) = &mut app.next {
            // Same source as now when its bingeGroup matches: put that stream first.
            if let Some(mut streams) = streams {
                let pick = selection::binge_pick(&streams, app.binge_group.as_deref());
                if pick > 0 {
                    let stream = streams.remove(pick);
                    streams.insert(0, stream);
                }
                log!("Luma Player: Next Up has {} streams ready", streams.len());
                next.streams = Some(streams);
            }
        }
    });
}

async fn resume_point(epoch: u64) -> Option<resume::Point> {
    let (key, media) = update(epoch, |app| (app.media_key(), app.media.clone()))?;
    let local = key.as_deref().and_then(store::position);
    let trakt = match media {
        Some(media) => {
            let episodes = matches!(media, Media::Episode { .. });
            trakt_send(move |app, token| app.playback(token, episodes))
                .await
                .and_then(Result::ok)
                .filter(|reply| reply.ok())
                .and_then(|reply| trakt::find_playback(&reply.text, &media))
        }
        None => None,
    };
    resume::newer(trakt, local)
}

/// Sends a Trakt request with the current token, refreshing it first when due and once more
/// on a 401. None when Trakt isn't configured or signed in.
async fn trakt_send(
    build: impl Fn(&trakt::App, &str) -> luma_core::http::Request,
) -> Option<Result<net::Reply, Failure>> {
    let (app, token) = with(|a| (a.config.trakt.clone(), a.token.clone()))?;
    let (app, mut token) = (app?, token?);
    if token.due(now_ms() / 1000.)
        && let Some(fresh) = refresh_token(&app, &token).await
    {
        token = fresh;
    }
    let reply = net::send(&build(&app, &token.access), 10_000).await;
    if let Ok(r) = &reply
        && r.status == 401
        && let Some(fresh) = refresh_token(&app, &token).await
    {
        return Some(net::send(&build(&app, &fresh.access), 10_000).await);
    }
    Some(reply)
}

async fn refresh_token(app: &trakt::App, token: &Token) -> Option<Token> {
    let reply = net::send(&app.refresh(&token.refresh), 10_000).await.ok()?;
    if !reply.ok() {
        log!("Luma Player: Trakt token refresh answered {}", reply.status);
        if reply.status == 400 || reply.status == 401 {
            // The refresh token is dead: sign in again next time.
            store::save_token(None);
            with(|a| a.token = None);
        }
        return None;
    }
    let fresh = Token::from_reply(&reply.text)?;
    store::save_token(Some(&fresh));
    with(|a| a.token = Some(fresh.clone()));
    Some(fresh)
}

/// The device sign-in on first launch: code and QR on screen until the phone approves, or
/// the viewer chooses Not now (held off for a week).
async fn ensure_signed_in(epoch: u64) {
    let Some((Some(app), token)) = update(epoch, |a| (a.config.trakt.clone(), a.token.clone()))
    else {
        return;
    };
    if token.is_some() || store::sign_in_skipped(now_ms()) {
        return;
    }
    for _attempt in 0..3 {
        let Ok(reply) = net::send(&app.device_code(), 10_000).await else {
            return;
        };
        let Ok(code) = trakt::parse_device_code(&reply.text) else {
            log!("Luma Player: Trakt sign-in answered {}", reply.status);
            return;
        };
        let qr = qr_svg(&code.verification_url);
        let shown = update(epoch, |a| {
            a.screen = Screen::SignIn {
                code: code.user_code.clone(),
                url: code.verification_url.clone(),
                qr,
            };
            a.render();
        });
        if shown.is_none() {
            return;
        }
        let mut interval = code.interval;
        let deadline = now_ms() + code.expires_in * 1000.;
        while now_ms() < deadline {
            dom::sleep((interval * 1000.) as i32).await;
            let skipped = update(epoch, |a| a.sign_in_skipped).unwrap_or(true);
            if skipped {
                return;
            }
            let Ok(reply) = net::send(&app.device_token(&code.device_code), 10_000).await else {
                continue;
            };
            match trakt::poll_reply(reply.status, &reply.text) {
                trakt::Poll::Pending => {}
                trakt::Poll::SlowDown => interval += 5.,
                trakt::Poll::Signed(token) => {
                    store::save_token(Some(&token));
                    update(epoch, |a| {
                        a.token = Some(token);
                        a.screen = Screen::Loading("Signed in to Trakt".into());
                        a.toast("Signed in to Trakt");
                        a.render();
                    });
                    return;
                }
                trakt::Poll::Expired => break,
                trakt::Poll::Denied | trakt::Poll::Failed(_) => {
                    update(epoch, |a| {
                        a.screen = Screen::Loading("Finding the best stream…".into());
                        a.toast("Trakt sign-in didn't finish; playing without it");
                    });
                    return;
                }
            }
        }
    }
}

fn qr_svg(text: &str) -> String {
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(260, 260)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
        Err(_) => String::new(),
    }
}

/// Scrobbles that failed for want of a network last time, sent again.
async fn replay_queue() {
    let queued = store::queue_take();
    if queued.is_empty() {
        return;
    }
    log!("Luma Player: replaying {} queued scrobbles", queued.len());
    for (path, body) in queued {
        let (p, b) = (path.clone(), body.clone());
        match trakt_send(move |app, token| app.replay(token, &p, &b)).await {
            Some(Ok(reply)) if trakt::settled(reply.status) => {}
            Some(Ok(reply)) if reply.status == 401 || reply.status == 403 => {}
            None => {}
            _ => store::queue_push(&path, &body),
        }
    }
}

/// IntroDB's segments for this episode (or film), for Skip Intro, Skip Recap and Next Up.
async fn fetch_segments(epoch: u64) {
    let Some((Some(base), Some(imdb), episode)) = update(epoch, |app| {
        (
            app.config.introdb.clone(),
            app.launch.imdb.clone(),
            app.launch.episode_key(),
        )
    }) else {
        return;
    };
    let reply = net::send(&luma_core::segments::request(&base, &imdb, episode), 8_000).await;
    let segments = match reply {
        Ok(reply) if reply.ok() => luma_core::segments::parse(&reply.text),
        Ok(reply) => {
            log!("Luma Player IntroDB: answered {}", reply.status);
            Segments::default()
        }
        Err(error) => {
            log!("Luma Player IntroDB: {error}");
            Segments::default()
        }
    };
    log!(
        "Luma Player IntroDB: intro {}, recap {}, credits {}",
        segments.intro.is_some(),
        segments.recap.is_some(),
        segments.outro.is_some()
    );
    update(epoch, |app| app.segments = Some(segments));
}

/// Trakt's watched episodes and this show's paused ones, for ticks and bars on the shelf.
async fn load_show_progress(epoch: u64) {
    let Some((Some(imdb), Some(Media::Episode { show, .. }))) =
        update(epoch, |app| (app.launch.imdb.clone(), app.media.clone()))
    else {
        return;
    };
    let watched = trakt_send(move |app, token| app.progress(token, &imdb))
        .await
        .and_then(Result::ok)
        .filter(|reply| reply.ok())
        .map(|reply| trakt::parse_watched(&reply.text))
        .unwrap_or_default();
    let paused = trakt_send(|app, token| app.playback(token, true))
        .await
        .and_then(Result::ok)
        .filter(|reply| reply.ok())
        .map(|reply| trakt::parse_show_playback(&reply.text, &show))
        .unwrap_or_default();
    update(epoch, move |app| {
        app.watched.extend(watched);
        for (episode, percent) in paused {
            app.partial.insert(episode, percent);
        }
        app.render();
    });
}
