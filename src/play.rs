//! The Play page: the PC's recently played games (from the console
//! agent, cached on the TV), with box art, stats and details, and a moving backdrop of the
//! focused game's screenshots. Picking a game wakes the PC, readies it and launches that
//! game through Hydra; "Open Hydra" readies it without a game (the console tile does too).
//!
//! Only shown with a `console` block in preferences.json.

use crate::app_artwork::Artwork;
use crate::console::{ArtKind, Game, Library};
use crate::state::{Direction, Effect, Page, State, Target};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The Play tab's page index (Nav target): after Explore's, so the others keep theirs.
pub const PLAY_PAGE: usize = 4;
/// Focus rests this long on a game before the backdrop changes to it (and its screenshots
/// load), so moving along the rail does not thrash.
pub const DWELL: Duration = Duration::from_millis(350);
/// Each screenshot of the moving backdrop shows this long, then crossfades.
pub const SHOT_SLIDE: Duration = Duration::from_secs(6);
pub const SHOT_FADE: Duration = Duration::from_millis(900);
/// The info panel's entrance after focus moves.
pub const PANEL_IN: Duration = Duration::from_millis(220);
/// The focused cover's spring.
pub const SPRING: Duration = Duration::from_millis(280);
/// The details sheet's entrance.
pub const SHEET_IN: Duration = Duration::from_millis(320);

/// Cover rail geometry (1920 x 1080 layout).
pub const RAIL_X: f32 = 120.;
pub const RAIL_Y: f32 = 640.;
pub const COVER_W: f32 = 236.;
pub const COVER_H: f32 = 354.;
pub const COVER_GAP: f32 = 30.;
pub const COVER_PITCH: f32 = COVER_W + COVER_GAP;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum PlayButton {
    Play,
    Details,
    Hydra,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub key: String,
    pub since: Instant,
}

#[derive(Clone, Debug)]
pub struct Sheet {
    pub key: String,
    pub opened: Instant,
    /// The screenshot shown as the backdrop, when one is focused.
    pub shot: Option<usize>,
}

#[derive(Default)]
pub struct Play {
    /// The games shown: the PC's library less the hidden ones (unless they are shown).
    pub library: Option<Library>,
    /// Everything the PC sent.
    all: Option<Library>,
    /// Hidden games' keys (the preferences' `hidden_games`).
    pub hidden: HashSet<String>,
    /// The hidden games are on the rail too, marked, to unhide.
    pub show_hidden: bool,
    /// Decoded art by game key and kind.
    pub art: HashMap<(String, ArtKind), Arc<Artwork>>,
    /// Asked for (or failed while the PC was asleep): not asked again until it wakes.
    requested: HashSet<(String, ArtKind)>,
    /// The focused game on the rail.
    pub index: usize,
    pub focused_at: Option<Instant>,
    /// The game that was focused before, shrinking back.
    pub previous: Option<(usize, Instant)>,
    /// The game behind the page (after DWELL), and the one before it, fading out.
    pub preview: Option<Preview>,
    /// The outgoing preview, and when it began fading.
    pub previous_preview: Option<(Preview, Instant)>,
    pub sheet: Option<Sheet>,
    /// The rail's scroll, gliding to `rail_target`.
    pub rail: f32,
    rail_tick: Option<Instant>,
    /// The backdrop's screenshot number, for redrawing on each change with Reduce motion.
    slide_step: usize,
}

impl Play {
    pub fn games(&self) -> &[Game] {
        self.library.as_ref().map_or(&[], |library| &library.games)
    }

    pub fn game(&self, index: usize) -> Option<&Game> {
        self.games().get(index)
    }

    pub fn find(&self, key: &str) -> Option<&Game> {
        self.games().iter().find(|game| game.key == key)
    }

    pub fn image(&self, key: &str, kind: ArtKind) -> Option<&Arc<Artwork>> {
        self.art.get(&(key.to_owned(), kind))
    }

    /// Where the rail scrolls for the focused game: it stays in the left third once the
    /// rail is longer than the screen.
    pub fn rail_target(&self) -> f32 {
        let count = self.games().len() + usize::from(self.hidden_count() > 0);
        let visible = ((1920. - RAIL_X) / COVER_PITCH).floor() as usize;
        if count <= visible {
            return 0.;
        }
        let max = (count - visible) as f32 * COVER_PITCH + COVER_PITCH * 0.5;
        (self.index.saturating_sub(1) as f32 * COVER_PITCH).min(max)
    }

    /// The cover's scale: springing up when focused (a little past 1.12, then back), easing
    /// down when focus leaves.
    pub fn cover_scale(&self, index: usize, focused: bool, now: Instant, reduced: bool) -> f32 {
        const UP: f32 = 0.12;
        if reduced {
            return if focused { 1. + UP } else { 1. };
        }
        if focused {
            let t = self
                .focused_at
                .map_or(1., |at| {
                    now.duration_since(at).as_secs_f32() / SPRING.as_secs_f32()
                })
                .min(1.);
            return 1. + UP * ease_out_back(t);
        }
        match self.previous {
            Some((previous, at)) if previous == index => {
                let t = (now.duration_since(at).as_secs_f32() / 0.18).min(1.);
                1. + UP * (1. - ease_out_cubic(t))
            }
            _ => 1.,
        }
    }

    /// Covers for every game; the backdrop, logo and screenshots only for the previewed
    /// game (and the one being launched).
    pub fn wanted(&self, launching: Option<&str>) -> Vec<(Game, ArtKind)> {
        let mut wanted = Vec::new();
        let focus = self.preview.as_ref().map(|preview| preview.key.as_str());
        for (index, game) in self.games().iter().enumerate() {
            let near = Some(game.key.as_str()) == focus
                || Some(game.key.as_str()) == launching
                || (index == 0 && focus.is_none());
            let mut kinds = vec![ArtKind::Cover];
            // Without a cover, the poster is made from the backdrop and logo.
            if !game.has(ArtKind::Cover) {
                kinds.extend([ArtKind::Hero, ArtKind::Logo]);
            }
            if near {
                kinds.extend([ArtKind::Hero, ArtKind::Logo]);
                kinds.extend(game.shots());
            }
            for kind in kinds {
                let entry = (game.key.clone(), kind);
                if game.has(kind)
                    && !self.art.contains_key(&entry)
                    && !self.requested.contains(&entry)
                {
                    wanted.push((game.clone(), kind));
                }
            }
        }
        wanted
    }

    pub fn mark_requested(&mut self, key: &str, kind: ArtKind) {
        self.requested.insert((key.to_owned(), kind));
    }

    /// The PC woke: what failed while it slept may load now.
    pub fn retry_art(&mut self) {
        self.requested.clear();
    }

    /// Keeps covers, the first game's art and the previewed (or launching) game's art;
    /// drops other screenshots, logos and backdrops (their textures go too).
    pub fn evict(&mut self, launching: Option<&str>) {
        let keep: Vec<String> = self
            .preview
            .iter()
            .map(|preview| preview.key.clone())
            .chain(self.previous_preview.iter().map(|(old, _)| old.key.clone()))
            .chain(launching.map(str::to_owned))
            .chain(self.game(0).map(|game| game.key.clone()))
            .collect();
        let coverless: HashSet<String> = self
            .games()
            .iter()
            .filter(|game| !game.has(ArtKind::Cover))
            .map(|game| game.key.clone())
            .collect();
        let poster = |key: &String, kind: ArtKind| {
            kind == ArtKind::Cover
                || (coverless.contains(key) && matches!(kind, ArtKind::Hero | ArtKind::Logo))
        };
        self.art
            .retain(|(key, kind), _| poster(key, *kind) || keep.contains(key));
        let art = &self.art;
        self.requested.retain(|entry| {
            poster(&entry.0, entry.1) || keep.contains(&entry.0) || art.contains_key(entry)
        });
    }

    pub fn all(&self) -> Option<&Library> {
        self.all.as_ref()
    }

    pub fn is_hidden(&self, key: &str) -> bool {
        self.hidden.contains(key)
    }

    /// How many of the PC's games are hidden.
    pub fn hidden_count(&self) -> usize {
        self.all.as_ref().map_or(0, |library| {
            library
                .games
                .iter()
                .filter(|game| self.hidden.contains(&game.key))
                .count()
        })
    }

    pub fn set_library(&mut self, library: Library) {
        self.all = Some(library);
        self.refilter();
    }

    /// Recomputes the games shown after the library, the hidden set or `show_hidden` change.
    pub fn refilter(&mut self) {
        if self.hidden_count() == 0 {
            self.show_hidden = false;
        }
        let Some(mut library) = self.all.clone() else {
            return;
        };
        if !self.show_hidden {
            library
                .games
                .retain(|game| !self.hidden.contains(&game.key));
        }
        self.show(library);
    }

    fn show(&mut self, library: Library) {
        let focused = self.game(self.index).map(|game| game.key.clone());
        let keys: HashSet<String> = library.games.iter().map(|game| game.key.clone()).collect();
        self.art.retain(|(key, _), _| keys.contains(key));
        self.requested.retain(|(key, _)| keys.contains(key));
        self.library = Some(library);
        // Keep the same game focused when it is still there, else the one now in its place.
        self.index = focused
            .and_then(|key| self.games().iter().position(|game| game.key == key))
            .unwrap_or(self.index)
            .min(self.games().len().saturating_sub(1));
        if let Some(preview) = &self.preview
            && !keys.contains(&preview.key)
        {
            self.preview = None;
        }
    }

    pub fn animating(&self, now: Instant) -> bool {
        (self.rail - self.rail_target()).abs() > 0.5
            || self
                .focused_at
                .is_some_and(|at| now.duration_since(at) < DWELL.max(SPRING))
            || self
                .previous_preview
                .as_ref()
                .is_some_and(|(_, at)| now.duration_since(*at) < SHOT_FADE)
            || self
                .sheet
                .as_ref()
                .is_some_and(|sheet| now.duration_since(sheet.opened) < SHEET_IN)
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    let c1 = 1.70158;
    let c3 = c1 + 1.;
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}

// ---- Formatting ----

/// "41.3 h", "45 min", or "Not played yet".
pub fn hours(ms: f64, language: crate::i18n::Language) -> String {
    let minutes = ms / 60_000.;
    if minutes < 1. {
        language.t("playNotPlayed").into()
    } else if minutes < 60. {
        language.format(
            "playMinutes",
            &[("count", format!("{}", minutes.round() as u32).as_str())],
        )
    } else {
        let hours = minutes / 60.;
        let shown = if hours < 100. {
            format!("{:.1}", hours)
        } else {
            format!("{}", hours.round() as u32)
        };
        language.format("playHours", &[("count", shown.as_str())])
    }
}

/// "Played today", "Played 2 days ago", "Played 3 weeks ago", or None.
pub fn played_ago(
    last: Option<&str>,
    now: time::OffsetDateTime,
    language: crate::i18n::Language,
) -> Option<String> {
    let at = crate::console::parse_time(last?)?;
    let days = (now - at).whole_days().max(0);
    Some(match days {
        0 => language.t("playedToday").into(),
        1 => language.t("playedYesterday").into(),
        2..=13 => language.format("playedDaysAgo", &[("count", days.to_string().as_str())]),
        14..=59 => language.format(
            "playedWeeksAgo",
            &[("count", (days / 7).to_string().as_str())],
        ),
        _ => language.format(
            "playedMonthsAgo",
            &[("count", (days / 30).to_string().as_str())],
        ),
    })
}

/// Achievements as a fraction (0 to 1), when the game has any.
pub fn achievement_share(game: &Game) -> Option<f32> {
    (game.achievements.total > 0)
        .then(|| game.achievements.unlocked as f32 / game.achievements.total as f32)
}

/// "48.2 GB" / "820 MB".
pub fn size(bytes: Option<f64>) -> Option<String> {
    let bytes = bytes?;
    let gb = bytes / 1e9;
    Some(if gb >= 1. {
        format!("{:.1} GB", gb)
    } else {
        format!("{} MB", (bytes / 1e6).round() as u64)
    })
}

/// The release year from Steam's date ("24 Feb, 2022" or "Feb 2022").
pub fn year(date: &str) -> Option<&str> {
    date.split([' ', ','])
        .rev()
        .find(|part| part.len() == 4 && part.bytes().all(|b| b.is_ascii_digit()))
}

/// The chips under the title: genres, year, developer.
pub fn chips(game: &Game) -> Vec<String> {
    let mut chips: Vec<String> = game.genres.iter().take(2).cloned().collect();
    if let Some(year) = year(&game.release_date) {
        chips.push(year.into());
    }
    if !game.developer.is_empty() {
        chips.push(game.developer.clone());
    }
    chips
}

// ---- State ----

impl State {
    pub fn play_available(&self) -> bool {
        self.preferences.console.is_some()
    }

    /// The tabs as drawn, by page index: Home, Explore, (Play,) Apps, Settings.
    pub fn nav_order(&self) -> &'static [usize] {
        if self.play_available() {
            &[0, crate::state::EXPLORE_PAGE, PLAY_PAGE, 1, 2]
        } else {
            &crate::state::NAV_ORDER
        }
    }

    pub fn play_entry(&self) -> Target {
        if self.play.games().is_empty() && self.play.hidden_count() > 0 {
            Target::PlayHidden
        } else if self.play.games().is_empty() {
            Target::PlayAction(PlayButton::Hydra)
        } else {
            Target::PlayGame(self.play.index)
        }
    }

    fn play_buttons(&self) -> &'static [PlayButton] {
        if self.play.games().is_empty() {
            &[PlayButton::Hydra]
        } else {
            &[PlayButton::Play, PlayButton::Details, PlayButton::Hydra]
        }
    }

    pub fn set_console_library(&mut self, library: Library) {
        self.play.hidden = self.preferences.hidden_games.iter().cloned().collect();
        if self.play.all() == Some(&library) {
            return;
        }
        self.play.set_library(library);
        if let Target::PlayGame(index) = self.focus
            && index != self.play.index
        {
            self.focus = Target::PlayGame(self.play.index);
        }
        if let Some(sheet) = &self.play.sheet
            && self.play.find(&sheet.key).is_none()
        {
            self.play.sheet = None;
            self.focus = self.play_entry();
        }
        self.dirty = true;
    }

    pub fn set_console_art(&mut self, key: String, kind: ArtKind, image: Option<Artwork>) {
        match image {
            Some(image) if self.play.find(&key).is_some() => {
                self.play.art.insert((key, kind), Arc::new(image));
                self.dirty = true;
            }
            _ => {}
        }
    }

    /// The game being launched, if any (its art stays loaded).
    fn launching_key(&self) -> Option<String> {
        self.console
            .as_ref()
            .and_then(|session| session.game.as_ref())
            .map(|game| game.key.clone())
    }

    pub fn wanted_console_art(&mut self) -> Vec<(Game, ArtKind)> {
        let launching = self.launching_key();
        let wanted = self.play.wanted(launching.as_deref());
        for (game, kind) in &wanted {
            self.play.mark_requested(&game.key, *kind);
        }
        wanted
    }

    /// Once a frame: the rail glides, focus follows the pointer, and after DWELL the
    /// backdrop moves to the focused game.
    pub fn play_tick(&mut self) {
        let now = Instant::now();
        if let Target::PlayGame(index) = self.focus
            && index != self.play.index
            && index < self.play.games().len()
        {
            self.play_focus_game(index, now);
        }
        // Off the page, the first game's art loads, ready for when it opens.
        let wanted_key = match self.page {
            Page::Play => self
                .play
                .sheet
                .as_ref()
                .map(|sheet| sheet.key.clone())
                .or_else(|| {
                    self.play
                        .focused_at
                        .is_none_or(|at| now.duration_since(at) >= DWELL)
                        .then(|| self.play.game(self.play.index).map(|game| game.key.clone()))
                        .flatten()
                }),
            _ => self.play.game(0).map(|game| game.key.clone()),
        };
        if let Some(key) = wanted_key
            && self
                .play
                .preview
                .as_ref()
                .is_none_or(|preview| preview.key != key)
        {
            let previous = self.play.preview.take().map(|preview| (preview, now));
            self.play.previous_preview = if self.preferences.reduced_motion {
                None
            } else {
                previous
            };
            self.play.preview = Some(Preview { key, since: now });
            let launching = self.launching_key();
            self.play.evict(launching.as_deref());
            self.dirty = true;
        }
        let target = self.play.rail_target();
        let last = self.play.rail_tick.replace(now);
        if (self.play.rail - target).abs() > 0.5 {
            if self.preferences.reduced_motion {
                self.play.rail = target;
            } else {
                let dt = last.map_or(0.016, |at| now.duration_since(at).as_secs_f32().min(0.1));
                self.play.rail += (target - self.play.rail) * (1. - (-dt * 12.).exp());
            }
            self.dirty = true;
        } else {
            self.play.rail = target;
        }
        // Luma draws only when something changed: the moving backdrop changes every frame
        // (with Reduce motion, at each new screenshot).
        match self.page {
            Page::Play if !self.preferences.reduced_motion || self.play.animating(now) => {
                self.dirty = true;
            }
            Page::Play => {
                let step = self.play.preview.as_ref().map_or(0, |preview| {
                    (now.duration_since(preview.since).as_secs_f32() / SHOT_SLIDE.as_secs_f32())
                        as usize
                });
                if step != self.play.slide_step {
                    self.play.slide_step = step;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    pub fn play_animating(&self) -> bool {
        let now = Instant::now();
        match self.page {
            Page::Play => !self.preferences.reduced_motion || self.play.animating(now),
            _ => false,
        }
    }

    fn play_focus_game(&mut self, index: usize, now: Instant) {
        if index != self.play.index {
            self.play.previous = Some((self.play.index, now));
        }
        self.play.index = index;
        self.play.focused_at = Some(now);
        self.focus = Target::PlayGame(index);
        self.dirty = true;
    }

    /// Arrows on the Play page. None leaves the key to the generic handling (the tabs).
    pub fn play_direction(&mut self, direction: Direction) -> Option<Vec<Effect>> {
        if self.page != Page::Play {
            return None;
        }
        let now = Instant::now();
        let count = self.play.games().len();
        if self.play.sheet.is_some() {
            let shots = self
                .play
                .sheet
                .as_ref()
                .and_then(|sheet| self.play.find(&sheet.key))
                .map_or(0, |game| game.shots().len());
            match (self.focus.clone(), direction) {
                (Target::PlayDetailStart, Direction::Down) => self.focus = Target::PlayHide,
                (Target::PlayHide, Direction::Up) => self.focus = Target::PlayDetailStart,
                (Target::PlayDetailStart | Target::PlayHide, Direction::Right) if shots > 0 => {
                    self.set_sheet_shot(0);
                }
                (Target::PlayShot(0), Direction::Left) => {
                    self.focus = Target::PlayDetailStart;
                    if let Some(sheet) = &mut self.play.sheet {
                        sheet.shot = None;
                    }
                }
                (Target::PlayShot(index), Direction::Left) => self.set_sheet_shot(index - 1),
                (Target::PlayShot(index), Direction::Right) if index + 1 < shots => {
                    self.set_sheet_shot(index + 1);
                }
                (Target::PlayShot(_), Direction::Up) => {
                    self.focus = Target::PlayDetailStart;
                    if let Some(sheet) = &mut self.play.sheet {
                        sheet.shot = None;
                    }
                }
                _ => return Some(vec![]),
            }
            self.sound = Some(crate::state::Sound::Move);
            self.dirty = true;
            return Some(vec![]);
        }
        let buttons = self.play_buttons();
        let next = match (self.focus.clone(), direction) {
            (Target::Nav(PLAY_PAGE), Direction::Down) => Some(self.play_entry()),
            (Target::PlayGame(index), Direction::Left) if index > 0 => {
                self.play_focus_game(index - 1, now);
                None
            }
            (Target::PlayGame(index), Direction::Right) if index + 1 < count => {
                self.play_focus_game(index + 1, now);
                None
            }
            (Target::PlayGame(_), Direction::Right) if self.play.hidden_count() > 0 => {
                Some(Target::PlayHidden)
            }
            (Target::PlayGame(_) | Target::PlayHidden, Direction::Up) => {
                Some(Target::PlayAction(buttons[0]))
            }
            (Target::PlayGame(_), _) => return Some(vec![]),
            (Target::PlayHidden, Direction::Left) if count > 0 => {
                self.play_focus_game(count - 1, now);
                None
            }
            (Target::PlayHidden, _) => return Some(vec![]),
            (Target::PlayAction(button), Direction::Left | Direction::Right) => {
                let position = buttons.iter().position(|b| *b == button).unwrap_or(0);
                let position = match direction {
                    Direction::Left => position.saturating_sub(1),
                    _ => (position + 1).min(buttons.len() - 1),
                };
                Some(Target::PlayAction(buttons[position]))
            }
            (Target::PlayAction(_), Direction::Up) => Some(Target::Nav(PLAY_PAGE)),
            (Target::PlayAction(_), Direction::Down) if count > 0 => {
                Some(Target::PlayGame(self.play.index))
            }
            (Target::PlayAction(_), Direction::Down) if self.play.hidden_count() > 0 => {
                Some(Target::PlayHidden)
            }
            (Target::PlayAction(_), Direction::Down) => return Some(vec![]),
            _ => return None,
        };
        if let Some(next) = next {
            if next == self.focus {
                return Some(vec![]);
            }
            self.focus = next;
        }
        self.sound = Some(crate::state::Sound::Move);
        self.dirty = true;
        Some(vec![])
    }

    fn set_sheet_shot(&mut self, index: usize) {
        self.focus = Target::PlayShot(index);
        if let Some(sheet) = &mut self.play.sheet {
            sheet.shot = Some(index);
        }
    }

    pub fn open_play_sheet(&mut self) {
        let Some(game) = self.play.game(self.play.index) else {
            return;
        };
        self.play.sheet = Some(Sheet {
            key: game.key.clone(),
            opened: Instant::now(),
            shot: None,
        });
        self.focus = Target::PlayDetailStart;
        self.dirty = true;
    }

    /// Back on the Play page: the sheet closes, then focus goes up to the tab.
    pub fn play_back(&mut self) -> bool {
        if self.page != Page::Play {
            return false;
        }
        if self.play.sheet.take().is_some() {
            self.focus = Target::PlayGame(self.play.index);
            self.dirty = true;
            return true;
        }
        if !matches!(self.focus, Target::Nav(_)) {
            self.focus = Target::Nav(PLAY_PAGE);
            self.dirty = true;
            return true;
        }
        false
    }

    /// OK on a Play control. None when the focus is something else.
    pub fn play_press(&mut self) -> Option<Vec<Effect>> {
        let game = match self.focus.clone() {
            Target::PlayHide => return Some(self.play_toggle_hidden()),
            Target::PlayHidden => {
                self.play.show_hidden = !self.play.show_hidden;
                self.play.refilter();
                self.focus = if self.play.games().is_empty() {
                    self.play_entry()
                } else {
                    Target::PlayHidden
                };
                if self.play.show_hidden
                    && let Some(index) = self
                        .play
                        .games()
                        .iter()
                        .position(|game| self.play.is_hidden(&game.key))
                {
                    self.play_focus_game(index, Instant::now());
                }
                self.sound = Some(crate::state::Sound::Select);
                self.dirty = true;
                return Some(vec![]);
            }
            Target::PlayGame(index) => self.play.game(index).cloned(),
            Target::PlayAction(PlayButton::Play) => self.play.game(self.play.index).cloned(),
            Target::PlayDetailStart | Target::PlayShot(_) => self
                .play
                .sheet
                .as_ref()
                .and_then(|sheet| self.play.find(&sheet.key))
                .cloned(),
            Target::PlayAction(PlayButton::Details) => {
                self.open_play_sheet();
                return Some(vec![]);
            }
            Target::PlayAction(PlayButton::Hydra) => return Some(self.start_console_with(None)),
            _ => return None,
        };
        if self.launching {
            return Some(vec![]);
        }
        Some(match game {
            Some(game) => self.start_console_with(Some(game)),
            None => vec![],
        })
    }

    /// Holding OK on a cover opens its details.
    /// Hides the details sheet's game (or unhides it), saving the choice; the sheet closes
    /// and focus stays where the game was on the rail.
    fn play_toggle_hidden(&mut self) -> Vec<Effect> {
        let Some(key) = self.play.sheet.as_ref().map(|sheet| sheet.key.clone()) else {
            return vec![];
        };
        let list = &mut self.preferences.hidden_games;
        match list.iter().position(|hidden| *hidden == key) {
            Some(position) => {
                list.remove(position);
            }
            None => list.push(key),
        }
        self.play.hidden = list.iter().cloned().collect();
        self.play.sheet = None;
        self.play.refilter();
        self.focus = self.play_entry();
        if let Target::PlayGame(index) = self.focus {
            self.play_focus_game(index, Instant::now());
        }
        self.sound = Some(crate::state::Sound::Back);
        self.dirty = true;
        vec![Effect::Save]
    }

    /// Holding OK: a cover opens its details; on the details sheet, hides the game.
    pub fn play_hold(&mut self) -> Option<Vec<Effect>> {
        if self.page == Page::Play
            && self.play.sheet.is_some()
            && matches!(
                self.focus,
                Target::PlayDetailStart | Target::PlayHide | Target::PlayShot(_)
            )
        {
            return Some(self.play_toggle_hidden());
        }
        if let Target::PlayGame(index) = self.focus {
            self.play.index = index;
            self.open_play_sheet();
            return Some(vec![]);
        }
        None
    }
}
