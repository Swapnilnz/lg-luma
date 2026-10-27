//! TMDB: artwork and runtime for the loading screen, IMDb IDs, and season lists for Next Up.

use crate::http::{Request, encode};
use crate::ids::Kind;
use crate::next::Season;
use serde_json::Value;

pub const API: &str = "https://api.themoviedb.org/3";
pub const IMAGES: &str = "https://image.tmdb.org/t/p";

fn tmdb_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Movie => "movie",
        Kind::Series => "tv",
    }
}

/// A v4 read token is a JWT (sent as a bearer); a v3 key goes in the query, as Luma does.
fn request(key: &str, path: &str, query: &[(&str, &str)]) -> Request {
    let key = key.trim();
    let mut pairs: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect();
    let bearer = key.starts_with("eyJ");
    if !bearer {
        pairs.push(format!("api_key={}", encode(key)));
    }
    let url = if pairs.is_empty() {
        format!("{API}{path}")
    } else {
        format!("{API}{path}?{}", pairs.join("&"))
    };
    let request = Request::get(url).header("Accept", "application/json");
    if bearer {
        request.header("Authorization", format!("Bearer {key}"))
    } else {
        request
    }
}

pub fn details(key: &str, kind: Kind, id: u64) -> Request {
    request(
        key,
        &format!("/{}/{id}", tmdb_kind(kind)),
        &[
            ("append_to_response", "images,external_ids"),
            ("include_image_language", "en,null"),
        ],
    )
}

pub fn find(key: &str, imdb: &str) -> Request {
    request(
        key,
        &format!("/find/{}", encode(imdb)),
        &[("external_source", "imdb_id")],
    )
}

pub fn season(key: &str, show: u64, season: u32) -> Request {
    request(key, &format!("/tv/{show}/season/{season}"), &[])
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub overview: String,
    pub year: String,
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    /// Minutes: the film's, or a typical episode's.
    pub runtime: Option<f64>,
    pub imdb: Option<String>,
    pub seasons: Vec<Season>,
    pub last_aired: Option<(u32, u32)>,
}

fn image(path: Option<&str>, size: &str) -> Option<String> {
    path.filter(|p| p.starts_with('/'))
        .map(|p| format!("{IMAGES}/{size}{p}"))
}

pub fn parse_details(text: &str, kind: Kind) -> Result<Meta, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Bad TMDB reply: {e}"))?;
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let title = match kind {
        Kind::Movie => s("title"),
        Kind::Series => s("name"),
    };
    if title.is_empty() {
        return Err(v
            .get("status_message")
            .and_then(Value::as_str)
            .unwrap_or("TMDB has no such title")
            .to_owned());
    }
    let date = match kind {
        Kind::Movie => s("release_date"),
        Kind::Series => s("first_air_date"),
    };
    // An English logo first, then one without text language.
    let logos = v.pointer("/images/logos").and_then(Value::as_array);
    let logo = logos.and_then(|logos| {
        ["en", ""].iter().find_map(|want| {
            logos.iter().find(|l| {
                l.get("iso_639_1").and_then(Value::as_str).unwrap_or("") == *want
                    && !l
                        .get("file_path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .ends_with(".svg")
            })
        })
    });
    let runtime = match kind {
        Kind::Movie => v.get("runtime").and_then(Value::as_f64),
        Kind::Series => v
            .get("episode_run_time")
            .and_then(Value::as_array)
            .and_then(|times| times.first())
            .and_then(Value::as_f64)
            .or_else(|| {
                v.pointer("/last_episode_to_air/runtime")
                    .and_then(Value::as_f64)
            }),
    }
    .filter(|minutes| *minutes > 0.);
    let seasons = v
        .get("seasons")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|s| {
                    Some(Season {
                        number: s.get("season_number")?.as_u64()? as u32,
                        episodes: s.get("episode_count").and_then(Value::as_u64).unwrap_or(0)
                            as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let last_aired = v.get("last_episode_to_air").and_then(|e| {
        Some((
            e.get("season_number")?.as_u64()? as u32,
            e.get("episode_number")?.as_u64()? as u32,
        ))
    });
    Ok(Meta {
        title,
        overview: s("overview"),
        year: date.chars().take(4).collect(),
        backdrop: image(v.get("backdrop_path").and_then(Value::as_str), "w1280"),
        logo: logo.and_then(|l| image(l.get("file_path").and_then(Value::as_str), "w500")),
        runtime,
        imdb: v
            .pointer("/external_ids/imdb_id")
            .or_else(|| v.get("imdb_id"))
            .and_then(Value::as_str)
            .filter(|id| crate::ids::valid_imdb(id))
            .map(str::to_owned),
        seasons,
        last_aired,
    })
}

/// The TMDB ID for an IMDb ID, from `/find`.
pub fn parse_find(text: &str, kind: Kind) -> Option<u64> {
    let v: Value = serde_json::from_str(text).ok()?;
    let list = match kind {
        Kind::Movie => "movie_results",
        Kind::Series => "tv_results",
    };
    v.get(list)?.as_array()?.first()?.get("id")?.as_u64()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Episode {
    pub number: u32,
    pub name: String,
    pub overview: String,
    pub runtime: Option<f64>,
    pub still: Option<String>,
}

pub fn parse_season(text: &str) -> Vec<Episode> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return vec![];
    };
    v.get("episodes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    Some(Episode {
                        number: e.get("episode_number")?.as_u64()? as u32,
                        name: e
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        overview: e
                            .get("overview")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        runtime: e.get("runtime").and_then(Value::as_f64),
                        still: image(e.get("still_path").and_then(Value::as_str), "w300"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v3_keys_go_in_the_query_and_v4_tokens_in_a_header() {
        let v3 = details("abc123", Kind::Movie, 603);
        assert!(
            v3.url
                .starts_with("https://api.themoviedb.org/3/movie/603?")
        );
        assert!(v3.url.ends_with("&api_key=abc123"));
        let v4 = season("eyJhbGc.x.y", 1399, 2);
        assert_eq!(v4.url, "https://api.themoviedb.org/3/tv/1399/season/2");
        assert!(
            v4.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v == "Bearer eyJhbGc.x.y")
        );
    }

    #[test]
    fn parses_show_details() {
        let meta = parse_details(
            r#"{"name":"Game of Thrones","first_air_date":"2011-04-17","backdrop_path":"/b.jpg",
                "episode_run_time":[],"last_episode_to_air":{"season_number":8,"episode_number":6,"runtime":80},
                "seasons":[{"season_number":0,"episode_count":40},{"season_number":1,"episode_count":10}],
                "external_ids":{"imdb_id":"tt0944947"},
                "images":{"logos":[{"iso_639_1":"fr","file_path":"/fr.png"},{"iso_639_1":"en","file_path":"/en.png"}]}}"#,
            Kind::Series,
        )
        .unwrap();
        assert_eq!(meta.year, "2011");
        assert_eq!(
            meta.logo.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/en.png")
        );
        assert_eq!(
            meta.backdrop.as_deref(),
            Some("https://image.tmdb.org/t/p/w1280/b.jpg")
        );
        assert_eq!(meta.runtime, Some(80.));
        assert_eq!(meta.imdb.as_deref(), Some("tt0944947"));
        assert_eq!(meta.seasons.len(), 2);
        assert_eq!(meta.last_aired, Some((8, 6)));
        assert!(parse_details(r#"{"status_message":"Invalid API key"}"#, Kind::Movie).is_err());
        assert_eq!(
            parse_find(
                r#"{"movie_results":[{"id":603}],"tv_results":[]}"#,
                Kind::Movie
            ),
            Some(603)
        );
        assert_eq!(
            parse_find(r#"{"movie_results":[],"tv_results":[]}"#, Kind::Series),
            None
        );
        let eps = parse_season(
            r#"{"episodes":[{"episode_number":1,"name":"Winter Is Coming","still_path":"/s.jpg"}]}"#,
        );
        assert_eq!(eps[0].name, "Winter Is Coming");
    }
}
