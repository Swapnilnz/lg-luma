//! Home rows from Trakt and MDBList whose cards open a title in another app (Nuvio) through a
//! `contentTarget` deep link. All HTTP, decoding and sign-in run in `feeds_service`; this module
//! only holds what Home draws, each row's horizontal scroll, and the remote navigation between
//! rows.
use crate::app_artwork::Artwork;
use crate::state::{
    Control, Direction, Effect, HOME_STRIP_LEFT, HOME_STRIP_RIGHT, HOME_TILE_PITCH,
    HOME_TILE_WIDTH, Rect, State, Target,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

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
}

impl Feeds {
    /// Artwork URLs of the rows near the screen.
    pub fn wanted_images(&self) -> HashSet<&String> {
        self.near
            .iter()
            .filter_map(|row| self.rows.get(*row))
            .flat_map(|row| &row.items)
            .filter_map(|item| item.image.as_ref())
            .collect()
    }

    /// Drops artwork no nearby row shows.
    pub fn prune_images(&mut self) {
        let wanted: HashSet<String> = self.wanted_images().into_iter().cloned().collect();
        self.images.retain(|url, _| wanted.contains(url));
    }

    /// The rows whose cards lie within about two rows of the screen, from the last scene.
    pub fn near_rows(controls: &[Control]) -> BTreeSet<usize> {
        controls
            .iter()
            .filter_map(|control| match control.target {
                Target::Feed(row, _) if control.rect.y > -700. && control.rect.y < 1800. => {
                    Some(row)
                }
                _ => None,
            })
            .collect()
    }

    fn max_scroll(&self, row: usize) -> f32 {
        let count = self.rows.get(row).map_or(0, |row| row.items.len());
        if count == 0 {
            return 0.;
        }
        let content = (count - 1) as f32 * HOME_TILE_PITCH + HOME_TILE_WIDTH;
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
                let next = nearest_feed(controls, x, |_| true, false)?;
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
