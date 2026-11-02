//! The Explore page: its name and a line about the focused title (or the search), the filter
//! bar (My Lists, the type, Genre and Sort, Surprise Me), the grid of posters, and an open
//! dropdown over it all.
use super::{Assets, EmptyState, Scene, c, clip_words, empty, poster, skeleton, text};
use crate::feeds::explore::{
    Category, Choices, EXPLORE_COLUMNS, EXPLORE_FILTER_H, EXPLORE_FILTER_Y, EXPLORE_GRID_TOP,
    EXPLORE_GRID_X, EXPLORE_INFO_Y, EXPLORE_RIGHT, EXPLORE_TITLE_Y, Kind, MENU_ROW, MENU_SHOWN,
    card_rect,
};
use crate::feeds::{CARD_RADIUS, Feeds, Item, Status};
use crate::renderer::{Draw, WHITE};
use crate::state::{Rect, State, Target};

/// Where the screen ends for Explore's grid (the content's bottom on Home and Explore).
const BOTTOM: f32 = 1053.6;
const FULL: Rect = Rect::new(0., 0., 1920., 1080.);

/// Behind everything: a dark wash over the wallpaper, and the last focused title's backdrop,
/// dim, crossfading from the one before.
pub fn backdrop(out: &mut Scene, state: &State) {
    let explore = &state.feeds.explore;
    let reduced_motion = state.preferences.reduced_motion;
    out.draw.push(Draw::RoundedGradient(
        FULL,
        0.,
        c(0x10141a, 0.88),
        c(0x06080c, 0.96),
    ));
    fn image<'a>(
        state: &'a State,
        url: &'a Option<String>,
    ) -> Option<(&'a std::sync::Arc<crate::app_artwork::Artwork>, &'a String)> {
        url.as_ref()
            .and_then(|url| Some((state.feeds.images.get(url)?, url)))
    }
    let fade = explore.backdrop_fade(reduced_motion);
    // A slow drift, as the Top Shelf's: one pass per crossfade, never restarting mid-way.
    let drift = if reduced_motion {
        0.
    } else {
        (explore.backdrop_since.elapsed().as_secs_f32() / 24.).min(1.)
    };
    let zoom = 1.03 + 0.04 * drift;
    let frame = Rect::new(
        960. - 960. * zoom,
        540. - 540. * zoom,
        1920. * zoom,
        1080. * zoom,
    );
    if fade < 1.
        && let Some((previous, _)) = image(state, &explore.previous)
    {
        out.draw
            .push(Draw::AppImage(previous.clone(), frame, [1., 1., 1., 0.4]));
    }
    if let Some((current, url)) = image(state, &explore.backdrop) {
        let reveal = state.feeds.reveal(url, reduced_motion);
        out.draw.push(Draw::AppImage(
            current.clone(),
            frame,
            [1., 1., 1., 0.4 * fade * reveal],
        ));
    }
    // Shade the top for the tabs and words, and the bottom for the posters.
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 0., 1920., 420.),
        0.,
        c(0x080b12, 0.82),
        c(0x080b12, 0.42),
    ));
    out.draw.push(Draw::RoundedGradient(
        Rect::new(0., 420., 1920., 660.),
        0.,
        c(0x080b12, 0.42),
        c(0x080b12, 0.9),
    ));
}

pub fn render(out: &mut Scene, state: &State, assets: &Assets) {
    let feeds = &state.feeds;
    if !feeds.can_search && feeds.rows.is_empty() {
        empty(
            out,
            state,
            assets,
            EmptyState {
                rect: Rect::new(84., 240., 1752., 258.7),
                icon: "grid",
                title: "Nothing to explore yet",
                hint: "Add a TMDB api_key to feeds.json to search every movie, show and anime \
                       here, or rows to browse your lists.",
                action: None,
            },
        );
        return;
    }
    heading(out, state, assets);
    let anchors = filters(out, state, assets);
    grid(out, state, assets);
    out.draw.push(Draw::EdgeFade(None));
    out.draw.push(Draw::Clip(Some(state.content_viewport())));
    if feeds.explore.dropdown.is_some() {
        dropdown(out, state, assets, &anchors);
    }
}

/// A category's mark on the My Lists pill and in its dropdown.
fn category_icon(feeds: &Feeds, category: Category) -> &'static str {
    match category {
        Category::Everything => "search",
        Category::Watchlist => "bookmark",
        Category::Row(index) => {
            let source = feeds.rows.get(index).map_or("", |row| row.source.as_str());
            match source {
                "trakt:continue" | "trakt:playback" | "trakt:up_next" | "trakt:upnext" => "recent",
                "trakt:watchlist" => "bookmark",
                "trakt:calendar" => "calendar",
                _ if source.starts_with("smart:because") => "sparkles",
                _ if source.starts_with("smart:new_season") => "star",
                _ => "tv",
            }
        }
    }
}

/// "2024  ·  TMDB 7.8" for a title: its year and rating from the search, else its preview's
/// details; a row's episode first.
fn about_line(feeds: &Feeds, item: &Item) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !item.subtitle.is_empty() {
        parts.push(item.subtitle.clone());
    }
    match feeds.explore.about.get(&item.link) {
        Some((year, rating)) => {
            if let Some(year) = year {
                parts.push(year.to_string());
            }
            if let Some(details) = feeds.details.get(&item.link) {
                if let Some(extent) = &details.extent {
                    parts.push(extent.clone());
                }
                if !details.genres.is_empty() {
                    parts.push(details.genres.join(", "));
                }
            }
            if let Some(rating) = rating {
                parts.push(format!("TMDB {rating:.1}"));
            }
        }
        None => {
            if let Some(details) = feeds.details.get(&item.link) {
                parts.push(super::meta_line(details));
                if let Some(rating) = details.rating {
                    parts.push(format!("TMDB {rating:.1}"));
                }
            }
        }
    }
    parts.retain(|part| !part.is_empty());
    parts.join("  ·  ")
}

/// The page's name (Explore, or the list's), and below it the focused title or the search.
fn heading(out: &mut Scene, state: &State, assets: &Assets) {
    let feeds = &state.feeds;
    let explore = &feeds.explore;
    let title = match explore.category {
        Category::Everything => "Explore".to_owned(),
        category => feeds.category_title(category),
    };
    out.text(
        &title,
        Rect::new(EXPLORE_GRID_X, EXPLORE_TITLE_Y, 1500., 54.),
        text(42., 650, -0.03),
        WHITE,
        false,
    );
    let focused = match state.focus {
        Target::ExploreItem(index) => explore.items.get(index),
        _ => None,
    };
    let info_style = text(18.4, 500, 0.);
    match focused {
        Some(item) => {
            // The focused title's name, bright, then what's known about it, dimmer.
            let name = clip_words(&item.title, 48);
            let name_style = text(18.4, 650, 0.);
            let name_width = assets.width(&name, name_style);
            out.text(
                &name,
                Rect::new(EXPLORE_GRID_X, EXPLORE_INFO_Y, name_width + 4., 24.),
                name_style,
                WHITE,
                false,
            );
            let about = about_line(feeds, item);
            if !about.is_empty() {
                out.text(
                    format!("·  {about}"),
                    Rect::new(
                        EXPLORE_GRID_X + name_width + 14.,
                        EXPLORE_INFO_Y,
                        EXPLORE_RIGHT - EXPLORE_GRID_X - name_width - 14.,
                        24.,
                    ),
                    info_style,
                    c(0xffffff, 0.66),
                    false,
                );
            }
        }
        None => {
            let line = match explore.category {
                Category::Everything => explore.query.summary(),
                _ => "From My Lists".to_owned(),
            };
            out.text(
                line,
                Rect::new(EXPLORE_GRID_X, EXPLORE_INFO_Y, 1400., 24.),
                info_style,
                c(0xffffff, 0.62),
                false,
            );
        }
    }
}

/// A dropdown pill: a mark, its label, and a chevron; returns its rectangle.
#[allow(clippy::too_many_arguments)]
fn select(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    target: Target,
    icon: &str,
    label: &str,
    x: f32,
    active: bool,
) -> Rect {
    let (y, h) = (EXPLORE_FILTER_Y, EXPLORE_FILTER_H);
    let style = text(18.4, 600, -0.01);
    let label = clip_words(label, 26);
    let width = 24. + 30. + assets.width(&label, style) + 12. + 18. + 22.;
    let rect = Rect::new(x, y, width, h);
    let focused = state.focus == target
        || state
            .feeds
            .explore
            .dropdown
            .as_ref()
            .is_some_and(|dropdown| dropdown.anchor == target);
    out.control(target, rect, false);
    out.pill(rect, focused, active);
    let ink = if focused { c(0x1d1d1f, 1.) } else { WHITE };
    out.icon(
        assets,
        icon,
        Rect::new(rect.x + 24., rect.y + 14., 20., 20.),
        ink,
    );
    out.text(
        &label,
        Rect::new(rect.x + 54., rect.y + (h - 23.) / 2., rect.w, 23.),
        style,
        ink,
        false,
    );
    out.icon(
        assets,
        "down",
        Rect::new(rect.x + rect.w - 22. - 18., rect.y + 15., 18., 18.),
        if focused {
            c(0x1d1d1f, 0.7)
        } else {
            c(0xffffff, 0.6)
        },
    );
    if focused {
        out.ring(rect, h / 2.);
    }
    rect
}

/// The filter bar: My Lists, the type, Genre and Sort (searching everything), Surprise Me at
/// the far right. Returns the dropdown pills' rectangles, for the open dropdown.
fn filters(out: &mut Scene, state: &State, assets: &Assets) -> Vec<(Target, Rect)> {
    let feeds = &state.feeds;
    let explore = &feeds.explore;
    let everything = explore.category == Category::Everything;
    let (y, h) = (EXPLORE_FILTER_Y, EXPLORE_FILTER_H);
    let mut anchors = Vec::new();
    let source = match explore.category {
        Category::Everything => "My Lists".to_owned(),
        category => feeds.category_title(category),
    };
    let rect = select(
        out,
        state,
        assets,
        Target::ExploreSource,
        category_icon(feeds, explore.category),
        &source,
        EXPLORE_GRID_X,
        !everything,
    );
    anchors.push((Target::ExploreSource, rect));
    let mut x = rect.x + rect.w + 18.;
    out.shape(Rect::new(x, y + 10., 1.5, h - 20.), 1., c(0xffffff, 0.16));
    x += 18.;
    // Anime is told apart only when searching everything.
    for (index, kind) in Kind::ALL.iter().enumerate() {
        if *kind == Kind::Anime && !everything {
            continue;
        }
        let rect = chip(
            out,
            state,
            assets,
            Target::ExploreType(index),
            kind.label(),
            None,
            None,
            x,
            y,
            h,
            explore.query.kind == *kind,
        );
        x = rect.x + rect.w + 10.;
    }
    if everything && feeds.can_search {
        out.shape(
            Rect::new(x + 8., y + 10., 1.5, h - 20.),
            1.,
            c(0xffffff, 0.16),
        );
        x += 26.;
        let genre = explore.query.genre_name().unwrap_or("Genre");
        let rect = select(
            out,
            state,
            assets,
            Target::ExploreGenre,
            "grid",
            genre,
            x,
            explore.query.genre.is_some(),
        );
        anchors.push((Target::ExploreGenre, rect));
        x = rect.x + rect.w + 10.;
        let rect = select(
            out,
            state,
            assets,
            Target::ExploreSort,
            "sort",
            explore.query.sort.label(),
            x,
            false,
        );
        anchors.push((Target::ExploreSort, rect));
    }
    if !explore.items.is_empty() {
        let label = "Surprise me";
        let style = text(18.4, 600, -0.01);
        let width = 48. + 30. + assets.width(label, style);
        chip(
            out,
            state,
            assets,
            Target::ExploreSurprise,
            label,
            Some("sparkles"),
            None,
            EXPLORE_RIGHT - width,
            y,
            h,
            false,
        );
    }
    anchors
}

/// The grid: seven posters across, each with its episode or year under it, rising in when
/// its titles change.
fn grid(out: &mut Scene, state: &State, assets: &Assets) {
    let feeds = &state.feeds;
    let explore = &feeds.explore;
    let reduced_motion = state.preferences.reduced_motion;
    let scroll = explore.drawn_scroll(reduced_motion);
    let entrance = explore.entrance(reduced_motion);
    let lift = (1. - entrance) * 26.;
    let top = EXPLORE_GRID_TOP - 24.;
    let clip = Rect::new(
        EXPLORE_GRID_X - 40.,
        top,
        EXPLORE_RIGHT - EXPLORE_GRID_X + 60.,
        BOTTOM - top,
    );
    out.draw.push(Draw::Clip(Some(clip)));
    out.draw.push(Draw::EdgeFade(Some([
        if scroll > 1. { top } else { top - 2. },
        if scroll > 1. { top + 40. } else { top - 1. },
        BOTTOM - 30.,
        BOTTOM,
    ])));
    let start = out.draw.len();
    if explore.items.is_empty() {
        empty_grid(out, state, assets);
    }
    let mut focused_card = None;
    for (index, item) in explore.items.iter().enumerate() {
        let base = card_rect(index, scroll);
        let rect = Rect::new(base.x, base.y + lift, base.w, base.h);
        let target = Target::ExploreItem(index);
        out.control(
            target.clone(),
            Rect::new(rect.x, rect.y, rect.w, rect.h + 30.),
            false,
        );
        if rect.y + rect.h + 40. < top || rect.y > BOTTOM {
            continue;
        }
        if state.focus == target {
            focused_card = Some((item, rect, target));
            continue;
        }
        grid_card(out, state, assets, item, rect, &target);
    }
    if let Some((item, rect, target)) = focused_card {
        grid_card(out, state, assets, item, rect, &target);
    }
    if entrance < 1. {
        out.fade_range(start, out.draw.len(), entrance, 1.);
    }
}

fn grid_card(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    item: &Item,
    rect: Rect,
    target: &Target,
) {
    let start = out.draw.len();
    let focused = state.focus == *target;
    let scale = state.feeds.scale(target, state.preferences.reduced_motion);
    let drawn = poster(out, state, assets, item, rect, focused, scale);
    if state.pressed.as_ref() == Some(target) {
        out.fade(start, 1., 0.85);
    }
    let caption = if item.subtitle.is_empty() {
        state
            .feeds
            .explore
            .about
            .get(&item.link)
            .and_then(|(year, _)| *year)
            .map(|year| year.to_string())
            .unwrap_or_default()
    } else {
        item.subtitle.clone()
    };
    if !caption.is_empty() {
        out.text(
            clip_words(&caption, 30),
            Rect::new(rect.x + 2., drawn.y + drawn.h + 10., rect.w - 4., 20.),
            text(15.2, if focused { 600 } else { 500 }, 0.),
            if focused {
                c(0xffffff, 1.)
            } else {
                c(0xececec, 0.55)
            },
            false,
        );
    }
}

/// An empty grid's panel.
fn notice<'a>(
    rect: Rect,
    icon: &'a str,
    title: &'a str,
    hint: &'a str,
    action: Option<Target>,
) -> EmptyState<'a> {
    EmptyState {
        rect,
        icon,
        title,
        hint,
        action,
    }
}

/// An empty grid: placeholders while it loads, Trakt's sign-in, a failed search, or nothing
/// matching.
fn empty_grid(out: &mut Scene, state: &State, assets: &Assets) {
    let feeds = &state.feeds;
    let explore = &feeds.explore;
    let row = feeds
        .category_row(explore.category)
        .and_then(|index| feeds.rows.get(index));
    let rect = Rect::new(
        EXPLORE_GRID_X,
        EXPLORE_GRID_TOP + 6.,
        EXPLORE_RIGHT - EXPLORE_GRID_X,
        258.7,
    );
    if let Some(row) = row
        && matches!(row.status, Status::SignIn { .. })
    {
        empty(
            out,
            state,
            assets,
            notice(
                rect,
                "bookmark",
                "Sign in to Trakt",
                "Your lists appear here once Luma is signed in to Trakt. The code to enter is \
                 on Home.",
                None,
            ),
        );
        return;
    }
    if explore.category == Category::Everything && !feeds.can_search {
        empty(
            out,
            state,
            assets,
            notice(
                rect,
                "search",
                "Searching needs TMDB",
                "Add a TMDB api_key to feeds.json to search every movie, show and anime here. \
                 My Lists still shows your rows.",
                None,
            ),
        );
        return;
    }
    if let Some(error) = explore
        .error
        .as_deref()
        .filter(|_| explore.category == Category::Everything)
    {
        empty(
            out,
            state,
            assets,
            notice(
                rect,
                "alert",
                "Couldn't search TMDB",
                error,
                Some(Target::ExploreRetry),
            ),
        );
        return;
    }
    if feeds.explore_loading() {
        for index in 0..EXPLORE_COLUMNS * 2 {
            skeleton(out, card_rect(index, 0.), CARD_RADIUS);
        }
        return;
    }
    let (icon, title, hint, action) = if explore.filtered() {
        (
            "search",
            "Nothing matches",
            "Try another type or genre, or clear the filters.",
            Some(Target::ExploreClear),
        )
    } else if explore.category == Category::Watchlist {
        (
            "bookmark",
            "Nothing here yet",
            "Add titles to your Trakt Watchlist from a title's page or by holding OK on a \
             poster.",
            None,
        )
    } else {
        (
            "grid",
            "Nothing here yet",
            "This list has no titles right now.",
            None,
        )
    };
    empty(out, state, assets, notice(rect, icon, title, hint, action));
}

/// An open dropdown under its pill: its choices (the one in effect ticked), scrolling past
/// MENU_SHOWN. While it is open, only its choices take the pointer.
fn dropdown(out: &mut Scene, state: &State, assets: &Assets, anchors: &[(Target, Rect)]) {
    let feeds = &state.feeds;
    let Some(dropdown) = &feeds.explore.dropdown else {
        return;
    };
    let Some((_, anchor)) = anchors
        .iter()
        .find(|(target, _)| *target == dropdown.anchor)
    else {
        return;
    };
    out.controls.retain(|control| {
        !matches!(
            control.target,
            Target::ExploreSource
                | Target::ExploreType(_)
                | Target::ExploreGenre
                | Target::ExploreSort
                | Target::ExploreSurprise
                | Target::ExploreItem(_)
                | Target::ExploreClear
                | Target::ExploreRetry
        )
    });
    let start = out.draw.len();
    let shown = dropdown.labels.len().min(MENU_SHOWN);
    let style = text(18.4, 550, -0.01);
    let widest = dropdown
        .labels
        .iter()
        .map(|label| assets.width(&clip_words(label, 34), style))
        .fold(0., f32::max);
    let width = (widest + 24. + 30. + 44. + 24.).max(anchor.w).max(320.);
    let height = shown as f32 * MENU_ROW + 20.;
    let panel = Rect::new(
        anchor.x.min(EXPLORE_RIGHT - width),
        anchor.y + anchor.h + 12.,
        width,
        height,
    );
    out.modal = Some(panel);
    // The rest of the page dims a little beneath it.
    out.shape(FULL, 0., c(0x080b12, 0.35));
    out.shadow(panel, 22., 80., 24., 0.5);
    out.draw.push(Draw::RoundedGradient(
        panel,
        22.,
        c(0x34383d, 0.98),
        c(0x25292e, 0.99),
    ));
    out.border(panel, 22., 1., c(0xffffff, 0.12));
    for (row, index) in (dropdown.scroll..dropdown.scroll + shown).enumerate() {
        let Some(label) = dropdown.labels.get(index) else {
            break;
        };
        let rect = Rect::new(
            panel.x + 10.,
            panel.y + 10. + row as f32 * MENU_ROW,
            panel.w - 20.,
            MENU_ROW - 6.,
        );
        let target = Target::ExploreOption(index);
        let focused = state.focus == target;
        if focused {
            out.draw.push(Draw::RoundedGradient(
                rect,
                14.,
                c(0xfafafa, 1.),
                c(0xe5e5e5, 1.),
            ));
        }
        let ink = if focused { c(0x1d1d1f, 1.) } else { WHITE };
        let icon = match &dropdown.choices {
            Choices::Source(categories) => categories
                .get(index)
                .map(|category| category_icon(feeds, *category)),
            Choices::Genre(_) | Choices::Sort(_) => None,
        };
        let mut left = rect.x + 18.;
        if let Some(icon) = icon {
            out.icon(
                assets,
                icon,
                Rect::new(left, rect.y + (rect.h - 20.) / 2., 20., 20.),
                ink,
            );
            left += 32.;
        }
        out.text(
            clip_words(label, 34),
            Rect::new(left, rect.y + (rect.h - 23.) / 2., rect.w - 80., 23.),
            style,
            ink,
            false,
        );
        if index == dropdown.chosen {
            out.icon(
                assets,
                "check",
                Rect::new(
                    rect.x + rect.w - 40.,
                    rect.y + (rect.h - 20.) / 2.,
                    20.,
                    20.,
                ),
                if focused { ink } else { c(0x64d2ff, 1.) },
            );
        }
        out.control(target, rect, false);
    }
    // More above or below: a hint at the panel's edge.
    if dropdown.scroll > 0 {
        out.icon(
            assets,
            "up",
            Rect::new(panel.x + panel.w / 2. - 9., panel.y - 2., 18., 18.),
            c(0xffffff, 0.5),
        );
    }
    if dropdown.scroll + shown < dropdown.labels.len() {
        out.icon(
            assets,
            "down",
            Rect::new(
                panel.x + panel.w / 2. - 9.,
                panel.y + panel.h - 16.,
                18.,
                18.,
            ),
            c(0xffffff, 0.5),
        );
    }
    if !state.preferences.reduced_motion {
        let t = (dropdown.opened.elapsed().as_secs_f32() / 0.22).min(1.);
        if t < 1. {
            out.fade(start, 1. - (1. - t).powi(3), 1.);
        }
    }
}

/// A filter chip: a pill with an optional mark, white when focused, filled when chosen.
#[allow(clippy::too_many_arguments)]
fn chip(
    out: &mut Scene,
    state: &State,
    assets: &Assets,
    target: Target,
    label: &str,
    icon: Option<&str>,
    count: Option<usize>,
    x: f32,
    y: f32,
    height: f32,
    selected: bool,
) -> Rect {
    let focused = state.focus == target;
    let size = if height >= 44. { 18.4 } else { 16.8 };
    let style = text(size, 600, -0.01);
    let count_style = text(size - 2., 600, 0.);
    let count_label = count.map(|count| count.to_string()).unwrap_or_default();
    let count_width = if count_label.is_empty() {
        0.
    } else {
        assets.width(&count_label, count_style) + 10.
    };
    let icon_width = if icon.is_some() { 30. } else { 0. };
    let pad = if height >= 44. { 24. } else { 18. };
    let width = pad * 2. + icon_width + assets.width(label, style) + count_width;
    let rect = Rect::new(x, y, width, height);
    out.control(target, rect, false);
    if rect.x > EXPLORE_RIGHT + 40. || rect.x + rect.w < EXPLORE_GRID_X - 40. {
        return rect;
    }
    out.pill(rect, focused, selected);
    let ink = if focused {
        c(0x1d1d1f, 1.)
    } else if selected {
        WHITE
    } else {
        c(0xeeeeee, 0.78)
    };
    let mut left = rect.x + pad;
    if let Some(icon) = icon {
        let mark = 20.;
        out.icon(
            assets,
            icon,
            Rect::new(left, rect.y + (height - mark) / 2., mark, mark),
            ink,
        );
        left += icon_width;
    }
    let text_y = rect.y + (height - size * 1.25) / 2.;
    out.text(
        label,
        Rect::new(left, text_y, rect.w, size * 1.25),
        style,
        ink,
        false,
    );
    if !count_label.is_empty() {
        out.text(
            count_label,
            Rect::new(
                left + assets.width(label, style) + 10.,
                text_y + 1.,
                count_width,
                size * 1.25,
            ),
            count_style,
            if focused {
                c(0x1d1d1f, 0.5)
            } else {
                c(0xffffff, 0.45)
            },
            false,
        );
    }
    if focused {
        out.ring(rect, height / 2.);
    }
    rect
}
