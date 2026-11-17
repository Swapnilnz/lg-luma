//! "Browse by Platform": one Home row of streaming services' wordmarks, from feeds.json's
//! `platforms:netflix,disney-plus,…` source. The row is built here (nothing is fetched);
//! picking a tile opens Explore on what that service streams, through TMDB Discover's watch
//! providers.

use super::Item;

/// A streaming service on the row.
#[derive(Debug)]
pub struct Platform {
    /// Its key in the source, and its wordmark's (`assets/logos/<key>.svg`).
    pub key: &'static str,
    pub title: &'static str,
    /// The tile's background and the wordmark's tint.
    pub background: u32,
    pub ink: u32,
    /// TMDB watch-provider IDs, `|` between several (any of them).
    pub providers: &'static str,
    /// The region whose catalogue it is: TMDB knows availability per country.
    pub region: &'static str,
    /// Whether it has a wordmark; without one its name is lettered.
    pub logo: bool,
}

const fn platform(
    key: &'static str,
    title: &'static str,
    background: u32,
    ink: u32,
    providers: &'static str,
    region: &'static str,
    logo: bool,
) -> Platform {
    Platform {
        key,
        title,
        background,
        ink,
        providers,
        region,
        logo,
    }
}

/// The services known, with their TMDB providers (checked against
/// `/watch/providers/{movie,tv}` on 2026-11-17). Stan and Binge exist only in Australia.
pub const PLATFORMS: [Platform; 9] = [
    platform("netflix", "Netflix", 0x141414, 0xe50914, "8", "US", true),
    platform(
        "disney-plus",
        "Disney+",
        0x0b1f4d,
        0xffffff,
        "337",
        "US",
        true,
    ),
    platform("hbo-max", "HBO Max", 0x0a0a12, 0xffffff, "1899", "US", true),
    platform(
        "prime-video",
        "Prime Video",
        0x0f171e,
        0x1a98ff,
        "9",
        "US",
        true,
    ),
    platform(
        "apple-tv",
        "Apple TV+",
        0x000000,
        0xffffff,
        "350",
        "US",
        true,
    ),
    // Paramount+ is two providers on TMDB now: Premium and Essential.
    platform(
        "paramount-plus",
        "Paramount+",
        0x0064ff,
        0xffffff,
        "2303|2616",
        "US",
        true,
    ),
    platform(
        "crunchyroll",
        "Crunchyroll",
        0xf47521,
        0xffffff,
        "283",
        "US",
        true,
    ),
    platform("stan", "Stan", 0x0072ce, 0xffffff, "21", "AU", false),
    platform("binge", "Binge", 0x101010, 0xffffff, "385", "AU", false),
];

const SOURCE: &str = "platforms:";
const LINK: &str = "platform:";

/// Whether a row's source is a platform row.
pub fn is_source(source: &str) -> bool {
    source.trim().starts_with(SOURCE)
}

/// The platforms a source names, in its order, each once; unknown keys are left out.
pub fn parse(source: &str) -> Vec<usize> {
    let Some(list) = source.trim().strip_prefix(SOURCE) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for key in list.split(',').map(str::trim) {
        if let Some(index) = PLATFORMS.iter().position(|platform| platform.key == key)
            && !found.contains(&index)
        {
            found.push(index);
        }
    }
    found
}

/// The row's cards: one per platform, its link naming it.
pub fn items(source: &str) -> Vec<Item> {
    parse(source)
        .into_iter()
        .map(|index| Item {
            title: PLATFORMS[index].title.into(),
            subtitle: String::new(),
            link: format!("{LINK}{}", PLATFORMS[index].key),
            image: None,
            progress: None,
            episode: None,
            badge: None,
        })
        .collect()
}

/// The platform a card stands for.
pub fn from_link(link: &str) -> Option<usize> {
    let key = link.strip_prefix(LINK)?;
    PLATFORMS.iter().position(|platform| platform.key == key)
}

pub fn get(index: usize) -> Option<&'static Platform> {
    PLATFORMS.get(index)
}
