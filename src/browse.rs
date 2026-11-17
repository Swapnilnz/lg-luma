//! Pages over Home beside title pages: a "Browse by Platform" page (what one streaming
//! service has, movies or shows, the focused title large above them) and a person's page
//! (a director, writer or actor, and what they made).
use super::explore::{BACKDROP_FADE, Kind, Query};
use super::{
    Back, CARD_HEIGHT, CARD_PITCH, CARD_WIDTH, Feeds, Item, Outgoing, PAGE_ENTRANCE, Person,
    PersonInfo, SECTION_HEADER, TITLE_LEFT, TITLE_RIGHT,
};
use crate::state::{Direction, Effect, Sound, State, Target};
use std::time::{Duration, Instant};

/// The platform page's kinds, as its pills show them.
pub const PLATFORM_KINDS: [Kind; 2] = [Kind::Movies, Kind::Shows];
/// Where the platform page's row of posters starts.
pub const PLATFORM_ROW_Y: f32 = 712.;
/// Where a person's rows of titles start, and the distance between them.
pub const PERSON_ROWS_Y: f32 = 560.;
pub const PERSON_ROW_PITCH: f32 = SECTION_HEADER + CARD_HEIGHT + 56.;

/// A Browse by Platform page, over Home.
pub struct PlatformPage {
    /// Into `platforms::PLATFORMS`.
    pub platform: usize,
    /// Movies or Shows.
    pub kind: Kind,
    /// The title shown large: the one last focused in the row.
    pub hero: usize,
    /// The row's horizontal scroll.
    pub scroll: f32,
    /// The search last sent for this page, and its failure.
    pub asked: Option<(u64, Query)>,
    pub error: Option<String>,
    /// Home's focus and scroll, for Back.
    pub restore: Target,
    pub home_scroll: f32,
    pub opened: Instant,
    /// The hero's backdrop, the one before it (crossfading out), and since when.
    pub backdrop: Option<String>,
    pub previous: Option<String>,
    pub backdrop_since: Instant,
}

impl PlatformPage {
    /// The TMDB Discover search behind the row: popular titles of the kind on the service.
    pub fn query(&self) -> Query {
        Query {
            kind: self.kind,
            provider: Some(self.platform),
            ..Query::default()
        }
    }

    pub fn platform(&self) -> Option<&'static super::platforms::Platform> {
        super::platforms::get(self.platform)
    }

    /// How far the page has come in, 0 to 1.
    pub fn entrance(&self, reduced_motion: bool) -> f32 {
        if reduced_motion {
            return 1.;
        }
        let t = (self.opened.elapsed().as_secs_f32() / PAGE_ENTRANCE.as_secs_f32()).min(1.);
        1. - (1. - t).powi(3)
    }

    /// How far the hero's backdrop has faded in over the one before, 0 to 1.
    pub fn backdrop_fade(&self, reduced_motion: bool) -> f32 {
        if reduced_motion || self.previous.is_none() {
            return 1.;
        }
        (self.backdrop_since.elapsed().as_secs_f32() / BACKDROP_FADE.as_secs_f32()).min(1.)
    }
}

/// A person's page, over a title page's place.
pub struct PersonPage {
    /// Who, as the title page knew them: name and photo until their page arrives.
    pub person: Person,
    pub info: Option<Result<PersonInfo, String>>,
    /// Each row's horizontal scroll, and the page's.
    pub scroll: Vec<f32>,
    pub page_scroll: f32,
    /// Where Back finally lands, and the pages before this one.
    pub restore: Target,
    pub back: Vec<Back>,
    pub opened: Instant,
}

impl PersonPage {
    pub fn info(&self) -> Option<&PersonInfo> {
        self.info.as_ref()?.as_ref().ok()
    }

    pub fn rows(&self) -> &[(String, Vec<Item>)] {
        self.info().map_or(&[][..], |info| info.rows.as_slice())
    }

    pub fn entrance(&self, reduced_motion: bool) -> f32 {
        if reduced_motion {
            return 1.;
        }
        let t = (self.opened.elapsed().as_secs_f32() / PAGE_ENTRANCE.as_secs_f32()).min(1.);
        1. - (1. - t).powi(3)
    }

    /// The top of row `row`, unscrolled.
    pub fn row_y(row: usize) -> f32 {
        PERSON_ROWS_Y + row as f32 * PERSON_ROW_PITCH
    }
}

/// Scrolls a row so card `index` sits fully inside the page's width.
fn reveal(scroll: &mut f32, index: usize) {
    let x = index as f32 * CARD_PITCH;
    let visible = TITLE_RIGHT - TITLE_LEFT;
    if x < *scroll {
        *scroll = x;
    } else if x + CARD_WIDTH > *scroll + visible {
        *scroll = x + CARD_WIDTH - visible;
    }
}

/// Posters either side of a row's view that are kept, and how many in all.
fn window(scroll: f32, count: usize) -> std::ops::Range<usize> {
    let first = ((scroll / CARD_PITCH) as usize).saturating_sub(3);
    first..(first + 14).min(count)
}

impl Feeds {
    /// Whether a page lies over Home: a title's, a person's or a platform's.
    pub fn page_open(&self) -> bool {
        self.title.is_some() || self.person.is_some() || self.platform.is_some()
    }

    /// The open platform page's titles, once found.
    pub fn platform_items(&self) -> Vec<&Item> {
        self.platform
            .as_ref()
            .and_then(|page| self.explore.found.get(&page.query()))
            .map(|found| found.iter().map(|found| &found.item).collect())
            .unwrap_or_default()
    }

    /// The title the platform page shows large.
    pub fn platform_hero(&self) -> Option<&Item> {
        let page = self.platform.as_ref()?;
        self.platform_items().get(page.hero).copied()
    }

    /// A title in a person's rows.
    pub fn person_credit(&self, row: usize, index: usize) -> Option<&Item> {
        self.person.as_ref()?.rows().get(row)?.1.get(index)
    }

    /// The platform page's posters near its view, for the poster worker.
    pub fn platform_posters(&self) -> Vec<String> {
        let Some(page) = &self.platform else {
            return Vec::new();
        };
        let items = self.platform_items();
        items[window(page.scroll, items.len())]
            .iter()
            .filter_map(|item| item.image.clone())
            .collect()
    }

    /// Artwork the platform and person pages show: posters near their view, the hero's
    /// backdrops, a person's photo.
    pub fn browse_images(&self) -> Vec<&String> {
        let mut images: Vec<&String> = Vec::new();
        if let Some(page) = &self.platform {
            let items = self.platform_items();
            images.extend(
                items[window(page.scroll, items.len())]
                    .iter()
                    .filter_map(|item| item.image.as_ref()),
            );
            images.extend(page.backdrop.iter().chain(page.previous.iter()));
        }
        if let Some(page) = &self.person {
            images.extend(page.person.image.iter());
            if let Some(info) = page.info() {
                images.extend(info.image.iter());
                for (row, (_, items)) in info.rows.iter().enumerate() {
                    let scroll = page.scroll.get(row).copied().unwrap_or(0.);
                    images.extend(
                        items[window(scroll, items.len())]
                            .iter()
                            .filter_map(|item| item.image.as_ref()),
                    );
                }
            }
        }
        images
    }

    /// Whether a platform or person page is still moving.
    pub fn browse_animating(&self) -> bool {
        let settling = |opened: Instant, extra: Duration| {
            opened.elapsed() < PAGE_ENTRANCE + extra + Duration::from_millis(40)
        };
        self.platform.as_ref().is_some_and(|page| {
            settling(page.opened, Duration::ZERO)
                || page.previous.is_some() && settling(page.backdrop_since, BACKDROP_FADE)
        }) || self
            .person
            .as_ref()
            .is_some_and(|page| settling(page.opened, Duration::ZERO))
    }
}

impl State {
    /// A Browse by Platform tile: that service's page over Home, on its movies.
    pub fn open_platform_page(&mut self, platform: usize) -> Vec<Effect> {
        if super::platforms::get(platform).is_none() {
            return vec![];
        }
        if !self.feeds.can_search {
            self.notify("Browse by Platform needs a TMDB key in feeds.json");
            return vec![];
        }
        self.feeds.preview = None;
        self.feeds.menu = None;
        self.feeds.platform = Some(PlatformPage {
            platform,
            kind: PLATFORM_KINDS[0],
            hero: 0,
            scroll: 0.,
            asked: None,
            error: None,
            restore: self.focus.clone(),
            home_scroll: self.scroll,
            opened: Instant::now(),
            backdrop: None,
            previous: None,
            backdrop_since: Instant::now() - BACKDROP_FADE,
        });
        self.focus = Target::PlatformItem(0);
        self.sound = Some(Sound::Select);
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// Back on a platform page: Home, on the tile it opened from.
    pub fn close_platform(&mut self) {
        let Some(page) = self.feeds.platform.take() else {
            return;
        };
        self.focus = page.restore;
        self.scroll = page.home_scroll;
        self.feeds.prune_images();
        self.reconcile_focus();
        self.pointer = false;
        self.dirty = true;
    }

    /// Once a frame with a platform page open: its search, and the hero's backdrop.
    pub fn platform_tick(&mut self) {
        let can_search = self.feeds.can_search;
        let Some(page) = self.feeds.platform.as_mut() else {
            return;
        };
        let query = page.query();
        let explore = &mut self.feeds.explore;
        if can_search
            && !explore.found.contains_key(&query)
            && page.asked.is_none_or(|(_, asked)| asked != query)
            && page.error.is_none()
        {
            explore.requests += 1;
            page.asked = Some((explore.requests, query));
            self.feeds
                .outbox
                .push(Outgoing::Discover(explore.requests, query));
        }
        // The hero's backdrop, once it is here; the last one stays until then.
        let count = self.feeds.platform_items().len();
        if let Some(page) = self.feeds.platform.as_mut()
            && page.hero >= count
            && count > 0
        {
            page.hero = count - 1;
        }
        let url = self
            .feeds
            .platform_hero()
            .and_then(|item| self.feeds.details.get(&item.link))
            .and_then(|details| details.backdrop.clone())
            .filter(|url| self.feeds.images.contains_key(url));
        let Some(page) = self.feeds.platform.as_mut() else {
            return;
        };
        if let Some(url) = url
            && page.backdrop.as_ref() != Some(&url)
        {
            page.previous = page.backdrop.replace(url);
            page.backdrop_since = Instant::now();
            self.dirty = true;
        }
        if page.previous.is_some() && page.backdrop_since.elapsed() >= BACKDROP_FADE {
            page.previous = None;
            self.feeds.prune_images();
            self.dirty = true;
        }
    }

    /// A search for a platform page answered (or failed).
    pub fn platform_discovered(&mut self, id: u64, error: Option<String>) {
        if let Some(page) = self.feeds.platform.as_mut()
            && page.asked.is_some_and(|(asked, _)| asked == id)
        {
            page.error = error;
            self.dirty = true;
        }
    }

    /// Remote navigation on a platform page: the pills, Play and Details, and the row, top
    /// to bottom. Moving along the row brings each title up large.
    pub fn platform_direction(&mut self, direction: Direction) -> Vec<Effect> {
        let count = self.feeds.platform_items().len();
        let Some(page) = self.feeds.platform.as_mut() else {
            return vec![];
        };
        let kind = PLATFORM_KINDS
            .iter()
            .position(|kind| *kind == page.kind)
            .unwrap_or(0);
        let next = match (self.focus.clone(), direction) {
            (Target::PlatformKind(index), Direction::Left) => {
                Target::PlatformKind(index.saturating_sub(1))
            }
            (Target::PlatformKind(index), Direction::Right) => {
                Target::PlatformKind((index + 1).min(PLATFORM_KINDS.len() - 1))
            }
            (Target::PlatformKind(_), Direction::Down) if count > 0 => Target::PlatformPlay,
            (Target::PlatformKind(index), _) => Target::PlatformKind(index),
            (Target::PlatformPlay, Direction::Right) => Target::PlatformDetails,
            (Target::PlatformDetails, Direction::Left) => Target::PlatformPlay,
            (Target::PlatformPlay | Target::PlatformDetails, Direction::Up) => {
                Target::PlatformKind(kind)
            }
            (Target::PlatformPlay | Target::PlatformDetails, Direction::Down) => {
                Target::PlatformItem(page.hero.min(count.saturating_sub(1)))
            }
            (target @ (Target::PlatformPlay | Target::PlatformDetails), _) => target,
            (Target::PlatformItem(_), Direction::Up) => Target::PlatformPlay,
            (Target::PlatformItem(index), Direction::Left) => {
                Target::PlatformItem(index.saturating_sub(1))
            }
            (Target::PlatformItem(index), Direction::Right) => {
                Target::PlatformItem((index + 1).min(count.saturating_sub(1)))
            }
            (Target::PlatformItem(index), _) => Target::PlatformItem(index),
            // Focus elsewhere (a row still loading): back on the pills.
            _ => Target::PlatformKind(kind),
        };
        let next = match next {
            // Nothing to show yet: the pills keep focus.
            Target::PlatformItem(_) | Target::PlatformPlay | Target::PlatformDetails
                if count == 0 =>
            {
                Target::PlatformKind(kind)
            }
            next => next,
        };
        if let Target::PlatformItem(index) = next {
            page.hero = index;
            reveal(&mut page.scroll, index);
        }
        self.focus = next;
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// OK on a platform page. None when none is open.
    pub fn platform_activate(&mut self) -> Option<Vec<Effect>> {
        self.feeds.platform.as_ref()?;
        Some(match self.focus.clone() {
            Target::PlatformKind(index) => {
                let kind = PLATFORM_KINDS[index.min(PLATFORM_KINDS.len() - 1)];
                if let Some(page) = self.feeds.platform.as_mut()
                    && page.kind != kind
                {
                    page.kind = kind;
                    page.hero = 0;
                    page.scroll = 0.;
                    page.error = None;
                    self.feeds.prune_images();
                }
                vec![]
            }
            Target::PlatformPlay if !self.launching => self
                .feeds
                .platform_hero()
                .cloned()
                .map(|item| vec![Effect::LaunchTitle(item)])
                .unwrap_or_default(),
            Target::PlatformDetails => {
                if let Some(item) = self.feeds.platform_hero().cloned() {
                    self.open_title(item, Target::PlatformDetails);
                }
                vec![]
            }
            Target::PlatformItem(index) if !self.launching => {
                if let Some(page) = self.feeds.platform.as_mut() {
                    page.hero = index;
                }
                match self
                    .feeds
                    .platform_items()
                    .get(index)
                    .map(|item| (*item).clone())
                {
                    Some(item) => self.select_card(item),
                    None => vec![],
                }
            }
            _ => vec![],
        })
    }

    /// Opens a person's page, asking for it; `back` holds the pages it was opened over.
    pub fn open_person(&mut self, person: Person, restore: Target, back: Vec<Back>) {
        self.feeds.menu = None;
        self.feeds.title = None;
        self.feeds.outbox.push(Outgoing::Person(person.id));
        self.feeds.person = Some(PersonPage {
            person,
            info: None,
            scroll: Vec::new(),
            page_scroll: 0.,
            restore,
            back,
            opened: Instant::now(),
        });
        self.focus = Target::PersonCredit(0, 0);
        self.pointer = false;
        self.dirty = true;
    }

    /// A person's page arrived from the title worker.
    pub fn person_arrived(&mut self, id: u64, result: Result<PersonInfo, String>) {
        let Some(page) = self
            .feeds
            .person
            .as_mut()
            .filter(|page| page.person.id == id)
        else {
            return;
        };
        if let Err(error) = &result {
            eprintln!("Luma feeds: person {id}: {error}");
        }
        page.info = Some(result);
        page.scroll = vec![0.; page.rows().len()];
        self.reveal_person_focus();
        self.dirty = true;
    }

    /// Keeps focus on a person's page on a credit there is, and in view.
    pub(crate) fn reveal_person_focus(&mut self) {
        let Some(page) = self.feeds.person.as_mut() else {
            return;
        };
        let Target::PersonCredit(row, index) = self.focus else {
            return;
        };
        let rows = page.rows();
        if rows.is_empty() {
            return;
        }
        let row = row.min(rows.len() - 1);
        let index = index.min(rows[row].1.len().saturating_sub(1));
        if page.scroll.len() < rows.len() {
            page.scroll.resize(rows.len(), 0.);
        }
        reveal(&mut page.scroll[row], index);
        let bottom = PersonPage::row_y(row) + SECTION_HEADER + CARD_HEIGHT + 48.;
        page.page_scroll = (bottom - 1080.).max(0.);
        self.focus = Target::PersonCredit(row, index);
    }

    /// Remote navigation on a person's page: along a row, and between rows.
    pub fn person_direction(&mut self, direction: Direction) -> Vec<Effect> {
        let Some(page) = self.feeds.person.as_ref() else {
            return vec![];
        };
        let rows = page.rows();
        let Target::PersonCredit(row, index) = self.focus else {
            self.focus = Target::PersonCredit(0, 0);
            self.reveal_person_focus();
            return vec![];
        };
        if rows.is_empty() {
            return vec![];
        }
        let scroll = |row: usize| page.scroll.get(row).copied().unwrap_or(0.);
        // Between rows, the title nearest where focus was across the screen.
        let across = |to: usize| {
            let x = index as f32 * CARD_PITCH - scroll(row) + scroll(to);
            ((x / CARD_PITCH).round().max(0.) as usize).min(rows[to].1.len().saturating_sub(1))
        };
        let (row, index) = match direction {
            Direction::Left => (row, index.saturating_sub(1)),
            Direction::Right => (row, (index + 1).min(rows[row].1.len().saturating_sub(1))),
            Direction::Up if row > 0 => (row - 1, across(row - 1)),
            Direction::Down if row + 1 < rows.len() => (row + 1, across(row + 1)),
            _ => (row, index),
        };
        self.focus = Target::PersonCredit(row, index);
        self.reveal_person_focus();
        self.pointer = false;
        self.dirty = true;
        vec![]
    }

    /// OK on a person's page: the title's page, over this one. None when none is open.
    pub fn person_activate(&mut self) -> Option<Vec<Effect>> {
        self.feeds.person.as_ref()?;
        if let Target::PersonCredit(row, index) = self.focus
            && let Some(item) = self.feeds.person_credit(row, index).cloned()
        {
            self.person_open_credit(item);
        }
        Some(vec![])
    }

    /// Opens a title from a person's page; Back returns to the person, on that title.
    pub(crate) fn person_open_credit(&mut self, item: Item) {
        let Some(page) = self.feeds.person.take() else {
            return;
        };
        let mut back = page.back;
        back.push(Back::Person(page.person, self.focus.clone()));
        self.open_title(item, page.restore);
        if let Some(title) = &mut self.feeds.title {
            title.back = back;
        }
        self.sound = Some(Sound::Select);
    }

    /// Back on a person's page: the page it was opened over.
    pub fn person_back(&mut self) {
        let Some(page) = self.feeds.person.take() else {
            return;
        };
        self.page_back(page.back, page.restore);
    }
}
