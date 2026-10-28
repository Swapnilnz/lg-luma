//! Drawing: the app's state as HTML in a few fixed layers over the `<video>`.
//! Each layer is only written when its HTML changes.

use crate::app::{App, OsdButton, OsdFocus, Panel, Screen, SubChoice};
use crate::dom::{self, escape, now_ms};
use luma_core::resume::Offer;
use luma_core::selection::{language_name, same_language};
use luma_core::time::{clock, short, wall_clock};
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
    AutoSkip,
    AudioLanguage,
    SubtitleLanguage,
    SubtitlesOn,
    Trakt,
    Speed(f64),
    ShowAllSubs,
    Anime,
    Spoilers,
}

impl RowId {
    /// Rows changed with Left/Right, drawn with a chevron.
    fn adjustable(&self) -> bool {
        matches!(
            self,
            RowId::SubSize
                | RowId::SubBox
                | RowId::SubOffset
                | RowId::Autoplay
                | RowId::NextCard
                | RowId::Countdown
                | RowId::AutoSkip
                | RowId::Anime
                | RowId::Spoilers
                | RowId::AudioLanguage
                | RowId::SubtitleLanguage
                | RowId::SubtitlesOn
        )
    }
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
        Panel::Speed => crate::app::SPEEDS
            .iter()
            .map(|&rate| {
                let mut r = row(
                    RowId::Speed(rate),
                    if rate == 1. {
                        "Normal".to_owned()
                    } else {
                        format!("{rate}×")
                    },
                    "",
                );
                r.selected = (app.speed - rate).abs() < 0.01;
                r
            })
            .collect(),
        // The shelf and the dial draw themselves; they have no rows.
        Panel::Episodes | Panel::Delay => vec![],
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
                if dom::palm().is_some() {
                    "This TV doesn't list the file's audio tracks".into()
                } else {
                    "Desktop Chrome doesn't list audio tracks; the TV does".into()
                }
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
    let (menu, hidden) = app.sub_menu();
    let mut options: std::collections::HashMap<String, usize> = Default::default();
    for candidate in &menu {
        let language = language_label(&candidate.entry.language);
        let label = if candidate.entry.forced {
            format!("{language} (Forced)")
        } else {
            language.clone()
        };
        let id = match candidate.choice {
            SubChoice::Embedded(i) => RowId::SubEmbedded(i),
            SubChoice::Addon(i) => RowId::SubAddon(i),
            SubChoice::Off => continue,
        };
        let mut r = row(id, label, "");
        r.detail = if candidate.embedded {
            if candidate.label.is_empty() {
                "In the file".into()
            } else {
                format!("In the file · {}", candidate.label)
            }
        } else if candidate.entry.forced {
            "Only the lines in other languages".into()
        } else if candidate.label.is_empty() {
            let n = options.entry(language).or_default();
            *n += 1;
            format!("Addon · option {n}")
        } else {
            candidate.label.clone()
        };
        r.selected = app.sub_choice == candidate.choice;
        if r.selected && !candidate.embedded && app.sub_track.is_none() {
            r.value = "Loading…".into();
        }
        rows.push(r);
    }
    if hidden > 0 {
        rows.push(row(
            RowId::ShowAllSubs,
            "Other languages",
            format!("Show {hidden}"),
        ));
    } else if app.show_all_subs {
        rows.push(row(RowId::ShowAllSubs, "Other languages", "Hide"));
    }
    if menu.is_empty() && app.addon_subs.is_empty() && !app.config.subtitle_addons.is_empty() {
        rows.push(Row {
            label: "No subtitles found yet".into(),
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
    let mut timing = row(
        RowId::SubOffset,
        "Timing",
        delay_label(app.settings.subtitle_offset_ms),
    );
    timing.detail = "OK for the dial".into();
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
            match s.next_up_percent {
                at if at <= 0. => "When the credits start".to_owned(),
                at if at >= 100. => "At the end".to_owned(),
                at => format!("At {at:.0}%"),
            },
        ),
        row(
            RowId::Countdown,
            "Countdown",
            format!("{:.0} s", s.countdown_seconds),
        ),
        {
            let mut r = row(
                RowId::AutoSkip,
                "Skip intros automatically",
                on_off(s.auto_skip),
            );
            r.detail = "Intros and recaps from IntroDB; off shows a Skip button".into();
            r
        },
        {
            let mut r = row(RowId::Spoilers, "Hide spoilers", on_off(s.hide_spoilers));
            r.detail = "Blur stills of episodes you haven't watched".into();
            r
        },
        heading("Languages"),
        {
            let mut r = row(RowId::Anime, "Anime defaults", on_off(s.anime_defaults));
            r.detail = "Japanese audio and English subtitles for anime".into();
            r
        },
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
    // Kept while the stage fades out, so the picture appears under the full logo.
    let stage_html = stage(app);
    if !stage_html.is_empty() {
        dom::set_html_with("stage", &stage_html, "lstatus");
    }
    let loading = matches!(app.screen, Screen::Loading(_)) || app.starting();
    if loading {
        dom::set_html("lstatus", &escape(&app.loading_status()));
    }
    dom::set_var("stage", "--p", &format!("{:.3}", app.load_progress()));
    let stage_class = if matches!(app.screen, Screen::Playing) && !app.starting() {
        "stage"
    } else if loading {
        "stage on loading"
    } else {
        "stage on"
    };
    dom::set_class("stage", stage_class);
    let pause = app.pause_screen();
    dom::set_html(
        "pause",
        &if pause {
            pause_screen(app)
        } else {
            String::new()
        },
    );
    dom::set_class("pause", if pause { "pause on" } else { "pause" });
    dom::set_html("osd", &osd(app));
    dom::set_class("osd", if app.osd_visible() { "osd on" } else { "osd" });
    let (panel_html, panel_class) = match app.panel {
        Some(Panel::Episodes) => (shelf(app), "panel shelf on"),
        Some(p) => (panel(app, p), "panel popover on"),
        None => (String::new(), "panel"),
    };
    dom::set_html("panel", &panel_html);
    dom::set_class("panel", panel_class);
    // The card is written once per episode; only its countdown changes after that, so the
    // card's rise-in and its still don't replay every second (which made it flash).
    let (card, count, run) = if pause {
        (String::new(), String::new(), 0.)
    } else {
        next_up(app)
    };
    dom::set_html_with("nextup", &card, "nextcount");
    if !card.is_empty() {
        dom::set_html("nextcount", &escape(&count));
        // The Play button fills as the countdown runs; it holds while paused.
        dom::set_var("nextup", "--run", &format!("{run:.3}"));
    }
    let skip = app.skip_offer().map(|(kind, _)| {
        format!(
            r#"<div class="skipbtn{}" data-skip="1">{}</div>"#,
            if app.osd_visible() { "" } else { " focus" },
            kind.label()
        )
    });
    dom::set_class(
        "skip",
        match (&skip, app.osd_visible()) {
            (Some(_), true) => "skip on raised",
            (Some(_), false) => "skip on",
            (None, _) => "skip",
        },
    );
    if let Some(html) = &skip {
        dom::set_html("skip", html);
    }
    dom::set_html(
        "toast",
        &app.toast
            .as_ref()
            .map(|(text, _)| format!(r#"<div class="toast">{}</div>"#, escape(text)))
            .unwrap_or_default(),
    );
    dom::set_class(
        "spinner",
        if app.buffering() && app.panel.is_none() && !pause && !app.starting() {
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
        keep_focus_visible();
    }
}

/// Scrolls the panel's list (never the page) so the focused row is in view. The page itself
/// must not scroll: scrollIntoView on a panel still sliding in moved the whole player.
fn keep_focus_visible() {
    let document = dom::document();
    let (Ok(Some(list)), Ok(Some(row))) = (
        document.query_selector("#panel .rows"),
        document.query_selector("#panel .row.focus"),
    ) else {
        return;
    };
    let (Ok(list), Ok(row)) = (
        list.dyn_into::<web_sys::HtmlElement>(),
        row.dyn_into::<web_sys::HtmlElement>(),
    ) else {
        return;
    };
    let top = row.offset_top();
    let bottom = top + row.offset_height();
    let view = list.client_height();
    let scroll = list.scroll_top();
    if top < scroll + 12 {
        list.set_scroll_top((top - 12).max(0));
    } else if bottom > scroll + view - 12 {
        list.set_scroll_top(bottom - view + 12);
    }
}

/// The local time `ms` from now, e.g. `10:32 pm`.
fn time_of_day(ms_from_now: f64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(now_ms() + ms_from_now.max(0.)));
    wall_clock(date.get_hours(), date.get_minutes())
}

fn backdrop(app: &App) -> String {
    match app.meta.as_ref().and_then(|m| m.backdrop.as_ref()) {
        Some(url) => format!(r#"<img class="backdrop" src="{}" alt="">"#, escape(url)),
        None => r#"<div class="backdrop plain"></div>"#.into(),
    }
}

fn hero(app: &App) -> String {
    let mark = match app.meta.as_ref().and_then(|m| m.logo.as_ref()) {
        Some(logo) => format!(
            r#"<img class="logo" src="{}" alt="{}">"#,
            escape(logo),
            escape(&app.title())
        ),
        None => format!(r#"<h1>{}</h1>"#, escape(&app.title())),
    };
    // The mark twice: a faint ghost, and a copy that fills in over it as loading goes on.
    let title = format!(
        r#"<div class="markfill"><div class="ghost">{mark}</div><div class="fill">{mark}</div></div>"#
    );
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
        Screen::Playing if !app.starting() => String::new(),
        Screen::Loading(_) | Screen::Playing => format!(
            r#"{}<div class="shade"></div><div class="hero">{}<div class="status" id="lstatus"></div></div>"#,
            backdrop(app),
            hero(app),
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
    } else {
        String::new()
    };
    let episode = app
        .episode_line()
        .map(|line| format!(r#"<div class="e">{}</div>"#, escape(&line)))
        .unwrap_or_default();
    let remaining = if duration > 0. {
        let left = (duration - position) / app.speed.max(0.1);
        format!(
            "-{}<span class=\"ends\">Ends at {}</span>",
            clock(duration - position),
            time_of_day(left * 1000.)
        )
    } else {
        String::new()
    };
    let scrub_focus = app.osd_focus == OsdFocus::Scrubber && app.panel.is_none();
    let buttons = app.osd_buttons();
    let button_html = |(i, b): (usize, &OsdButton)| -> String {
        let focus = app.panel.is_none() && app.osd_focus == OsdFocus::Button(i);
        let active = app.panel.is_some() && b.panel() == app.panel;
        let icon = match b {
            OsdButton::PlayPause if paused || !app.first_frame => icons::PLAY,
            OsdButton::PlayPause => icons::PAUSE,
            OsdButton::Episodes => icons::EPISODES,
            OsdButton::Next => icons::NEXT,
            OsdButton::Tracks => icons::SUBTITLES,
            OsdButton::Speed => icons::SPEED,
            OsdButton::Streams => icons::STREAMS,
            OsdButton::Info => icons::INFO,
            OsdButton::Settings => icons::SETTINGS,
        };
        let label = match b {
            OsdButton::PlayPause if paused => "Play".to_owned(),
            OsdButton::PlayPause => "Pause".to_owned(),
            OsdButton::Speed if (app.speed - 1.).abs() > 0.01 => format!("Speed · {}×", app.speed),
            other => other.label().to_owned(),
        };
        let badge = match b {
            OsdButton::Speed if (app.speed - 1.).abs() > 0.01 => {
                format!(r#"<span class="badge">{}×</span>"#, app.speed)
            }
            _ => String::new(),
        };
        format!(
            r#"<div class="obtn{}{}" data-osd="{i}">{}{badge}<span class="tip">{}</span></div>"#,
            if focus { " focus" } else { "" },
            if active { " active" } else { "" },
            icons::svg(icon),
            escape(&label)
        )
    };
    let row: String = buttons.iter().enumerate().map(button_html).collect();
    format!(
        r#"<div class="top"><div class="t">{}</div>{episode}</div>{center}
        <div class="bottom">
          <div class="scrub{}">
            <div class="bar"><div class="buf" style="width:{buffered:.2}%"></div><div class="played" style="width:{played:.2}%"></div><div class="knob" style="left:{played:.2}%"></div></div>
            <div class="times"><span>{}</span><span>{remaining}</span></div>
          </div>
          <div class="controls"><div class="group">{row}</div></div>
        </div>"#,
        escape(&app.title()),
        if scrub_focus { " focus" } else { "" },
        clock(position),
    )
}

/// Paused for 10 s: what's on, the blurb, and the time, as Nuvio shows it.
fn pause_screen(app: &App) -> String {
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
        .map(|line| format!(r#"<div class="pep">{}</div>"#, escape(&line)))
        .unwrap_or_default();
    let blurb = app
        .overview()
        .map(|text| format!(r#"<p class="pblurb">{}</p>"#, escape(&text)))
        .unwrap_or_default();
    let position = app.video.position();
    let duration = app.video.duration();
    let left = if duration > 0. {
        format!(" · {} left", short(duration - position))
    } else {
        String::new()
    };
    format!(
        r#"<div class="pshade"></div>
        <div class="pclock">{}</div>
        <div class="pinfo"><div class="plabel">You're watching</div>{title}{episode}{blurb}<div class="ptime">Paused at {}{left}</div></div>"#,
        time_of_day(0.),
        clock(position),
    )
}

/// The Episodes shelf: season tabs over a row of episode stills, as Apple TV draws it.
fn shelf(app: &App) -> String {
    const PITCH: i32 = 428;
    const WINDOW: i32 = 1680;
    let current = app.launch.episode_key();
    let tabs: String = app
        .shelf_seasons()
        .iter()
        .map(|&season| {
            format!(
                r#"<div class="tab{}{}" data-season="{season}">Season {season}</div>"#,
                if season == app.shelf_season {
                    " on"
                } else {
                    ""
                },
                if season == app.shelf_season && app.shelf_row == 0 {
                    " focus"
                } else {
                    ""
                },
            )
        })
        .collect();
    let body = match app.shelf_episodes() {
        None => r#"<div class="empty">Loading episodes…</div>"#.to_owned(),
        Some(list) if list.is_empty() => {
            r#"<div class="empty">No episode list from TMDB for this season.</div>"#.to_owned()
        }
        Some(list) => {
            let total = list.len() as i32 * PITCH;
            let offset = ((app.shelf_focus as i32 - 1).max(0) * PITCH).min((total - WINDOW).max(0));
            let cards: String = list
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let now = current == Some((app.shelf_season, e.number));
                    let focus = app.shelf_row == 1 && i == app.shelf_focus;
                    let thumb = match &e.still {
                        Some(url) => format!(r#"<img src="{}" alt="">"#, escape(url)),
                        None => format!(r#"<div class="noimg">{}</div>"#, e.number),
                    };
                    let meta = e.runtime.map(|m| short(m * 60.)).unwrap_or_default();
                    let at = (app.shelf_season, e.number);
                    // Watched: a tick. Partly watched: how far, as Apple TV draws it.
                    let watched = app.watched.contains(&at) && !now;
                    // Hide spoilers: stills of episodes not started yet are blurred.
                    let spoiler = app.settings.hide_spoilers
                        && !now
                        && !app.watched.contains(&at)
                        && !app.partial.contains_key(&at);
                    let progress = app
                        .partial
                        .get(&at)
                        .filter(|p| **p > 1. && **p < 95. && !watched)
                        .map(|p| {
                            format!(r#"<div class="prog"><div style="width:{p:.1}%"></div></div>"#)
                        })
                        .unwrap_or_default();
                    let tick = if watched {
                        format!(r#"<span class="tick">{}</span>"#, icons::svg(icons::CHECK))
                    } else {
                        String::new()
                    };
                    format!(
                        r#"<div class="ep{}{}{}{}" data-ep="{i}"><div class="thumb">{thumb}{}{tick}{progress}</div><div class="en">Episode {}{}</div><div class="et">{}</div></div>"#,
                        if focus { " focus" } else { "" },
                        if now { " now" } else { "" },
                        if watched { " watched" } else { "" },
                        if spoiler { " spoiler" } else { "" },
                        if now { r#"<span class="nowtag">Now Playing</span>"# } else { "" },
                        e.number,
                        if meta.is_empty() { String::new() } else { format!(" · {meta}") },
                        escape(&e.name),
                    )
                })
                .collect();
            format!(
                r#"<div class="cards-window"><div class="cards" style="transform:translateX(-{offset}px)">{cards}</div></div>"#
            )
        }
    };
    format!(
        r#"<div class="shelf-inner"><div class="stitle">{}</div><div class="tabs">{tabs}</div>{body}</div>"#,
        escape(&app.title())
    )
}

/// The Next Up card's HTML, its countdown text (drawn into `#nextcount` separately) and how
/// far the countdown has run, 0 to 1. Apple TV's: a large 16:9 still, full size, with the
/// episode over it and a Play button that fills as the countdown runs.
fn next_up(app: &App) -> (String, String, f64) {
    let none = (String::new(), String::new(), 0.);
    let Some(next) = &app.next else {
        return none;
    };
    let Some(shown) = next.shown_at else {
        return none;
    };
    if next.dismissed || app.panel.is_some() || !matches!(app.screen, Screen::Playing) {
        return none;
    }
    let label = luma_core::ids::episode_label(next.at);
    let name = next.name.clone().unwrap_or_default();
    // Hide spoilers: the show's backdrop rather than a still from the episode.
    let spoiler = app.settings.hide_spoilers && !app.watched.contains(&next.at);
    let backdrop = app.meta.as_ref().and_then(|m| m.backdrop.clone());
    let still = next
        .still
        .as_deref()
        .filter(|_| !spoiler)
        .map(luma_core::tmdb::full_size);
    let art = still
        .or(backdrop)
        .map(|url| format!(r#"<img class="still" src="{}" alt="">"#, escape(&url)))
        .unwrap_or_else(|| r#"<div class="still plain"></div>"#.to_owned());
    let runtime = next
        .runtime
        .filter(|minutes| *minutes > 0.)
        .map(|minutes| format!(r#"<div class="meta">{}</div>"#, short(minutes * 60.)))
        .unwrap_or_default();
    let counting = app.settings.autoplay_next && !app.ended;
    let countdown = app.settings.countdown_seconds.max(1.);
    let elapsed = ((now_ms() - shown) / 1000.).clamp(0., countdown);
    let (count, run) = if counting {
        let left = (countdown - elapsed).ceil().max(0.);
        (format!("Playing in {left:.0}"), elapsed / countdown)
    } else {
        ("Play".to_owned(), 0.)
    };
    let card = format!(
        r#"<div class="card"><div class="art">{art}<div class="artshade"></div><div class="over"><div class="label">Up Next</div><div class="ep">{}</div><div class="name">{}</div>{runtime}</div></div><div class="actions"><div class="play"><div class="fill"></div><div class="pl">{}<span id="nextcount"></span></div></div><div class="hint">Back to keep watching</div></div></div>"#,
        escape(&label),
        escape(&name),
        icons::svg(icons::PLAY),
    );
    (card, count, run)
}

/// `+0.5 s`, `−1.2 s` or `In sync`.
fn delay_label(ms: i32) -> String {
    match ms {
        0 => "In sync".to_owned(),
        ms if ms > 0 => format!("+{:.1} s", f64::from(ms) / 1000.),
        ms => format!("−{:.1} s", f64::from(-ms) / 1000.),
    }
}

/// The subtitle timing dial: a ruler that slides under a fixed needle, from −30 s to +30 s.
fn delay_dial(app: &App) -> String {
    const PX_PER_SECOND: f64 = 110.;
    const CENTER: f64 = 289.;
    let ms = app.settings.subtitle_offset_ms;
    let max = crate::app::MAX_OFFSET_MS / 1000;
    let ticks: String = (-max * 2..=max * 2)
        .map(|half| {
            let seconds = f64::from(half) / 2.;
            let x = (seconds + f64::from(max)) * PX_PER_SECOND;
            if half % 2 == 0 {
                let label = match half / 2 {
                    0 => "0".to_owned(),
                    n if n > 0 => format!("+{n}"),
                    n => format!("−{}", -n),
                };
                format!(r#"<div class="tk major" style="left:{x:.0}px"><span>{label}</span></div>"#)
            } else {
                format!(r#"<div class="tk" style="left:{x:.0}px"></div>"#)
            }
        })
        .collect();
    let offset = CENTER - (f64::from(ms) / 1000. + f64::from(max)) * PX_PER_SECOND;
    let meaning = match ms {
        0 => "Subtitles are in sync with the picture".to_owned(),
        ms if ms > 0 => format!("Subtitles appear {:.1} s later", f64::from(ms) / 1000.),
        ms => format!("Subtitles appear {:.1} s earlier", f64::from(-ms) / 1000.),
    };
    format!(
        r#"<div class="sheet dial"><div class="ptitle">Subtitle Timing</div>
        <div class="dial-value{}">{}</div><div class="dial-sub">{meaning}</div>
        <div class="ruler"><div class="ticks" style="transform:translateX({offset:.0}px)">{ticks}</div><div class="needle"></div></div>
        <div class="dial-hint">◀ ▶ 0.1 s · ▲ ▼ 1 s · OK Done</div></div>"#,
        if ms == 0 { " zero" } else { "" },
        delay_label(ms),
    )
}

fn panel(app: &App, panel: Panel) -> String {
    if panel == Panel::Delay {
        return delay_dial(app);
    }
    let title = match panel {
        Panel::Picker => "Streams",
        Panel::Tracks => "Audio & Subtitles",
        Panel::Settings => "Settings",
        Panel::Info => "Info",
        Panel::Speed => "Playback Speed",
        Panel::Episodes => "Episodes",
        Panel::Delay => "Subtitle Timing",
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
            if r.disabled {
                class.push_str(" disabled");
            }
            let check = if r.id.is_some() {
                format!(
                    r#"<div class="chk">{}</div>"#,
                    if r.selected {
                        icons::svg(icons::CHECK)
                    } else {
                        String::new()
                    }
                )
            } else {
                String::new()
            };
            let detail = if r.detail.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="d">{}</div>"#, escape(&r.detail))
            };
            let chevron = r.id.as_ref().is_some_and(RowId::adjustable);
            let value = if r.value.is_empty() && !chevron {
                String::new()
            } else {
                format!(
                    r#"<div class="v">{}{}</div>"#,
                    escape(&r.value),
                    if chevron {
                        icons::svg(icons::CHEVRON)
                    } else {
                        String::new()
                    }
                )
            };
            let data = if r.id.is_some() {
                format!(r#" data-row="{i}""#)
            } else {
                String::new()
            };
            format!(
                r#"<div class="{class}"{data}>{check}<div class="l">{}{detail}</div>{value}</div>"#,
                escape(&r.label)
            )
        })
        .collect();
    format!(
        r#"<div class="sheet"><div class="ptitle">{title}</div>{notice}<div class="rows">{rows}</div></div>"#
    )
}

/// Line icons (24 x 24, after Lucide), drawn in the current text colour.
mod icons {
    pub fn svg(body: &str) -> String {
        format!(
            r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"#
        )
    }

    pub const PLAY: &str = r#"<path d="M7 4.5v15l12.5-7.5z" fill="currentColor"/>"#;
    pub const PAUSE: &str = r#"<rect x="6" y="4.5" width="4" height="15" rx="1" fill="currentColor" stroke="none"/><rect x="14" y="4.5" width="4" height="15" rx="1" fill="currentColor" stroke="none"/>"#;
    pub const EPISODES: &str = r#"<rect x="3" y="4" width="7" height="7" rx="1.5"/><rect x="3" y="13" width="7" height="7" rx="1.5"/><path d="M14 6h7M14 9.5h4.5M14 15h7M14 18.5h4.5"/>"#;
    pub const NEXT: &str =
        r#"<path d="M5 4.5v15l11-7.5z" fill="currentColor"/><path d="M19 5v14"/>"#;
    pub const SUBTITLES: &str = r#"<rect x="2.5" y="4.5" width="19" height="15" rx="3"/><path d="M7 15h4M14 15h3M7 11.5h2M12 11.5h5"/>"#;
    pub const SPEED: &str = r#"<path d="M12 14.5l4-4.5"/><path d="M3.4 18.5a10 10 0 1 1 17.2 0"/><circle cx="12" cy="14.5" r="1" fill="currentColor"/>"#;
    pub const STREAMS: &str = r#"<path d="M12.8 2.2a2 2 0 0 0-1.6 0L2.6 6.1a1 1 0 0 0 0 1.8l8.6 3.9a2 2 0 0 0 1.6 0l8.6-3.9a1 1 0 0 0 0-1.8z"/><path d="M2 12.6l9.2 4.2a2 2 0 0 0 1.6 0L22 12.6"/><path d="M2 17.6l9.2 4.2a2 2 0 0 0 1.6 0L22 17.6"/>"#;
    pub const INFO: &str = r#"<circle cx="12" cy="12" r="9.5"/><path d="M12 16.5v-5M12 7.8h.01"/>"#;
    pub const SETTINGS: &str = r#"<path d="M20 7h-9M14 17H4"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/>"#;
    pub const CHECK: &str = r#"<path d="M20 6 9 17l-5-5"/>"#;
    pub const CHEVRON: &str = r#"<path d="m9 18 6-6-6-6"/>"#;
}
