//! The Play page (box art rail, info panel, moving screenshot backdrop, details sheet),
//! and the console launch: the picked card lifts, grows into a full-screen
//! view of the game, waits with an orbit and a step track while the PC wakes, bursts when it
//! is ready, and fades to black before the TV switches to the PC.
use super::{Assets, Scene, c, clip_words, text};
use crate::console::{ArtKind, Game, Phase, Session};
use crate::play::{
    COVER_H, COVER_PITCH, COVER_W, ConfirmPhase, DELETE_CHOICES, DeleteChoice, PANEL_IN,
    PlayButton, RAIL_X, RAIL_Y, SHEET_IN, achievement_share, chips, delete_error_message,
    ease_out_cubic, hours, played_ago, size,
};
use crate::renderer::{Blend, Draw, Filter, Pattern, Quad, QuadImage, WHITE};
use crate::state::{Rect, State, Target};
use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Instant;

const FULL: Rect = Rect::new(0., 0., 1920., 1080.);
/// The console's colours: deep violet to night blue, and the accent for focus and progress.
pub const TOP: u32 = 0x5b3df5;
pub const BOTTOM: u32 = 0x10195a;
const ACCENT: u32 = 0x8b7bff;
const GOOD: u32 = 0x3ddc84;
/// A low battery, and a failed delete.
const LOW: u32 = 0xff5c5c;
const COVER_RADIUS: f32 = 14.;

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn scaled(rect: Rect, scale: f32) -> Rect {
    let (cx, cy) = rect.center();
    let (w, h) = (rect.w * scale, rect.h * scale);
    Rect::new(cx - w / 2., cy - h / 2., w, h)
}

/// An image filling `rect` (cropped), rounded, tinted.
fn picture(
    out: &mut Scene,
    image: &Arc<crate::app_artwork::Artwork>,
    rect: Rect,
    radius: f32,
    tint: [f32; 4],
    filter: Filter,
) {
    let mut quad = Quad::new(rect, tint);
    quad.corners = [radius; 4];
    quad.image = Some(QuadImage::App(image.clone()));
    quad.filter = filter;
    out.draw.push(Draw::Quad(Box::new(quad)));
}

/// A gradient running left to right (a vertical one, turned): `left` to `right`.
fn horizontal(out: &mut Scene, rect: Rect, left: [f32; 4], right: [f32; 4]) {
    let (cx, cy) = rect.center();
    let mut quad = Quad::new(
        Rect::new(cx - rect.h / 2., cy - rect.w / 2., rect.h, rect.w),
        left,
    );
    // Turned a quarter clockwise, its top ends up on the right.
    quad.colors = [right, left];
    quad.angle = 90.;
    out.draw.push(Draw::Quad(Box::new(quad)));
}

/// A light sweep across `rect` (a focused card's sheen), `t` from 0 to 1.
fn sheen(out: &mut Scene, rect: Rect, t: f32, strength: f32) {
    if !(0. ..1.).contains(&t) {
        return;
    }
    let width = rect.w * 0.35;
    let skew = rect.h * 0.35;
    let x = mix(
        rect.x - width - skew,
        rect.x + rect.w + skew,
        ease_out_cubic(t),
    );
    // Only once the band is properly inside: clamped at an edge it would be a sliver.
    if x + width + skew < rect.x + width * 0.5 || x > rect.x + rect.w - width * 0.5 {
        return;
    }
    let clampx = |x: f32| x.clamp(rect.x, rect.x + rect.w);
    let alpha = strength * (1. - t).powf(0.5);
    let mut quad = Quad::new(rect, c(0xffffff, alpha));
    quad.colors = [c(0xffffff, alpha), c(0xffffff, alpha * 0.25)];
    quad.blend = Blend::Screen;
    quad.points = Some([
        [clampx(x + skew), rect.y],
        [clampx(x + skew + width), rect.y],
        [clampx(x + width), rect.y + rect.h],
        [clampx(x), rect.y + rect.h],
    ]);
    out.draw.push(Draw::Quad(Box::new(quad)));
}

/// A slow, gentle grain over generated surfaces.
fn grain(out: &mut Scene, rect: Rect, radius: f32, strength: f32) {
    let mut quad = Quad::new(rect, c(0xffffff, 0.05));
    quad.corners = [radius; 4];
    quad.filter = Filter::Pattern(Pattern::Grain, 2., strength);
    quad.blend = Blend::Screen;
    out.draw.push(Draw::Quad(Box::new(quad)));
}

/// The violet surface with drifting light, for a game without art and the empty page.
fn aurora(out: &mut Scene, rect: Rect, radius: f32, t: f32, effects: bool) {
    out.draw.push(Draw::RoundedGradient(
        rect,
        radius,
        c(TOP, 1.),
        c(BOTTOM, 1.),
    ));
    if !effects {
        return;
    }
    let blobs = [
        (0.22, 0.30, 0.55, 0x9d6bff, 0.42, 0.11),
        (0.74, 0.62, 0.48, 0x2f6bff, 0.36, 0.08),
        (0.52, 0.90, 0.60, 0xff5ec4, 0.20, 0.06),
    ];
    for (index, (x, y, size, rgb, alpha, speed)) in blobs.into_iter().enumerate() {
        let phase = t * speed * TAU + index as f32 * 2.1;
        let (cx, cy) = (
            rect.x + rect.w * (x + 0.06 * phase.sin()),
            rect.y + rect.h * (y + 0.05 * (phase * 1.3).cos()),
        );
        let d = rect.w.max(rect.h) * size;
        out.draw.push(Draw::Shadow(
            Rect::new(cx - d / 2., cy - d / 2., d, d),
            d / 2.,
            d * 0.45,
            c(rgb, alpha),
        ));
    }
    grain(out, rect, radius, 0.35);
}

fn game_art<'a>(
    state: &'a State,
    game: &Game,
    kinds: &[ArtKind],
) -> Option<&'a Arc<crate::app_artwork::Artwork>> {
    kinds
        .iter()
        .find_map(|kind| state.play.image(&game.key, *kind))
}

/// A cover, its art or a generated poster.
fn cover(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    game: &Game,
    rect: Rect,
    brightness: f32,
    grey: bool,
) {
    match state.play.image(&game.key, ArtKind::Cover) {
        Some(image) => picture(
            out,
            image,
            rect,
            COVER_RADIUS * rect.w / COVER_W,
            [brightness, brightness, brightness, 1.],
            if grey { Filter::Grey } else { Filter::None },
        ),
        None if state.play.image(&game.key, ArtKind::Hero).is_some() => {
            // A poster from the backdrop: its middle, darkened toward the foot, the logo
            // (else the title) across the bottom.
            let start = out.draw.len();
            let radius = COVER_RADIUS * rect.w / COVER_W;
            if let Some(hero) = state.play.image(&game.key, ArtKind::Hero) {
                picture(
                    out,
                    hero,
                    rect,
                    radius,
                    [0.95, 0.95, 0.95, 1.],
                    if grey { Filter::Grey } else { Filter::None },
                );
            }
            out.draw.push(Draw::RoundedGradient(
                Rect::new(rect.x, rect.y + rect.h * 0.4, rect.w, rect.h * 0.6),
                radius,
                c(0x05060a, 0.),
                c(0x05060a, 0.85),
            ));
            let area = Rect::new(
                rect.x + rect.w * 0.1,
                rect.y + rect.h * 0.62,
                rect.w * 0.8,
                rect.h * 0.3,
            );
            match state.play.image(&game.key, ArtKind::Logo) {
                Some(logo) => {
                    let (w, h) = (logo.pixels.width() as f32, logo.pixels.height() as f32);
                    let scale = (area.w / w).min(area.h / h);
                    let fit = Rect::new(
                        area.x + (area.w - w * scale) / 2.,
                        area.y + (area.h - h * scale) / 2.,
                        w * scale,
                        h * scale,
                    );
                    out.draw.push(Draw::AppImage(logo.clone(), fit, WHITE));
                }
                None => out.text(
                    clip_words(&game.title, 40),
                    area,
                    text((rect.w / 7.4).clamp(16., 34.), 650, -0.03).leading(1.1),
                    WHITE,
                    true,
                ),
            }
            out.fade(start, 1., brightness);
        }
        None => {
            let start = out.draw.len();
            aurora(
                out,
                rect,
                COVER_RADIUS,
                game.key.len() as f32,
                !state.preferences.reduced_effects,
            );
            let size = (rect.w / 7.4).clamp(16., 34.);
            out.icon(
                assets,
                "gaming",
                Rect::new(
                    rect.x + rect.w / 2. - size * 0.7,
                    rect.y + rect.h * 0.28,
                    size * 1.4,
                    size * 1.4,
                ),
                c(0xffffff, 0.8),
            );
            out.text(
                clip_words(&game.title, 40),
                Rect::new(
                    rect.x + 14.,
                    rect.y + rect.h * 0.5,
                    rect.w - 28.,
                    rect.h * 0.4,
                ),
                text(size, 650, -0.03).leading(1.1),
                WHITE,
                true,
            );
            out.fade(start, 1., brightness);
        }
    }
}

fn input_label(state: &State) -> String {
    state
        .preferences
        .console
        .as_ref()
        .and_then(|config| config.input.strip_prefix("com.webos.app.hdmi"))
        .map_or("HDMI".into(), |n| format!("HDMI {n}"))
}

/// The status line: whether the PC is up, or playing.
fn status(state: &State, game: Option<&Game>) -> (String, u32, f32) {
    let language = state.preferences.language;
    if game.is_some_and(|game| game.running) {
        (language.t("playingNow").into(), GOOD, 1.)
    } else if state.console_awake == Some(true) {
        (language.t("pcReady").into(), GOOD, 1.)
    } else {
        (language.t("pcAsleep").into(), 0xc8cbd6, 0.55)
    }
}

fn status_line(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    game: Option<&Game>,
    x: f32,
    y: f32,
) {
    let (label, rgb, alpha) = status(state, game);
    let dot = Rect::new(x, y + 7., 12., 12.);
    if rgb == GOOD {
        out.draw
            .push(Draw::Shadow(dot.inset(-3.), 9., 10., c(GOOD, 0.5)));
    }
    out.shape(dot, 6., c(rgb, alpha));
    let style = text(19.2, 550, -0.01);
    out.text(
        label.clone(),
        Rect::new(x + 22., y, assets.width(&label, style) + 8., 26.),
        style,
        c(0xffffff, 0.78),
        false,
    );
}

// ---- The Play page ----

/// The page behind everything: the previewed game's screenshots drifting and crossfading
/// (its library art until they load), shaded for the panel and the rail.
pub fn backdrop(out: &mut Scene, state: &State, now: Instant) {
    let play = &state.play;
    let reduced = state.preferences.reduced_motion;
    out.shape(FULL, 0., c(0x06070b, 1.));
    let layer = |out: &mut Scene, key: &str, since: Instant, alpha: f32| {
        let Some(game) = play.find(key) else {
            return false;
        };
        let shots: Vec<_> = game
            .shots()
            .into_iter()
            .filter(|kind| play.image(&game.key, *kind).is_some())
            .collect();
        let pinned = play
            .sheet
            .as_ref()
            .filter(|sheet| sheet.key == game.key)
            .and_then(|sheet| sheet.shot)
            .and_then(|index| game.shots().get(index).copied())
            .and_then(|kind| play.image(&game.key, kind));
        let blur = play.sheet.is_some() && pinned.is_none() && !state.preferences.reduced_effects;
        let draw =
            |out: &mut Scene, image: &Arc<crate::app_artwork::Artwork>, t: f32, alpha: f32| {
                // Ken Burns: a slow push in, drifting one way then the other.
                let (zoom, dx) = if reduced {
                    (1.04, 0.)
                } else {
                    (1.04 + 0.06 * t, (t - 0.5) * 40.)
                };
                let rect = scaled(FULL, zoom);
                picture(
                    out,
                    image,
                    Rect::new(rect.x + dx, rect.y, rect.w, rect.h),
                    0.,
                    [0.92, 0.92, 0.92, alpha],
                    if blur {
                        Filter::Blur(18.)
                    } else {
                        Filter::None
                    },
                );
            };
        if let Some(image) = pinned {
            draw(out, image, 0.5, alpha);
            return true;
        }
        let slide_len = crate::play::SHOT_SLIDE.as_secs_f32();
        let elapsed = now.duration_since(since).as_secs_f32();
        if !shots.is_empty() {
            let step = (elapsed / slide_len) as usize;
            let into = (elapsed - step as f32 * slide_len) / slide_len;
            let fade = if step == 0 || reduced {
                1.
            } else {
                (into * slide_len / crate::play::SHOT_FADE.as_secs_f32()).min(1.)
            };
            let current = shots[step % shots.len()];
            if fade < 1. {
                // The outgoing shot keeps drifting under the crossfade instead of stopping.
                let before = shots[(step + shots.len() - 1) % shots.len()];
                if let Some(image) = play.image(&game.key, before) {
                    draw(out, image, 1. + into, alpha);
                }
            }
            if let Some(image) = play.image(&game.key, current) {
                draw(out, image, into, alpha * fade);
            }
            return true;
        }
        if let Some(image) = game_art(state, game, &[ArtKind::Hero, ArtKind::Cover]) {
            // Easing to a stop rather than halting.
            let x = (elapsed / 30.).min(1.);
            draw(out, image, 1. - (1. - x).powi(2), alpha);
            return true;
        }
        false
    };
    let mut drawn = false;
    if let Some((old, at)) = &play.previous_preview {
        let t = now.duration_since(*at).as_secs_f32() / crate::play::SHOT_FADE.as_secs_f32();
        if t < 1. {
            // Its slideshow carries on where it was while it fades.
            drawn |= layer(out, &old.key, old.since, 1.);
        }
    }
    if let Some(preview) = &play.preview {
        let fade = play.previous_preview.as_ref().map_or(1., |(_, at)| {
            (now.duration_since(*at).as_secs_f32() / crate::play::SHOT_FADE.as_secs_f32()).min(1.)
        });
        drawn |= layer(out, &preview.key, preview.since, fade);
    }
    if !drawn {
        aurora(
            out,
            FULL,
            0.,
            seconds(now),
            !state.preferences.reduced_effects && !reduced,
        );
    }
    // Shading: the panel side, the rail and the header.
    horizontal(
        out,
        Rect::new(0., 0., 1250., 1080.),
        c(0x05060a, 0.92),
        c(0x05060a, 0.),
    );
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 520., 1920., 560.),
        0.,
        c(0x05060a, 0.),
        c(0x05060a, 0.9),
    ));
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 0., 1920., 170.),
        0.,
        c(0x05060a, 0.55),
        c(0x05060a, 0.),
    ));
}

/// Seconds since Luma started, for slow ambient motion.
fn seconds(now: Instant) -> f32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    now.duration_since(*START.get_or_init(Instant::now))
        .as_secs_f32()
}

fn stat_tile(
    out: &mut Scene,
    assets: &Assets,
    rect: Rect,
    icon: &str,
    value: &str,
    label: &str,
    progress: Option<f32>,
) {
    out.shape(rect, 16., c(0x0b0d14, 0.5));
    out.border(rect, 16., 1., c(0xffffff, 0.09));
    out.icon(
        assets,
        icon,
        Rect::new(rect.x + 18., rect.y + 18., 20., 20.),
        c(0xffffff, 0.6),
    );
    out.text(
        label.to_uppercase(),
        Rect::new(rect.x + 46., rect.y + 19., rect.w - 56., 20.),
        text(13.6, 650, 0.12),
        c(0xffffff, 0.6),
        false,
    );
    out.text(
        value,
        Rect::new(rect.x + 18., rect.y + 44., rect.w - 30., 40.),
        text(32., 650, -0.03),
        WHITE,
        false,
    );
    if let Some(progress) = progress {
        let bar = Rect::new(rect.x + 18., rect.y + rect.h - 16., rect.w - 36., 4.);
        out.shape(bar, 2., c(0xffffff, 0.14));
        if progress > 0. {
            out.shape(
                Rect::new(bar.x, bar.y, bar.w * progress.clamp(0.02, 1.), bar.h),
                2.,
                c(ACCENT, 1.),
            );
        }
    }
}

fn button(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    (target, rect): (Target, Rect),
    icon: Option<&str>,
    label: &str,
    primary: bool,
) {
    let focused = state.focus == target && state.console.is_none();
    if focused {
        out.shadow(rect, rect.h / 2., 24., 8., 0.35);
        out.draw.push(Draw::RoundedGradient(
            rect,
            rect.h / 2.,
            c(0xffffff, 1.),
            c(0xe6e6ea, 1.),
        ));
    } else if primary {
        out.shape(rect, rect.h / 2., c(0xffffff, 0.9));
    } else {
        out.shape(rect, rect.h / 2., c(0x1a1d26, 0.62));
        out.border(rect, rect.h / 2., 1., c(0xffffff, 0.14));
    }
    let ink = if focused || primary {
        c(0x14161c, 1.)
    } else {
        c(0xffffff, 0.9)
    };
    let style = text(20.8, 650, -0.02);
    let icon_w = if icon.is_some() { 26. } else { 0. };
    let width = assets.width(label, style) + icon_w;
    let x = rect.x + (rect.w - width) / 2.;
    if let Some(icon) = icon {
        out.icon(
            assets,
            icon,
            Rect::new(x, rect.y + rect.h / 2. - 10., 20., 20.),
            ink,
        );
    }
    out.text(
        label,
        Rect::new(x + icon_w, rect.y + rect.h / 2. - 13., width + 4., 26.),
        style,
        ink,
        false,
    );
    if focused && !state.pointer {
        out.border(rect.inset(-5.), rect.h / 2. + 5., 2., c(0xf0f0f0, 0.9));
    }
    out.control(target, rect, false);
}

/// The panel above the rail: the focused game's logo (or title), chips, stats, description,
/// buttons and the PC's status. Slides in when focus moves.
fn panel(out: &mut Scene, state: &State, assets: &Assets, now: Instant) {
    let language = state.preferences.language;
    let play = &state.play;
    let start = out.draw.len();
    let x = RAIL_X;
    let t = play
        .focused_at
        .map_or(1., |at| {
            now.duration_since(at).as_secs_f32() / PANEL_IN.as_secs_f32()
        })
        .min(1.);
    let t = if state.preferences.reduced_motion {
        1.
    } else {
        ease_out_cubic(t)
    };
    let lift = 14. * (1. - t);
    let game = play.game(play.index);
    let buttons_y;
    if let Some(game) = game {
        // Logo, else the title.
        let logo = Rect::new(x, 146. + lift, 640., 150.);
        match play.image(&game.key, ArtKind::Logo) {
            Some(image) => {
                // Logos are drawn to fit (contain), left-aligned by trimming the box.
                let (w, h) = (image.pixels.width() as f32, image.pixels.height() as f32);
                let scale = (logo.w / w).min(logo.h / h);
                let rect = Rect::new(logo.x, logo.y + logo.h - h * scale, w * scale, h * scale);
                out.draw.push(Draw::AppImage(image.clone(), rect, WHITE));
            }
            None => {
                let style = text(
                    if game.title.chars().count() > 22 {
                        56.
                    } else {
                        76.
                    },
                    800,
                    -0.045,
                )
                .leading(1.02);
                out.text(
                    clip_words(&game.title, 48),
                    Rect::new(x, 170. + lift, 1100., 130.),
                    style,
                    WHITE,
                    false,
                );
            }
        }
        // Chips: genres, year, developer, controller support.
        let mut cx = x;
        let style = text(17.6, 600, 0.01);
        let mut chip_list = chips(game);
        match game.controller.as_str() {
            "full" => chip_list.push(language.t("controllerFull").into()),
            "partial" => chip_list.push(language.t("controllerPartial").into()),
            _ => {}
        }
        for (index, chip) in chip_list.iter().enumerate() {
            let w = assets.width(chip, style) + 28.;
            if cx + w > 1180. {
                break;
            }
            let rect = Rect::new(cx, 314. + lift, w, 32.);
            if index < game.genres.len().min(2) {
                out.shape(rect, 16., c(0xffffff, 0.12));
            } else {
                out.border(rect, 16., 1., c(0xffffff, 0.2));
            }
            out.text(
                chip.as_str(),
                Rect::new(rect.x, rect.y + 6., rect.w, 22.),
                style,
                c(0xffffff, 0.86),
                true,
            );
            cx += w + 10.;
        }
        // Stats.
        let played = hours(game.play_time_ms, language);
        let last = played_ago(
            game.last_played.as_deref(),
            time::OffsetDateTime::now_utc(),
            language,
        );
        let y = 362. + lift;
        stat_tile(
            out,
            assets,
            Rect::new(x, y, 236., 100.),
            "clock",
            &played,
            language.t("statPlayed"),
            None,
        );
        if let Some(last) = last {
            out.text(
                last,
                Rect::new(x, y + 108., 300., 22.),
                text(16., 500, 0.),
                c(0xffffff, 0.55),
                false,
            );
        }
        let achievements = if game.achievements.total > 0 {
            format!(
                "{} / {}",
                game.achievements.unlocked, game.achievements.total
            )
        } else {
            "\u{2014}".into()
        };
        stat_tile(
            out,
            assets,
            Rect::new(x + 252., y, 236., 100.),
            "trophy",
            &achievements,
            language.t("statAchievements"),
            achievement_share(game),
        );
        if let Some(share) = achievement_share(game) {
            out.text(
                format!("{}%", (share * 100.).round() as u32),
                Rect::new(x + 252., y + 108., 236., 22.),
                text(16., 600, 0.),
                c(ACCENT, 0.95),
                false,
            );
        }
        stat_tile(
            out,
            assets,
            Rect::new(x + 504., y, 236., 100.),
            "hard-drive",
            &size(game.installed_bytes).unwrap_or_else(|| "\u{2014}".into()),
            language.t("statSize"),
            None,
        );
        if !game.description.is_empty() {
            out.text(
                clip_words(&game.description, 150),
                Rect::new(x + 772., y + 4., 520., 96.),
                text(19.2, 400, 0.).leading(1.45),
                c(0xffffff, 0.74),
                false,
            );
        }
        buttons_y = 516. + lift;
    } else {
        // No games yet: a big mark and a word about where they come from.
        out.icon(assets, "gaming", Rect::new(x, 190. + lift, 96., 96.), WHITE);
        out.text(
            language.t("libraryEmptyTitle"),
            Rect::new(x, 310. + lift, 1200., 80.),
            text(64., 800, -0.045),
            WHITE,
            false,
        );
        out.text(
            language.t("libraryEmptyBody"),
            Rect::new(x, 396. + lift, 900., 64.),
            text(22.4, 400, 0.).leading(1.4),
            c(0xffffff, 0.72),
            false,
        );
        buttons_y = 500. + lift;
    }
    let mut bx = x;
    if game.is_some() {
        let w = 196.;
        button(
            out,
            state,
            assets,
            (
                Target::PlayAction(PlayButton::Play),
                Rect::new(bx, buttons_y, w, 60.),
            ),
            Some("play"),
            language.t("playButton"),
            true,
        );
        bx += w + 16.;
        let w = 170.;
        button(
            out,
            state,
            assets,
            (
                Target::PlayAction(PlayButton::Details),
                Rect::new(bx, buttons_y, w, 60.),
            ),
            None,
            language.t("playDetails"),
            false,
        );
        bx += w + 16.;
    }
    let w = 210.;
    button(
        out,
        state,
        assets,
        (
            Target::PlayAction(PlayButton::Hydra),
            Rect::new(bx, buttons_y, w, 60.),
        ),
        Some("gaming"),
        language.t("openHydra"),
        game.is_none(),
    );
    bx += w + 28.;
    status_line(out, state, assets, game, bx, buttons_y + 17.);
    if t < 1. {
        out.fade_range(start, out.draw.len(), t, 1.);
    }
}

/// The rail of box art. The focused cover springs up with a glow and a sheen; the others
/// sit back, dimmer.
fn rail(out: &mut Scene, state: &State, assets: &Assets, now: Instant) {
    let play = &state.play;
    let reduced = state.preferences.reduced_motion;
    let effects = !state.preferences.reduced_effects;
    let on_rail = matches!(state.focus, Target::PlayGame(_));
    out.draw
        .push(Draw::EdgeFadeX(Some([0., 70., 1830., 1920.])));
    let mut focused_draw = None;
    for (index, game) in play.games().iter().enumerate() {
        let x = RAIL_X + index as f32 * COVER_PITCH - play.rail;
        if x > 1920. || x + COVER_W < 0. {
            continue;
        }
        let rect = Rect::new(x, RAIL_Y, COVER_W, COVER_H);
        out.control(Target::PlayGame(index), rect, false);
        if index == play.index && on_rail {
            focused_draw = Some((index, game, rect));
            continue;
        }
        let scale = play.cover_scale(index, false, now, reduced);
        let rect = scaled(rect, scale);
        let selected = index == play.index;
        out.shadow(rect, COVER_RADIUS, 26., 12., 0.4);
        cover(
            out,
            state,
            assets,
            game,
            rect,
            match (selected, play.is_hidden(&game.key)) {
                (_, true) => 0.42,
                (true, false) => 0.92,
                (false, false) => 0.68,
            },
            false,
        );
        badges(out, state, assets, game, rect);
    }
    // The focused one last, over its neighbours.
    if let Some((index, game, rect)) = focused_draw {
        let scale = play.cover_scale(index, true, now, reduced);
        let rect = scaled(rect, scale);
        let since = play
            .focused_at
            .map_or(1., |at| now.duration_since(at).as_secs_f32());
        out.shadow(rect, COVER_RADIUS, 46., 26., 0.6);
        if effects {
            out.draw.push(Draw::Shadow(
                Rect::new(rect.x + 18., rect.y + rect.h - 40., rect.w - 36., 70.),
                30.,
                46.,
                c(0x7c5cff, 0.55),
            ));
        }
        cover(out, state, assets, game, rect, 1., false);
        out.border(rect.inset(1.), COVER_RADIUS * scale, 1.5, c(0xffffff, 0.55));
        if effects && !reduced {
            sheen(out, rect, (since - 0.08) / 0.75, 0.35);
        }
        badges(out, state, assets, game, rect);
        if !state.pointer {
            out.border(
                rect.inset(-6.),
                COVER_RADIUS * scale + 6.,
                2.5,
                c(0xf3f3f3, 0.95),
            );
        }
    }
    out.draw.push(Draw::EdgeFadeX(None));
}

/// A cover's PLAYING pill, favourite star and (when shown) hidden mark.
fn badges(out: &mut Scene, state: &State, assets: &Assets, game: &Game, rect: Rect) {
    let scale = rect.w / COVER_W;
    if state.play.is_hidden(&game.key) {
        let size = 40. * scale;
        let at = Rect::new(
            rect.x + (rect.w - size) / 2.,
            rect.y + (rect.h - size) / 2.,
            size,
            size,
        );
        out.shape(at, size / 2., c(0x000000, 0.55));
        out.icon(assets, "eye-off", at.inset(9. * scale), c(0xffffff, 0.9));
    }
    if game.running {
        let label = state.preferences.language.t("playingBadge");
        let style = text(12.8 * scale, 800, 0.14);
        let w = assets.width(label, style) + 22. * scale;
        let pill = Rect::new(rect.x + 10. * scale, rect.y + 10. * scale, w, 24. * scale);
        out.shape(pill, pill.h / 2., c(GOOD, 0.95));
        out.text(
            label,
            Rect::new(pill.x, pill.y + 5. * scale, pill.w, 16. * scale),
            style,
            c(0x062b14, 1.),
            true,
        );
    }
    if game.favorite {
        let size = 26. * scale;
        let at = Rect::new(
            rect.x + rect.w - size - 10. * scale,
            rect.y + 10. * scale,
            size,
            size,
        );
        out.shape(at, size / 2., c(0x000000, 0.45));
        out.icon(assets, "star", at.inset(5. * scale), c(0xffd166, 1.));
    }
}

/// The details sheet: the cover large, everything about the game, its screenshots (the
/// focused one fills the backdrop).
fn sheet(out: &mut Scene, state: &State, assets: &Assets, now: Instant) {
    let play = &state.play;
    let Some(sheet) = &play.sheet else {
        return;
    };
    let Some(game) = play.find(&sheet.key) else {
        return;
    };
    let language = state.preferences.language;
    let t = (now.duration_since(sheet.opened).as_secs_f32() / SHEET_IN.as_secs_f32()).min(1.);
    let t = if state.preferences.reduced_motion {
        1.
    } else {
        ease_out_cubic(t)
    };
    let shot = sheet.shot.is_some();
    out.shape(FULL, 0., c(0x05060a, (if shot { 0.35 } else { 0.72 }) * t));
    if shot {
        // A screenshot fills the screen: the text steps back to the bottom.
        horizontal(
            out,
            Rect::new(0., 0., 1100., 1080.),
            c(0x05060a, 0.85 * t),
            c(0x05060a, 0.),
        );
    }
    let start = out.draw.len();
    let lift = 30. * (1. - t);
    let cover_rect = Rect::new(140., 170. + lift, 340., 510.);
    out.shadow(cover_rect, COVER_RADIUS, 50., 26., 0.6);
    cover(out, state, assets, game, cover_rect, 1., false);
    out.border(cover_rect.inset(1.), COVER_RADIUS, 1.5, c(0xffffff, 0.4));
    badges(out, state, assets, game, cover_rect);
    button(
        out,
        state,
        assets,
        (
            Target::PlayDetailStart,
            Rect::new(140., 704. + lift, 340., 64.),
        ),
        Some("play"),
        language.t("playButton"),
        true,
    );
    let hidden = play.is_hidden(&game.key);
    button(
        out,
        state,
        assets,
        (Target::PlayHide, Rect::new(140., 784. + lift, 340., 56.)),
        Some(if hidden { "eye" } else { "eye-off" }),
        language.t(if hidden { "unhideGame" } else { "hideGame" }),
        false,
    );
    // Delete: dimmed, with the reason under it, while it cannot be done.
    let blocked = state.delete_blocked(game);
    let deleting = play.deleting.as_deref() == Some(game.key.as_str());
    let delete_rect = Rect::new(140., 856. + lift, 340., 56.);
    let before = out.draw.len();
    button(
        out,
        state,
        assets,
        (Target::PlayDelete, delete_rect),
        Some("trash"),
        language.t(if deleting {
            "deleteDeleting"
        } else {
            "deleteGame"
        }),
        false,
    );
    if blocked.is_some() && !deleting {
        out.fade(before, 0.45, 1.);
    }
    match blocked.filter(|_| !deleting) {
        Some(hint) => {
            let style = text(16., 550, 0.);
            out.text(
                language.t(hint),
                Rect::new(148., 924. + lift, 340., 22.),
                style,
                c(0xffffff, 0.55),
                false,
            );
        }
        None => status_line(out, state, assets, Some(game), 148., 926. + lift),
    }

    let x = 560.;
    out.text(
        clip_words(&game.title, 60),
        Rect::new(x, 166. + lift, 1240., 80.),
        text(
            if game.title.chars().count() > 26 {
                48.
            } else {
                64.
            },
            800,
            -0.045,
        ),
        WHITE,
        false,
    );
    let chip_text = chips(game).join("  \u{b7}  ");
    out.text(
        chip_text,
        Rect::new(x, 252. + lift, 1240., 26.),
        text(19.2, 550, 0.01),
        c(0xffffff, 0.75),
        false,
    );
    if !game.description.is_empty() {
        out.text(
            clip_words(&game.description, 520),
            Rect::new(x, 298. + lift, 1180., 190.),
            text(21.6, 400, 0.).leading(1.5),
            c(0xffffff, 0.82),
            false,
        );
    }
    // Stats: an achievement ring, then label/value pairs.
    let y = 512. + lift;
    ring(out, state, game, Rect::new(x, y, 132., 132.));
    let pairs = [
        (language.t("statPlayed"), hours(game.play_time_ms, language)),
        (
            language.t("statLastPlayed"),
            played_ago(
                game.last_played.as_deref(),
                time::OffsetDateTime::now_utc(),
                language,
            )
            .unwrap_or_else(|| "\u{2014}".into()),
        ),
        (
            language.t("statSize"),
            size(game.installed_bytes).unwrap_or_else(|| "\u{2014}".into()),
        ),
        (language.t("statDeveloper"), non_empty(&game.developer)),
        (language.t("statPublisher"), non_empty(&game.publisher)),
        (language.t("statReleased"), non_empty(&game.release_date)),
    ];
    for (index, (label, value)) in pairs.iter().enumerate() {
        let column = (index % 3) as f32;
        let row = (index / 3) as f32;
        let at = Rect::new(x + 172. + column * 340., y + 6. + row * 72., 320., 64.);
        out.text(
            label.to_uppercase(),
            Rect::new(at.x, at.y, at.w, 18.),
            text(13.6, 650, 0.12),
            c(0xffffff, 0.55),
            false,
        );
        out.text(
            clip_words(value, 30),
            Rect::new(at.x, at.y + 22., at.w, 30.),
            text(24., 600, -0.02),
            WHITE,
            false,
        );
    }
    // Screenshots.
    let shots = game.shots();
    if !shots.is_empty() {
        out.text(
            language.t("screenshots").to_uppercase(),
            Rect::new(x, 700. + lift, 600., 18.),
            text(13.6, 650, 0.12),
            c(0xffffff, 0.55),
            false,
        );
        for (index, kind) in shots.iter().enumerate() {
            let rect = Rect::new(x + index as f32 * 252., 730. + lift, 232., 130.);
            let focused = state.focus == Target::PlayShot(index);
            let rect = if focused { scaled(rect, 1.06) } else { rect };
            out.shadow(rect, 10., 18., 8., 0.4);
            match play.image(&game.key, *kind) {
                Some(image) => picture(
                    out,
                    image,
                    rect,
                    10.,
                    if focused {
                        WHITE
                    } else {
                        [0.75, 0.75, 0.75, 1.]
                    },
                    Filter::None,
                ),
                None => {
                    out.shape(rect, 10., c(0xffffff, 0.08));
                }
            }
            if focused && !state.pointer {
                out.border(rect.inset(-5.), 15., 2.5, c(0xf3f3f3, 0.95));
            }
            out.control(Target::PlayShot(index), rect, false);
        }
    }
    out.fade_range(start, out.draw.len(), t, 1.);
    if let Some(confirm) = &sheet.confirm {
        confirm_card(out, state, assets, game, confirm, now);
    }
}

/// The delete confirmation over the sheet: what goes and how much space it frees, then
/// Delete keeping the saves, Delete everything, Cancel. While the PC works, a spinner; if
/// it fails, why.
fn confirm_card(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    game: &Game,
    confirm: &crate::play::Confirm,
    now: Instant,
) {
    let language = state.preferences.language;
    let reduced = state.preferences.reduced_motion;
    let t = if reduced {
        1.
    } else {
        ease_out_cubic((now.duration_since(confirm.since).as_secs_f32() / 0.22).min(1.))
    };
    // The controls behind it are not reachable by pointer while it shows.
    out.controls
        .retain(|control| matches!(control.target, Target::PlayConfirm(_)));
    out.shape(FULL, 0., c(0x020308, 0.62));
    let start = out.draw.len();
    let card = Rect::new(560., 250. + 16. * (1. - t), 800., 520.);
    out.shadow(card, 28., 60., 24., 0.6);
    out.shape(card, 28., c(0x12141c, 0.97));
    out.border(card, 28., 1., c(0xffffff, 0.12));
    let x = card.x + 56.;
    let w = card.w - 112.;
    let failed = match confirm.phase {
        ConfirmPhase::Failed(error) => Some(error),
        _ => None,
    };
    out.icon(
        assets,
        if failed.is_some() { "alert" } else { "trash" },
        Rect::new(x, card.y + 52., 40., 40.),
        c(if failed.is_some() { LOW } else { 0xffffff }, 0.9),
    );
    out.text(
        clip_words(
            &language.format("deleteConfirmTitle", &[("game", game.title.as_str())]),
            60,
        ),
        Rect::new(x, card.y + 112., w, 46.),
        text(36., 750, -0.035),
        WHITE,
        false,
    );
    let detail = match confirm.phase {
        ConfirmPhase::Failed(error) => language.t(delete_error_message(error)).to_owned(),
        _ => size(game.installed_bytes).map_or_else(
            || language.t("deleteFromPc").to_owned(),
            |bytes| language.format("deleteFrees", &[("size", bytes.as_str())]),
        ),
    };
    out.text(
        detail,
        Rect::new(x, card.y + 168., w, 28.),
        text(21.6, 500, 0.),
        c(0xffffff, 0.72),
        false,
    );
    let row = |index: usize| Rect::new(x, card.y + 236. + index as f32 * 76., w, 60.);
    match confirm.phase {
        ConfirmPhase::Asking => {
            for (index, choice) in DELETE_CHOICES.into_iter().enumerate() {
                let (icon, label) = match choice {
                    DeleteChoice::KeepSaves => (Some("trash"), language.t("deleteKeepSaves")),
                    DeleteChoice::AllData => (Some("trash"), language.t("deleteAllData")),
                    DeleteChoice::Cancel => (None, language.t("cancel")),
                };
                button(
                    out,
                    state,
                    assets,
                    (Target::PlayConfirm(choice), row(index)),
                    icon,
                    label,
                    false,
                );
            }
        }
        ConfirmPhase::Deleting => {
            spinner(out, (x + 22., row(0).y + 30.), now, reduced);
            out.text(
                language.t("deleteDeleting"),
                Rect::new(x + 60., row(0).y + 16., w - 60., 30.),
                text(22.4, 600, -0.01),
                c(0xffffff, 0.9),
                false,
            );
            out.text(
                language.t("deleteBackground"),
                Rect::new(x, row(1).y + 8., w, 24.),
                text(17.6, 500, 0.),
                c(0xffffff, 0.5),
                false,
            );
        }
        ConfirmPhase::Failed(_) => {
            button(
                out,
                state,
                assets,
                (Target::PlayConfirm(DeleteChoice::Cancel), row(2)),
                None,
                language.t("close"),
                false,
            );
        }
    }
    if t < 1. {
        out.fade_range(start, out.draw.len(), t, 1.);
    }
}

/// A small ring of dots chasing round (still with Reduce motion).
fn spinner(out: &mut Scene, (cx, cy): (f32, f32), now: Instant, reduced: bool) {
    let spin = if reduced { 0. } else { seconds(now) / 1.1 };
    let count = 10;
    for index in 0..count {
        let p = index as f32 / count as f32;
        let angle = (p + spin) * TAU;
        let d = 3. + 4. * p;
        let (x, y) = (cx + 14. * angle.cos(), cy + 14. * angle.sin());
        out.shape(
            Rect::new(x - d / 2., y - d / 2., d, d),
            d / 2.,
            c(0xffffff, 0.2 + 0.8 * p),
        );
    }
}

/// Top right of the Play page, under the tabs, while the PC answers: each controller's
/// battery, then each drive's free space.
fn system_strip(out: &mut Scene, state: &State, assets: &Assets) {
    let Some(system) = state.play.system.as_ref() else {
        return;
    };
    if state.console_awake != Some(true)
        || (system.controllers.is_empty() && system.drives.is_empty())
    {
        return;
    }
    let language = state.preferences.language;
    let style = text(17.6, 600, 0.);
    let height = 44.;
    let y = 128.;
    let gap = 26.;
    // Each item: its width and how to draw it at x.
    enum Item<'a> {
        Pad(&'a crate::console::Pad, String),
        Drive(&'a crate::console::Drive, String),
    }
    let mut items = Vec::new();
    for pad in &system.controllers {
        items.push(Item::Pad(pad, format!("{}%", pad.battery)));
    }
    for drive in &system.drives {
        let free = size(Some(drive.free)).unwrap_or_default();
        let label = format!(
            "{}: {}",
            drive.letter,
            language.format("driveFree", &[("size", free.as_str())])
        );
        items.push(Item::Drive(drive, label));
    }
    let width = |item: &Item| match item {
        Item::Pad(pad, label) => {
            24. + 8. + 30. + 8. + assets.width(label, style) + if pad.charging { 20. } else { 0. }
        }
        Item::Drive(_, label) => 22. + 8. + assets.width(label, style) + 10. + 52.,
    };
    let total: f32 =
        items.iter().map(width).sum::<f32>() + gap * items.len().saturating_sub(1) as f32;
    let pill = Rect::new(1800. - total - 40., y, total + 40., height);
    out.shape(pill, height / 2., c(0x0b0d14, 0.5));
    out.border(pill, height / 2., 1., c(0xffffff, 0.09));
    let mut x = pill.x + 20.;
    let mid = y + height / 2.;
    let ink = c(0xffffff, 0.82);
    for item in &items {
        let w = width(item);
        match item {
            Item::Pad(pad, label) => {
                let low = pad.battery < 20 && !pad.charging;
                out.icon(assets, "gaming", Rect::new(x, mid - 12., 24., 24.), ink);
                // A battery: its outline, the nub and the fill.
                let body = Rect::new(x + 32., mid - 7.5, 27., 15.);
                out.border(body, 3.5, 1.5, c(0xffffff, 0.6));
                out.shape(
                    Rect::new(body.x + body.w + 1., mid - 3., 2.5, 6.),
                    1.,
                    c(0xffffff, 0.6),
                );
                let fill = (body.w - 5.) * (f32::from(pad.battery) / 100.).clamp(0.06, 1.);
                out.shape(
                    Rect::new(body.x + 2.5, body.y + 2.5, fill, body.h - 5.),
                    1.5,
                    if low {
                        c(LOW, 1.)
                    } else if pad.charging {
                        c(GOOD, 1.)
                    } else {
                        c(0xffffff, 0.9)
                    },
                );
                let mut tx = x + 32. + 30. + 8.;
                if pad.charging {
                    out.icon(
                        assets,
                        "zap",
                        Rect::new(tx - 2., mid - 9., 18., 18.),
                        c(GOOD, 1.),
                    );
                    tx += 20.;
                }
                out.text(
                    label.as_str(),
                    Rect::new(tx, mid - 12., assets.width(label, style) + 6., 24.),
                    style,
                    if low { c(LOW, 1.) } else { ink },
                    false,
                );
            }
            Item::Drive(drive, label) => {
                out.icon(assets, "hard-drive", Rect::new(x, mid - 11., 22., 22.), ink);
                let lw = assets.width(label, style);
                out.text(
                    label.as_str(),
                    Rect::new(x + 30., mid - 12., lw + 6., 24.),
                    style,
                    ink,
                    false,
                );
                let bar = Rect::new(x + 30. + lw + 10., mid - 2., 52., 4.);
                out.shape(bar, 2., c(0xffffff, 0.16));
                let used = drive.used();
                out.shape(
                    Rect::new(bar.x, bar.y, bar.w * used.max(0.04), bar.h),
                    2.,
                    c(if used > 0.9 { LOW } else { ACCENT }, 1.),
                );
            }
        }
        x += w + gap;
    }
}

fn non_empty(value: &str) -> String {
    if value.is_empty() {
        "\u{2014}".into()
    } else {
        value.into()
    }
}

/// Achievements as a ring of dots, lit to the share unlocked, the percentage inside.
fn ring(out: &mut Scene, state: &State, game: &Game, rect: Rect) {
    let (cx, cy) = rect.center();
    let radius = rect.w / 2. - 8.;
    let share = achievement_share(game);
    let dots = 48;
    let lit = share.map_or(0, |share| (share * dots as f32).round() as usize);
    for index in 0..dots {
        let angle = index as f32 / dots as f32 * TAU - TAU / 4.;
        let (x, y) = (cx + radius * angle.cos(), cy + radius * angle.sin());
        let d = if index < lit { 7. } else { 5. };
        out.shape(
            Rect::new(x - d / 2., y - d / 2., d, d),
            d / 2.,
            if index < lit {
                c(ACCENT, 1.)
            } else {
                c(0xffffff, 0.14)
            },
        );
    }
    let language = state.preferences.language;
    let (value, label) = match share {
        Some(share) => (
            format!("{}%", (share * 100.).round() as u32),
            format!(
                "{} / {}",
                game.achievements.unlocked, game.achievements.total
            ),
        ),
        None => ("\u{2014}".into(), language.t("statAchievements").into()),
    };
    out.text(
        value,
        Rect::new(rect.x, cy - 24., rect.w, 34.),
        text(28., 650, -0.03),
        WHITE,
        true,
    );
    out.text(
        label,
        Rect::new(rect.x, cy + 12., rect.w, 18.),
        text(13.6, 600, 0.04),
        c(0xffffff, 0.6),
        true,
    );
}

/// The Play page.
pub fn page(out: &mut Scene, state: &State, assets: &Assets) {
    let now = Instant::now();
    // The page is the whole screen (its backdrop is drawn before the content clip): the
    // focused cover may grow past the content's edges.
    out.draw.push(Draw::Clip(None));
    if state.play.sheet.is_some() {
        sheet(out, state, assets, now);
        return;
    }
    panel(out, state, assets, now);
    rail(out, state, assets, now);
    system_strip(out, state, assets);
}

// ---- The launch ----

/// The step track: Wake PC, then console mode, then the game (or Hydra).
fn steps(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    session: &Session,
    now: Instant,
    alpha: f32,
) {
    let language = state.preferences.language;
    let ready = session.ready(now);
    let reached = match session.phase {
        _ if ready => 3,
        Phase::Waking => 0,
        Phase::Starting => 1,
        Phase::Ready => 2,
        Phase::Failed => 0,
    };
    let last = if session.game.is_some() {
        language.t("stepLaunch")
    } else {
        language.t("openHydra")
    };
    let labels = [language.t("stepWake"), language.t("stepConsole"), last];
    let gap = 260.;
    let x0 = 960. - gap;
    let y = 788.;
    for index in 0..2 {
        let from = x0 + index as f32 * gap + 16.;
        let line = Rect::new(from, y - 1.5, gap - 32., 3.);
        out.shape(line, 1.5, c(0xffffff, 0.16 * alpha));
        if reached > index {
            out.shape(line, 1.5, c(ACCENT, alpha));
        }
    }
    let pulse = (seconds(now) * TAU / crate::console::PULSE.as_secs_f32()).sin() * 0.5 + 0.5;
    for (index, label) in labels.iter().enumerate() {
        let cx = x0 + index as f32 * gap;
        let dot = Rect::new(cx - 13., y - 13., 26., 26.);
        let failed = session.phase == Phase::Failed && index == reached;
        if index < reached {
            out.shape(dot, 13., c(ACCENT, alpha));
            out.icon(assets, "check", dot.inset(6.), c(0xffffff, alpha));
        } else if index == reached && !failed {
            let halo = dot.inset(-6. - 4. * pulse);
            out.border(
                halo,
                halo.w / 2.,
                2.,
                c(ACCENT, (0.25 + 0.4 * pulse) * alpha),
            );
            out.shape(dot, 13., c(0xffffff, 0.12 * alpha));
            out.shape(dot.inset(7.), 6., c(ACCENT, alpha));
        } else if failed {
            out.shape(dot, 13., c(0xff6b6b, 0.85 * alpha));
            out.icon(assets, "close", dot.inset(7.), c(0xffffff, alpha));
        } else {
            out.border(dot, 13., 2., c(0xffffff, 0.28 * alpha));
        }
        out.text(
            *label,
            Rect::new(cx - 130., y + 26., 260., 24.),
            text(17.6, if index <= reached { 650 } else { 500 }, 0.01),
            c(
                0xffffff,
                (if index <= reached { 0.92 } else { 0.5 }) * alpha,
            ),
            true,
        );
    }
}

/// The orbit: twelve dots chasing round, a halo breathing with the waiting pulse.
fn orbit(
    out: &mut Scene,
    session: &Session,
    center: (f32, f32),
    now: Instant,
    alpha: f32,
    reduced: bool,
    effects: bool,
) {
    let failed = session.phase == Phase::Failed;
    let elapsed = now.duration_since(session.started).as_secs_f32();
    let breathe = if reduced || failed {
        0.5
    } else {
        (elapsed * TAU / crate::console::PULSE.as_secs_f32()).sin() * 0.5 + 0.5
    };
    let (cx, cy) = center;
    if effects {
        let d = 150. + 30. * breathe;
        out.draw.push(Draw::Shadow(
            Rect::new(cx - d / 2., cy - d / 2., d, d),
            d / 2.,
            60.,
            c(
                if failed { 0xff6b6b } else { ACCENT },
                (0.18 + 0.2 * breathe) * alpha,
            ),
        ));
    }
    let spin = if reduced || failed { 0. } else { elapsed / 1.3 };
    let count = 12;
    for index in 0..count {
        let p = index as f32 / count as f32;
        let angle = (p + spin) * TAU;
        // The head is bright; the tail fades behind it.
        let trail = if reduced { 0.6 } else { p.powf(1.6) };
        let d = 5. + 5. * trail;
        let (x, y) = (cx + 38. * angle.cos(), cy + 38. * angle.sin());
        out.shape(
            Rect::new(x - d / 2., y - d / 2., d, d),
            d / 2.,
            c(
                if failed { 0xff8a8a } else { 0xffffff },
                (0.15 + 0.85 * trail) * alpha,
            ),
        );
    }
}

/// The launch: grows from `from` (the picked card) to the whole screen.
pub fn launch(out: &mut Scene, state: &State, assets: &Assets, session: &Session, from: Rect) {
    let now = Instant::now();
    let language = state.preferences.language;
    let reduced = state.preferences.reduced_motion;
    let effects = !state.preferences.reduced_effects;
    let elapsed = now.duration_since(session.started).as_secs_f32();
    let (lift, grow, appear) = if reduced {
        (0., 1., (elapsed / 0.25).min(1.))
    } else {
        (session.lift(now), session.grow(now), 1.)
    };
    let start = out.draw.len();
    // Home dims behind the card.
    out.shape(FULL, 0., c(0x04060c, 0.35 * lift + 0.6 * grow));
    let lifted = scaled(from, 1. + 0.06 * lift * (1. - grow));
    let rect = Rect::new(
        mix(lifted.x, 0., grow),
        mix(lifted.y, 0., grow),
        mix(lifted.w, 1920., grow),
        mix(lifted.h, 1080., grow),
    );
    let radius = mix(COVER_RADIUS + 4., 0., grow);
    out.shadow(
        rect,
        radius,
        40. + 40. * lift,
        20. * lift,
        0.55 * (1. - grow),
    );
    let art = session.game.as_ref().and_then(|game| {
        game_art(
            state,
            game,
            &[ArtKind::Hero, ArtKind::Shot(0), ArtKind::Cover],
        )
    });
    match art {
        Some(image) => {
            // The art zooms gently and pulls out of focus as it fills the screen.
            let zoom = scaled(rect, 1. + 0.05 * grow + 0.02 * (elapsed / 8.).min(1.));
            let dim = mix(1., 0.5, grow);
            picture(
                out,
                image,
                if grow < 1. { rect } else { zoom },
                radius,
                [dim, dim, dim, 1.],
                if effects && grow > 0.2 {
                    Filter::Blur(14. * grow)
                } else {
                    Filter::None
                },
            );
        }
        None => aurora(out, rect, radius, elapsed, effects && !reduced),
    }
    // A vignette for the text.
    out.draw.push(Draw::RoundedGradient(
        rect,
        radius,
        c(0x05060a, 0.25 * grow),
        c(0x05060a, 0.8 * grow),
    ));
    let content = ((grow - 0.4) / 0.6).clamp(0., 1.);
    let burst = session.burst(now);
    // Title: the logo if it loaded, else the name.
    let title = session.title(state.preferences.console.as_ref());
    let logo = session
        .game
        .as_ref()
        .and_then(|game| state.play.image(&game.key, ArtKind::Logo));
    let title_y = 300.;
    let logo_box = Rect::new(560., title_y, 800., 200.);
    let title_alpha = 0.3 + 0.7 * content;
    match logo {
        Some(image) => {
            let (w, h) = (image.pixels.width() as f32, image.pixels.height() as f32);
            let scale = (logo_box.w / w).min(logo_box.h / h);
            let r = Rect::new(
                960. - w * scale / 2.,
                logo_box.y + (logo_box.h - h * scale) / 2.,
                w * scale,
                h * scale,
            );
            out.draw
                .push(Draw::AppImage(image.clone(), r, [1., 1., 1., title_alpha]));
        }
        None => {
            let style = text(
                mix(
                    34.,
                    if title.chars().count() > 24 { 60. } else { 84. },
                    content,
                ),
                800,
                -0.05,
            )
            .leading(1.05);
            out.text(
                clip_words(title, 48),
                Rect::new(160., title_y + 50., 1600., 180.),
                style,
                c(0xffffff, title_alpha),
                true,
            );
        }
    }
    if let (Some(t), true) = (burst, effects && !reduced) {
        // Light sweeps over the title as it is ready.
        sheen(out, logo_box, t * 1.2, 0.5);
    }
    let center = (960., 600.);
    match burst {
        Some(t) if !reduced => {
            // The orbit gives way to a check, and a ring bursts out.
            let e = ease_out_cubic(t);
            let ring = mix(90., 1500., e);
            out.border(
                Rect::new(center.0 - ring / 2., center.1 - ring / 2., ring, ring),
                ring / 2.,
                mix(6., 1., e),
                c(ACCENT, 0.7 * (1. - t)),
            );
            let ring2 = mix(90., 900., ease_out_cubic((t * 1.4).min(1.)));
            out.border(
                Rect::new(center.0 - ring2 / 2., center.1 - ring2 / 2., ring2, ring2),
                ring2 / 2.,
                2.,
                c(0xffffff, 0.45 * (1. - t)),
            );
            let d = mix(40., 76., ease_out_back_small(t));
            let at = Rect::new(center.0 - d / 2., center.1 - d / 2., d, d);
            out.draw
                .push(Draw::Shadow(at.inset(-12.), d, 40., c(ACCENT, 0.6)));
            out.shape(at, d / 2., c(ACCENT, 1.));
            out.icon(assets, "check", at.inset(d * 0.24), WHITE);
            if effects {
                let mut flash = Quad::new(FULL, c(0xffffff, 0.22 * (1. - t).powi(2)));
                flash.blend = Blend::Screen;
                out.draw.push(Draw::Quad(Box::new(flash)));
            }
        }
        Some(_) => {
            let at = Rect::new(center.0 - 38., center.1 - 38., 76., 76.);
            out.shape(at, 38., c(ACCENT, 1.));
            out.icon(assets, "check", at.inset(18.), WHITE);
        }
        None => orbit(out, session, center, now, content, reduced, effects),
    }
    steps(out, state, assets, session, now, content);
    let label = match session.phase {
        _ if session.ready(now) => {
            language.format("consoleReady", &[("input", input_label(state).as_str())])
        }
        Phase::Waking => language.t("consoleWaking").into(),
        Phase::Starting => match &session.game {
            Some(game) => language.format("consoleLaunching", &[("game", game.title.as_str())]),
            None => language.t("consoleStarting").into(),
        },
        Phase::Ready => language.t("consoleStarting").into(),
        Phase::Failed => language.t("consoleNoAnswer").into(),
    };
    out.text(
        label,
        Rect::new(0., 690., 1920., 32.),
        text(24., 550, -0.01),
        c(0xece9ff, 0.9 * content),
        true,
    );
    if session.phase == Phase::Failed && session.ready_at.is_none() {
        let buttons = [
            (Target::ConsoleSwitch, language.t("switchAnyway")),
            (Target::ConsoleCancel, language.t("cancel")),
        ];
        let width = 236.;
        let gap = 24.;
        let left = 960. - (width * 2. + gap) / 2.;
        for (index, (target, label)) in buttons.into_iter().enumerate() {
            let rect = Rect::new(left + index as f32 * (width + gap), 880., width, 60.);
            let focused = state.focus == target;
            if focused {
                out.draw.push(Draw::RoundedGradient(
                    rect,
                    30.,
                    c(0xffffff, 1.),
                    c(0xe6e6ea, 1.),
                ));
            } else {
                out.shape(rect, 30., c(0x1a1d26, 0.62));
                out.border(rect, 30., 1., c(0xffffff, 0.16));
            }
            out.text(
                label,
                Rect::new(rect.x, rect.y + 17., rect.w, 26.),
                text(20.8, 650, -0.02),
                if focused {
                    c(0x14161c, 1.)
                } else {
                    c(0xffffff, 0.88)
                },
                true,
            );
            if focused && !state.pointer {
                out.border(rect.inset(-5.), 35., 2., c(0xf0f0f0, 0.95));
            }
            out.control(target, rect, false);
        }
    } else if session.ready_at.is_none() {
        out.text(
            language.t("backToCancel"),
            Rect::new(0., 1000., 1920., 22.),
            text(16., 550, 0.06),
            c(0xffffff, 0.45 * content),
            true,
        );
    }
    if appear < 1. {
        out.fade(start, appear, 1.);
    }
    // Black, just before LG's input change takes over the screen.
    let fade = if session.launched_at.is_some() {
        1.
    } else {
        session.fade(now)
    };
    if fade > 0. {
        out.shape(FULL, 0., c(0x000000, fade));
    }
}

fn ease_out_back_small(t: f32) -> f32 {
    crate::play::ease_out_back((t / 0.5).min(1.))
}
