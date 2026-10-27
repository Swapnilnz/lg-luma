//! The `<video>` element: LG's hardware pipeline behind a small API.
//!
//! Audio and text track lists are reached through Reflect, since web-sys gates them and the
//! TV's Chromium may or may not expose them (Phase 0, spikes 3 and 4).

use crate::dom::{get, set};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlVideoElement;

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub language: String,
    pub label: String,
    pub on: bool,
}

#[derive(Clone)]
pub struct Video(pub HtmlVideoElement);

impl Video {
    pub fn find() -> Option<Self> {
        crate::dom::document()
            .get_element_by_id("video")?
            .dyn_into::<HtmlVideoElement>()
            .ok()
            .map(Video)
    }

    pub fn target(&self) -> &web_sys::EventTarget {
        self.0.as_ref()
    }

    pub fn load(&self, url: &str) {
        self.0.set_src(url);
        self.0.load();
    }

    /// Drops the current source, so the decoder lets go of it.
    pub fn unload(&self) {
        let _ = self.0.pause();
        let _ = self.0.remove_attribute("src");
        self.0.load();
    }

    pub fn src(&self) -> String {
        self.0.src()
    }

    pub fn play(&self) {
        if let Ok(promise) = self.0.play() {
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await {
                    crate::log!("Luma Player: play() refused: {:?}", error.as_string());
                }
            });
        }
    }

    pub fn pause(&self) {
        let _ = self.0.pause();
    }

    pub fn paused(&self) -> bool {
        self.0.paused()
    }

    pub fn ended(&self) -> bool {
        self.0.ended()
    }

    pub fn position(&self) -> f64 {
        let t = self.0.current_time();
        if t.is_finite() { t } else { 0. }
    }

    pub fn set_position(&self, seconds: f64) {
        let duration = self.duration();
        let target = if duration > 0. {
            seconds.clamp(0., (duration - 1.).max(0.))
        } else {
            seconds.max(0.)
        };
        self.0.set_current_time(target);
    }

    /// Seconds, or 0 while unknown (or a live stream).
    pub fn duration(&self) -> f64 {
        let d = self.0.duration();
        if d.is_finite() && d > 0. { d } else { 0. }
    }

    /// Where buffering reaches from the current position.
    pub fn buffered_end(&self) -> f64 {
        let ranges = self.0.buffered();
        let now = self.position();
        (0..ranges.length())
            .filter_map(|i| Some((ranges.start(i).ok()?, ranges.end(i).ok()?)))
            .find(|(start, end)| *start <= now + 0.5 && now <= *end)
            .map_or(now, |(_, end)| end)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.0.video_width(), self.0.video_height())
    }

    pub fn error_code(&self) -> u16 {
        self.0.error().map_or(0, |e| e.code())
    }

    fn list(&self, name: &str) -> Vec<JsValue> {
        let list = get(self.0.as_ref(), name);
        if list.is_undefined() || list.is_null() {
            return vec![];
        }
        let length = get(&list, "length").as_f64().unwrap_or(0.) as u32;
        (0..length)
            .map(|i| js_sys::Reflect::get_u32(&list, i).unwrap_or(JsValue::UNDEFINED))
            .filter(|track| !track.is_undefined())
            .collect()
    }

    fn info(track: &JsValue, on_key: &str) -> TrackInfo {
        let text = |key: &str| get(track, key).as_string().unwrap_or_default();
        let on = match on_key {
            "enabled" => get(track, "enabled").as_bool().unwrap_or(false),
            _ => text("mode") == "showing",
        };
        TrackInfo {
            language: text("language"),
            label: text("label"),
            on,
        }
    }

    pub fn audio_tracks(&self) -> Vec<TrackInfo> {
        self.list("audioTracks")
            .iter()
            .map(|t| Self::info(t, "enabled"))
            .collect()
    }

    pub fn set_audio(&self, index: usize) {
        let tracks = self.list("audioTracks");
        if index >= tracks.len() {
            return;
        }
        // Enable the new one first: some decoders go silent with none enabled.
        set(&tracks[index], "enabled", &JsValue::TRUE);
        for (i, track) in tracks.iter().enumerate() {
            if i != index {
                set(track, "enabled", &JsValue::FALSE);
            }
        }
    }

    /// Subtitle and caption tracks embedded in the file, when the TV lists them.
    pub fn text_tracks(&self) -> Vec<TrackInfo> {
        self.list("textTracks")
            .iter()
            .filter(|t| {
                let kind = get(t, "kind").as_string().unwrap_or_default();
                kind == "subtitles" || kind == "captions" || kind.is_empty()
            })
            .map(|t| Self::info(t, "mode"))
            .collect()
    }

    /// Shows one embedded track (rendered by the TV), or none.
    pub fn set_text(&self, index: Option<usize>) {
        let tracks: Vec<JsValue> = self
            .list("textTracks")
            .into_iter()
            .filter(|t| {
                let kind = get(t, "kind").as_string().unwrap_or_default();
                kind == "subtitles" || kind == "captions" || kind.is_empty()
            })
            .collect();
        for (i, track) in tracks.iter().enumerate() {
            let mode = if Some(i) == index {
                "showing"
            } else {
                "disabled"
            };
            set(track, "mode", &JsValue::from_str(mode));
        }
    }
}
