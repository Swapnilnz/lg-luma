//! The launch contract between Luma and Luma Player.
//!
//! Luma serialises a [`Launch`] into the webOS launch params; the player parses the same
//! struct from `PalmSystem.launchParams` (or `webOSRelaunch`), so the two can't drift. Params
//! carry IDs and a display title only, never a secret.

use crate::ids::{Kind, valid_imdb};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Launch {
    pub kind: Kind,
    /// The key for AIOStreams and Trakt. When missing, the player looks it up from `tmdb`.
    pub imdb: Option<String>,
    pub tmdb: Option<u64>,
    /// Series only, both or neither. Neither means the next unwatched episode.
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// Shown on the loading screen until TMDB answers.
    pub title: Option<String>,
    /// Dev only: play this URL, skipping AIOStreams.
    pub url: Option<String>,
    /// Dev only: start here, in seconds.
    pub start: Option<f64>,
}

impl Launch {
    pub fn episode_key(&self) -> Option<(u32, u32)> {
        match (self.kind, self.season, self.episode) {
            (Kind::Series, Some(season), Some(number)) => Some((season, number)),
            _ => None,
        }
    }

    /// The same title at another episode (Next Up, or a picked episode).
    pub fn with_episode(&self, (season, number): (u32, u32)) -> Self {
        Self {
            season: Some(season),
            episode: Some(number),
            url: None,
            start: None,
            ..self.clone()
        }
    }

    /// Rejects params the player can't act on, with a message for the error screen.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(imdb) = &self.imdb
            && !valid_imdb(imdb)
        {
            return Err(format!("Not an IMDb ID: {imdb}"));
        }
        if self.imdb.is_none() && self.tmdb.is_none() && self.url.is_none() {
            return Err("No IMDb or TMDB ID to play".into());
        }
        if self.season.is_some() != self.episode.is_some() {
            return Err("Season and episode come together".into());
        }
        if self.kind == Kind::Movie && self.season.is_some() {
            return Err("A movie has no episodes".into());
        }
        if self.season == Some(0) && self.episode == Some(0) {
            return Err("No episode 0".into());
        }
        if let Some(url) = &self.url
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err("Only http(s) URLs play".into());
        }
        if self
            .title
            .as_ref()
            .is_some_and(|title| title.chars().count() > 300)
        {
            return Err("Title too long".into());
        }
        Ok(())
    }

    /// From one of Luma's Nuvio deep links: `nuvio://detail/<movie|series>/<imdb id>` or
    /// `nuvio://tmdb/<movie|series>/<tmdb id>`, with the card's episode and title.
    pub fn from_nuvio_link(link: &str, episode: Option<[u32; 2]>, title: &str) -> Option<Self> {
        let mut parts = link.strip_prefix("nuvio://")?.split('/');
        let scheme = parts.next()?;
        let kind = Kind::parse(parts.next()?)?;
        let id = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        let mut launch = Launch {
            kind,
            title: Some(title.trim().to_owned()).filter(|title| !title.is_empty()),
            ..Launch::default()
        };
        match scheme {
            "detail" if valid_imdb(id) => launch.imdb = Some(id.to_owned()),
            "tmdb" => launch.tmdb = Some(id.parse().ok()?),
            _ => return None,
        }
        if kind == Kind::Series
            && let Some([season, number]) = episode
            && number > 0
        {
            launch.season = Some(season);
            launch.episode = Some(number);
        }
        Some(launch)
    }
}

#[cfg(feature = "json")]
mod json {
    use super::*;
    use serde_json::{Map, Value, json};

    impl Launch {
        pub fn to_value(&self) -> Value {
            let mut map = Map::new();
            map.insert("type".into(), json!(self.kind.as_str()));
            if let Some(imdb) = &self.imdb {
                map.insert("imdb".into(), json!(imdb));
            }
            if let Some(tmdb) = self.tmdb {
                map.insert("tmdb".into(), json!(tmdb));
            }
            if let Some((season, number)) = self.episode_key() {
                map.insert("season".into(), json!(season));
                map.insert("episode".into(), json!(number));
            }
            if let Some(title) = &self.title {
                map.insert("title".into(), json!(title));
            }
            if let Some(url) = &self.url {
                map.insert("url".into(), json!(url));
            }
            if let Some(start) = self.start {
                map.insert("t".into(), json!(start));
            }
            Value::Object(map)
        }

        /// Parses and validates launch params. Numbers may arrive as strings (luna-send by hand).
        pub fn from_value(value: &Value) -> Result<Self, String> {
            let object = value.as_object().ok_or("Launch params are not an object")?;
            let text = |key: &str| {
                object
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let number = |key: &str| -> Option<f64> {
                match object.get(key)? {
                    Value::Number(n) => n.as_f64(),
                    Value::String(s) => s.trim().parse().ok(),
                    _ => None,
                }
            };
            let whole = |key: &str| -> Option<u64> {
                number(key)
                    .filter(|n| *n >= 0. && n.fract() == 0.)
                    .map(|n| n as u64)
            };
            let kind = text("type")
                .as_deref()
                .and_then(Kind::parse)
                .ok_or("Launch params need type: movie or series")?;
            let launch = Launch {
                kind,
                imdb: text("imdb"),
                tmdb: whole("tmdb").filter(|id| *id > 0),
                season: whole("season").map(|n| n as u32),
                episode: whole("episode").map(|n| n as u32),
                title: text("title"),
                url: text("url"),
                start: number("t").filter(|t| *t >= 0.),
            };
            launch.validate()?;
            Ok(launch)
        }

        /// From JSON text: `PalmSystem.launchParams` or the dev `?params=`.
        pub fn from_json(text: &str) -> Result<Self, String> {
            let value: Value =
                serde_json::from_str(text).map_err(|e| format!("Bad launch params: {e}"))?;
            Self::from_value(&value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_luma_deep_links() {
        let movie =
            Launch::from_nuvio_link("nuvio://detail/movie/tt0133093", None, "The Matrix").unwrap();
        assert_eq!(movie.kind, Kind::Movie);
        assert_eq!(movie.imdb.as_deref(), Some("tt0133093"));
        assert_eq!(movie.title.as_deref(), Some("The Matrix"));
        assert!(movie.validate().is_ok());

        let show =
            Launch::from_nuvio_link("nuvio://tmdb/series/1399", Some([2, 5]), " GoT ").unwrap();
        assert_eq!(show.tmdb, Some(1399));
        assert_eq!(show.episode_key(), Some((2, 5)));
        assert_eq!(show.title.as_deref(), Some("GoT"));

        // A movie ignores an episode, and junk links are refused.
        let movie = Launch::from_nuvio_link("nuvio://tmdb/movie/603", Some([1, 1]), "").unwrap();
        assert_eq!(movie.episode_key(), None);
        assert!(movie.title.is_none());
        assert!(Launch::from_nuvio_link("nuvio://detail/movie/603", None, "").is_none());
        assert!(Launch::from_nuvio_link("nuvio://tmdb/movie/x", None, "").is_none());
        assert!(Launch::from_nuvio_link("https://x/movie/1", None, "").is_none());
        assert!(Launch::from_nuvio_link("nuvio://tmdb/movie/1/x", None, "").is_none());
    }

    #[test]
    fn validates() {
        let ok = Launch {
            kind: Kind::Series,
            imdb: Some("tt0944947".into()),
            season: Some(1),
            episode: Some(2),
            ..Launch::default()
        };
        assert!(ok.validate().is_ok());
        assert!(
            Launch {
                episode: None,
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            Launch {
                kind: Kind::Movie,
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            Launch {
                imdb: Some("tt1".into()),
                ..ok.clone()
            }
            .validate()
            .is_err()
        );
        assert!(Launch::default().validate().is_err());
        assert!(
            Launch {
                url: Some("file:///etc/passwd".into()),
                ..Launch::default()
            }
            .validate()
            .is_err()
        );
        let next = ok.with_episode((2, 1));
        assert_eq!(next.episode_key(), Some((2, 1)));
        assert_eq!(next.imdb, ok.imdb);
    }

    #[cfg(feature = "json")]
    #[test]
    fn round_trips_through_json() {
        let launch = Launch {
            kind: Kind::Series,
            imdb: Some("tt0944947".into()),
            tmdb: Some(1399),
            season: Some(1),
            episode: Some(2),
            title: Some("Game of Thrones".into()),
            url: None,
            start: None,
        };
        let text = launch.to_value().to_string();
        assert_eq!(Launch::from_json(&text).unwrap(), launch);
        // luna-send by hand: numbers as strings.
        let by_hand = Launch::from_json(
            r#"{"type":"series","imdb":"tt0944947","tmdb":"1399","season":"1","episode":"2"}"#,
        )
        .unwrap();
        assert_eq!(by_hand.episode_key(), Some((1, 2)));
        assert!(Launch::from_json(r#"{"imdb":"tt0944947"}"#).is_err());
        assert!(Launch::from_json("[]").is_err());
        assert!(Launch::from_json("{").is_err());
    }
}
