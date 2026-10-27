//! localStorage: the Trakt token, resume points, settings and the scrobble queue.
//! A missing or full store never stops playback; it just forgets.

use crate::config::Defaults;
use luma_core::resume::Point;
use serde_json::{Map, Value, json};
use web_sys::Storage;

const TOKEN: &str = "luma.trakt.token";
const SKIP_SIGN_IN: &str = "luma.trakt.skip_until";
const POSITIONS: &str = "luma.positions";
const SETTINGS: &str = "luma.settings";
const QUEUE: &str = "luma.scrobble_queue";
/// Resume points kept, newest first.
const MAX_POSITIONS: usize = 300;
const MAX_QUEUE: usize = 50;

fn storage() -> Option<Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

pub fn put(key: &str, value: &str) {
    if let Some(storage) = storage()
        && storage.set_item(key, value).is_err()
    {
        crate::log!("Luma Player store: {key} not saved (storage full?)");
    }
}

pub fn remove(key: &str) {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(key);
    }
}

fn json_or(key: &str, fallback: Value) -> Value {
    get(key)
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(fallback)
}

pub fn token() -> Option<luma_core::trakt::Token> {
    luma_core::trakt::Token::from_json(&get(TOKEN)?)
}

pub fn save_token(token: Option<&luma_core::trakt::Token>) {
    match token {
        Some(token) => put(TOKEN, &token.to_json()),
        None => remove(TOKEN),
    }
}

/// "Not now" on the sign-in screen holds it off for a week.
pub fn sign_in_skipped(now_ms: f64) -> bool {
    get(SKIP_SIGN_IN)
        .and_then(|t| t.parse::<f64>().ok())
        .is_some_and(|until| now_ms < until)
}

pub fn skip_sign_in(now_ms: f64) {
    put(SKIP_SIGN_IN, &(now_ms + 7. * 86_400_000.).to_string());
}

pub fn clear_skip_sign_in() {
    remove(SKIP_SIGN_IN);
}

pub fn position(key: &str) -> Option<Point> {
    let all = json_or(POSITIONS, json!({}));
    let entry = all.get(key)?;
    let position = entry.get("p")?.as_f64()?;
    let duration = entry.get("d").and_then(Value::as_f64).unwrap_or(0.);
    Some(Point {
        position: Some(position),
        percent: luma_core::scrobble::percent(position, duration),
        at_ms: entry.get("at").and_then(Value::as_f64).unwrap_or(0.),
    })
}

pub fn save_position(key: &str, position: f64, duration: f64, now_ms: f64) {
    let mut all = match json_or(POSITIONS, json!({})) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    all.insert(
        key.into(),
        json!({"p": position.round(), "d": duration.round(), "at": now_ms}),
    );
    if all.len() > MAX_POSITIONS {
        let mut entries: Vec<(String, Value)> = all.into_iter().collect();
        entries.sort_by(|a, b| {
            let at = |v: &Value| v.get("at").and_then(Value::as_f64).unwrap_or(0.);
            at(&b.1).total_cmp(&at(&a.1))
        });
        entries.truncate(MAX_POSITIONS);
        all = entries.into_iter().collect();
    }
    put(POSITIONS, &Value::Object(all).to_string());
}

pub fn clear_position(key: &str) {
    if let Value::Object(mut all) = json_or(POSITIONS, json!({})) {
        all.remove(key);
        put(POSITIONS, &Value::Object(all).to_string());
    }
}

/// A Trakt call that failed for want of a network, kept to replay on the next launch.
pub fn queue_push(path: &str, body: &str) {
    let mut list = match json_or(QUEUE, json!([])) {
        Value::Array(list) => list,
        _ => vec![],
    };
    list.push(json!({"path": path, "body": body}));
    let skip = list.len().saturating_sub(MAX_QUEUE);
    put(
        QUEUE,
        &Value::Array(list.into_iter().skip(skip).collect()).to_string(),
    );
}

pub fn queue_take() -> Vec<(String, String)> {
    let list = json_or(QUEUE, json!([]));
    remove(QUEUE);
    list.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|v| {
                    Some((
                        v.get("path")?.as_str()?.to_owned(),
                        v.get("body")?.as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the viewer changes in Settings and Tracks; overrides the config's defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub autoplay_next: bool,
    pub next_card_seconds: f64,
    pub countdown_seconds: f64,
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub subtitles_on: bool,
    /// 0 small, 1 medium, 2 large.
    pub subtitle_size: u8,
    pub subtitle_box: bool,
    /// Milliseconds added to video time when finding cues (positive shows them later).
    pub subtitle_offset_ms: i32,
}

impl Settings {
    pub fn load(defaults: &Defaults) -> Self {
        let v = json_or(SETTINGS, json!({}));
        let list = |key: &str, fallback: &[String]| -> Vec<String> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| fallback.to_vec())
        };
        Settings {
            autoplay_next: v
                .get("autoplay_next")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.autoplay_next),
            next_card_seconds: v
                .get("next_card_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(defaults.next_card_seconds_left),
            countdown_seconds: v
                .get("countdown_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(defaults.countdown_seconds),
            audio_languages: list("audio_languages", &defaults.audio_languages),
            subtitle_languages: list("subtitle_languages", &defaults.subtitle_languages),
            subtitles_on: v
                .get("subtitles_on")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.subtitles_on),
            subtitle_size: v
                .get("subtitle_size")
                .and_then(Value::as_u64)
                .map_or(1, |n| n.min(2) as u8),
            subtitle_box: v
                .get("subtitle_box")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            subtitle_offset_ms: v
                .get("subtitle_offset_ms")
                .and_then(Value::as_i64)
                .map_or(0, |n| n.clamp(-30_000, 30_000) as i32),
        }
    }

    pub fn save(&self) {
        put(
            SETTINGS,
            &json!({
                "autoplay_next": self.autoplay_next,
                "next_card_seconds": self.next_card_seconds,
                "countdown_seconds": self.countdown_seconds,
                "audio_languages": self.audio_languages,
                "subtitle_languages": self.subtitle_languages,
                "subtitles_on": self.subtitles_on,
                "subtitle_size": self.subtitle_size,
                "subtitle_box": self.subtitle_box,
                "subtitle_offset_ms": self.subtitle_offset_ms,
            })
            .to_string(),
        );
    }
}
