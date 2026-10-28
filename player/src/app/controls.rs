//! The control bar's buttons, the Episodes shelf, playback speed and the pause screen.

use super::*;

pub const SPEEDS: [f64; 6] = [0.5, 0.75, 1., 1.25, 1.5, 2.];
/// Paused this long, the controls give way to the pause screen.
const PAUSE_SCREEN_MS: f64 = 10_000.;

/// Where focus sits while the controls are up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsdFocus {
    Scrubber,
    Button(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsdButton {
    PlayPause,
    Episodes,
    Next,
    Tracks,
    Speed,
    Streams,
    Info,
    Settings,
}

impl OsdButton {
    pub fn label(self) -> &'static str {
        match self {
            OsdButton::PlayPause => "Play/Pause",
            OsdButton::Episodes => "Episodes",
            OsdButton::Next => "Next Episode",
            OsdButton::Tracks => "Audio & Subtitles",
            OsdButton::Speed => "Playback Speed",
            OsdButton::Streams => "Streams",
            OsdButton::Info => "Info",
            OsdButton::Settings => "Settings",
        }
    }

    /// The panel a button opens, so it stays lit while that panel is up.
    pub fn panel(self) -> Option<Panel> {
        match self {
            OsdButton::Episodes => Some(Panel::Episodes),
            OsdButton::Tracks => Some(Panel::Tracks),
            OsdButton::Speed => Some(Panel::Speed),
            OsdButton::Streams => Some(Panel::Picker),
            OsdButton::Info => Some(Panel::Info),
            OsdButton::Settings => Some(Panel::Settings),
            _ => None,
        }
    }
}

impl App {
    /// The control bar, in one row from Play/Pause (seeking is Left/Right on the scrubber).
    pub fn osd_buttons(&self) -> Vec<OsdButton> {
        // Most used first: Play/Pause, Audio & Subtitles, Streams, then a show's Episodes.
        let mut buttons = vec![OsdButton::PlayPause, OsdButton::Tracks, OsdButton::Streams];
        if self.launch.kind == Kind::Series {
            buttons.push(OsdButton::Episodes);
            if self.next_target().is_some() {
                buttons.push(OsdButton::Next);
            }
        }
        buttons.extend([OsdButton::Speed, OsdButton::Info, OsdButton::Settings]);
        buttons
    }

    /// Down from the scrubber lands on Play/Pause.
    pub(super) fn default_button(&self) -> usize {
        0
    }

    /// The episode after this one, if the show has one that has aired.
    pub fn next_target(&self) -> Option<(u32, u32)> {
        if let Some(next) = &self.next {
            return Some(next.at);
        }
        let current = self.launch.episode_key()?;
        let meta = self.meta.as_ref()?;
        luma_core::next::after(current, &meta.seasons, meta.last_aired)
    }

    /// Arrows, OK and Back on the control bar. False for keys it leaves to the caller.
    pub(super) fn button_key(&mut self, key: Key, index: usize) -> bool {
        let buttons = self.osd_buttons();
        let last = buttons.len() - 1;
        let index = index.min(last);
        match key {
            Key::Left => self.osd_focus = OsdFocus::Button(index.saturating_sub(1)),
            Key::Right => self.osd_focus = OsdFocus::Button((index + 1).min(last)),
            Key::Up => self.osd_focus = OsdFocus::Scrubber,
            Key::Down => {}
            Key::Ok => self.press_button(buttons[index]),
            Key::Back => {
                self.osd_focus = OsdFocus::Scrubber;
                self.osd_until = 0.;
                return true;
            }
            _ => return false,
        }
        self.show_osd();
        true
    }

    pub fn press_button(&mut self, button: OsdButton) {
        match button {
            OsdButton::PlayPause => self.toggle_play(),
            OsdButton::Episodes => self.open_episodes(),
            OsdButton::Next => self.skip_to_next(),
            OsdButton::Tracks => self.open_panel(Panel::Tracks),
            OsdButton::Speed => self.open_panel(Panel::Speed),
            OsdButton::Streams => self.open_panel(Panel::Picker),
            OsdButton::Info => self.open_panel(Panel::Info),
            OsdButton::Settings => self.open_panel(Panel::Settings),
        }
    }

    /// A pointer click on a control-bar button.
    pub fn click_button(&mut self, index: usize) {
        if let Some(button) = self.osd_buttons().get(index).copied() {
            self.osd_focus = OsdFocus::Button(index);
            self.show_osd();
            self.press_button(button);
        }
    }

    /// Next Episode now, using Next Up's prefetched streams when it has them.
    fn skip_to_next(&mut self) {
        let Some(at) = self.next_target() else {
            return;
        };
        if self.next.as_ref().map(|next| next.at) != Some(at) {
            self.next = Some(NextUp {
                at,
                name: None,
                still: None,
                runtime: None,
                streams: None,
                shown_at: None,
                dismissed: false,
            });
        }
        self.play_next();
    }

    pub fn set_speed(&mut self, rate: f64) {
        self.speed = rate;
        self.video.set_rate(rate);
        self.panel = None;
        self.show_osd();
    }

    // ----- Episodes shelf -----

    /// Regular seasons with episodes, in order (specials left out).
    pub fn shelf_seasons(&self) -> Vec<u32> {
        let mut seasons: Vec<u32> = self
            .meta
            .as_ref()
            .map(|meta| {
                meta.seasons
                    .iter()
                    .filter(|s| s.number > 0 && s.episodes > 0)
                    .map(|s| s.number)
                    .collect()
            })
            .unwrap_or_default();
        seasons.sort_unstable();
        if seasons.is_empty()
            && let Some((season, _)) = self.launch.episode_key()
        {
            seasons.push(season);
        }
        seasons
    }

    pub fn shelf_episodes(&self) -> Option<&Vec<tmdb::Episode>> {
        self.seasons.get(&self.shelf_season)
    }

    pub(super) fn open_episodes(&mut self) {
        let (season, number) = self.launch.episode_key().unwrap_or((1, 1));
        self.shelf_season = season;
        self.shelf_row = 1;
        self.shelf_focus = self
            .seasons
            .get(&season)
            .and_then(|list| list.iter().position(|e| e.number == number))
            .unwrap_or(number.saturating_sub(1) as usize);
        self.ensure_season(season);
        self.merge_local_progress(season);
        self.panel = Some(Panel::Episodes);
    }

    /// Partly watched episodes this player saved itself, for the shelf's progress bars
    /// (Trakt's own paused points take precedence).
    fn merge_local_progress(&mut self, season: u32) {
        let (Some(imdb), Some(list)) = (self.launch.imdb.clone(), self.seasons.get(&season)) else {
            return;
        };
        let numbers: Vec<u32> = list.iter().map(|e| e.number).collect();
        for number in numbers {
            let key = luma_core::ids::media_key(Kind::Series, &imdb, Some((season, number)));
            if let Some(point) = store::position(&key)
                && point.percent > 0.
            {
                self.partial
                    .entry((season, number))
                    .or_insert(point.percent);
            }
        }
    }

    /// This episode is done: ticked on the shelf straight away, before Trakt says so.
    pub(super) fn mark_watched(&mut self) {
        if let Some(at) = self.launch.episode_key() {
            self.watched.insert(at);
            self.partial.remove(&at);
        }
    }

    // ----- Skip Intro / Skip Recap -----

    /// What can be skipped now, and where it ends: never while seeking, paused or in a panel.
    pub fn skip_offer(&self) -> Option<(luma_core::segments::Skip, f64)> {
        if !matches!(self.screen, Screen::Playing)
            || self.panel.is_some()
            || self.seek_preview.is_some()
            || !self.first_frame
            || self.pause_screen()
        {
            return None;
        }
        let offer = self.segments?.skip_at(self.video.position())?;
        (self.skip_hidden != Some(offer.0)).then_some(offer)
    }

    pub fn skip_now(&mut self) {
        if let Some((kind, end)) = self.skip_offer() {
            log!("Luma Player: {} to {end:.0} s", kind.label());
            self.skipped.push(kind);
            self.video.set_position(end);
        }
    }

    /// With "Skip intros automatically", each intro and recap is skipped once. The credits are
    /// never skipped for you: the scene after them is offered instead.
    pub(super) fn auto_skip(&mut self) {
        use luma_core::segments::Skip;
        if !self.settings.auto_skip || self.video.paused() {
            return;
        }
        if let Some((kind, _)) = self.skip_offer()
            && kind != Skip::PostCredits
            && !self.skipped.contains(&kind)
        {
            self.skip_now();
            self.toast(match kind {
                Skip::Intro => "Skipped the intro",
                Skip::Recap => "Skipped the recap",
                Skip::PostCredits => "Skipped to the scene after the credits",
            });
        }
    }

    /// A scene during or after the credits (TMDB's tag, or IntroDB's timestamps): said once,
    /// as the credits start, so nobody stops watching too soon.
    pub(super) fn stinger_notice(&mut self) {
        if self.stinger_told || !self.first_frame || self.video.paused() {
            return;
        }
        let duration = self.video.duration();
        if duration <= 300. {
            return;
        }
        let segments = self.segments.unwrap_or_default();
        let tagged = segments.post_credits.is_some()
            || (self.launch.kind == Kind::Movie && self.meta.as_ref().is_some_and(|m| m.stinger));
        let position = self.video.position();
        let credits = segments.outro.map_or(duration * 0.94, |outro| outro.start);
        if tagged
            && position >= credits
            && (segments.post_credits.is_none() || segments.scene_ahead(position))
        {
            self.stinger_told = true;
            log!("Luma Player: post-credits scene ahead");
            self.toast = Some((
                "There's a scene after the credits. Keep watching!".into(),
                now_ms() + 8_000.,
            ));
        }
    }

    /// On a laptop, a stream whose audio the browser can't decode (Dolby, DTS) plays silent:
    /// six seconds in, say so instead. The TV decodes those in hardware, so it isn't checked
    /// there.
    pub(super) fn check_audio(&mut self, now: f64) {
        if self.audio_checked
            || !self.first_frame
            || self.video.paused()
            || now - self.first_frame_ms < 6_000.
            || crate::dom::palm().is_some()
        {
            return;
        }
        self.audio_checked = true;
        let decoded =
            crate::dom::get(self.video.0.as_ref(), "webkitAudioDecodedByteCount").as_f64();
        if decoded == Some(0.) {
            log!("Luma Player: no audio decoded; likely Dolby or DTS, which desktop Chrome lacks");
            self.toast("This browser can't play this stream's audio (likely Dolby). It will play on the TV, or try another stream.");
        }
    }

    /// Fetches a season's episodes from TMDB once.
    fn ensure_season(&mut self, season: u32) {
        if self.seasons.contains_key(&season) || self.seasons_loading.contains(&season) {
            return;
        }
        let (Some(key), Some(show)) = (self.config.tmdb_key.clone(), self.tmdb_id) else {
            return;
        };
        self.seasons_loading.push(season);
        let epoch = self.epoch;
        spawn_local(async move {
            let episodes = match net::send(&tmdb::season(&key, show, season), 10_000).await {
                Ok(reply) if reply.ok() => tmdb::parse_season(&reply.text),
                _ => vec![],
            };
            update(epoch, move |app| {
                app.seasons_loading.retain(|s| *s != season);
                app.seasons.insert(season, episodes);
                app.merge_local_progress(season);
                if app.shelf_season == season {
                    let len = app.shelf_episodes().map_or(0, Vec::len);
                    app.shelf_focus = app.shelf_focus.min(len.saturating_sub(1));
                }
                app.render();
            });
        });
    }

    pub(super) fn shelf_key(&mut self, key: Key) {
        match key {
            Key::Back => {
                self.panel = None;
                self.show_osd();
            }
            Key::Up => self.shelf_row = 0,
            Key::Down => self.shelf_row = 1,
            Key::Left | Key::Right => {
                let step: i32 = if key == Key::Left { -1 } else { 1 };
                if self.shelf_row == 0 {
                    let seasons = self.shelf_seasons();
                    let at = seasons
                        .iter()
                        .position(|s| *s == self.shelf_season)
                        .unwrap_or(0) as i32;
                    let to = (at + step).clamp(0, seasons.len() as i32 - 1).max(0) as usize;
                    if let Some(&season) = seasons.get(to)
                        && season != self.shelf_season
                    {
                        self.shelf_season = season;
                        self.shelf_focus = 0;
                        self.ensure_season(season);
                        self.merge_local_progress(season);
                    }
                } else {
                    let len = self.shelf_episodes().map_or(0, Vec::len) as i32;
                    self.shelf_focus =
                        (self.shelf_focus as i32 + step).clamp(0, (len - 1).max(0)) as usize;
                }
            }
            Key::Ok => {
                if self.shelf_row == 0 {
                    self.shelf_row = 1;
                } else if let Some(episode) = self
                    .shelf_episodes()
                    .and_then(|list| list.get(self.shelf_focus))
                {
                    self.play_episode((self.shelf_season, episode.number));
                }
            }
            Key::Red => self.open_panel(Panel::Info),
            Key::Green => self.open_panel(Panel::Picker),
            Key::Yellow => self.open_panel(Panel::Tracks),
            Key::Blue => self.open_panel(Panel::Settings),
            Key::Stop => self.exit(),
            other => self.media_key_pressed(other),
        }
    }

    /// A pointer click on the shelf: a season tab, or an episode.
    pub fn click_shelf(&mut self, season: Option<u32>, episode: Option<usize>) {
        if let Some(season) = season {
            self.shelf_row = 0;
            self.shelf_season = season;
            self.shelf_focus = 0;
            self.ensure_season(season);
        } else if let Some(index) = episode {
            self.shelf_row = 1;
            self.shelf_focus = index;
            self.shelf_key(Key::Ok);
        }
    }

    /// Another episode from the shelf: this one is paused for Trakt and saved, then the new
    /// one starts (asking to resume if it has a saved place).
    fn play_episode(&mut self, at: (u32, u32)) {
        if Some(at) == self.launch.episode_key() {
            self.panel = None;
            return;
        }
        let today = crate::dom::today();
        if let Some(episode) = self
            .seasons
            .get(&at.0)
            .and_then(|list| list.iter().find(|e| e.number == at.1))
            && !luma_core::time::released(&episode.aired, &today)
        {
            let when = luma_core::time::coming_label(&episode.aired, &today);
            self.toast(format!("This episode isn't out yet ({when})"));
            return;
        }
        self.exit_scrobble(false);
        self.save_position(true);
        let launch = self.launch.with_episode(at);
        spawn_local(async move { begin(launch, None, false) });
    }

    // ----- Loading screen -----

    /// A stream is loading and the picture hasn't started: the logo screen stays up.
    pub fn starting(&self) -> bool {
        matches!(self.screen, Screen::Playing)
            && self.current.is_some()
            && self.wants_play
            && !self.first_frame
            && self.panel.is_none()
    }

    /// How far loading has got, 0 to 1, for the logo fill: details, streams, the stream
    /// loading, its first frame. Within a stage it creeps on, so a slow AIOStreams never
    /// looks stuck.
    pub fn load_progress(&self) -> f64 {
        let (stage, from, to) = if self.first_frame {
            (5, 1., 1.)
        } else if self.metadata {
            (4, 0.82, 0.96)
        } else if self.current.is_some() {
            (3, 0.62, 0.8)
        } else if !self.streams.is_empty() {
            (2, 0.5, 0.6)
        } else if self.meta.is_some() {
            (1, 0.22, 0.48)
        } else {
            (0, 0.06, 0.2)
        };
        let now = now_ms();
        let (seen, since) = self.load_stage.get();
        let since = if seen == stage {
            since
        } else {
            self.load_stage.set((stage, now));
            now
        };
        let creep = 1. - (-(now - since) / 6_000.).exp();
        from + (to - from) * creep
    }

    pub fn loading_status(&self) -> String {
        match &self.screen {
            Screen::Loading(status) => status.clone(),
            _ if self.metadata => "Buffering…".into(),
            _ => match self.current.and_then(|i| self.streams.get(i)) {
                Some(stream) if !stream.name.is_empty() => {
                    format!("Starting {}", stream.name.replace('\n', " "))
                }
                _ => "Starting…".into(),
            },
        }
    }

    // ----- Pause screen -----

    /// Paused for 10 s with nothing else open: the title, episode, blurb and clock.
    pub fn pause_screen(&self) -> bool {
        matches!(self.screen, Screen::Playing)
            && self.panel.is_none()
            && self.seek_preview.is_none()
            && !self.ended
            && self.first_frame
            && self.video.paused()
            && self
                .paused_at
                .is_some_and(|at| now_ms() - at >= PAUSE_SCREEN_MS)
    }

    /// Any key brings the controls back; OK and Play also resume.
    pub(super) fn leave_pause_screen(&mut self, key: Key) {
        self.paused_at = Some(now_ms());
        self.osd_focus = OsdFocus::Scrubber;
        self.show_osd();
        match key {
            Key::Ok | Key::Play | Key::PlayPause => self.toggle_play(),
            Key::Stop => self.exit(),
            _ => {}
        }
    }

    /// The blurb on the pause screen: the episode's, else the title's.
    pub fn overview(&self) -> Option<String> {
        self.episode_overview.clone().or_else(|| {
            self.meta
                .as_ref()
                .map(|m| m.overview.clone())
                .filter(|o| !o.is_empty())
        })
    }
}
