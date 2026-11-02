//! Explore: a grid of titles under a bar of filters. With "Everything" it searches all of TMDB
//! (Discover) for the chosen type (movies, shows, anime), genre and order, once per change of
//! filter, about sixty titles of each type; "My Lists" picks one of the rows instead (its full
//! list, past feeds.json's `limit`). It opens from its tab, from the "See all" tile at the end of
//! each Home row, and from the Top Shelf's Watchlist button.
use super::{CARD_HEIGHT, CARD_PITCH, CARD_WIDTH, Feeds, Item, Outgoing, card_link, link_series};
use crate::state::{Control, Direction, EXPLORE_PAGE, Effect, Page, Rect, State, Target};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The grid: seven posters across, under the filters.
pub const EXPLORE_COLUMNS: usize = 7;
pub const EXPLORE_GRID_X: f32 = 84.;
pub const EXPLORE_RIGHT: f32 = 1836.;
pub const EXPLORE_GRID_TOP: f32 = 322.;
pub const EXPLORE_ROW: f32 = CARD_HEIGHT + 46.;
/// Above the grid: the page's name, a line about the focused title (or the search), then
/// the filters.
pub const EXPLORE_TITLE_Y: f32 = 138.;
pub const EXPLORE_INFO_Y: f32 = 196.;
pub const EXPLORE_FILTER_Y: f32 = 238.;
pub const EXPLORE_FILTER_H: f32 = 48.;
/// A dropdown's rows, and how many show before it scrolls.
pub const MENU_ROW: f32 = 58.;
pub const MENU_SHOWN: usize = 11;
/// How long the grid takes to scroll to a row, and to rise in when its titles change.
const EXPLORE_SCROLL: Duration = Duration::from_millis(300);
pub const EXPLORE_ENTRANCE: Duration = Duration::from_millis(320);
/// The backdrop's crossfade from one title to the next.
pub const BACKDROP_FADE: Duration = Duration::from_millis(700);
/// A row's full list is asked for again after this long; until it comes (for a moment) the
/// grid waits for it rather than showing the row's first few and then growing.
const EXPAND_AGAIN: Duration = Duration::from_secs(10 * 60);
const EXPAND_WAIT: Duration = Duration::from_secs(6);
/// Searches remembered, so going back to a filter shows its titles at once.
const FOUND_KEPT: usize = 16;

/// What Explore shows: all of TMDB through the filters, the Watchlist, or one row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Category {
    #[default]
    Everything,
    Watchlist,
    Row(usize),
}

/// Movies, shows, both, or anime (Japanese animation, films and series).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    #[default]
    All,
    Movies,
    Shows,
    Anime,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::All, Kind::Movies, Kind::Shows, Kind::Anime];

    pub fn label(self) -> &'static str {
        match self {
            Kind::All => "All",
            Kind::Movies => "Movies",
            Kind::Shows => "Shows",
            Kind::Anime => "Anime",
        }
    }

    /// A row's titles filtered by type (anime isn't told apart in a row: it shows them all).
    fn admits(self, item: &Item) -> bool {
        match self {
            Kind::Movies => !link_series(&item.link),
            Kind::Shows => link_series(&item.link),
            Kind::All | Kind::Anime => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sort {
    #[default]
    Popular,
    Rating,
    Newest,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Popular, Sort::Rating, Sort::Newest];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Popular => "Popular",
            Sort::Rating => "Top rated",
            Sort::Newest => "Newest",
        }
    }
}

/// A genre: TMDB's genre ID for films and for TV (they differ, and TV lacks some), or a TMDB
/// keyword where there is none (anime's own: Isekai, Mecha...).
#[derive(Clone, Copy, Debug)]
pub struct Genre {
    pub name: &'static str,
    pub movie: Option<u32>,
    pub tv: Option<u32>,
    pub keyword: Option<&'static str>,
}

const fn genre(name: &'static str, movie: Option<u32>, tv: Option<u32>) -> Genre {
    Genre {
        name,
        movie,
        tv,
        keyword: None,
    }
}

const fn keyword(name: &'static str, keyword: &'static str) -> Genre {
    Genre {
        name,
        movie: None,
        tv: None,
        keyword: Some(keyword),
    }
}

pub const GENRES: [Genre; 19] = [
    genre("Action", Some(28), Some(10759)),
    genre("Adventure", Some(12), Some(10759)),
    genre("Animation", Some(16), Some(16)),
    genre("Comedy", Some(35), Some(35)),
    genre("Crime", Some(80), Some(80)),
    genre("Documentary", Some(99), Some(99)),
    genre("Drama", Some(18), Some(18)),
    genre("Family", Some(10751), Some(10751)),
    genre("Fantasy", Some(14), Some(10765)),
    genre("History", Some(36), None),
    genre("Horror", Some(27), None),
    genre("Kids", None, Some(10762)),
    genre("Music", Some(10402), None),
    genre("Mystery", Some(9648), Some(9648)),
    genre("Reality", None, Some(10764)),
    genre("Romance", Some(10749), None),
    genre("Sci-Fi", Some(878), Some(10765)),
    genre("Thriller", Some(53), None),
    genre("War", Some(10752), Some(10768)),
];

/// Anime's genres: the usual ones TMDB has, then its own, which TMDB keeps as keywords.
pub const ANIME_GENRES: [Genre; 19] = [
    genre("Action", Some(28), Some(10759)),
    genre("Adventure", Some(12), Some(10759)),
    genre("Comedy", Some(35), Some(35)),
    genre("Drama", Some(18), Some(18)),
    genre("Fantasy", Some(14), Some(10765)),
    genre("Mystery", Some(9648), Some(9648)),
    Genre {
        name: "Romance",
        movie: Some(10749),
        tv: None,
        keyword: Some("romance"),
    },
    genre("Sci-Fi", Some(878), Some(10765)),
    Genre {
        name: "Horror",
        movie: Some(27),
        tv: None,
        keyword: Some("horror"),
    },
    keyword("Isekai", "isekai"),
    keyword("Mecha", "mecha"),
    keyword("Shōnen", "shounen"),
    keyword("Shōjo", "shoujo"),
    keyword("Seinen", "seinen"),
    keyword("Slice of Life", "slice of life"),
    keyword("Sports", "sports"),
    keyword("Supernatural", "supernatural"),
    keyword("Psychological", "psychological"),
    keyword("Magical Girl", "magical girl"),
];

/// Explore's filters for a search of all of TMDB. `genre` indexes GENRES (ANIME_GENRES for
/// anime).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub kind: Kind,
    pub genre: Option<usize>,
    pub sort: Sort,
}

impl Query {
    pub fn genres(&self) -> &'static [Genre] {
        if self.kind == Kind::Anime {
            &ANIME_GENRES
        } else {
            &GENRES
        }
    }

    pub fn genre_name(&self) -> Option<&'static str> {
        self.genre
            .and_then(|index| self.genres().get(index))
            .map(|genre| genre.name)
    }

    /// Whether a genre can be searched for this type (TV has no Horror, films no Reality).
    fn offers(&self, genre: &Genre) -> bool {
        match self.kind {
            Kind::Movies => genre.movie.is_some(),
            Kind::Shows => genre.tv.is_some() || genre.keyword.is_some(),
            Kind::All | Kind::Anime => true,
        }
    }

    /// "Popular anime · Isekai", for the line under the page's name.
    pub fn summary(&self) -> String {
        let kind = match self.kind {
            Kind::All => "movies and shows",
            Kind::Movies => "movies",
            Kind::Shows => "shows",
            Kind::Anime => "anime",
        };
        let sort = match self.sort {
            Sort::Popular => "Popular",
            Sort::Rating => "Top rated",
            Sort::Newest => "Newest",
        };
        match self.genre_name() {
            Some(genre) => format!("{sort} {kind}  ·  {genre}"),
            None => format!("{sort} {kind}"),
        }
    }
}

/// A title TMDB found for the filters, with what it said about it.
#[derive(Clone, Debug)]
pub struct Found {
    pub item: Item,
    pub year: Option<u16>,
    pub rating: Option<f32>,
}

/// An open dropdown and what it offers.
#[derive(Clone, Debug)]
pub enum Choices {
    Source(Vec<Category>),
    Genre(Vec<Option<usize>>),
    Sort(Vec<Sort>),
}

pub struct Dropdown {
    pub choices: Choices,
    pub labels: Vec<String>,
    /// The choice in effect, ticked.
    pub chosen: usize,
    pub scroll: usize,
    pub opened: Instant,
    /// The pill it opened from, where focus returns.
    pub anchor: Target,
}

pub struct Explore {
    pub category: Category,
    pub query: Query,
    /// The titles shown, in order.
    pub items: Vec<Item>,
    /// Searches answered, by filters.
    pub found: HashMap<Query, Vec<Found>>,
    /// Years and ratings of titles found, by link, for the line about the focused one.
    pub about: HashMap<String, (Option<u16>, Option<f32>)>,
    /// The search last sent, and how many have been sent.
    pub asked: Option<(u64, Query)>,
    pub requests: u64,
    pub error: Option<String>,
    /// Rows or a search changed: the grid is worked out again on the next tick.
    pub stale: bool,
    /// The grid's scroll, and the scroll it eases from, since when.
    pub scroll: f32,
    pub scroll_from: f32,
    pub scroll_since: Option<Instant>,
    /// When the grid's titles last changed, for its entrance.
    pub changed: Instant,
    /// The posters the grid shows and the rows by them, for the worker to download.
    pub images: Vec<String>,
    /// The backdrop behind Explore: the last focused title's whose backdrop arrived, the one
    /// before it, which it crossfades from, and since when.
    pub backdrop: Option<String>,
    pub previous: Option<String>,
    pub backdrop_since: Instant,
    pub dropdown: Option<Dropdown>,
}

impl Default for Explore {
    fn default() -> Self {
        Self {
            category: Category::Everything,
            query: Query::default(),
            items: Vec::new(),
            found: HashMap::new(),
            about: HashMap::new(),
            asked: None,
            requests: 0,
            error: None,
            stale: true,
            scroll: 0.,
            scroll_from: 0.,
            scroll_since: None,
            changed: Instant::now() - EXPLORE_ENTRANCE,
            images: Vec::new(),
            backdrop: None,
            previous: None,
            backdrop_since: Instant::now() - BACKDROP_FADE,
            dropdown: None,
        }
    }
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

impl Explore {
    /// The grid's scroll as drawn: easing towards its target.
    pub fn drawn_scroll(&self, reduced_motion: bool) -> f32 {
        match self.scroll_since {
            Some(since) if !reduced_motion => {
                let t = since.elapsed().as_secs_f32() / EXPLORE_SCROLL.as_secs_f32();
                if t >= 1. {
                    self.scroll
                } else {
                    self.scroll_from + (self.scroll - self.scroll_from) * ease_out(t)
                }
            }
            _ => self.scroll,
        }
    }

    fn scroll_to(&mut self, target: f32, reduced_motion: bool) {
        if (target - self.scroll).abs() < 0.5 {
            return;
        }
        self.scroll_from = self.drawn_scroll(reduced_motion);
        self.scroll = target;
        self.scroll_since = Some(Instant::now());
    }

    /// How far the grid has risen in after a change, 0 to 1.
    pub fn entrance(&self, reduced_motion: bool) -> f32 {
        if reduced_motion {
            return 1.;
        }
        ease_out(self.changed.elapsed().as_secs_f32() / EXPLORE_ENTRANCE.as_secs_f32())
    }

    /// Whether anything is still moving, so Explore keeps redrawing.
    pub fn animating(&self) -> bool {
        self.scroll_since
            .is_some_and(|since| since.elapsed() < EXPLORE_SCROLL + Duration::from_millis(40))
            || self.changed.elapsed() < EXPLORE_ENTRANCE + Duration::from_millis(40)
            || self.backdrop_since.elapsed() < BACKDROP_FADE + Duration::from_millis(40)
            || self
                .dropdown
                .as_ref()
                .is_some_and(|dropdown| dropdown.opened.elapsed() < Duration::from_millis(260))
    }

    /// How far the backdrop has crossfaded to the current title, 0 to 1.
    pub fn backdrop_fade(&self, reduced_motion: bool) -> f32 {
        if reduced_motion {
            return 1.;
        }
        let t = self.backdrop_since.elapsed().as_secs_f32() / BACKDROP_FADE.as_secs_f32();
        let t = t.clamp(0., 1.);
        t * t * (3. - 2. * t)
    }

    /// The grid row at the top of the view.
    pub fn first_row(&self) -> usize {
        (self.scroll / EXPLORE_ROW).round().max(0.) as usize
    }

    /// Whether the filters narrow the search (or, for a list, its type).
    pub fn filtered(&self) -> bool {
        self.query.kind != Kind::All || self.query.genre.is_some()
    }
}

/// A grid card's rectangle at `scroll`.
pub fn card_rect(index: usize, scroll: f32) -> Rect {
    Rect::new(
        EXPLORE_GRID_X + (index % EXPLORE_COLUMNS) as f32 * CARD_PITCH,
        EXPLORE_GRID_TOP + (index / EXPLORE_COLUMNS) as f32 * EXPLORE_ROW - scroll,
        CARD_WIDTH,
        CARD_HEIGHT,
    )
}

impl Feeds {
    /// The Watchlist's row: feeds.json's own, or the hidden one Explore adds.
    pub fn watchlist_row(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.source == "trakt:watchlist")
    }

    /// What "My Lists" offers: Everything, My Watchlist, then each row.
    pub fn explore_categories(&self) -> Vec<Category> {
        let watchlist = self.watchlist_row();
        let mut list = vec![Category::Everything];
        if watchlist.is_some() {
            list.push(Category::Watchlist);
        }
        for (index, row) in self.rows.iter().enumerate() {
            if row.hidden || Some(index) == watchlist || !row.shown() {
                continue;
            }
            list.push(Category::Row(index));
        }
        list
    }

    pub fn category_title(&self, category: Category) -> String {
        match category {
            Category::Everything => "Everything".into(),
            Category::Watchlist => "My Watchlist".into(),
            Category::Row(index) => self
                .rows
                .get(index)
                .map_or_else(String::new, |row| row.title.clone()),
        }
    }

    /// The row a category lists, if it is one.
    pub fn category_row(&self, category: Category) -> Option<usize> {
        match category {
            Category::Everything => None,
            Category::Watchlist => self.watchlist_row(),
            Category::Row(index) => (index < self.rows.len()).then_some(index),
        }
    }

    /// A row's titles for Explore: its full list once fetched (with posters the row already
    /// has filled in), else the row's own.
    fn row_titles(&self, index: usize) -> Vec<Item> {
        let Some(row) = self.rows.get(index) else {
            return Vec::new();
        };
        match self.expanded.get(&index) {
            Some((items, _)) if !items.is_empty() => items
                .iter()
                .map(|item| {
                    let mut item = item.clone();
                    if let Some(own) = row.items.iter().find(|own| own.link == item.link) {
                        item.image = item.image.or_else(|| own.image.clone());
                        item.progress = item.progress.or(own.progress);
                    }
                    item
                })
                .collect(),
            _ => row.items.clone(),
        }
    }

    /// Whether the grid is waiting: for a search's answer, or for a row's full list (for a
    /// moment, so the grid doesn't show the first few and then grow).
    pub fn explore_loading(&self) -> bool {
        let explore = &self.explore;
        match self.category_row(explore.category) {
            None => !explore.found.contains_key(&explore.query) && explore.error.is_none(),
            Some(index) => {
                let row = &self.rows[index];
                (row.items.is_empty() && row.status == super::Status::Loading)
                    || (!self.expanded.contains_key(&index)
                        && self
                            .expand_asked
                            .get(&index)
                            .is_some_and(|asked| asked.elapsed() < EXPAND_WAIT))
            }
        }
    }

    /// Works the grid out again: the search's titles, or the row's, by type.
    pub fn refresh_explore(&mut self) {
        if !self.explore_categories().contains(&self.explore.category) {
            self.explore.category = Category::Everything;
        }
        let items = match self.category_row(self.explore.category) {
            None => self
                .explore
                .found
                .get(&self.explore.query)
                .map(|found| found.iter().map(|found| found.item.clone()).collect())
                .unwrap_or_default(),
            Some(_) if self.explore_loading() => Vec::new(),
            Some(index) => {
                let kind = self.explore.query.kind;
                self.row_titles(index)
                    .into_iter()
                    .filter(|item| kind.admits(item))
                    .collect()
            }
        };
        let explore = &mut self.explore;
        if items != explore.items {
            explore.changed = Instant::now();
        }
        explore.items = items;
        explore.stale = false;
    }

    /// The posters Explore wants: the grid rows in view and one either side.
    fn explore_wanted_images(&self) -> Vec<String> {
        let first = self.explore.first_row().saturating_sub(1) * EXPLORE_COLUMNS;
        let last = (self.explore.first_row() + 4) * EXPLORE_COLUMNS;
        self.explore
            .items
            .iter()
            .skip(first)
            .take(last.saturating_sub(first))
            .filter_map(|item| item.image.clone())
            .collect()
    }
}

impl State {
    /// A search's titles arrived; a late answer to one already overtaken is kept for later.
    pub fn discover_arrived(&mut self, id: u64, query: Query, result: Result<Vec<Found>, String>) {
        let explore = &mut self.feeds.explore;
        let current = explore.asked.is_some_and(|(asked, _)| asked == id);
        match result {
            Ok(found) => {
                if explore.found.len() >= FOUND_KEPT {
                    explore.found.clear();
                }
                for title in &found {
                    explore
                        .about
                        .insert(title.item.link.clone(), (title.year, title.rating));
                }
                explore.found.insert(query, found);
                if current {
                    explore.error = None;
                }
            }
            Err(error) if current => explore.error = Some(error),
            Err(_) => {}
        }
        explore.stale = true;
        self.dirty = true;
    }

    /// A row's full list arrived.
    pub fn expanded_arrived(&mut self, row: usize, result: Result<Vec<Item>, String>) {
        let items = match result {
            Ok(items) => items,
            Err(error) => {
                if self.page == Page::Explore {
                    self.notify(format!("Couldn't load the whole list: {error}"));
                }
                Vec::new()
            }
        };
        self.feeds.expanded.insert(row, (items, Instant::now()));
        self.feeds.explore.stale = true;
        self.dirty = true;
    }

    /// The feed worker started again with other rows (Settings > Lists): row numbers moved,
    /// so full lists are asked for again, and the new worker gets Explore's posters again.
    pub fn explore_rows_restarted(&mut self) {
        self.feeds.expanded.clear();
        self.feeds.expand_asked.clear();
        self.feeds.extra_sent.clear();
        if matches!(self.feeds.explore.category, Category::Row(_)) {
            self.feeds.explore.category = Category::Everything;
            self.feeds.explore.scroll = 0.;
            self.feeds.explore.dropdown = None;
            if self.page == Page::Explore
                && matches!(
                    self.focus,
                    Target::ExploreItem(_) | Target::ExploreOption(_)
                )
            {
                self.focus = Target::ExploreSource;
            }
        }
        self.feeds.explore.stale = true;
        self.dirty = true;
    }

    /// Opens Explore on a category: from a row's "See all" tile, the Top Shelf's Watchlist
    /// button, or the tab.
    pub fn open_explore(&mut self, category: Category) -> Vec<Effect> {
        self.feeds.preview = None;
        self.feeds.menu = None;
        if self.page != Page::Explore {
            self.open(EXPLORE_PAGE);
        }
        self.explore_select(category);
        self.focus = Target::ExploreSource;
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// Shows a category, from the top of its grid.
    fn explore_select(&mut self, category: Category) {
        let explore = &mut self.feeds.explore;
        explore.dropdown = None;
        if explore.category != category {
            explore.category = category;
            explore.scroll = 0.;
            explore.scroll_since = None;
            // Lists aren't told apart as anime.
            if category != Category::Everything && explore.query.kind == Kind::Anime {
                explore.query.kind = Kind::All;
                explore.query.genre = None;
            }
        }
        self.explore_refresh();
    }

    /// Where Down from the tab lands: always the filters' first, My Lists.
    pub fn explore_entry(&self) -> Target {
        Target::ExploreSource
    }

    /// Works the grid out again, keeping focus on the same title where it moved.
    pub fn explore_refresh(&mut self) {
        let focused = match self.focus {
            Target::ExploreItem(index) => self
                .feeds
                .explore
                .items
                .get(index)
                .map(|item| item.link.clone()),
            _ => None,
        };
        self.feeds.refresh_explore();
        let count = self.feeds.explore.items.len();
        let reduced_motion = self.preferences.reduced_motion;
        if let Target::ExploreItem(index) = self.focus {
            let found = focused.and_then(|link| {
                self.feeds
                    .explore
                    .items
                    .iter()
                    .position(|item| item.link == link)
            });
            self.focus = match (found, count) {
                (_, 0) => Target::ExploreSource,
                (Some(found), _) => Target::ExploreItem(found),
                (None, _) => Target::ExploreItem(index.min(count - 1)),
            };
            if let Target::ExploreItem(index) = self.focus {
                let target = (index / EXPLORE_COLUMNS) as f32 * EXPLORE_ROW;
                self.feeds.explore.scroll_to(target, reduced_motion);
            }
        }
        if count == 0 && self.focus == Target::ExploreSurprise {
            self.focus = Target::ExploreSource;
        }
        let rows = count.div_ceil(EXPLORE_COLUMNS);
        let max = rows.saturating_sub(1) as f32 * EXPLORE_ROW;
        if self.feeds.explore.scroll > max {
            self.feeds.explore.scroll_to(max, reduced_motion);
        }
        self.dirty = true;
    }

    /// The filters changed: the grid starts again from the top.
    fn explore_filtered(&mut self) {
        let explore = &mut self.feeds.explore;
        explore.scroll = 0.;
        explore.scroll_since = None;
        explore.error = None;
        self.explore_refresh();
    }

    /// Once a frame while Explore is in front: the grid after new rows or answers, the search
    /// or full list to ask for, the posters to download and the backdrop behind it all.
    pub fn explore_tick(&mut self) {
        if self.page != Page::Explore {
            let explore = &mut self.feeds.explore;
            explore.dropdown = None;
            if !explore.images.is_empty() || explore.backdrop.is_some() {
                explore.images.clear();
                explore.backdrop = None;
                explore.previous = None;
                self.feeds.prune_images();
            }
            return;
        }
        let category = self.feeds.explore.category;
        match self.feeds.category_row(category) {
            // One search per change of filter: never again for the same filters.
            None => {
                let explore = &mut self.feeds.explore;
                let query = explore.query;
                if self.feeds.can_search
                    && !explore.found.contains_key(&query)
                    && explore.asked.is_none_or(|(_, asked)| asked != query)
                    && explore.error.is_none()
                {
                    explore.requests += 1;
                    explore.asked = Some((explore.requests, query));
                    let id = explore.requests;
                    self.feeds.outbox.push(Outgoing::Discover(id, query));
                }
            }
            Some(row) => {
                let due = self
                    .feeds
                    .expand_asked
                    .get(&row)
                    .is_none_or(|asked| asked.elapsed() >= EXPAND_AGAIN);
                if due {
                    self.feeds.expand_asked.insert(row, Instant::now());
                    self.feeds.outbox.push(Outgoing::Expand(row));
                    self.feeds.explore.stale = true;
                }
            }
        }
        // The wait for a full list is over (it came, or took too long): show what there is.
        if self.feeds.explore.items.is_empty() && !self.feeds.explore_loading() {
            self.feeds.explore.stale = true;
        }
        if self.feeds.explore.stale {
            self.explore_refresh();
        }
        let images = self.feeds.explore_wanted_images();
        if images != self.feeds.explore.images {
            self.feeds.explore.images = images;
            self.feeds.prune_images();
        }
        // The focused title's backdrop, once it is here; the last one stays until then.
        if let Some(url) = card_link(&self.feeds, &self.focus)
            .and_then(|link| self.feeds.details.get(&link))
            .and_then(|details| details.backdrop.clone())
            .filter(|url| self.feeds.images.contains_key(url))
            && self.feeds.explore.backdrop.as_ref() != Some(&url)
        {
            let explore = &mut self.feeds.explore;
            explore.previous = explore.backdrop.replace(url);
            explore.backdrop_since = Instant::now();
            self.dirty = true;
        }
        // The crossfade done, the title before can go.
        if self.feeds.explore.previous.is_some()
            && self.feeds.explore.backdrop_since.elapsed() >= BACKDROP_FADE
        {
            self.feeds.explore.previous = None;
            self.feeds.prune_images();
            self.dirty = true;
        }
    }

    /// The filter bar's controls, left to right.
    fn explore_bar(controls: &[Control]) -> Vec<(Target, Rect)> {
        let mut line: Vec<(Target, Rect)> = controls
            .iter()
            .filter(|control| {
                matches!(
                    control.target,
                    Target::ExploreSource
                        | Target::ExploreType(_)
                        | Target::ExploreGenre
                        | Target::ExploreSort
                        | Target::ExploreSurprise
                )
            })
            .map(|control| (control.target.clone(), control.rect))
            .collect();
        line.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        line
    }

    /// The control of a line nearest to `x`.
    fn nearest_in(line: &[(Target, Rect)], x: f32) -> Option<Target> {
        line.iter()
            .min_by(|a, b| {
                (a.1.center().0 - x)
                    .abs()
                    .total_cmp(&(b.1.center().0 - x).abs())
            })
            .map(|(target, _)| target.clone())
    }

    /// The card of the grid's top row in view nearest to `x`.
    fn grid_nearest(&self, x: f32) -> Option<Target> {
        let explore = &self.feeds.explore;
        let start = explore.first_row() * EXPLORE_COLUMNS;
        (start..(start + EXPLORE_COLUMNS).min(explore.items.len()))
            .min_by(|a, b| {
                let distance =
                    |index: &usize| (card_rect(*index, explore.scroll).center().0 - x).abs();
                distance(a).total_cmp(&distance(b))
            })
            .map(Target::ExploreItem)
    }

    /// Remote navigation on the Explore page. None leaves the move to the usual routes (the
    /// tabs).
    pub fn explore_direction(
        &mut self,
        direction: Direction,
        controls: &[Control],
    ) -> Option<Vec<Effect>> {
        // An open dropdown takes Up and Down; Left and Right do nothing in it.
        if let Some(dropdown) = &mut self.feeds.explore.dropdown {
            let count = dropdown.labels.len();
            let index = match self.focus {
                Target::ExploreOption(index) => index,
                _ => dropdown.chosen,
            };
            let next = match direction {
                Direction::Up => index.saturating_sub(1),
                Direction::Down => (index + 1).min(count.saturating_sub(1)),
                _ => index,
            };
            if next < dropdown.scroll {
                dropdown.scroll = next;
            } else if next >= dropdown.scroll + MENU_SHOWN {
                dropdown.scroll = next + 1 - MENU_SHOWN;
            }
            self.focus = Target::ExploreOption(next);
            self.pointer = false;
            self.dirty = true;
            return Some(vec![]);
        }
        let reduced_motion = self.preferences.reduced_motion;
        let bar = Self::explore_bar(controls);
        let count = self.feeds.explore.items.len();
        let x = controls
            .iter()
            .find(|control| control.target == self.focus)
            .map_or(EXPLORE_GRID_X, |control| control.rect.center().0);
        let action = controls
            .iter()
            .find(|control| matches!(control.target, Target::ExploreClear | Target::ExploreRetry))
            .map(|control| control.target.clone());
        let next = match self.focus.clone() {
            target @ (Target::ExploreSource
            | Target::ExploreType(_)
            | Target::ExploreGenre
            | Target::ExploreSort
            | Target::ExploreSurprise) => {
                let position = bar.iter().position(|(other, _)| *other == target)?;
                match direction {
                    Direction::Left if position > 0 => bar[position - 1].0.clone(),
                    Direction::Right if position + 1 < bar.len() => bar[position + 1].0.clone(),
                    Direction::Up => Target::Nav(EXPLORE_PAGE),
                    Direction::Down => match self.grid_nearest(x).or(action) {
                        Some(next) => next,
                        None => return Some(vec![]),
                    },
                    _ => return Some(vec![]),
                }
            }
            Target::ExploreItem(index) => {
                let column = index % EXPLORE_COLUMNS;
                match direction {
                    Direction::Left if column > 0 => Target::ExploreItem(index - 1),
                    Direction::Right if column + 1 < EXPLORE_COLUMNS && index + 1 < count => {
                        Target::ExploreItem(index + 1)
                    }
                    Direction::Up if index >= EXPLORE_COLUMNS => {
                        Target::ExploreItem(index - EXPLORE_COLUMNS)
                    }
                    Direction::Up => Self::nearest_in(&bar, x).unwrap_or(Target::ExploreSource),
                    Direction::Down if (index / EXPLORE_COLUMNS + 1) * EXPLORE_COLUMNS < count => {
                        Target::ExploreItem((index + EXPLORE_COLUMNS).min(count - 1))
                    }
                    _ => return Some(vec![]),
                }
            }
            Target::ExploreClear | Target::ExploreRetry => match direction {
                Direction::Up => Self::nearest_in(&bar, x).unwrap_or(Target::ExploreSource),
                _ => return Some(vec![]),
            },
            _ => return None,
        };
        if let Target::ExploreItem(index) = next {
            let target = (index / EXPLORE_COLUMNS) as f32 * EXPLORE_ROW;
            self.feeds.explore.scroll_to(target, reduced_motion);
        }
        self.focus(next, controls);
        Some(vec![])
    }

    /// Opens a dropdown under its pill, on the choice in effect.
    fn open_dropdown(&mut self, anchor: Target) {
        let feeds = &self.feeds;
        let query = feeds.explore.query;
        let (choices, labels, chosen) = match anchor {
            Target::ExploreSource => {
                let categories = feeds.explore_categories();
                let labels = categories
                    .iter()
                    .map(|category| feeds.category_title(*category))
                    .collect();
                let chosen = categories
                    .iter()
                    .position(|category| *category == feeds.explore.category)
                    .unwrap_or(0);
                (Choices::Source(categories), labels, chosen)
            }
            Target::ExploreGenre => {
                let mut genres: Vec<Option<usize>> = vec![None];
                genres.extend(
                    query
                        .genres()
                        .iter()
                        .enumerate()
                        .filter(|(_, genre)| query.offers(genre))
                        .map(|(index, _)| Some(index)),
                );
                let labels = genres
                    .iter()
                    .map(|genre| match genre {
                        Some(index) => query.genres()[*index].name.to_owned(),
                        None => "Any genre".to_owned(),
                    })
                    .collect();
                let chosen = genres
                    .iter()
                    .position(|genre| *genre == query.genre)
                    .unwrap_or(0);
                (Choices::Genre(genres), labels, chosen)
            }
            _ => {
                let labels = Sort::ALL
                    .iter()
                    .map(|sort| sort.label().to_owned())
                    .collect();
                let chosen = Sort::ALL
                    .iter()
                    .position(|sort| *sort == query.sort)
                    .unwrap_or(0);
                (Choices::Sort(Sort::ALL.to_vec()), labels, chosen)
            }
        };
        self.feeds.explore.dropdown = Some(Dropdown {
            choices,
            labels,
            chosen,
            scroll: chosen.saturating_sub(MENU_SHOWN / 2),
            opened: Instant::now(),
            anchor,
        });
        self.focus = Target::ExploreOption(chosen);
        self.pointer = false;
        self.dirty = true;
    }

    /// Back with a dropdown open closes it. False when none is.
    pub fn explore_back(&mut self) -> bool {
        match self.feeds.explore.dropdown.take() {
            Some(dropdown) => {
                self.focus = dropdown.anchor;
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// OK on the Explore page. None when focus is not on one of its controls.
    pub fn explore_activate(&mut self) -> Option<Vec<Effect>> {
        if self.page != Page::Explore {
            return None;
        }
        Some(match self.focus.clone() {
            anchor @ (Target::ExploreSource | Target::ExploreGenre | Target::ExploreSort) => {
                self.open_dropdown(anchor);
                vec![]
            }
            Target::ExploreOption(index) => {
                let dropdown = self.feeds.explore.dropdown.take()?;
                self.focus = dropdown.anchor.clone();
                match dropdown.choices {
                    Choices::Source(categories) => {
                        if let Some(category) = categories.get(index).copied() {
                            self.explore_select(category);
                        }
                    }
                    Choices::Genre(genres) => {
                        self.feeds.explore.query.genre = genres.get(index).copied().flatten();
                        self.explore_filtered();
                    }
                    Choices::Sort(sorts) => {
                        if let Some(sort) = sorts.get(index).copied() {
                            self.feeds.explore.query.sort = sort;
                        }
                        self.explore_filtered();
                    }
                }
                vec![]
            }
            Target::ExploreType(index) => {
                let kind = Kind::ALL[index.min(Kind::ALL.len() - 1)];
                let query = &mut self.feeds.explore.query;
                // Anime has its own genres: switching to or from it clears the genre, as
                // does a genre this type lacks.
                if (kind == Kind::Anime) != (query.kind == Kind::Anime) {
                    query.genre = None;
                }
                query.kind = kind;
                if let Some(genre) = query.genre.and_then(|index| query.genres().get(index))
                    && !query.offers(genre)
                {
                    query.genre = None;
                }
                self.explore_filtered();
                vec![]
            }
            Target::ExploreClear => {
                let explore = &mut self.feeds.explore;
                explore.query.kind = Kind::All;
                explore.query.genre = None;
                self.explore_filtered();
                self.focus = Target::ExploreType(0);
                vec![]
            }
            Target::ExploreRetry => {
                let explore = &mut self.feeds.explore;
                explore.error = None;
                explore.asked = None;
                self.focus = Target::ExploreSource;
                vec![]
            }
            // Surprise Me: a title picked at random from the grid, its page opened.
            Target::ExploreSurprise if !self.launching => {
                let count = self.feeds.explore.items.len();
                if count == 0 {
                    return Some(vec![]);
                }
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |time| {
                        time.subsec_nanos() as usize ^ time.as_secs() as usize
                    });
                let index = seed.wrapping_mul(2654435761) % count;
                let target = (index / EXPLORE_COLUMNS) as f32 * EXPLORE_ROW;
                let reduced_motion = self.preferences.reduced_motion;
                self.feeds.explore.scroll_to(target, reduced_motion);
                self.focus = Target::ExploreItem(index);
                let item = self.feeds.explore.items[index].clone();
                self.select_card(item)
            }
            Target::ExploreItem(index) if !self.launching => {
                match self.feeds.explore.items.get(index).cloned() {
                    Some(item) => self.select_card(item),
                    None => vec![],
                }
            }
            Target::ExploreSurprise | Target::ExploreItem(_) => vec![],
            _ => return None,
        })
    }

    /// The pointer's wheel over Explore: the grid (or an open dropdown) scrolls a row at a
    /// time.
    pub fn explore_wheel(&mut self, y: i32) {
        let reduced_motion = self.preferences.reduced_motion;
        let explore = &mut self.feeds.explore;
        if let Some(dropdown) = &mut explore.dropdown {
            let max = dropdown.labels.len().saturating_sub(MENU_SHOWN);
            dropdown.scroll = if y > 0 {
                dropdown.scroll.saturating_sub(1)
            } else {
                (dropdown.scroll + 1).min(max)
            };
        } else {
            let rows = explore.items.len().div_ceil(EXPLORE_COLUMNS);
            let max = rows.saturating_sub(1) as f32 * EXPLORE_ROW;
            let target = (explore.scroll - y as f32 * EXPLORE_ROW).clamp(0., max);
            let target = (target / EXPLORE_ROW).round() * EXPLORE_ROW;
            explore.scroll_to(target, reduced_motion);
        }
        self.pointer = true;
        self.dirty = true;
    }
}
