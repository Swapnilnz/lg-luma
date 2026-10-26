//! Home rows from Trakt and MDBList whose cards open a title in another app (Nuvio) through a
//! `contentTarget` deep link. All HTTP, decoding and sign-in run in `feeds_service`; this module
//! only holds what Home draws, each row's horizontal scroll, and the remote navigation between
//! rows.
use crate::app_artwork::Artwork;
use crate::state::{
    Control, Direction, Effect, HOME_STRIP_LEFT, HOME_STRIP_RIGHT, Rect, State, Target, TextField,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

/// Portrait poster cards, Apple TV style.
pub const CARD_WIDTH: f32 = 216.;
pub const CARD_HEIGHT: f32 = 324.;
pub const CARD_PITCH: f32 = 240.;
pub const CARD_RADIUS: f32 = 14.4;
/// How long a poster takes to fade in once it arrives.
const REVEAL: Duration = Duration::from_millis(450);
/// Search results: a grid of posters below the query field.
pub const SEARCH_COLUMNS: usize = 7;
pub const SEARCH_TOP: f32 = 296.;
pub const SEARCH_ROW: f32 = CARD_HEIGHT + 40.;
/// Typing pauses this long before the query goes out.
const SEARCH_PAUSE: Duration = Duration::from_millis(350);

/// The open Search overlay.
#[derive(Default)]
pub struct Search {
    pub query: String,
    /// The query last sent to TMDB.
    pub sent: Option<String>,
    pub results: Vec<Item>,
    pub loading: bool,
    pub error: Option<String>,
    /// When the query last changed; None sends it at once.
    pub edited: Option<Instant>,
    pub scroll: f32,
    /// Where focus returns when Search closes.
    pub restore: Option<Target>,
    /// Waiting for LG's speech recognition.
    pub listening: bool,
    /// The voice request whose reply this Search is waiting for.
    pub voice: u64,
}

/// Created by the remote's mic button through LG Input Hook (see feeds.md): Luma then listens.
pub const VOICE_TRIGGER: &str = "/tmp/luma-voice-search";

/// One card: what it shows and the deep link it opens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    /// `contentTarget` for the launch, e.g. `nuvio://detail/series/tt0944947`.
    pub link: String,
    /// Artwork URL, drawn once `Feeds::images` holds it.
    pub image: Option<String>,
    /// Watched fraction, 0 to 1, for a progress bar.
    pub progress: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Loading,
    Ready,
    Error(String),
    /// Trakt device sign-in: open `url` on a phone and enter `code`.
    SignIn {
        url: String,
        code: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub title: String,
    /// Trakt rows keep their artwork in memory wherever Home is scrolled.
    #[serde(default)]
    pub pinned: bool,
    pub status: Status,
    pub items: Vec<Item>,
}

impl Row {
    /// A finished row without items takes no space on Home.
    pub fn shown(&self) -> bool {
        !(self.items.is_empty() && self.status == Status::Ready)
    }
}

#[derive(Default)]
pub struct Feeds {
    /// The app every card launches.
    pub app_id: String,
    pub rows: Vec<Row>,
    /// Horizontal scroll per row, like `State::home_scroll` for favourites.
    pub scroll: Vec<f32>,
    pub images: HashMap<String, Arc<Artwork>>,
    /// Whether Home was on screen at the last tick: returning to it refreshes Trakt rows.
    pub visible: bool,
    /// Rows on or near the screen. Only their artwork is kept, so a long list of rows does
    /// not hold hundreds of images.
    pub near: BTreeSet<usize>,
    /// When each poster arrived, while it is still fading in.
    pub arrived: HashMap<String, Instant>,
    /// The "Keep loaded posters" setting last sent to the worker.
    pub keep: bool,
    pub search: Option<Search>,
    /// Search needs a TMDB key.
    pub can_search: bool,
    /// Remote keys that open Search, from feeds.json.
    pub search_keys: Vec<i32>,
    /// Numbers voice requests, so a late reply never fills a later Search.
    pub voice_requests: u64,
}

impl Feeds {
    /// Artwork URLs Home holds on to: every row's when loaded posters are kept, otherwise
    /// those of the rows near the screen and of pinned rows.
    pub fn wanted_images(&self) -> HashSet<&String> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(index, row)| self.keep || row.pinned || self.near.contains(index))
            .flat_map(|(_, row)| &row.items)
            .chain(self.search.iter().flat_map(|search| &search.results))
            .filter_map(|item| item.image.as_ref())
            .collect()
    }

    /// Drops artwork no nearby row shows.
    pub fn prune_images(&mut self) {
        let wanted: HashSet<String> = self.wanted_images().into_iter().cloned().collect();
        self.images.retain(|url, _| wanted.contains(url));
        self.arrived.retain(|url, _| wanted.contains(url));
    }

    /// A poster finished downloading: it fades in from now.
    pub fn image_arrived(&mut self, url: String, image: Arc<Artwork>, reduced_motion: bool) {
        if !reduced_motion {
            self.arrived.insert(url.clone(), Instant::now());
        }
        self.images.insert(url, image);
    }

    /// How far a poster has faded in, 0 to 1, eased out.
    pub fn reveal(&self, url: &str, reduced_motion: bool) -> f32 {
        if reduced_motion {
            return 1.;
        }
        let Some(arrived) = self.arrived.get(url) else {
            return 1.;
        };
        let t = (arrived.elapsed().as_secs_f32() / REVEAL.as_secs_f32()).clamp(0., 1.);
        1. - (1. - t).powi(3)
    }

    /// Whether a poster is still fading in, so Home keeps redrawing.
    pub fn fading(&self) -> bool {
        self.arrived
            .values()
            .any(|arrived| arrived.elapsed() < REVEAL)
    }

    /// Forgets finished fades. True when one just finished and needs its last frame.
    pub fn finish_fades(&mut self) -> bool {
        let before = self.arrived.len();
        self.arrived.retain(|_, arrived| arrived.elapsed() < REVEAL);
        self.arrived.len() != before
    }

    /// The rows whose cards lie within about two rows of the screen, from the last scene.
    /// The rows on screen, the row just above, and `ahead` rows below, from the last scene.
    pub fn near_rows(&self, controls: &[Control], ahead: usize) -> BTreeSet<usize> {
        let visible: BTreeSet<usize> = controls
            .iter()
            .filter_map(|control| match control.target {
                Target::Feed(row, _)
                    if control.rect.y + control.rect.h > 108.8 && control.rect.y < 1054. =>
                {
                    Some(row)
                }
                _ => None,
            })
            .collect();
        let (Some(&first), Some(&last)) = (visible.first(), visible.last()) else {
            return BTreeSet::new();
        };
        // Count only rows that are drawn: finished empty rows take no space on Home.
        let shown: Vec<usize> = (0..self.rows.len())
            .filter(|row| self.rows[*row].shown())
            .collect();
        let position = |row: usize| shown.iter().position(|shown| *shown == row);
        let (Some(start), Some(end)) = (position(first), position(last)) else {
            return visible;
        };
        shown[start.saturating_sub(1)..(end + 1 + ahead).min(shown.len())]
            .iter()
            .copied()
            .collect()
    }

    fn max_scroll(&self, row: usize) -> f32 {
        let count = self.rows.get(row).map_or(0, |row| row.items.len());
        if count == 0 {
            return 0.;
        }
        let content = (count - 1) as f32 * CARD_PITCH + CARD_WIDTH;
        (content - (HOME_STRIP_RIGHT - HOME_STRIP_LEFT)).max(0.)
    }
}

fn in_strip(rect: Rect) -> bool {
    rect.x >= HOME_STRIP_LEFT - 0.1 && rect.x + rect.w <= HOME_STRIP_RIGHT + 0.1
}

fn distance(control: &Control, x: f32) -> f32 {
    (control.rect.center().0 - x).abs()
}

/// The fully visible card nearest to `x` in the first (or, with `last`, the last) row whose
/// index `rows` accepts.
fn nearest_feed(
    controls: &[Control],
    x: f32,
    rows: impl Fn(usize) -> bool,
    last: bool,
) -> Option<Target> {
    let candidates: Vec<(usize, &Control)> = controls
        .iter()
        .filter_map(|control| match control.target {
            Target::Feed(row, _) if rows(row) && in_strip(control.rect) => Some((row, control)),
            _ => None,
        })
        .collect();
    let row = if last {
        candidates.iter().map(|(row, _)| *row).max()?
    } else {
        candidates.iter().map(|(row, _)| *row).min()?
    };
    candidates
        .iter()
        .filter(|(candidate, _)| *candidate == row)
        .min_by(|a, b| distance(a.1, x).total_cmp(&distance(b.1, x)))
        .map(|(_, control)| control.target.clone())
}

/// The search bar or its mic button, whichever is nearer to `x`.
fn nearest_bar(controls: &[Control], x: f32) -> Option<Target> {
    controls
        .iter()
        .filter(|control| matches!(control.target, Target::FeedSearch | Target::FeedVoice))
        .min_by(|a, b| distance(a, x).total_cmp(&distance(b, x)))
        .map(|control| control.target.clone())
}

fn nearest_favorite(controls: &[Control], x: f32) -> Option<Target> {
    controls
        .iter()
        .filter(|control| matches!(control.target, Target::App(_)) && in_strip(control.rect))
        .min_by(|a, b| distance(a, x).total_cmp(&distance(b, x)))
        .map(|control| control.target.clone())
}

impl State {
    /// Rows from the feed worker. Keeps focus on a card that still exists and drops artwork
    /// no row shows any more.
    pub fn set_feed_rows(&mut self, rows: Vec<Row>) {
        let feeds = &mut self.feeds;
        feeds.rows = rows;
        feeds.scroll.resize(feeds.rows.len(), 0.);
        for row in 0..feeds.rows.len() {
            let max = feeds.max_scroll(row);
            feeds.scroll[row] = feeds.scroll[row].clamp(0., max);
        }
        feeds.prune_images();
        if let Target::Feed(row, index) = self.focus {
            let count = self.feeds.rows.get(row).map_or(0, |row| row.items.len());
            if count == 0 {
                self.focus = self.first_content();
            } else if index >= count {
                self.focus = Target::Feed(row, count - 1);
            }
        }
        self.dirty = true;
    }

    /// Scrolls a row so its focused card sits fully inside the strip.
    pub fn reveal_feed(&mut self, row: usize, rect: Rect) {
        let max = self.feeds.max_scroll(row);
        let Some(scroll) = self.feeds.scroll.get_mut(row) else {
            return;
        };
        if rect.x < HOME_STRIP_LEFT {
            *scroll += rect.x - HOME_STRIP_LEFT;
        } else if rect.x + rect.w > HOME_STRIP_RIGHT {
            *scroll += rect.x + rect.w - HOME_STRIP_RIGHT;
        }
        *scroll = scroll.clamp(0., max);
    }

    /// Remote navigation into, within and out of the feed rows on Home. `None` leaves the move
    /// to the regular routes (favourites to inputs, spatial search).
    pub fn feed_direction(
        &mut self,
        direction: Direction,
        controls: &[Control],
    ) -> Option<Vec<Effect>> {
        let x = controls
            .iter()
            .find(|control| control.target == self.focus)?
            .rect
            .center()
            .0;
        match self.focus.clone() {
            Target::App(_) if matches!(direction, Direction::Down) => {
                let next = nearest_bar(controls, x)
                    .or_else(|| nearest_feed(controls, x, |_| true, false))?;
                self.focus(next, controls);
                Some(vec![])
            }
            bar @ (Target::FeedSearch | Target::FeedVoice) => {
                let next = match direction {
                    Direction::Up => nearest_favorite(controls, x)?,
                    Direction::Down => nearest_feed(controls, x, |_| true, false)?,
                    Direction::Left if bar == Target::FeedVoice => Target::FeedSearch,
                    Direction::Right
                        if bar == Target::FeedSearch
                            && controls.iter().any(|c| c.target == Target::FeedVoice) =>
                    {
                        Target::FeedVoice
                    }
                    _ => return Some(vec![]),
                };
                self.focus(next, controls);
                Some(vec![])
            }
            Target::Feed(row, index) => {
                let next = match direction {
                    Direction::Left | Direction::Right => {
                        let count = self.feeds.rows.get(row).map_or(0, |row| row.items.len());
                        if count == 0 {
                            return Some(vec![]);
                        }
                        let next = if matches!(direction, Direction::Left) {
                            index.saturating_sub(1)
                        } else {
                            (index + 1).min(count - 1)
                        };
                        Target::Feed(row, next)
                    }
                    Direction::Up => nearest_feed(controls, x, |other| other < row, true)
                        .or_else(|| nearest_bar(controls, x))
                        .or_else(|| nearest_favorite(controls, x))?,
                    Direction::Down => nearest_feed(controls, x, |other| other > row, false)?,
                };
                self.focus(next, controls);
                Some(vec![])
            }
            _ => None,
        }
    }
}

impl State {
    /// Opens Search with the keyboard ready. Needs a TMDB key.
    pub fn open_search(&mut self) -> Vec<Effect> {
        if self.overlay.is_some() || self.recents_open() || self.feeds.search.is_some() {
            return vec![];
        }
        if !self.feeds.can_search {
            self.notify("Add a TMDB api_key to feeds.json to search");
            return vec![];
        }
        self.moving = None;
        self.feeds.search = Some(Search {
            restore: Some(self.focus.clone()),
            ..Search::default()
        });
        self.focus = Target::SearchField;
        self.open_keyboard(TextField::Search);
        self.dirty = true;
        vec![]
    }

    pub fn close_search(&mut self) {
        let restore = self.feeds.search.take().and_then(|search| search.restore);
        self.feeds.prune_images();
        self.focus = restore.unwrap_or(Target::FeedSearch);
        self.reconcile_focus();
        self.pointer = false;
        self.dirty = true;
    }

    /// The query to send now, once typing has paused.
    pub fn search_tick(&mut self) -> Option<String> {
        let search = self.feeds.search.as_mut()?;
        if search.sent.as_deref() == Some(search.query.as_str())
            || search
                .edited
                .is_some_and(|edited| edited.elapsed() < SEARCH_PAUSE)
        {
            return None;
        }
        search.sent = Some(search.query.clone());
        search.loading = !search.query.trim().is_empty();
        search.error = None;
        self.dirty = true;
        Some(search.query.clone())
    }

    pub fn search_results(&mut self, query: String, result: Result<Vec<Item>, String>) {
        let Some(search) = self.feeds.search.as_mut() else {
            return;
        };
        if search.sent.as_deref() != Some(query.as_str()) {
            return;
        }
        search.loading = false;
        match result {
            Ok(results) => {
                search.results = results;
                search.error = None;
            }
            Err(error) => {
                search.results.clear();
                search.error = Some(error);
            }
        }
        search.scroll = 0.;
        // New results replace the grid: focus goes back to the query field above it.
        if matches!(self.focus, Target::SearchResult(_)) {
            self.focus = Target::SearchField;
        }
        // Posters of earlier queries are no longer shown.
        self.feeds.prune_images();
        self.dirty = true;
    }

    /// Remote navigation inside Search: the query field above a grid of posters.
    pub fn search_direction(&mut self, direction: Direction) -> Vec<Effect> {
        let Some(search) = self.feeds.search.as_mut() else {
            return vec![];
        };
        let count = search.results.len();
        let next = match (self.focus.clone(), direction) {
            (Target::SearchResult(index), Direction::Left) if index % SEARCH_COLUMNS > 0 => {
                Target::SearchResult(index - 1)
            }
            (Target::SearchResult(index), Direction::Right)
                if (index + 1) % SEARCH_COLUMNS > 0 && index + 1 < count =>
            {
                Target::SearchResult(index + 1)
            }
            (Target::SearchResult(index), Direction::Up) if index >= SEARCH_COLUMNS => {
                Target::SearchResult(index - SEARCH_COLUMNS)
            }
            (Target::SearchResult(index), Direction::Up)
                if index % SEARCH_COLUMNS == SEARCH_COLUMNS - 1 =>
            {
                Target::SearchVoice
            }
            (Target::SearchResult(_), Direction::Up) => Target::SearchField,
            (Target::SearchResult(index), Direction::Down)
                if (index / SEARCH_COLUMNS + 1) * SEARCH_COLUMNS < count =>
            {
                Target::SearchResult((index + SEARCH_COLUMNS).min(count - 1))
            }
            (Target::SearchField, Direction::Down) if count > 0 => Target::SearchResult(0),
            (Target::SearchField, Direction::Right) => Target::SearchVoice,
            (Target::SearchVoice, Direction::Left) => Target::SearchField,
            (Target::SearchVoice, Direction::Down) if count > 0 => {
                Target::SearchResult((SEARCH_COLUMNS - 1).min(count - 1))
            }
            (Target::SearchResult(_) | Target::SearchField | Target::SearchVoice, _) => {
                self.focus.clone()
            }
            _ => Target::SearchField,
        };
        // Keep the focused row on screen: it stays the second visible row once below the first.
        if let Target::SearchResult(index) = next {
            search.scroll = (index / SEARCH_COLUMNS).saturating_sub(1) as f32 * SEARCH_ROW;
        } else {
            search.scroll = 0.;
        }
        self.focus = next;
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// Opens Search listening through the remote's microphone, then searches what was heard.
    pub fn start_voice(&mut self) -> Vec<Effect> {
        if self.feeds.search.is_none() {
            let effects = self.open_search();
            if self.feeds.search.is_none() {
                return effects;
            }
        }
        // Voice replaces typing: put the TV keyboard away.
        if self.text_input_active() {
            self.overlay = None;
            self.panel_started = None;
        }
        let Some(search) = self.feeds.search.as_mut() else {
            return vec![];
        };
        if search.listening {
            return vec![];
        }
        self.feeds.voice_requests += 1;
        let id = self.feeds.voice_requests;
        search.listening = true;
        search.voice = id;
        search.error = None;
        self.focus = Target::SearchVoice;
        self.dirty = true;
        vec![Effect::VoiceSearch(id)]
    }

    /// What the microphone heard becomes the query, searched at once.
    pub fn voice_result(&mut self, id: u64, result: Result<String, String>) {
        let Some(search) = self.feeds.search.as_mut() else {
            return;
        };
        if !search.listening || search.voice != id {
            return;
        }
        search.listening = false;
        match result {
            Ok(text) if !text.is_empty() => {
                search.query = text.chars().take(80).collect();
                search.edited = None;
                search.error = None;
            }
            Ok(_) => search.error = Some("Didn't catch that. Press the mic to try again.".into()),
            Err(error) => search.error = Some(format!("Voice search failed: {error}")),
        }
        self.dirty = true;
    }

    /// OK inside Search. None when Search is closed.
    pub fn search_activate(&mut self) -> Option<Vec<Effect>> {
        let search = self.feeds.search.as_ref()?;
        Some(match self.focus.clone() {
            Target::SearchField => {
                self.open_keyboard(TextField::Search);
                vec![]
            }
            Target::SearchVoice => self.start_voice(),
            Target::SearchResult(index) if !self.launching => search
                .results
                .get(index)
                .map(|item| vec![Effect::LaunchTitle(item.link.clone())])
                .unwrap_or_default(),
            _ => vec![],
        })
    }
}
