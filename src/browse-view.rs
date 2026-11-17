//! A Browse by Platform page (the service's mark, Movies and Shows pills, the focused title
//! large with Play and Details, a row of titles) and a person's page (photo, name and bio,
//! then rows of what they made).
use super::{
    Assets, Scene, c, clip_words, held_image, icon_button, meta_line, search_card, skeleton, text,
    title_mark,
};
use crate::feeds::browse::{PERSON_ROWS_Y, PLATFORM_KINDS, PLATFORM_ROW_Y, PersonPage};
use crate::feeds::explore::Kind;
use crate::feeds::{
    CARD_HEIGHT, CARD_PITCH, CARD_RADIUS, CARD_WIDTH, SECTION_HEADER, TITLE_BUTTON_HEIGHT,
};
use crate::renderer::{Draw, Quad, WHITE};
use crate::state::{Rect, State, Target};

const FULL: Rect = Rect::new(0., 0., 1920., 1080.);
const LEFT: f32 = 84.;
const BASE: u32 = 0x080b12;

fn on_screen(rect: Rect) -> bool {
    rect.x < 1920. && rect.x + rect.w > 0. && rect.y < 1080. && rect.y + rect.h > 0.
}

/// Darkens the screen's left side, fading out by `width`, so words read over a backdrop.
fn side_shade(out: &mut Scene, width: f32, alpha: f32) {
    // A vertical gradient turned a quarter: its top lands on the left.
    let mut quad = Quad::new(
        Rect::new(width / 2. - 540., 540. - width / 2., 1080., width),
        c(BASE, alpha),
    );
    quad.colors = [c(BASE, alpha), c(BASE, 0.)];
    quad.angle = -90.;
    out.draw.push(Draw::Quad(Box::new(quad)));
}

/// A section's name over a row.
fn header(out: &mut Scene, label: &str, y: f32) {
    out.text(
        label,
        Rect::new(LEFT, y + 3., 1200., 31.),
        text(23.2, 550, -0.035),
        WHITE,
        false,
    );
}

/// A row of posters at `y`, scrolled by `scroll`, each target made by `target`.
#[allow(clippy::too_many_arguments)]
fn poster_row(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    items: &[&crate::feeds::Item],
    y: f32,
    scroll: f32,
    target: impl Fn(usize) -> Target,
) {
    let mut focused = None;
    for (index, item) in items.iter().enumerate() {
        let rect = Rect::new(
            LEFT + index as f32 * CARD_PITCH - scroll,
            y,
            CARD_WIDTH,
            CARD_HEIGHT,
        );
        if !on_screen(rect) {
            continue;
        }
        let target = target(index);
        out.control(target.clone(), rect, false);
        if state.focus == target {
            focused = Some((item, rect, target));
            continue;
        }
        search_card(out, state, assets, item, rect, &target);
    }
    // The focused poster grows over its neighbours.
    if let Some((item, rect, target)) = focused {
        search_card(out, state, assets, item, rect, &target);
    }
}

/// A Browse by Platform page, over Home.
pub fn platform_page(out: &mut Scene, state: &State, assets: &Assets) {
    let Some(page) = &state.feeds.platform else {
        return;
    };
    let Some(platform) = page.platform() else {
        return;
    };
    let reduced_motion = state.preferences.reduced_motion;
    let entrance = page.entrance(reduced_motion);
    let start = out.draw.len();
    out.shape(FULL, 0., c(BASE, 1.));
    // The hero's backdrop, crossfading from the one before.
    let fade = page.backdrop_fade(reduced_motion);
    for (url, alpha) in [(page.previous.as_ref(), 1.), (page.backdrop.as_ref(), fade)] {
        if let Some((image, url)) = held_image(state, url) {
            let reveal = state.feeds.reveal(url, reduced_motion);
            out.draw.push(Draw::AppImage(
                image.clone(),
                FULL,
                [0.9, 0.9, 0.9, alpha * reveal],
            ));
        }
    }
    // The service's colour, faint at the top; shade for the words on the left and the row.
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 0., 1920., 260.),
        0.,
        c(platform.background, 0.55),
        c(platform.background, 0.),
    ));
    side_shade(out, 1400., 0.92);
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 380., 1920., 700.),
        0.,
        c(BASE, 0.),
        c(BASE, 0.97),
    ));
    let lift = (1. - entrance) * 24.;
    // The service's mark, top left.
    let mark = Rect::new(LEFT, 52. + lift, 260., 64.);
    if platform.logo {
        let (ratio, _) = crate::branding::for_key(platform.key)
            .map_or((0.62, 64.), |brand| brand.artwork_size());
        let w = (mark.h * ratio * 3.).min(mark.w);
        out.draw.push(Draw::TintedImage(
            assets.path(format!("logos/{}.lvec", platform.key)),
            Rect::new(mark.x, mark.y, w, mark.h),
            0.,
            true,
            c(0xffffff, 1.),
        ));
    } else {
        out.text(
            platform.title.to_uppercase(),
            Rect::new(mark.x, mark.y + 4., 600., 56.),
            text(46., 800, -0.02),
            WHITE,
            false,
        );
    }
    // Movies and TV Shows, top right.
    let mut x = 1836.;
    for (index, kind) in PLATFORM_KINDS.iter().enumerate().rev() {
        let label = match kind {
            Kind::Shows => "TV Shows",
            _ => "Movies",
        };
        let style = text(18.4, 600, -0.01);
        let w = assets.width(label, style) + 56.;
        x -= w;
        let rect = Rect::new(x, 58. + lift, w, 52.);
        let target = Target::PlatformKind(index);
        let focused = state.focus == target;
        out.pill(rect, focused, *kind == page.kind);
        out.text(
            label,
            Rect::new(rect.x, rect.y + 13., rect.w, 26.),
            style,
            if focused { c(0x2c2c2c, 1.) } else { WHITE },
            true,
        );
        if focused {
            out.ring(rect, rect.h / 2.);
        }
        out.control(target, rect, false);
        x -= 12.;
    }
    let items = state.feeds.platform_items();
    let hero = state.feeds.platform_hero();
    let top = |y: f32| y + lift;
    match hero {
        Some(item) => {
            let details = state.feeds.details.get(&item.link);
            title_mark(
                out,
                state,
                details,
                &item.title,
                LEFT,
                top(400.),
                (620., 170.),
                60.,
            );
            match details {
                Some(details) => {
                    let mut meta = meta_line(details);
                    if let Some(rating) = details.rating {
                        meta = if meta.is_empty() {
                            format!("TMDB {rating:.1}")
                        } else {
                            format!("{meta}  ·  TMDB {rating:.1}")
                        };
                    }
                    out.text(
                        meta,
                        Rect::new(LEFT, top(418.), 1200., 28.),
                        text(20., 550, 0.),
                        c(0xffffff, 0.82),
                        false,
                    );
                    out.text(
                        clip_words(&details.overview, 250),
                        Rect::new(LEFT, top(458.), 1000., 92.),
                        text(19.2, 400, 0.).leading(1.45),
                        c(0xffffff, 0.86),
                        false,
                    );
                }
                None => {
                    skeleton(out, Rect::new(LEFT, top(424.), 380., 20.), 10.);
                    skeleton(out, Rect::new(LEFT, top(466.), 940., 18.), 9.);
                    skeleton(out, Rect::new(LEFT, top(496.), 820., 18.), 9.);
                }
            }
            let play = icon_button(
                out,
                assets,
                "Play",
                "play",
                LEFT,
                top(568.),
                TITLE_BUTTON_HEIGHT,
                state.focus == Target::PlatformPlay,
            );
            out.control(Target::PlatformPlay, play, false);
            let more = icon_button(
                out,
                assets,
                "Details",
                "more",
                play.x + play.w + 16.,
                top(568.),
                TITLE_BUTTON_HEIGHT,
                state.focus == Target::PlatformDetails,
            );
            out.control(Target::PlatformDetails, more, false);
        }
        None => {
            let words = match &page.error {
                Some(error) => format!("Couldn't load {}: {error}", platform.title),
                None if page.asked.is_some()
                    && state.feeds.explore.found.contains_key(&page.query()) =>
                {
                    format!("Nothing streaming on {} here right now", platform.title)
                }
                None => {
                    skeleton(out, Rect::new(LEFT, top(330.), 520., 64.), 12.);
                    skeleton(out, Rect::new(LEFT, top(424.), 380., 20.), 10.);
                    skeleton(out, Rect::new(LEFT, top(466.), 940., 18.), 9.);
                    String::new()
                }
            };
            out.text(
                words,
                Rect::new(LEFT, top(400.), 1400., 32.),
                text(22.4, 500, 0.),
                c(0xffffff, 0.7),
                false,
            );
        }
    }
    let kind = match page.kind {
        Kind::Shows => "shows",
        _ => "movies",
    };
    header(
        out,
        &format!("Popular {kind} on {}", platform.title),
        top(PLATFORM_ROW_Y - SECTION_HEADER),
    );
    if items.is_empty() && page.error.is_none() {
        for index in 0..8 {
            skeleton(
                out,
                Rect::new(
                    LEFT + index as f32 * CARD_PITCH,
                    top(PLATFORM_ROW_Y),
                    CARD_WIDTH,
                    CARD_HEIGHT,
                ),
                CARD_RADIUS,
            );
        }
    }
    poster_row(
        out,
        state,
        assets,
        &items,
        top(PLATFORM_ROW_Y),
        page.scroll,
        Target::PlatformItem,
    );
    if entrance < 1. {
        out.fade(start, entrance, 1.);
    }
}

/// A person's page: photo, name, what they are known for and their story, then their
/// films and shows by department.
pub fn person_page(out: &mut Scene, state: &State, assets: &Assets) {
    let Some(page) = &state.feeds.person else {
        return;
    };
    let reduced_motion = state.preferences.reduced_motion;
    let entrance = page.entrance(reduced_motion);
    let start = out.draw.len();
    out.shape(FULL, 0., c(BASE, 1.));
    // The focused title's backdrop, faint, once its details are here.
    let backdrop = match state.focus {
        Target::PersonCredit(row, index) => state
            .feeds
            .person_credit(row, index)
            .and_then(|item| state.feeds.details.get(&item.link))
            .and_then(|details| details.backdrop.as_ref()),
        _ => None,
    };
    if let Some((image, url)) = held_image(state, backdrop) {
        let reveal = state.feeds.reveal(url, reduced_motion);
        out.draw.push(Draw::AppImage(
            image.clone(),
            Rect::new(0., -page.page_scroll * 0.3, 1920., 1080.),
            [0.7, 0.7, 0.7, reveal * 0.6],
        ));
    }
    out.draw
        .push(Draw::RoundedGradient(FULL, 0., c(BASE, 0.6), c(BASE, 0.97)));
    side_shade(out, 1100., 0.7);
    let lift = (1. - entrance) * 24.;
    let s = page.page_scroll;
    let top = |y: f32| y - s + lift;
    let info = page.info();
    // The photo, or initials.
    let photo = Rect::new(LEFT, top(120.), 240., 360.);
    let image = info
        .and_then(|info| info.image.as_ref())
        .filter(|url| state.feeds.images.contains_key(*url))
        .or(page.person.image.as_ref());
    out.shadow(photo, CARD_RADIUS, 30., 10., 0.4);
    match held_image(state, image) {
        Some((image, url)) => {
            let reveal = state.feeds.reveal(url, reduced_motion);
            let mut quad = Quad::new(photo, [1., 1., 1., reveal]);
            quad.corners = [CARD_RADIUS; 4];
            quad.image = Some(crate::renderer::QuadImage::App(image.clone()));
            out.draw.push(Draw::Quad(Box::new(quad)));
        }
        None => {
            out.draw.push(Draw::RoundedGradient(
                photo,
                CARD_RADIUS,
                c(0x3a4048, 1.),
                c(0x22272d, 1.),
            ));
            let initials: String = page
                .person
                .name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect();
            out.text(
                initials,
                Rect::new(photo.x, photo.y + photo.h / 2. - 34., photo.w, 68.),
                text(56., 600, 0.),
                c(0xffffff, 0.6),
                true,
            );
        }
    }
    out.border(photo, CARD_RADIUS, 1., c(0xffffff, 0.08));
    let x = photo.x + photo.w + 56.;
    let name = info
        .map(|info| info.name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(&page.person.name);
    out.text(
        name,
        Rect::new(x, top(130.), 1300., 76.),
        text(60., 650, -0.03),
        WHITE,
        false,
    );
    match (&page.info, info) {
        (_, Some(info)) => {
            let mut line = Vec::new();
            if !info.department.is_empty() {
                line.push(format!("Known for {}", info.department));
            }
            if !info.born.is_empty() {
                line.push(info.born.clone());
            }
            out.text(
                line.join("  ·  "),
                Rect::new(x, top(222.), 1300., 28.),
                text(20., 550, 0.),
                c(0xffffff, 0.75),
                false,
            );
            let bio = if info.bio.is_empty() {
                format!("{name} as {}.", page.person.role)
            } else {
                clip_words(&info.bio, 520)
            };
            out.text(
                bio,
                Rect::new(x, top(272.), 1180., 200.),
                text(19.2, 400, 0.).leading(1.5),
                c(0xffffff, 0.84),
                false,
            );
        }
        (Some(Err(error)), None) => out.text(
            format!("Couldn't load this page: {error}"),
            Rect::new(x, top(230.), 1200., 30.),
            text(18.4, 450, 0.),
            c(0xf3ddbd, 1.),
            false,
        ),
        _ => {
            skeleton(out, Rect::new(x, top(228.), 360., 20.), 10.);
            skeleton(out, Rect::new(x, top(282.), 1100., 18.), 9.);
            skeleton(out, Rect::new(x, top(312.), 1060., 18.), 9.);
            skeleton(out, Rect::new(x, top(342.), 700., 18.), 9.);
        }
    }
    for (row, (label, items)) in page.rows().iter().enumerate() {
        let y = top(PersonPage::row_y(row));
        if y > 1080. || y + SECTION_HEADER + CARD_HEIGHT < 0. {
            continue;
        }
        header(out, label, y);
        let items: Vec<&crate::feeds::Item> = items.iter().collect();
        poster_row(
            out,
            state,
            assets,
            &items,
            y + SECTION_HEADER,
            page.scroll.get(row).copied().unwrap_or(0.),
            |index| Target::PersonCredit(row, index),
        );
    }
    if info.is_some() && page.rows().is_empty() {
        out.text(
            "No films or shows to show",
            Rect::new(LEFT, top(PERSON_ROWS_Y), 1200., 30.),
            text(20., 500, 0.),
            c(0xffffff, 0.6),
            false,
        );
    }
    if entrance < 1. {
        out.fade(start, entrance, 1.);
    }
}
