//! Home rows from Trakt and MDBList whose cards open a title in another app (Nuvio) through a
//! `contentTarget` deep link. All HTTP, decoding and sign-in run in `feeds_service`; this module
//! only holds what Home draws, each row's horizontal scroll, and the remote navigation between
//! rows.
use crate::app_artwork::Artwork;
use crate::state::{
    Control, Direction, Effect, HOME_STRIP_LEFT, HOME_STRIP_RIGHT, Page, Rect, State, Target,
    TextField,
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
    /// When Search opened, for its entrance, and when listening began, for the mic's pulse.
    pub opened: Option<Instant>,
    pub listening_since: Option<Instant>,
}

/// How long Search takes to rise into place.
pub const SEARCH_ENTRANCE: Duration = Duration::from_millis(400);
/// The focused poster grows by this much, over this long.
pub const EXPAND_SCALE: f32 = 0.08;
const EXPAND: Duration = Duration::from_millis(180);
/// The preview fades in and out over these; its backdrop drifts for KEN_BURNS.
const PREVIEW_IN: Duration = Duration::from_millis(550);
const PREVIEW_OUT: Duration = Duration::from_millis(300);
pub const KEN_BURNS: Duration = Duration::from_secs(20);
/// Where the previewed row's posters settle on screen, below the title's details.
const PREVIEW_CARD_Y: f32 = 640.;
/// A title's details are fetched once focus has rested this long, so scrolling past is free.
const DETAILS_DWELL: Duration = Duration::from_millis(200);

/// What a preview shows about a title, from TMDB.
#[derive(Clone, Debug, Default)]
pub struct Details {
    pub title: String,
    pub overview: String,
    pub year: String,
    pub rating: Option<f32>,
    /// Runtime, or number of seasons.
    pub extent: Option<String>,
    pub genres: Vec<String>,
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    /// A YouTube video ID.
    pub trailer: Option<String>,
}

/// The title Home previews: backdrop, logo and details behind and above its row.
pub struct Preview {
    pub link: String,
    pub target: Target,
    pub shown: Instant,
    pub hiding: Option<Instant>,
    /// Home's scroll before the preview moved the row into place, for a smooth slide.
    pub scroll_from: f32,
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
    /// The episode a card stands for (season, number): the paused or next one, or one airing.
    #[serde(default)]
    pub episode: Option<[u32; 2]>,
    /// A short label drawn on the poster's corner, such as "Tonight" or "New season".
    #[serde(default)]
    pub badge: Option<String>,
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
    /// Its titles also take turns in the Top Shelf banner.
    #[serde(default)]
    pub hero: bool,
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
    /// The focused control and since when, for the poster's growth and the preview's delay.
    pub focus_since: Option<(Target, Instant)>,
    pub details: HashMap<String, Details>,
    /// The title whose details were last asked for.
    pub requested: Option<String>,
    pub preview: Option<Preview>,
    /// Trakt is configured, so cards offer Watchlist, Mark watched and Rate.
    pub trakt: bool,
    /// The Top Shelf banner's slides, while it has titles.
    pub hero: Option<Slides>,
    /// The screensaver, while it runs.
    pub ambient: Option<Ambient>,
    /// A title's full page, over Home or Search.
    pub title: Option<TitlePage>,
    /// The quick-actions menu of a card.
    pub menu: Option<Menu>,
    /// What Trakt says about titles lately opened in a menu or page.
    pub trakt_titles: HashMap<String, TraktTitle>,
    /// Titles last sent for their backdrop and logo (Top Shelf and screensaver slides).
    pub prefetched: Vec<String>,
    /// Requests for the feed workers, sent by `main::feeds_tick`.
    pub outbox: Vec<Outgoing>,
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
            // The backdrop and logo of the title in focus, previewed, on a slide or a page.
            .chain(
                self.held_links()
                    .into_iter()
                    .filter_map(|link| self.details.get(link))
                    .flat_map(|details| details.backdrop.iter().chain(details.logo.iter())),
            )
            .chain(self.title.iter().flat_map(TitlePage::images))
            .collect()
    }

    /// Titles whose details and artwork stay in memory: the one focus rests on, the preview,
    /// the Top Shelf's and screensaver's current and next slides, and the open title page.
    pub fn held_links(&self) -> Vec<&String> {
        let mut links: Vec<&String> = [
            self.requested.as_ref(),
            self.preview.as_ref().map(|preview| &preview.link),
            self.title.as_ref().map(|page| &page.link),
        ]
        .into_iter()
        .flatten()
        .collect();
        links.extend(self.slide_links());
        links
    }

    /// The current and next slides of the Top Shelf and the screensaver.
    pub fn slide_links(&self) -> Vec<&String> {
        let mut links = Vec::new();
        if let Some(slides) = &self.hero {
            let items = self.hero_items();
            if !items.is_empty() {
                for offset in [0, 1] {
                    let (row, index) = items[(slides.index + offset) % items.len()];
                    if let Some(item) = self.rows.get(row).and_then(|row| row.items.get(index)) {
                        links.push(&item.link);
                    }
                }
            }
        }
        if let Some(ambient) = &self.ambient
            && !ambient.items.is_empty()
        {
            for offset in [0, 1] {
                let item = &ambient.items[(ambient.slides.index + offset) % ambient.items.len()];
                links.push(&item.link);
            }
        }
        links
    }

    /// The Top Shelf's titles: from rows marked for it, one card per title, at most eight.
    pub fn hero_items(&self) -> Vec<(usize, usize)> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for (row_index, row) in self.rows.iter().enumerate().filter(|(_, row)| row.hero) {
            for (index, item) in row.items.iter().enumerate() {
                if items.len() < HERO_LIMIT && seen.insert(item.link.as_str()) {
                    items.push((row_index, index));
                }
            }
        }
        items
    }

    /// The Top Shelf's current title.
    pub fn hero_item(&self) -> Option<&Item> {
        let slides = self.hero.as_ref()?;
        let items = self.hero_items();
        let (row, index) = *items.get(slides.index % items.len().max(1))?;
        self.rows.get(row)?.items.get(index)
    }

    /// The screensaver's titles: every row's, taken in turn, one card per title.
    fn ambient_items(&self) -> Vec<Item> {
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        let longest = self
            .rows
            .iter()
            .map(|row| row.items.len())
            .max()
            .unwrap_or(0);
        for index in 0..longest {
            for row in &self.rows {
                if let Some(item) = row.items.get(index)
                    && items.len() < AMBIENT_LIMIT
                    && seen.insert(item.link.clone())
                {
                    items.push(item.clone());
                }
            }
        }
        items
    }

    /// Slides whose backdrop and logo are not in memory yet, for the artwork worker.
    pub fn prefetch_wanted(&self) -> Vec<String> {
        self.slide_links()
            .into_iter()
            .filter(|link| !self.details.contains_key(*link) || !self.artwork_loaded(link))
            .cloned()
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

    /// Whether Search is rising into place or listening, so it keeps redrawing.
    pub fn search_animating(&self) -> bool {
        self.search.as_ref().is_some_and(|search| {
            search.listening
                || search.opened.is_some_and(|opened| {
                    opened.elapsed() < SEARCH_ENTRANCE + Duration::from_millis(50)
                })
        })
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
        let hero = || {
            controls
                .iter()
                .any(|control| control.target == Target::Hero)
                .then_some(Target::Hero)
        };
        match self.focus.clone() {
            Target::App(_) if matches!(direction, Direction::Down) => {
                let next = nearest_bar(controls, x)
                    .or_else(hero)
                    .or_else(|| nearest_feed(controls, x, |_| true, false))?;
                self.focus(next, controls);
                Some(vec![])
            }
            Target::Hero => {
                match direction {
                    Direction::Left | Direction::Right => {
                        self.hero_step(matches!(direction, Direction::Right));
                    }
                    Direction::Up => {
                        let next = nearest_bar(controls, x)
                            .or_else(|| nearest_favorite(controls, 960.))?;
                        self.focus(next, controls);
                    }
                    Direction::Down => {
                        let next = nearest_feed(controls, 200., |_| true, false)?;
                        self.focus(next, controls);
                    }
                }
                Some(vec![])
            }
            bar @ (Target::FeedSearch | Target::FeedVoice) => {
                let next = match direction {
                    Direction::Up => nearest_favorite(controls, x)?,
                    Direction::Down => {
                        hero().or_else(|| nearest_feed(controls, x, |_| true, false))?
                    }
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
                        .or_else(hero)
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
            opened: Some(Instant::now()),
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
        self.focus = match restore {
            // The bar may be hidden from Home (Settings): fall back to the first card.
            Some(Target::FeedSearch | Target::FeedVoice) | None
                if !self.preferences.feed_search_bar =>
            {
                self.first_content()
            }
            Some(target) => target,
            None => Target::FeedSearch,
        };
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
        search.listening_since = Some(Instant::now());
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
            Target::SearchResult(index) if !self.launching => {
                let link = search.results.get(index).map(|item| item.link.clone());
                match link {
                    // Search is done once a title opens: Home is back as it was on return.
                    Some(link) => {
                        self.close_search();
                        vec![Effect::LaunchTitle(link)]
                    }
                    None => vec![],
                }
            }
            _ => vec![],
        })
    }
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

/// The title link of a card, in a feed row or in Search's results.
fn card_link(feeds: &Feeds, target: &Target) -> Option<String> {
    card_item(feeds, target).map(|item| item.link.clone())
}

/// The card behind a focus target: a feed row's, Search's, the Top Shelf's, a title page's
/// (the page's own title, or one of More Like This).
fn card_item<'a>(feeds: &'a Feeds, target: &Target) -> Option<&'a Item> {
    match target {
        Target::Feed(row, index) => feeds.rows.get(*row)?.items.get(*index),
        Target::SearchResult(index) => feeds.search.as_ref()?.results.get(*index),
        Target::Hero => feeds.hero_item(),
        Target::TitleMore(index) => feeds.title.as_ref()?.more()?.get(*index),
        Target::TitleButton(_)
        | Target::TitleSeason(_)
        | Target::TitleEpisode(_)
        | Target::TitleCast(_) => feeds.title.as_ref().map(|page| &page.item),
        _ => None,
    }
}

impl Feeds {
    /// How much bigger `target` is drawn: it grows when focused.
    pub fn scale(&self, target: &Target, reduced_motion: bool) -> f32 {
        match &self.focus_since {
            Some((focused, since)) if focused == target => {
                let t = if reduced_motion {
                    1.
                } else {
                    since.elapsed().as_secs_f32() / EXPAND.as_secs_f32()
                };
                1. + EXPAND_SCALE * ease_out(t)
            }
            _ => 1.,
        }
    }

    /// How far the preview has faded in, 0 to 1.
    pub fn preview_progress(&self, reduced_motion: bool) -> f32 {
        let Some(preview) = &self.preview else {
            return 0.;
        };
        if reduced_motion {
            return if preview.hiding.is_some() { 0. } else { 1. };
        }
        match preview.hiding {
            Some(hiding) => {
                1. - ease_out(hiding.elapsed().as_secs_f32() / PREVIEW_OUT.as_secs_f32())
            }
            None => ease_out(preview.shown.elapsed().as_secs_f32() / PREVIEW_IN.as_secs_f32()),
        }
    }

    /// Whether the focused poster or the preview is still moving.
    pub fn card_animating(&self) -> bool {
        self.focus_since
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed() < EXPAND + Duration::from_millis(40))
            || self.preview.as_ref().is_some_and(|preview| {
                preview.hiding.is_some() || preview.shown.elapsed() < KEN_BURNS
            })
    }

    pub fn details_arrived(&mut self, link: String, result: Result<Details, String>) {
        // A failed lookup is not kept: focus resting on the title again tries once more.
        let Ok(details) = result else {
            return;
        };
        if self.details.len() > 80 {
            let keep: HashSet<String> = self.held_links().into_iter().cloned().collect();
            self.details.retain(|known, _| keep.contains(known));
        }
        self.details.insert(link, details);
    }

    /// Whether a title's backdrop and logo are in memory (or it has none).
    fn artwork_loaded(&self, link: &str) -> bool {
        self.details.get(link).is_none_or(|details| {
            [&details.backdrop, &details.logo]
                .into_iter()
                .flatten()
                .all(|url| self.images.contains_key(url))
        })
    }
}

impl State {
    /// Follows focus for the poster's growth and the preview: asks for the focused title's
    /// details once focus rests on it, and shows its preview after the configured delay.
    /// Returns a title link whose details should be fetched now.
    pub fn preview_tick(&mut self, controls: &[Control]) -> Option<String> {
        let now = Instant::now();
        if self.feeds.focus_since.as_ref().map(|(target, _)| target) != Some(&self.focus) {
            self.feeds.focus_since = Some((self.focus.clone(), now));
            // Each rest on a title may ask again, e.g. for artwork freed since.
            self.feeds.requested = None;
            self.dirty = true;
        }
        let since = self
            .feeds
            .focus_since
            .as_ref()
            .map_or(now, |(_, since)| *since);
        // "Immediately" still waits a moment, so scrolling along a row does not flash previews.
        let delay = match self.preferences.feed_preview_delay {
            0 => None,
            1 => Some(Duration::from_millis(250)),
            seconds => Some(Duration::from_secs(u64::from(seconds.min(6) - 1))),
        };
        let previewable = self.page == Page::Home
            && self.overlay.is_none()
            && self.feeds.search.is_none()
            && self.feeds.title.is_none()
            && self.feeds.menu.is_none()
            && self.feeds.ambient.is_none()
            && self.moving.is_none()
            && delay.is_some()
            && matches!(self.focus, Target::Feed(..));
        let drawn_scroll = self.drawn_scroll();
        let focused_link = card_link(&self.feeds, &self.focus);
        if let Some(preview) = &mut self.feeds.preview {
            if preview.hiding.is_none()
                && (!previewable
                    || preview.target != self.focus
                    || focused_link.as_deref() != Some(preview.link.as_str()))
            {
                preview.hiding = Some(now);
                // Keep Home where it is drawn now: no jump if focus moved mid-slide.
                self.scroll = drawn_scroll;
                self.dirty = true;
            }
            if preview
                .hiding
                .is_some_and(|hiding| hiding.elapsed() >= PREVIEW_OUT)
            {
                self.feeds.preview = None;
                self.dirty = true;
            }
        }
        let link = card_link(&self.feeds, &self.focus)?;
        let mut request = None;
        // Ask once focus rests: for details not known yet, or whose artwork has since gone.
        if since.elapsed() >= DETAILS_DWELL
            && (!self.feeds.details.contains_key(&link) || !self.feeds.artwork_loaded(&link))
            && self.feeds.requested.as_deref() != Some(link.as_str())
        {
            self.feeds.requested = Some(link.clone());
            request = Some(link.clone());
        }
        let showing = self
            .feeds
            .preview
            .as_ref()
            .is_some_and(|preview| preview.hiding.is_none());
        if previewable
            && !showing
            && delay.is_some_and(|delay| since.elapsed() >= delay)
            && self.feeds.details.contains_key(&link)
            && let Some(control) = controls.iter().find(|control| control.target == self.focus)
        {
            // Slide the row down to sit below the title's details; the rest of Home fades.
            let scroll_from = self.scroll;
            self.scroll = (self.scroll + control.rect.y - PREVIEW_CARD_Y).max(0.);
            self.feeds.preview = Some(Preview {
                link,
                target: self.focus.clone(),
                shown: now,
                hiding: None,
                scroll_from,
            });
            self.dirty = true;
        }
        request
    }

    /// Home's scroll as drawn: the row slides into place while the preview fades in.
    pub fn drawn_scroll(&self) -> f32 {
        match &self.feeds.preview {
            Some(preview) if preview.hiding.is_none() => {
                let progress = self.feeds.preview_progress(self.preferences.reduced_motion);
                preview.scroll_from + (self.scroll - preview.scroll_from) * progress
            }
            _ => self.scroll,
        }
    }

    /// Play, or holding OK, on a card: its trailer in LG's YouTube app. None when focus is
    /// not on a card.
    pub fn play_trailer(&mut self) -> Option<Vec<Effect>> {
        let link = card_link(&self.feeds, &self.focus)?;
        let trailer = self
            .feeds
            .details
            .get(&link)
            .map(|details| details.trailer.clone());
        match trailer {
            Some(Some(key)) => return Some(vec![Effect::Trailer(key)]),
            Some(None) => self.notify("No trailer for this title"),
            None => self.notify("Finding the trailer... try again in a moment"),
        }
        Some(vec![])
    }
}

// ---- Top Shelf, screensaver, title pages and quick actions ----

/// The Top Shelf: a full-width banner above the rows. Each slide shows this long, then
/// crossfades to the next over HERO_FADE.
pub const HERO_HEIGHT: f32 = 560.;
const HERO_LIMIT: usize = 8;
const HERO_SLIDE: Duration = Duration::from_secs(9);
pub const HERO_FADE: Duration = Duration::from_millis(700);
/// The screensaver's slides: each shows this long and drifts while it does.
const AMBIENT_LIMIT: usize = 40;
pub const AMBIENT_SLIDE: Duration = Duration::from_secs(12);
pub const AMBIENT_FADE: Duration = Duration::from_millis(1400);
/// After this long the screensaver lets LG's own screen saver take over again.
pub const AMBIENT_MAX: Duration = Duration::from_secs(60 * 60);
/// The title page's and the menu's entrance.
pub const PAGE_ENTRANCE: Duration = Duration::from_millis(320);

/// Title page layout, shared by drawing and remote navigation.
pub const TITLE_LEFT: f32 = 84.;
pub const TITLE_RIGHT: f32 = 1836.;
pub const TITLE_BUTTONS_Y: f32 = 600.;
pub const TITLE_BUTTON_HEIGHT: f32 = 64.;
pub const SEASON_HEIGHT: f32 = 52.;
pub const SEASON_WIDTH: f32 = 176.;
pub const SEASON_PITCH: f32 = 192.;
pub const EPISODE_WIDTH: f32 = 384.;
pub const EPISODE_HEIGHT: f32 = 216.;
pub const EPISODE_PITCH: f32 = 408.;
pub const EPISODE_CAPTION: f32 = 70.;
pub const CAST_WIDTH: f32 = 150.;
pub const CAST_HEIGHT: f32 = 225.;
pub const CAST_PITCH: f32 = 174.;
pub const CAST_CAPTION: f32 = 62.;
pub const SECTION_HEADER: f32 = 50.;
const SECTION_GAP: f32 = 56.;

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

/// Slides that take turns: the current one, and the one it is crossfading from.
#[derive(Clone, Debug)]
pub struct Slides {
    pub index: usize,
    pub since: Instant,
    pub from: Option<usize>,
}

impl Slides {
    fn new() -> Self {
        Self {
            index: 0,
            since: Instant::now(),
            from: None,
        }
    }

    fn step(&mut self, count: usize, forward: bool) {
        if count == 0 {
            return;
        }
        self.from = Some(self.index);
        self.index = if forward {
            (self.index + 1) % count
        } else {
            (self.index + count - 1) % count
        };
        self.since = Instant::now();
    }

    /// How far the current slide has faded in over the one before, 0 to 1.
    pub fn fade(&self, duration: Duration, reduced_motion: bool) -> f32 {
        if reduced_motion || self.from.is_none() {
            return 1.;
        }
        ease_in_out(self.since.elapsed().as_secs_f32() / duration.as_secs_f32())
    }

    fn fading(&self, duration: Duration) -> bool {
        self.from.is_some() && self.since.elapsed() < duration + Duration::from_millis(40)
    }
}

/// The screensaver: its titles, taken when it started, and which one shows.
pub struct Ambient {
    pub items: Vec<Item>,
    pub slides: Slides,
    pub started: Instant,
}

/// A person in a title's cast.
#[derive(Clone, Debug, Default)]
pub struct Person {
    pub name: String,
    pub role: String,
    pub image: Option<String>,
}

/// A season of a show, as the title page lists it.
#[derive(Clone, Debug, Default)]
pub struct Season {
    pub number: u32,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Episode {
    pub season: u32,
    pub number: u32,
    pub name: String,
    pub runtime: Option<u32>,
    pub still: Option<String>,
    /// Air date, YYYY-MM-DD.
    pub aired: String,
}

/// Everything a title page shows beyond a preview's details, from TMDB and MDBList.
#[derive(Clone, Debug, Default)]
pub struct TitleInfo {
    pub details: Details,
    pub series: bool,
    /// Australian classification (M, MA15+...), else the US one.
    pub certification: String,
    /// "IMDb 8.1", "RT 93%" and the like.
    pub ratings: Vec<String>,
    /// "Directed by ..." or "Created by ...".
    pub credit: String,
    pub cast: Vec<Person>,
    pub seasons: Vec<Season>,
    pub more: Vec<Item>,
}

/// What Trakt knows about a title for this account.
#[derive(Clone, Debug, Default)]
pub struct TraktTitle {
    pub in_watchlist: bool,
    pub watched: bool,
    /// Watched episodes of a show.
    pub episodes: HashSet<[u32; 2]>,
    /// A show's next unwatched episode.
    pub next: Option<[u32; 2]>,
}

/// A change made on Trakt from a menu or title page.
#[derive(Clone, Debug, PartialEq)]
pub enum TraktAction {
    WatchlistAdd,
    WatchlistRemove,
    /// A movie, watched now.
    Watched,
    /// One episode of a show, watched now.
    Episode(u32, u32),
    /// 1 to 10.
    Rate(u8),
    /// Keep a show out of Up Next.
    HideUpNext,
}

/// Requests for the feed workers, queued by the UI and sent each frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Title(String),
    Season(String, u32),
    TraktState(String),
    Trakt(String, TraktAction),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TitleButton {
    Play,
    Trailer,
    Watchlist,
    Watched,
    Rate,
    HideUpNext,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Section {
    Buttons,
    Seasons,
    Episodes,
    Cast,
    More,
}

/// A title's full page.
pub struct TitlePage {
    pub link: String,
    /// The card it opened from: its title, caption and episode.
    pub item: Item,
    pub info: Option<Result<TitleInfo, String>>,
    /// Episodes of each season looked at.
    pub episodes: HashMap<u32, Result<Vec<Episode>, String>>,
    /// The season shown, as an index into `TitleInfo::seasons`.
    pub season: usize,
    /// Whether the season has been chosen from the remote, so arriving data does not move it.
    pub season_chosen: bool,
    /// Horizontal scroll of the seasons, episodes, cast and More Like This rows.
    pub scroll: [f32; 4],
    /// Vertical scroll of the page.
    pub page_scroll: f32,
    pub restore: Target,
    pub opened: Instant,
    /// Pages this one was opened over from More Like This; Back returns to them.
    pub back: Vec<Item>,
}

impl TitlePage {
    pub fn info(&self) -> Option<&TitleInfo> {
        self.info.as_ref()?.as_ref().ok()
    }

    pub fn series(&self) -> bool {
        self.info()
            .map_or(self.link.contains("/series/"), |info| info.series)
    }

    pub fn more(&self) -> Option<&Vec<Item>> {
        self.info().map(|info| &info.more)
    }

    pub fn cast(&self) -> &[Person] {
        self.info().map_or(&[][..], |info| info.cast.as_slice())
    }

    pub fn seasons(&self) -> &[Season] {
        self.info().map_or(&[][..], |info| info.seasons.as_slice())
    }

    /// The season shown and its episodes, once loaded.
    pub fn shown_episodes(&self) -> Option<&Vec<Episode>> {
        let number = self.seasons().get(self.season)?.number;
        self.episodes.get(&number)?.as_ref().ok()
    }

    pub fn buttons(&self, trakt: bool) -> Vec<TitleButton> {
        let mut buttons = vec![TitleButton::Play];
        if self
            .info()
            .is_some_and(|info| info.details.trailer.is_some())
        {
            buttons.push(TitleButton::Trailer);
        }
        if trakt {
            buttons.extend([
                TitleButton::Watchlist,
                TitleButton::Watched,
                TitleButton::Rate,
            ]);
            if self.series() {
                buttons.push(TitleButton::HideUpNext);
            }
        }
        buttons
    }

    /// The page's sections from top to bottom: each with its top and height, unscrolled.
    pub fn sections(&self) -> Vec<(Section, f32, f32)> {
        let mut sections = vec![(Section::Buttons, TITLE_BUTTONS_Y, TITLE_BUTTON_HEIGHT)];
        let mut y = TITLE_BUTTONS_Y + TITLE_BUTTON_HEIGHT + SECTION_GAP;
        if self.series() && !self.seasons().is_empty() {
            sections.push((Section::Seasons, y, SEASON_HEIGHT));
            y += SEASON_HEIGHT + 28.;
            sections.push((Section::Episodes, y, EPISODE_HEIGHT + EPISODE_CAPTION));
            y += EPISODE_HEIGHT + EPISODE_CAPTION + SECTION_GAP;
        }
        if !self.cast().is_empty() {
            let height = SECTION_HEADER + CAST_HEIGHT + CAST_CAPTION;
            sections.push((Section::Cast, y, height));
            y += height + SECTION_GAP;
        }
        if self.more().is_some_and(|more| !more.is_empty()) {
            sections.push((Section::More, y, SECTION_HEADER + CARD_HEIGHT));
        }
        sections
    }

    fn count(&self, section: Section, trakt: bool) -> usize {
        match section {
            Section::Buttons => self.buttons(trakt).len(),
            Section::Seasons => self.seasons().len(),
            Section::Episodes => self.shown_episodes().map_or(0, Vec::len),
            Section::Cast => self.cast().len(),
            Section::More => self.more().map_or(0, Vec::len),
        }
    }

    /// Artwork the page shows: cast photos, episode stills, More Like This posters.
    fn images(&self) -> Vec<&String> {
        let mut images: Vec<&String> = self
            .cast()
            .iter()
            .filter_map(|person| person.image.as_ref())
            .collect();
        images.extend(
            self.shown_episodes()
                .into_iter()
                .flatten()
                .filter_map(|episode| episode.still.as_ref()),
        );
        images.extend(
            self.more()
                .into_iter()
                .flatten()
                .filter_map(|item| item.image.as_ref()),
        );
        images
    }

    /// Scrolls a row so `index` sits fully inside the page's width.
    fn reveal_column(&mut self, row: usize, index: usize, pitch: f32, width: f32) {
        let x = index as f32 * pitch;
        let visible = TITLE_RIGHT - TITLE_LEFT;
        let scroll = &mut self.scroll[row];
        if x < *scroll {
            *scroll = x;
        } else if x + width > *scroll + visible {
            *scroll = x + width - visible;
        }
    }

    /// Scrolls the page so `section` is on screen, keeping the buttons' header in view at top.
    fn reveal_section(&mut self, section: Section) {
        let sections = self.sections();
        let find = |wanted: Section| sections.iter().find(|(kind, ..)| *kind == wanted);
        let Some(&(_, top, height)) = find(section) else {
            return;
        };
        // Season pills come with the episodes below them.
        let bottom = match (section, find(Section::Episodes)) {
            (Section::Seasons, Some(&(_, top, height))) => top + height,
            _ => top + height,
        };
        self.page_scroll = match section {
            Section::Buttons => 0.,
            _ => (bottom + 60. - 1080.).max(0.).min((top - 130.).max(0.)),
        };
    }

    /// The episode to land on in the shown season: the next unwatched one, else the first.
    fn landing_episode(&self, trakt: Option<&TraktTitle>) -> usize {
        let Some(episodes) = self.shown_episodes() else {
            return 0;
        };
        let next = trakt.and_then(|trakt| trakt.next).or(self.item.episode);
        next.and_then(|[season, number]| {
            episodes
                .iter()
                .position(|episode| episode.season == season && episode.number == number)
        })
        .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuEntry {
    Details,
    Trailer,
    Watchlist,
    Watched,
    Rate,
    RateValue(u8),
    HideUpNext,
}

/// A card's quick actions, opened by holding OK.
pub struct Menu {
    pub link: String,
    pub item: Item,
    pub entries: Vec<MenuEntry>,
    pub restore: Target,
    pub opened: Instant,
}

/// Trakt's ratings offered from a menu: its 10-point scale, in five steps.
pub const RATINGS: [(u8, &str); 5] = [
    (10, "Loved it"),
    (8, "Liked it"),
    (6, "It was OK"),
    (4, "Didn't like it"),
    (2, "Hated it"),
];

fn link_series(link: &str) -> bool {
    link.contains("/series/")
}

/// "S2, E4".
pub fn episode_label([season, number]: [u32; 2]) -> String {
    format!("S{season}, E{number}")
}

impl Feeds {
    /// Whether the Top Shelf, screensaver, title page or menu is still moving.
    pub fn extras_animating(&self, reduced_motion: bool) -> bool {
        self.hero
            .as_ref()
            .is_some_and(|slides| slides.fading(HERO_FADE))
            || self.ambient.as_ref().is_some_and(|ambient| {
                // The screensaver drifts all the time; with reduced motion it only changes.
                !reduced_motion || ambient.slides.fading(AMBIENT_FADE)
            })
            || (!reduced_motion
                && (self
                    .title
                    .as_ref()
                    .is_some_and(|page| page.opened.elapsed() < PAGE_ENTRANCE)
                    || self
                        .menu
                        .as_ref()
                        .is_some_and(|menu| menu.opened.elapsed() < PAGE_ENTRANCE)))
    }

    /// Whether a title's artwork is in memory, or it has none.
    pub fn slide_ready(&self, link: &str) -> bool {
        self.details.contains_key(link) && self.artwork_loaded(link)
    }

    pub fn trakt_title(&self, link: &str) -> Option<&TraktTitle> {
        self.trakt_titles.get(link)
    }

    /// A title page's data arrived from the title worker.
    pub fn title_arrived(&mut self, link: String, result: Result<TitleInfo, String>) {
        if let Ok(info) = &result {
            self.details_arrived(link.clone(), Ok(info.details.clone()));
        }
        let trakt = self.trakt_titles.get(&link).cloned();
        let Some(page) = self.title.as_mut().filter(|page| page.link == link) else {
            return;
        };
        if let Err(error) = &result {
            eprintln!("Luma feeds: title {link}: {error}");
        }
        page.info = Some(result);
        if !page.season_chosen {
            page.season = initial_season(page, trakt.as_ref());
        }
        if page.series()
            && let Some(season) = page.seasons().get(page.season)
        {
            let number = season.number;
            if !page.episodes.contains_key(&number) {
                self.outbox.push(Outgoing::Season(link, number));
            }
        }
    }

    pub fn season_arrived(
        &mut self,
        link: String,
        season: u32,
        result: Result<Vec<Episode>, String>,
    ) {
        if let Some(page) = self.title.as_mut().filter(|page| page.link == link) {
            page.episodes.insert(season, result);
        }
    }

    pub fn trakt_arrived(&mut self, link: String, result: Result<TraktTitle, String>) {
        match result {
            Ok(trakt) => {
                if self.trakt_titles.len() > 60 {
                    self.trakt_titles.clear();
                }
                self.trakt_titles.insert(link.clone(), trakt.clone());
                // A show's page moves to the season of the next episode, until chosen.
                if let Some(page) = self.title.as_mut().filter(|page| page.link == link)
                    && !page.season_chosen
                    && page.info().is_some()
                {
                    let season = initial_season(page, Some(&trakt));
                    if season != page.season {
                        page.season = season;
                        if let Some(number) = page.seasons().get(season).map(|s| s.number)
                            && !page.episodes.contains_key(&number)
                        {
                            self.outbox.push(Outgoing::Season(link, number));
                        }
                    }
                }
            }
            Err(error) => eprintln!("Luma feeds: Trakt state {link}: {error}"),
        }
    }
}

/// The season a show's page opens on: that of the card's episode, else of the next one to
/// watch, else the first.
fn initial_season(page: &TitlePage, trakt: Option<&TraktTitle>) -> usize {
    let wanted = page
        .item
        .episode
        .or_else(|| trakt.and_then(|trakt| trakt.next))
        .map(|[season, _]| season);
    wanted
        .and_then(|season| {
            page.seasons()
                .iter()
                .position(|candidate| candidate.number == season)
        })
        .unwrap_or(0)
}

impl State {
    /// Turns the Top Shelf's slides: one every HERO_SLIDE while Home is in front, waiting for
    /// the next slide's artwork (for at most one more slide's time).
    pub fn hero_tick(&mut self) {
        let count = if self.preferences.feed_hero {
            self.feeds.hero_items().len()
        } else {
            0
        };
        if count == 0 {
            if self.feeds.hero.take().is_some() {
                if self.focus == Target::Hero {
                    self.focus = self.first_content();
                }
                self.dirty = true;
            }
            return;
        }
        let paused = self.page != Page::Home
            || self.overlay.is_some()
            || self.feeds.search.is_some()
            || self.feeds.title.is_some()
            || self.feeds.menu.is_some()
            || self.feeds.ambient.is_some()
            || !self.feeds.visible;
        let next_link = {
            let slides = self.feeds.hero.get_or_insert_with(Slides::new);
            if slides.index >= count {
                slides.index = 0;
                slides.from = None;
            }
            let items = self.feeds.hero_items();
            let (row, index) = items[(slides_index(&self.feeds) + 1) % count];
            self.feeds.rows[row].items[index].link.clone()
        };
        let ready = self.feeds.slide_ready(&next_link);
        let Some(slides) = self.feeds.hero.as_mut() else {
            return;
        };
        let elapsed = slides.since.elapsed();
        if !paused && count > 1 && ((elapsed >= HERO_SLIDE && ready) || elapsed >= HERO_SLIDE * 2) {
            slides.step(count, true);
            self.dirty = true;
        }
        if slides.from.is_some() && slides.since.elapsed() >= HERO_FADE {
            slides.from = None;
            self.dirty = true;
        }
    }

    /// Left and Right on the Top Shelf turn its slides by hand.
    fn hero_step(&mut self, forward: bool) {
        let count = self.feeds.hero_items().len();
        if let Some(slides) = &mut self.feeds.hero {
            slides.step(count, forward);
            self.dirty = true;
        }
    }

    /// Starts the screensaver after the configured idle time, and turns its slides.
    pub fn ambient_tick(&mut self, idle: Duration, visible: bool) {
        if let Some(ambient) = &self.feeds.ambient {
            let count = ambient.items.len();
            let next = &ambient.items[(ambient.slides.index + 1) % count.max(1)].link;
            let ready = self.feeds.slide_ready(next);
            let Some(ambient) = self.feeds.ambient.as_mut() else {
                return;
            };
            let elapsed = ambient.slides.since.elapsed();
            if count > 1 && ((elapsed >= AMBIENT_SLIDE && ready) || elapsed >= AMBIENT_SLIDE * 2) {
                ambient.slides.step(count, true);
                self.dirty = true;
            }
            if ambient.slides.from.is_some() && ambient.slides.since.elapsed() >= AMBIENT_FADE {
                ambient.slides.from = None;
                self.dirty = true;
            }
            return;
        }
        let Some(delay) = ambient_delay(self.preferences.feed_ambient) else {
            return;
        };
        if !visible
            || idle < delay
            || self.overlay.is_some()
            || self.recents_open()
            || self.launching
            || self.moving.is_some()
        {
            return;
        }
        let items = self.feeds.ambient_items();
        if items.is_empty() {
            return;
        }
        self.feeds.menu = None;
        self.feeds.ambient = Some(Ambient {
            items,
            slides: Slides::new(),
            started: Instant::now(),
        });
        self.pressed = None;
        self.dirty = true;
    }

    /// Any key ends the screensaver; OK also opens the title it showed.
    pub fn ambient_input(&mut self, ok: bool) {
        let Some(ambient) = self.feeds.ambient.take() else {
            return;
        };
        self.dirty = true;
        if !ok || !self.feeds.can_search {
            return;
        }
        let Some(item) = ambient.items.get(ambient.slides.index).cloned() else {
            return;
        };
        self.feeds.search = None;
        self.feeds.title = None;
        if self.page != Page::Home {
            self.open(0);
        }
        let restore = match self.focus.clone() {
            Target::Nav(_) => self.first_content(),
            focus => focus,
        };
        self.open_title(item, restore);
    }

    /// Opens a title's page over Home (or Search), and asks for its data.
    pub fn open_title(&mut self, item: Item, restore: Target) {
        let link = item.link.clone();
        self.feeds.menu = None;
        self.feeds.title = Some(TitlePage {
            link: link.clone(),
            item,
            info: None,
            episodes: HashMap::new(),
            season: 0,
            season_chosen: false,
            scroll: [0.; 4],
            page_scroll: 0.,
            restore,
            opened: Instant::now(),
            back: Vec::new(),
        });
        self.feeds.outbox.push(Outgoing::Title(link.clone()));
        if self.feeds.trakt {
            self.feeds.outbox.push(Outgoing::TraktState(link));
        }
        self.focus = Target::TitleButton(0);
        self.pointer = false;
        self.dirty = true;
    }

    /// Menu on a card: its title page. False when focus is not on a card.
    pub fn open_card_title(&mut self) -> bool {
        if self.overlay.is_some() || self.feeds.menu.is_some() {
            return false;
        }
        // Title pages come from TMDB, as Search does: without a key there are none.
        if !self.feeds.can_search {
            return false;
        }
        if matches!(
            self.focus,
            Target::TitleButton(_)
                | Target::TitleSeason(_)
                | Target::TitleEpisode(_)
                | Target::TitleCast(_)
        ) {
            return true;
        }
        let Some(item) = card_item(&self.feeds, &self.focus).cloned() else {
            return false;
        };
        if let Target::TitleMore(_) = self.focus {
            self.title_more(item);
        } else {
            self.open_title(item, self.focus.clone());
        }
        true
    }

    /// Opens a More Like This title over the page it came from.
    fn title_more(&mut self, item: Item) {
        let Some(page) = self.feeds.title.take() else {
            return;
        };
        let mut back = page.back;
        back.push(page.item);
        let restore = page.restore;
        self.open_title(item, restore);
        if let Some(page) = &mut self.feeds.title {
            page.back = back;
        }
    }

    /// Back on a title page: the page it was opened over, else what was focused before.
    fn title_back(&mut self) {
        let Some(mut page) = self.feeds.title.take() else {
            return;
        };
        if let Some(item) = page.back.pop() {
            let back = std::mem::take(&mut page.back);
            self.open_title(item, page.restore);
            if let Some(page) = &mut self.feeds.title {
                page.back = back;
                // Returning is not an entrance.
                page.opened = Instant::now() - PAGE_ENTRANCE;
            }
            return;
        }
        self.focus = page.restore;
        self.feeds.prune_images();
        self.reconcile_focus();
        self.pointer = false;
        self.dirty = true;
    }

    fn title_target(section: Section, index: usize) -> Target {
        match section {
            Section::Buttons => Target::TitleButton(index),
            Section::Seasons => Target::TitleSeason(index),
            Section::Episodes => Target::TitleEpisode(index),
            Section::Cast => Target::TitleCast(index),
            Section::More => Target::TitleMore(index),
        }
    }

    /// Remote navigation on a title page: Left and Right along a section, Up and Down
    /// between them. Moving along the season pills shows that season's episodes.
    pub fn title_direction(&mut self, direction: Direction) -> Vec<Effect> {
        let trakt = self.feeds.trakt;
        let trakt_title = self
            .feeds
            .title
            .as_ref()
            .and_then(|page| self.feeds.trakt_titles.get(&page.link))
            .cloned();
        let Some(page) = self.feeds.title.as_mut() else {
            return vec![];
        };
        let (section, index) = match self.focus {
            Target::TitleButton(index) => (Section::Buttons, index),
            Target::TitleSeason(index) => (Section::Seasons, index),
            Target::TitleEpisode(index) => (Section::Episodes, index),
            Target::TitleCast(index) => (Section::Cast, index),
            Target::TitleMore(index) => (Section::More, index),
            _ => (Section::Buttons, 0),
        };
        let sections: Vec<Section> = page
            .sections()
            .into_iter()
            .map(|(section, ..)| section)
            .filter(|section| *section == Section::Episodes || page.count(*section, trakt) > 0)
            .collect();
        let position = sections.iter().position(|s| *s == section).unwrap_or(0);
        let (section, index) = match direction {
            Direction::Left => (section, index.saturating_sub(1)),
            Direction::Right => (
                section,
                (index + 1).min(page.count(section, trakt).saturating_sub(1)),
            ),
            Direction::Up | Direction::Down => {
                let next = if matches!(direction, Direction::Up) {
                    position.checked_sub(1)
                } else {
                    Some(position + 1)
                };
                // Episodes still loading cannot take focus: skip over them.
                let mut next = next;
                while let Some(candidate) = next.and_then(|n| sections.get(n))
                    && *candidate == Section::Episodes
                    && page.count(Section::Episodes, trakt) == 0
                {
                    next = if matches!(direction, Direction::Up) {
                        next.and_then(|n| n.checked_sub(1))
                    } else {
                        next.map(|n| n + 1)
                    };
                }
                match next.and_then(|n| sections.get(n)) {
                    Some(&target) => {
                        let index = match target {
                            Section::Seasons => page.season,
                            Section::Episodes if section == Section::Seasons => {
                                page.landing_episode(trakt_title.as_ref())
                            }
                            Section::Episodes => {
                                page.scroll[1].div_euclid(EPISODE_PITCH).max(0.) as usize
                            }
                            _ => 0,
                        };
                        (
                            target,
                            index.min(page.count(target, trakt).saturating_sub(1)),
                        )
                    }
                    None => (section, index),
                }
            }
        };
        let new_season = section == Section::Seasons && index != page.season;
        match section {
            Section::Seasons => page.reveal_column(0, index, SEASON_PITCH, SEASON_WIDTH),
            Section::Episodes => page.reveal_column(1, index, EPISODE_PITCH, EPISODE_WIDTH),
            Section::Cast => page.reveal_column(2, index, CAST_PITCH, CAST_WIDTH),
            Section::More => page.reveal_column(3, index, CARD_PITCH, CARD_WIDTH),
            Section::Buttons => {}
        }
        page.reveal_section(section);
        if new_season {
            self.select_season(index);
        }
        self.focus = Self::title_target(section, index);
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// OK on a title page. None when no page is open.
    pub fn title_activate(&mut self) -> Option<Vec<Effect>> {
        let page = self.feeds.title.as_ref()?;
        let link = page.link.clone();
        Some(match self.focus.clone() {
            Target::TitleButton(index) => {
                let Some(button) = page.buttons(self.feeds.trakt).get(index).copied() else {
                    return Some(vec![]);
                };
                match button {
                    TitleButton::Play if !self.launching => vec![Effect::LaunchTitle(link)],
                    TitleButton::Play => vec![],
                    TitleButton::Trailer => self.play_trailer().unwrap_or_default(),
                    TitleButton::Watchlist => {
                        self.toggle_watchlist(&link);
                        vec![]
                    }
                    TitleButton::Watched => {
                        let item = page.item.clone();
                        self.mark_watched(&link, &item);
                        vec![]
                    }
                    TitleButton::Rate => {
                        let item = page.item.clone();
                        self.open_menu(item, true);
                        vec![]
                    }
                    TitleButton::HideUpNext => {
                        self.trakt_action(&link, TraktAction::HideUpNext);
                        vec![]
                    }
                }
            }
            // Nuvio's links open a show's page, not one episode: the show it is.
            Target::TitleEpisode(_) if !self.launching => vec![Effect::LaunchTitle(link)],
            // Pointing and clicking a season pill shows it, as moving onto it does.
            Target::TitleSeason(index) => {
                self.select_season(index);
                vec![]
            }
            Target::TitleMore(index) => {
                if let Some(item) = page.more().and_then(|more| more.get(index)).cloned() {
                    self.title_more(item);
                }
                vec![]
            }
            _ => vec![],
        })
    }

    /// Shows a season's episodes on the open title page, asking for them when new.
    fn select_season(&mut self, index: usize) {
        let Some(page) = self.feeds.title.as_mut() else {
            return;
        };
        if index == page.season || index >= page.seasons().len() {
            return;
        }
        page.season = index;
        page.season_chosen = true;
        page.scroll[1] = 0.;
        let number = page.seasons()[index].number;
        if !page.episodes.contains_key(&number) {
            self.feeds
                .outbox
                .push(Outgoing::Season(page.link.clone(), number));
        }
        self.dirty = true;
    }

    /// The pointer's wheel scrolls an open title page. False when none is open.
    pub fn title_wheel(&mut self, y: i32) -> bool {
        let Some(page) = self.feeds.title.as_mut() else {
            return false;
        };
        let bottom = page
            .sections()
            .last()
            .map_or(0., |(_, top, height)| top + height);
        let max = (bottom + 60. - 1080.).max(0.);
        page.page_scroll = (page.page_scroll - y as f32 * 90.).clamp(0., max);
        self.pointer = true;
        self.dirty = true;
        true
    }

    /// Holding OK on a card: its quick actions. None when focus is not on a card.
    pub fn card_hold(&mut self) -> Option<Vec<Effect>> {
        if self.overlay.is_some() || self.feeds.menu.is_some() {
            return None;
        }
        // On a title page, holding OK on an episode marks it watched.
        if let Target::TitleEpisode(index) = self.focus
            && self.feeds.trakt
        {
            let page = self.feeds.title.as_ref()?;
            let episode = page.shown_episodes()?.get(index)?;
            let action = TraktAction::Episode(episode.season, episode.number);
            let link = page.link.clone();
            self.trakt_action(&link, action);
            return Some(vec![]);
        }
        if matches!(
            self.focus,
            Target::TitleButton(_) | Target::TitleSeason(_) | Target::TitleCast(_)
        ) {
            return Some(vec![]);
        }
        let item = card_item(&self.feeds, &self.focus)?.clone();
        self.open_menu(item, false);
        Some(vec![])
    }

    /// Opens a card's quick actions, or (with `rating`) the ratings to give it.
    fn open_menu(&mut self, item: Item, rating: bool) {
        let link = item.link.clone();
        let entries = if rating {
            RATINGS
                .iter()
                .map(|(value, _)| MenuEntry::RateValue(*value))
                .collect()
        } else {
            let mut entries = Vec::new();
            if self.feeds.can_search {
                entries.push(MenuEntry::Details);
            }
            let trailer = self
                .feeds
                .details
                .get(&link)
                .is_none_or(|details| details.trailer.is_some());
            if trailer && self.feeds.can_search {
                entries.push(MenuEntry::Trailer);
            }
            if self.feeds.trakt {
                entries.extend([MenuEntry::Watchlist, MenuEntry::Watched, MenuEntry::Rate]);
                if link_series(&link) {
                    entries.push(MenuEntry::HideUpNext);
                }
            }
            entries
        };
        // Without TMDB or Trakt a card has nothing to offer.
        if entries.is_empty() {
            return;
        }
        if self.feeds.trakt && !self.feeds.trakt_titles.contains_key(&link) {
            self.feeds.outbox.push(Outgoing::TraktState(link.clone()));
        }
        let restore = self
            .feeds
            .menu
            .take()
            .map_or_else(|| self.focus.clone(), |menu| menu.restore);
        self.feeds.menu = Some(Menu {
            link,
            item,
            entries,
            restore,
            opened: Instant::now(),
        });
        self.focus = Target::MenuItem(0);
        self.pointer = false;
        self.dirty = true;
    }

    fn close_menu(&mut self) {
        if let Some(menu) = self.feeds.menu.take() {
            self.focus = menu.restore;
        }
        self.pointer = false;
        self.dirty = true;
    }

    pub fn menu_direction(&mut self, direction: Direction) -> Vec<Effect> {
        let Some(menu) = &self.feeds.menu else {
            return vec![];
        };
        let count = menu.entries.len();
        if let Target::MenuItem(index) = self.focus {
            let next = match direction {
                Direction::Up => index.saturating_sub(1),
                Direction::Down => (index + 1).min(count.saturating_sub(1)),
                _ => index,
            };
            self.focus = Target::MenuItem(next);
        } else {
            self.focus = Target::MenuItem(0);
        }
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// OK in a quick-actions menu. None when no menu is open.
    pub fn menu_activate(&mut self) -> Option<Vec<Effect>> {
        let menu = self.feeds.menu.as_ref()?;
        let Target::MenuItem(index) = self.focus else {
            return Some(vec![]);
        };
        let Some(entry) = menu.entries.get(index).copied() else {
            return Some(vec![]);
        };
        let link = menu.link.clone();
        let item = menu.item.clone();
        Some(match entry {
            MenuEntry::Rate => {
                self.open_menu(item, true);
                vec![]
            }
            MenuEntry::Details => {
                let restore = self.feeds.menu.take().map(|menu| menu.restore);
                let restore = restore.unwrap_or_else(|| self.first_content());
                let open_page = self.feeds.title.as_ref().map(|page| page.link == link);
                match open_page {
                    // From a page's More Like This: over that page.
                    Some(_) if matches!(restore, Target::TitleMore(_)) => self.title_more(item),
                    Some(true) => self.focus = Target::TitleButton(0),
                    _ => self.open_title(item, restore),
                }
                vec![]
            }
            MenuEntry::Trailer => {
                self.close_menu();
                let trailer = self.feeds.details.get(&link).map(|d| d.trailer.clone());
                match trailer {
                    Some(Some(key)) => return Some(vec![Effect::Trailer(key)]),
                    Some(None) => self.notify("No trailer for this title"),
                    None => {
                        self.feeds.requested = None;
                        self.notify("Finding the trailer... try again in a moment");
                    }
                }
                vec![]
            }
            MenuEntry::Watchlist => {
                self.close_menu();
                self.toggle_watchlist(&link);
                vec![]
            }
            MenuEntry::Watched => {
                self.close_menu();
                self.mark_watched(&link, &item);
                vec![]
            }
            MenuEntry::RateValue(value) => {
                self.close_menu();
                self.trakt_action(&link, TraktAction::Rate(value));
                vec![]
            }
            MenuEntry::HideUpNext => {
                self.close_menu();
                self.trakt_action(&link, TraktAction::HideUpNext);
                vec![]
            }
        })
    }

    /// Back with a menu or title page open. False when neither is.
    pub fn feeds_back(&mut self) -> bool {
        if self.feeds.menu.is_some() {
            self.close_menu();
            return true;
        }
        if self.feeds.title.is_some() {
            self.title_back();
            return true;
        }
        false
    }

    fn toggle_watchlist(&mut self, link: &str) {
        let listed = self
            .feeds
            .trakt_titles
            .get(link)
            .is_some_and(|trakt| trakt.in_watchlist);
        self.trakt_action(
            link,
            if listed {
                TraktAction::WatchlistRemove
            } else {
                TraktAction::WatchlistAdd
            },
        );
    }

    /// Marks a movie watched, or a show's next episode (or the card's episode).
    fn mark_watched(&mut self, link: &str, item: &Item) {
        if !link_series(link) {
            self.trakt_action(link, TraktAction::Watched);
            return;
        }
        let episode = self
            .feeds
            .trakt_titles
            .get(link)
            .and_then(|trakt| trakt.next)
            .or(item.episode);
        match episode {
            Some([season, number]) => {
                self.trakt_action(link, TraktAction::Episode(season, number));
            }
            None => self.notify("Open the title's page and hold OK on an episode to mark it"),
        }
    }

    /// Sends a change to Trakt and shows it at once where it can (the Watchlist button).
    fn trakt_action(&mut self, link: &str, action: TraktAction) {
        let trakt = self.feeds.trakt_titles.entry(link.to_owned()).or_default();
        match &action {
            TraktAction::WatchlistAdd => trakt.in_watchlist = true,
            TraktAction::WatchlistRemove => trakt.in_watchlist = false,
            TraktAction::Watched => trakt.watched = true,
            TraktAction::Episode(season, number) => {
                trakt.episodes.insert([*season, *number]);
            }
            TraktAction::Rate(_) | TraktAction::HideUpNext => {}
        }
        self.feeds
            .outbox
            .push(Outgoing::Trakt(link.to_owned(), action));
        self.notify("Updating Trakt...");
    }

    /// A change on Trakt finished: say how it went, and look the title up again.
    pub fn trakt_done(&mut self, link: String, result: Result<String, String>) {
        match result {
            Ok(message) => self.notify(message),
            Err(error) => {
                eprintln!("Luma feeds: Trakt change {link}: {error}");
                self.notify(format!("Trakt: {error}"));
            }
        }
        if self
            .feeds
            .menu
            .as_ref()
            .is_some_and(|menu| menu.link == link)
            || self
                .feeds
                .title
                .as_ref()
                .is_some_and(|page| page.link == link)
        {
            self.feeds.outbox.push(Outgoing::TraktState(link));
        } else {
            self.feeds.trakt_titles.remove(&link);
        }
    }
}

/// How long Home stays idle before the screensaver: None when it is off.
pub fn ambient_delay(setting: u8) -> Option<Duration> {
    match setting {
        0 => None,
        1 => Some(Duration::from_secs(5 * 60)),
        2 => Some(Duration::from_secs(10 * 60)),
        3 => Some(Duration::from_secs(15 * 60)),
        _ => Some(Duration::from_secs(30 * 60)),
    }
}

impl Feeds {
    /// Whether any row has titles for the screensaver to show.
    pub fn has_titles(&self) -> bool {
        self.rows.iter().any(|row| !row.items.is_empty())
    }
}

/// The Top Shelf's current slide index, or 0.
fn slides_index(feeds: &Feeds) -> usize {
    feeds.hero.as_ref().map_or(0, |slides| slides.index)
}
