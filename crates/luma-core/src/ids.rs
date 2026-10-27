//! IMDb and TMDB IDs, and the keys built from them.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    #[default]
    Movie,
    Series,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Movie => "movie",
            Kind::Series => "series",
        }
    }

    /// `movie`, or `series` (also `show`, `tv` and `episode`, as other apps name it).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" | "movies" => Some(Kind::Movie),
            "series" | "show" | "shows" | "tv" | "episode" => Some(Kind::Series),
            _ => None,
        }
    }
}

/// `tt` and 5 to 10 digits.
pub fn valid_imdb(id: &str) -> bool {
    id.strip_prefix("tt").is_some_and(|digits| {
        (5..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

/// The ID a Stremio addon expects: `tt0133093`, or `tt0944947:1:2` for an episode.
pub fn stremio_id(imdb: &str, episode: Option<(u32, u32)>) -> String {
    match episode {
        Some((season, number)) => format!("{imdb}:{season}:{number}"),
        None => imdb.to_owned(),
    }
}

/// The key a local resume point is stored under: `movie:tt0133093`, `series:tt0944947:1:2`.
pub fn media_key(kind: Kind, imdb: &str, episode: Option<(u32, u32)>) -> String {
    format!("{}:{}", kind.as_str(), stremio_id(imdb, episode))
}

/// `S1 E2`, the way Apple TV labels an episode.
pub fn episode_label((season, number): (u32, u32)) -> String {
    format!("S{season} E{number}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_imdb_ids() {
        assert!(valid_imdb("tt0133093"));
        assert!(valid_imdb("tt12345678"));
        assert!(!valid_imdb("tt12"));
        assert!(!valid_imdb("0133093"));
        assert!(!valid_imdb("tt0133093x"));
        assert!(!valid_imdb("tt../../x"));
    }

    #[test]
    fn builds_keys() {
        assert_eq!(stremio_id("tt0944947", Some((1, 2))), "tt0944947:1:2");
        assert_eq!(media_key(Kind::Movie, "tt0133093", None), "movie:tt0133093");
        assert_eq!(Kind::parse("Show"), Some(Kind::Series));
        assert_eq!(Kind::parse("film"), None);
        assert_eq!(episode_label((3, 10)), "S3 E10");
    }
}
