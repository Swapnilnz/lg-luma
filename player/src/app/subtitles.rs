//! Subtitle choices for the Tracks panel (forced first, other languages tucked away), the
//! automatic picks, and the timing dial.

use super::*;
use luma_core::selection::{SubEntry, forced_for, is_forced, same_language, subtitle_order};

/// Steps of the timing dial: Left/Right and Up/Down.
const FINE_MS: i32 = 100;
const COARSE_MS: i32 = 1000;
pub const MAX_OFFSET_MS: i32 = 30_000;
/// Options shown per language before the rest are left out.
const PER_LANGUAGE: usize = 6;

/// One subtitle the viewer could pick: from the file, or from an addon.
#[derive(Clone, Debug)]
pub struct SubCandidate {
    pub choice: SubChoice,
    pub entry: SubEntry,
    /// The addon's or file's own label: release name, "SDH".
    pub label: String,
    pub embedded: bool,
}

impl App {
    /// Every subtitle on offer, embedded tracks first, in no particular order.
    pub fn sub_candidates(&self) -> Vec<SubCandidate> {
        let embedded = self
            .video
            .text_tracks()
            .into_iter()
            .enumerate()
            .map(|(i, track)| SubCandidate {
                choice: SubChoice::Embedded(i),
                entry: SubEntry {
                    language: track.language.clone(),
                    forced: is_forced(&track.label),
                },
                label: track.label,
                embedded: true,
            });
        let addons = self
            .addon_subs
            .iter()
            .enumerate()
            .map(|(i, sub)| SubCandidate {
                choice: SubChoice::Addon(i),
                entry: SubEntry {
                    language: sub.lang.clone(),
                    forced: sub.forced,
                },
                label: sub.label.clone(),
                embedded: false,
            });
        embedded.chain(addons).collect()
    }

    /// The Tracks panel's list: forced ones first, the preferred languages only unless
    /// "Other languages" is open, and how many are left out.
    pub fn sub_menu(&self) -> (Vec<SubCandidate>, usize) {
        let candidates = self.sub_candidates();
        let entries: Vec<SubEntry> = candidates.iter().map(|c| c.entry.clone()).collect();
        let (order, hidden) = subtitle_order(
            &entries,
            &self.settings.subtitle_languages,
            self.show_all_subs,
            PER_LANGUAGE,
        );
        (
            order.into_iter().map(|i| candidates[i].clone()).collect(),
            hidden,
        )
    }

    /// What to turn on by itself: full subtitles in the wanted language (never a forced
    /// track for that), or with subtitles off, a forced track in the first preferred
    /// language, so foreign-language lines are still subtitled (as Netflix does).
    pub(super) fn auto_subtitles(&self, from_addons: bool) -> Option<SubChoice> {
        let candidates: Vec<SubCandidate> = self
            .sub_candidates()
            .into_iter()
            .filter(|c| c.embedded != from_addons)
            .collect();
        let entries: Vec<SubEntry> = candidates.iter().map(|c| c.entry.clone()).collect();
        let index = match self.subtitle_wish() {
            Some(language) => entries
                .iter()
                .position(|e| !e.forced && same_language(&e.language, &language)),
            // Off by default: forced lines still show. Off chosen by the viewer: nothing.
            None if self.subtitles_turned_off() => None,
            None => {
                let language = self.settings.subtitle_languages.first()?;
                forced_for(&entries, language)
            }
        }?;
        Some(candidates[index].choice.clone())
    }

    /// Whether the viewer picked Off, for this title or last time for this show.
    fn subtitles_turned_off(&self) -> bool {
        match &self.subtitle_choice_wish {
            Some(wish) => wish.is_none(),
            None => self
                .show_key()
                .is_some_and(|key| store::show_prefs(&key).subtitles == Some(None)),
        }
    }

    // ----- Timing dial -----

    /// The dial's keys: Left/Right 0.1 s, Up/Down 1 s, OK or Back return to Tracks.
    pub(super) fn delay_key(&mut self, key: Key) {
        let step = match key {
            Key::Left => -FINE_MS,
            Key::Right => FINE_MS,
            Key::Down => -COARSE_MS,
            Key::Up => COARSE_MS,
            Key::Ok | Key::Back => {
                self.settings.save();
                self.open_panel(Panel::Tracks);
                self.focus_row(ui::RowId::SubOffset);
                return;
            }
            Key::Stop => return self.exit(),
            other => return self.media_key_pressed(other),
        };
        self.nudge_delay(step);
    }

    pub fn nudge_delay(&mut self, ms: i32) {
        self.settings.subtitle_offset_ms =
            (self.settings.subtitle_offset_ms + ms).clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
    }

    /// Puts panel focus on a row, when it is listed.
    pub(super) fn focus_row(&mut self, id: ui::RowId) {
        if let Some(panel) = self.panel
            && let Some(index) = ui::rows(self, panel)
                .iter()
                .position(|r| r.id.as_ref() == Some(&id))
        {
            self.focus = index;
        }
    }
}
