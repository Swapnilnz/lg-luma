//! Drawing: the app's state as HTML in a few fixed layers over the `<video>`.
//! Each layer is only written when its HTML changes.

use crate::app::{App, Panel, Screen, SubChoice};
use crate::dom::{self, escape, now_ms};
use luma_core::resume::Offer;
use luma_core::selection::{language_name, normalize_language, same_language};
use luma_core::time::clock;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

#[derive(Clone, Debug, PartialEq)]
pub enum RowId {
    Stream(usize),
    Audio(usize),
    SubOff,
    SubEmbedded(usize),
    SubAddon(usize),
    SubSize,
    SubBox,
    SubOffset,
    Autoplay,
    NextCard,
    Countdown,
    AudioLanguage,
    SubtitleLanguage,
    SubtitlesOn,
    Trakt,
}

/// One line in a panel. Rows without an id are headings or text and never take focus.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub id: Option<RowId>,
    pub label: String,
    pub detail: String,
    pub value: String,
    pub selected: bool,
    pub disabled: bool,
}

fn heading(label: &str) -> Row {
    Row {
        label: label.into(),
        ..Row::default()
    }
}

fn row(id: RowId, label: impl Into<String>, value: impl Into<String>) -> Row {
    Row {
        id: Some(id),
        label: label.into(),
        value: value.into(),
        ..Row::default()
    }
}

const AUDIO_LANGUAGES: &[&str] = &[
    "original", "en", "es", "fr", "de", "it", "pt", "ja", "ko", "hi", "mr", "zh",
];

/// The next language in Settings' list, either way; audio adds "Original" (the file's default).
pub fn cycle_language(now: &str, step: i32, audio: bool) -> String {
    let list: Vec<&str> = AUDIO_LANGUAGES
        .iter()
        .copied()
        .filter(|l| audio || *l != "original")
        .collect();
    let at = list
        .iter()
        .position(|l| *l == now || (*l != "original" && same_language(l, now)))
        .unwrap_or(0) as i32;
    let n = list.len() as i32;
    let dir = if step < 0 { -1 } else { 1 };
    list[((at + dir + n) % n) as usize].into()
}

fn language_label(code: &str) -> String {
    if code == "original" {
        "Original".into()
    } else {
        language_name(code)
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

pub fn offer_label(offer: &Offer) -> String {
    match offer.seconds {
        Some(seconds) => clock(seconds),
        None => format!("{:.0}%", offer.percent),
    }
}

pub fn rows(app: &App, panel: Panel) -> Vec<Row> {
    match panel {
        Panel::Picker => picker_rows(app),
        Panel::Tracks => track_rows(app),
        Panel::Settings => settings_rows(app),
        Panel::Info => info_rows(app),
    }
}

fn picker_rows(app: &App) -> Vec<Row> {
    app.streams
        .iter()
        .enumerate()
        .map(|(i, stream)| {
            let failure = app.attempts.failure(i);
            let mut detail = stream.description.clone();
            if let Some(reason) = failure {
                detail = format!("Didn't play: {reason}\n{detail}");
            }
            Row {
                id: Some(RowId::Stream(i)),
                label: if stream.name.is_empty() {
                    format!("Stream {}", i + 1)
                } else {
                    stream.name.clone()
                },
                detail,
                value: if app.current == Some(i) {
                    "Playing".into()
                } else {
                    String::new()
                },
                selected: app.current == Some(i),
                disabled: failure.is_some(),
            }
        })
        .collect()
}

fn track_rows(app: &App) -> Vec<Row> {
    let mut rows = vec![heading("Audio")];
    let audio = app.video.audio_tracks();
    if audio.len() <= 1 {
        rows.push(Row {
            label: "Default track".into(),
            detail: if audio.is_empty() {
                "This TV doesn't list the file's audio tracks".into()
            } else {
                String::new()
            },
            ..Row::default()
        });
    } else {
        for (i, track) in audio.iter().enumerate() {
            let mut r = row(RowId::Audio(i), language_label(&track.language), "");
            r.detail = track.label.clone();
            r.selected = track.on;
            r.value = if track.on {
                "✓".into()
            } else {
                String::new()
            };
            rows.push(r);
        }
    }
    rows.push(heading("Subtitles"));
    let mut off = row(RowId::SubOff, "Off", "");
    off.selected = app.sub_choice == SubChoice::Off;
    rows.push(off);
    for (i, track) in app.video.text_tracks().iter().enumerate() {
        let mut r = row(RowId::SubEmbedded(i), language_label(&track.language), "");
        r.detail = if track.label.is_empty() {
            "In the file".into()
        } else {
            format!("In the file · {}", track.label)
        };
        r.selected = app.sub_choice == SubChoice::Embedded(i);
        rows.push(r);
    }
    // Addon subtitles: the preferred languages first (up to six each), then one per language.
    let preferred = &app.settings.subtitle_languages;
    let mut shown: Vec<usize> = vec![];
    for want in preferred {
        shown.extend(
            app.addon_subs
                .iter()
                .enumerate()
                .filter(|(_, s)| same_language(&s.lang, want))
                .map(|(i, _)| i)
                .take(6),
        );
    }
    let mut seen: Vec<String> = preferred.iter().map(|l| normalize_language(l)).collect();
    for (i, sub) in app.addon_subs.iter().enumerate() {
        let lang = normalize_language(&sub.lang);
        if !seen.contains(&lang) && shown.len() < 40 {
            seen.push(lang);
            shown.push(i);
        }
    }
    let mut count: std::collections::HashMap<String, usize> = Default::default();
    for i in shown {
        let sub = &app.addon_subs[i];
        let n = count.entry(normalize_language(&sub.lang)).or_default();
        *n += 1;
        let mut r = row(RowId::SubAddon(i), language_label(&sub.lang), "");
        r.detail = format!("Addon · option {n}");
        r.selected = app.sub_choice == SubChoice::Addon(i);
        if r.selected && app.sub_track.is_none() {
            r.value = "Loading…".into();
        }
        rows.push(r);
    }
    if app.addon_subs.is_empty() && !app.config.subtitle_addons.is_empty() {
        rows.push(Row {
            label: "No addon subtitles yet".into(),
            ..Row::default()
        });
    }
    rows.push(heading("Style"));
    let size = ["Small", "Medium", "Large"][usize::from(app.settings.subtitle_size.min(2))];
    rows.push(row(RowId::SubSize, "Size", size));
    rows.push(row(
        RowId::SubBox,
        "Background",
        on_off(app.settings.subtitle_box),
    ));
    let offset = app.settings.subtitle_offset_ms;
    let mut timing = row(
        RowId::SubOffset,
        "Timing",
        match offset {
            0 => "In sync".to_owned(),
            ms if ms > 0 => format!("{:.2} s later", f64::from(ms) / 1000.),
            ms => format!("{:.2} s earlier", f64::from(-ms) / 1000.),
        },
    );
    timing.detail = "◀ ▶ to adjust · OK to reset".into();
    rows.push(timing);
    rows
}

fn settings_rows(app: &App) -> Vec<Row> {
    let s = &app.settings;
    let mut rows = vec![
        heading("Playback"),
        row(
            RowId::Autoplay,
            "Autoplay next episode",
            on_off(s.autoplay_next),
        ),
        row(
            RowId::NextCard,
            "Next Up appears",
            format!("{:.0} s before the end", s.next_card_seconds),
        ),
        row(
            RowId::Countdown,
            "Countdown",
            format!("{:.0} s", s.countdown_seconds),
        ),
        heading("Languages"),
        row(
            RowId::AudioLanguage,
            "Audio",
            language_label(s.audio_languages.first().map_or("original", String::as_str)),
        ),
        row(
            RowId::SubtitleLanguage,
            "Subtitles",
            language_label(s.subtitle_languages.first().map_or("en", String::as_str)),
        ),
        row(
            RowId::SubtitlesOn,
            "Subtitles by default",
            on_off(s.subtitles_on),
        ),
        heading("Trakt"),
    ];
    if app.config.trakt.is_some() {
        let mut r = row(
            RowId::Trakt,
            if app.token.is_some() {
                "Signed in"
            } else {
                "Not signed in"
            },
            if app.token.is_some() {
                "Sign out"
            } else {
                "Sign in"
            },
        );
        r.detail = if app.token.is_some() {
            "Scrobbling and resume points sync with Trakt".into()
        } else {
            "Watching isn't sent to Trakt".into()
        };
        rows.push(r);
    } else {
        rows.push(Row {
            label: "Add trakt.client_id to the player's config".into(),
            ..Row::default()
        });
    }
    rows
}

fn info_rows(app: &App) -> Vec<Row> {
    let mut rows = vec![];
    let year = app
        .meta
        .as_ref()
        .map(|m| m.year.clone())
        .unwrap_or_default();
    rows.push(Row {
        label: if year.is_empty() {
            app.title()
        } else {
            format!("{} ({year})", app.title())
        },
        detail: app.episode_line().unwrap_or_default(),
        ..Row::default()
    });
    if let Some(overview) = app
        .meta
        .as_ref()
        .map(|m| m.overview.clone())
        .filter(|o| !o.is_empty())
    {
        rows.push(Row {
            detail: overview,
            ..Row::default()
        });
    }
    if let Some(stream) = app.current.and_then(|i| app.streams.get(i)) {
        rows.push(heading("Stream"));
        rows.push(Row {
            label: stream.name.clone(),
            detail: stream.description.clone(),
            ..Row::default()
        });
        let (w, h) = app.video.size();
        if w > 0 {
            rows.push(Row {
                label: format!("{w} × {h}"),
                detail: format!(
                    "{} of {}",
                    clock(app.video.position()),
                    clock(app.video.duration())
                ),
                ..Row::default()
            });
        }
    }
    rows
}

// ----- Layers -----

pub fn render(app: &App) {
    dom::set_html("stage", &stage(app));
    dom::set_class(
        "stage",
        if matches!(app.screen, Screen::Playing) {
            "stage"
        } else {
            "stage on"
        },
    );
    dom::set_html("osd", &osd(app));
    dom::set_class("osd", if app.osd_visible() { "osd on" } else { "osd" });
    let panel_html = app.panel.map(|p| panel(app, p)).unwrap_or_default();
    dom::set_html("panel", &panel_html);
    dom::set_class(
        "panel",
        if app.panel.is_some() {
            "panel on"
        } else {
            "panel"
        },
    );
    dom::set_html("nextup", &next_up(app));
    dom::set_html(
        "toast",
        &app.toast
            .as_ref()
            .map(|(text, _)| format!(r#"<div class="toast">{}</div>"#, escape(text)))
            .unwrap_or_default(),
    );
    dom::set_class(
        "spinner",
        if app.buffering() && app.panel.is_none() {
            "spinner on"
        } else {
            "spinner"
        },
    );
    dom::set_class(
        "subs",
        &format!(
            "subs size-{}{}{}",
            app.settings.subtitle_size,
            if app.settings.subtitle_box {
                " box"
            } else {
                ""
            },
            if app.osd_visible() { " raised" } else { "" }
        ),
    );
    if app.panel.is_some() {
        scroll_focus_into_view();
    }
}

fn scroll_focus_into_view() {
    let Ok(Some(el)) = dom::document().query_selector("#panel .row.focus") else {
        return;
    };
    let f = dom::get(el.as_ref(), "scrollIntoView");
    if let Ok(f) = f.dyn_into::<js_sys::Function>() {
        let options = js_sys::Object::new();
        dom::set(&options, "block", &JsValue::from_str("nearest"));
        let _ = f.call1(el.as_ref(), &options);
    }
}

fn backdrop(app: &App) -> String {
    match app.meta.as_ref().and_then(|m| m.backdrop.as_ref()) {
        Some(url) => format!(r#"<img class="backdrop" src="{}" alt="">"#, escape(url)),
        None => r#"<div class="backdrop plain"></div>"#.into(),
    }
}

fn hero(app: &App) -> String {
    let title = match app.meta.as_ref().and_then(|m| m.logo.as_ref()) {
        Some(logo) => format!(
            r#"<img class="logo" src="{}" alt="{}">"#,
            escape(logo),
            escape(&app.title())
        ),
        None => format!(r#"<h1>{}</h1>"#, escape(&app.title())),
    };
    let episode = app
        .episode_line()
        .map(|line| format!(r#"<div class="episode">{}</div>"#, escape(&line)))
        .unwrap_or_default();
    format!("{title}{episode}")
}

fn button(label: &str, focused: bool, index: usize) -> String {
    format!(
        r#"<div class="button{}" data-row="{index}">{}</div>"#,
        if focused { " focus" } else { "" },
        escape(label)
    )
}

fn stage(app: &App) -> String {
    match &app.screen {
        Screen::Playing => String::new(),
        Screen::Loading(status) => format!(
            r#"{}<div class="shade"></div><div class="hero">{}<div class="status"><span class="dot"></span>{}</div></div>"#,
            backdrop(app),
            hero(app),
            escape(status)
        ),
        Screen::SignIn { code, url, qr } => format!(
            r#"{}<div class="shade strong"></div>
            <div class="signin">
              <div class="qr">{qr}</div>
              <div class="copy">
                <h2>Sign in to Trakt</h2>
                <p>So your progress and watched episodes stay in sync with Luma.</p>
                <p class="small">Scan the code, or go to <b>{}</b> and enter</p>
                <div class="code">{}</div>
                {}
              </div>
            </div>"#,
            backdrop(app),
            escape(url.trim_start_matches("https://")),
            escape(code),
            button("Not now", true, 0)
        ),
        Screen::Resume(offer) => format!(
            r#"{}<div class="shade"></div><div class="hero">{}
               <div class="buttons">{}{}</div></div>"#,
            backdrop(app),
            hero(app),
            button(
                &format!("Resume from {}", offer_label(offer)),
                app.focus == 0,
                0
            ),
            button("Start over", app.focus == 1, 1)
        ),
        Screen::Message { title, body, retry } => format!(
            r#"{}<div class="shade strong"></div><div class="hero message"><h1>{}</h1><p>{}</p><div class="buttons">{}</div></div>"#,
            backdrop(app),
            escape(title),
            escape(body),
            if *retry {
                format!(
                    "{}{}",
                    button("Try again", true, 0),
                    r#"<div class="hint">Back returns to Luma</div>"#
                )
            } else {
                button("Back to Luma", true, 0)
            }
        ),
    }
}

fn osd(app: &App) -> String {
    if !matches!(app.screen, Screen::Playing) || app.current.is_none() {
        return String::new();
    }
    let duration = app.video.duration();
    let position = app.seek_preview.unwrap_or_else(|| app.video.position());
    let pct = |t: f64| {
        if duration > 0. {
            (t / duration * 100.).clamp(0., 100.)
        } else {
            0.
        }
    };
    let played = pct(position);
    let buffered = pct(app.video.buffered_end());
    let paused = app.video.paused() && app.first_frame;
    let center = if app.seek_preview.is_some() {
        format!(r#"<div class="center seek">{}</div>"#, clock(position))
    } else if paused {
        r#"<div class="center"><div class="pause-icon"><span></span><span></span></div></div>"#
            .into()
    } else {
        String::new()
    };
    let episode = app
        .episode_line()
        .map(|line| format!(r#"<div class="e">{}</div>"#, escape(&line)))
        .unwrap_or_default();
    format!(
        r#"<div class="top"><div class="t">{}</div>{episode}</div>{center}
        <div class="bottom">
          <div class="bar"><div class="buf" style="width:{buffered:.2}%"></div><div class="played" style="width:{played:.2}%"></div><div class="knob" style="left:{played:.2}%"></div></div>
          <div class="times"><span>{}</span><span>{}</span></div>
          <div class="keys"><span class="k red"></span>Info<span class="k green"></span>Streams<span class="k yellow"></span>Audio &amp; Subtitles<span class="k blue"></span>Settings</div>
        </div>"#,
        escape(&app.title()),
        clock(position),
        if duration > 0. {
            format!("-{}", clock(duration - position))
        } else {
            String::new()
        },
    )
}

fn next_up(app: &App) -> String {
    let Some(next) = &app.next else {
        return String::new();
    };
    let Some(shown) = next.shown_at else {
        return String::new();
    };
    if next.dismissed || app.panel.is_some() || !matches!(app.screen, Screen::Playing) {
        return String::new();
    }
    let label = luma_core::ids::episode_label(next.at);
    let name = match &next.name {
        Some(name) => format!("{label} · {name}"),
        None => label,
    };
    let still = next
        .still
        .as_ref()
        .map(|url| format!(r#"<img class="still" src="{}" alt="">"#, escape(url)))
        .unwrap_or_default();
    let count = if app.settings.autoplay_next && !app.ended {
        let left = (app.settings.countdown_seconds - (now_ms() - shown) / 1000.)
            .ceil()
            .max(0.);
        format!("Playing in {left:.0}")
    } else {
        "Press OK to play".into()
    };
    format!(
        r#"<div class="card">{still}<div class="text"><div class="label">Next Episode</div><div class="name">{}</div><div class="count">{count}</div><div class="hint">OK Play now · Back Hide</div></div></div>"#,
        escape(&name)
    )
}

fn panel(app: &App, panel: Panel) -> String {
    let title = match panel {
        Panel::Picker => "Streams",
        Panel::Tracks => "Audio & Subtitles",
        Panel::Settings => "Settings",
        Panel::Info => "Info",
    };
    let notice = app
        .notice
        .as_ref()
        .filter(|_| panel == Panel::Picker)
        .map(|n| format!(r#"<div class="notice">{}</div>"#, escape(n)))
        .unwrap_or_default();
    let rows: String = rows(app, panel)
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut class = String::from("row");
            if r.id.is_none() {
                class.push_str(if r.label.is_empty() || !r.detail.is_empty() {
                    " text"
                } else {
                    " heading"
                });
            }
            if r.id.is_some() && i == app.focus {
                class.push_str(" focus");
            }
            if r.selected {
                class.push_str(" selected");
            }
            if r.disabled {
                class.push_str(" disabled");
            }
            let detail = if r.detail.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="d">{}</div>"#, escape(&r.detail))
            };
            let value = if r.value.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="v">{}</div>"#, escape(&r.value))
            };
            let data = if r.id.is_some() {
                format!(r#" data-row="{i}""#)
            } else {
                String::new()
            };
            format!(
                r#"<div class="{class}"{data}><div class="l">{}{detail}</div>{value}</div>"#,
                escape(&r.label)
            )
        })
        .collect();
    format!(
        r#"<div class="sheet"><div class="ptitle">{title}</div>{notice}<div class="rows">{rows}</div></div>"#
    )
}
