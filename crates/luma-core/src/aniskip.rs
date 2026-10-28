//! Anime openings and endings from AniSkip (https://api.aniskip.com), keyed by MyAnimeList
//! ID, which ARM (https://arm.haglund.dev) maps from a TMDB show and season. Both are free and
//! need no key. Coverage of anime is far wider than IntroDB's.

use crate::http::Request;
use crate::segments::{Segments, Span};

pub const ARM: &str = "https://arm.haglund.dev/api/v2";
pub const API: &str = "https://api.aniskip.com/v2";

/// ARM's entries for a TMDB show: one per anime (MyAnimeList counts seasons apart).
pub fn ids_request(tmdb: u64) -> Request {
    Request::get(format!("{ARM}/themoviedb?id={tmdb}")).header("Accept", "application/json")
}

/// The opening and ending of one episode of a MyAnimeList entry.
pub fn skip_request(mal: u64, episode: u32) -> Request {
    Request::get(format!(
        "{API}/skip-times/{mal}/{episode}?types=op&types=ed&episodeLength=0"
    ))
    .header("Accept", "application/json")
}

/// The MyAnimeList ID for a TMDB season: the entry ARM gives that season, else (season 1) the
/// one with no season.
#[cfg(feature = "json")]
pub fn parse_mal(text: &str, season: u32) -> Option<u64> {
    use serde_json::Value;
    let value: Value = serde_json::from_str(text).ok()?;
    let entries: Vec<&Value> = match &value {
        Value::Array(list) => list.iter().collect(),
        other => vec![other],
    };
    let mal = |entry: &Value| entry.get("myanimelist").and_then(Value::as_u64);
    let season_of = |entry: &Value| entry.get("themoviedb-season").and_then(Value::as_u64);
    let exact = entries
        .iter()
        .copied()
        .filter(|entry| season_of(entry) == Some(u64::from(season)))
        .find_map(mal);
    let seasonless = || {
        entries
            .iter()
            .copied()
            .filter(|entry| season_of(entry).is_none())
            .find_map(mal)
    };
    exact.or_else(|| if season == 1 { seasonless() } else { None })
}

/// AniSkip's reply as segments: the opening as the intro, the ending as the credits.
#[cfg(feature = "json")]
pub fn parse_skips(text: &str) -> Segments {
    use serde_json::Value;
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Segments::default();
    };
    let mut segments = Segments::default();
    for result in value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let interval = result.get("interval");
        let at = |key: &str| interval.and_then(|i| i.get(key)).and_then(Value::as_f64);
        let (Some(start), Some(end)) = (at("startTime"), at("endTime")) else {
            continue;
        };
        if start < 0. || end <= start + 3. {
            continue;
        }
        let span = Some(Span { start, end });
        match result.get("skipType").and_then(Value::as_str) {
            Some("op") => segments.intro = segments.intro.or(span),
            Some("ed") => segments.outro = segments.outro.or(span),
            _ => {}
        }
    }
    segments
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;

    #[test]
    fn finds_the_season_and_its_skips() {
        let arm = r#"[{"myanimelist":1,"themoviedb-season":null},
            {"myanimelist":2,"themoviedb-season":2}]"#;
        assert_eq!(parse_mal(arm, 2), Some(2));
        assert_eq!(parse_mal(arm, 1), Some(1));
        assert_eq!(parse_mal(arm, 3), None);
        assert_eq!(parse_mal(r#"{"myanimelist":7}"#, 1), Some(7));
        let skips = parse_skips(
            r#"{"found":true,"results":[
                {"interval":{"startTime":90.5,"endTime":180.5},"skipType":"op"},
                {"interval":{"startTime":1290,"endTime":1380},"skipType":"ed"}]}"#,
        );
        assert_eq!(
            skips.intro,
            Some(Span {
                start: 90.5,
                end: 180.5
            })
        );
        assert_eq!(skips.outro.map(|s| s.start), Some(1290.));
        assert_eq!(parse_skips("nope"), Segments::default());
    }
}
