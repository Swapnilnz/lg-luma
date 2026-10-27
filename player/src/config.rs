//! The player's config, baked in at build time from `config.local.json` (see build.rs).
//! The IPK only goes on your own TV; never publish it, since the AIOStreams URL is a credential.

use luma_core::stremio;
use luma_core::trakt::App as TraktApp;
use serde_json::Value;

const RAW: &str = include_str!(concat!(env!("OUT_DIR"), "/config.json"));

#[derive(Clone, Debug)]
pub struct Defaults {
    pub audio_languages: Vec<String>,
    pub subtitle_languages: Vec<String>,
    pub subtitles_on: bool,
    pub autoplay_next: bool,
    pub next_card_seconds_left: f64,
    pub countdown_seconds: f64,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// AIOStreams' addon base (manifest URL without `manifest.json`).
    pub streams: Option<String>,
    pub subtitle_addons: Vec<String>,
    pub tmdb_key: Option<String>,
    pub trakt: Option<TraktApp>,
    pub defaults: Defaults,
    /// Dev builds post their log here (tools/devlog.mjs).
    pub log_url: Option<String>,
}

/// A configured value, or None for blanks and the example's placeholders.
fn real(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty() && !v.contains("YOUR_") && !v.contains("/..."))
        .map(str::to_owned)
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    let list: Vec<String> = value?
        .as_array()?
        .iter()
        .filter_map(|v| real(v.as_str()))
        .collect();
    (!list.is_empty()).then_some(list)
}

pub fn load() -> Config {
    let v: Value = serde_json::from_str(RAW).unwrap_or_else(|e| {
        crate::log!("Luma Player config: {e}");
        Value::Null
    });
    let at = |path: &str| v.pointer(path);
    let d = at("/defaults");
    let get = |key: &str| d.and_then(|d| d.get(key));
    Config {
        streams: real(at("/aiostreams/manifest_url").and_then(Value::as_str))
            .map(|u| stremio::base(&u)),
        subtitle_addons: strings(at("/subtitle_addons"))
            .unwrap_or_default()
            .iter()
            .map(|u| stremio::base(u))
            .collect(),
        tmdb_key: real(at("/tmdb/api_key").and_then(Value::as_str)),
        trakt: real(at("/trakt/client_id").and_then(Value::as_str)).map(|client_id| TraktApp {
            client_id,
            client_secret: real(at("/trakt/client_secret").and_then(Value::as_str)),
        }),
        defaults: Defaults {
            audio_languages: strings(get("audio_languages")).unwrap_or_else(|| vec!["en".into()]),
            subtitle_languages: strings(get("subtitle_languages"))
                .unwrap_or_else(|| vec!["en".into()]),
            subtitles_on: get("subtitles_on")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            autoplay_next: get("autoplay_next")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            next_card_seconds_left: get("next_card_seconds_left")
                .and_then(Value::as_f64)
                .unwrap_or(45.)
                .clamp(10., 300.),
            countdown_seconds: get("countdown_seconds")
                .and_then(Value::as_f64)
                .unwrap_or(10.)
                .clamp(3., 60.),
        },
        log_url: real(at("/dev/log_url").and_then(Value::as_str)),
    }
}
