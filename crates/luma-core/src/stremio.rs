//! Stremio addon requests (AIOStreams, subtitle addons) and their replies.

use crate::http::encode;
use crate::ids::Kind;

/// One stream as AIOStreams lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    /// AIOStreams' first line, e.g. `[RD+] 4K DV`.
    pub name: String,
    /// Its detail lines: release, size, languages.
    pub description: String,
    /// Empty for torrents (`infoHash` only).
    pub url: String,
    pub binge_group: Option<String>,
    pub filename: Option<String>,
    /// `behaviorHints.proxyHeaders`: `<video>` can't send them.
    pub needs_headers: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subtitle {
    pub id: String,
    pub url: String,
    /// As the addon gives it: usually ISO 639-2 (`eng`), sometimes 639-1 or a name.
    pub lang: String,
}

/// A manifest URL without `manifest.json` and trailing slashes: the addon's base.
pub fn base(manifest_url: &str) -> String {
    let url = manifest_url.trim();
    let url = url.strip_suffix("manifest.json").unwrap_or(url);
    url.trim_end_matches('/').to_owned()
}

pub fn streams_url(base: &str, kind: Kind, id: &str) -> String {
    format!(
        "{base}/stream/{}/{}.json",
        kind.as_str(),
        encode(id).replace("%3A", ":")
    )
}

/// With the playing file's name when known, so addons can match its release.
pub fn subtitles_url(base: &str, kind: Kind, id: &str, filename: Option<&str>) -> String {
    let id = encode(id).replace("%3A", ":");
    match filename.filter(|name| !name.is_empty()) {
        Some(name) => format!(
            "{base}/subtitles/{}/{id}/filename={}.json",
            kind.as_str(),
            encode(name)
        ),
        None => format!("{base}/subtitles/{}/{id}.json", kind.as_str()),
    }
}

#[cfg(feature = "json")]
mod json {
    use super::*;
    use serde_json::Value;

    fn text(value: &Value, key: &str) -> Option<String> {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    /// `{"streams": [...]}` in AIOStreams' order.
    pub fn parse_streams(text_json: &str) -> Result<Vec<Stream>, String> {
        let value: Value =
            serde_json::from_str(text_json).map_err(|e| format!("Bad stream list: {e}"))?;
        let list = value
            .get("streams")
            .and_then(Value::as_array)
            .ok_or("No streams in the reply")?;
        Ok(list
            .iter()
            .map(|stream| {
                let hints = stream.get("behaviorHints");
                let hint = |key: &str| hints.and_then(|hints| text(hints, key));
                Stream {
                    name: text(stream, "name").unwrap_or_default(),
                    // Older addons put the details in `title`.
                    description: text(stream, "description")
                        .or_else(|| text(stream, "title"))
                        .unwrap_or_default(),
                    url: text(stream, "url").unwrap_or_default(),
                    binge_group: hint("bingeGroup"),
                    filename: hint("filename"),
                    needs_headers: hints
                        .and_then(|hints| hints.get("proxyHeaders"))
                        .is_some_and(|headers| !headers.is_null()),
                }
            })
            .collect())
    }

    /// `{"subtitles": [{"id", "url", "lang"}]}`.
    pub fn parse_subtitles(text_json: &str) -> Result<Vec<Subtitle>, String> {
        let value: Value =
            serde_json::from_str(text_json).map_err(|e| format!("Bad subtitle list: {e}"))?;
        let list = value
            .get("subtitles")
            .and_then(Value::as_array)
            .ok_or("No subtitles in the reply")?;
        Ok(list
            .iter()
            .filter_map(|sub| {
                let url = text(sub, "url").filter(|url| url.starts_with("http"))?;
                Some(Subtitle {
                    id: text(sub, "id").unwrap_or_else(|| url.clone()),
                    url,
                    lang: text(sub, "lang").unwrap_or_else(|| "und".into()),
                })
            })
            .collect())
    }
}

#[cfg(feature = "json")]
pub use json::{parse_streams, parse_subtitles};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_addon_urls() {
        let base = base("https://aio.example/stremio/abc/xyz/manifest.json");
        assert_eq!(base, "https://aio.example/stremio/abc/xyz");
        assert_eq!(
            streams_url(&base, Kind::Series, "tt0944947:1:2"),
            "https://aio.example/stremio/abc/xyz/stream/series/tt0944947:1:2.json"
        );
        assert_eq!(
            subtitles_url(
                "https://subs",
                Kind::Movie,
                "tt0133093",
                Some("The Matrix 1999.mkv")
            ),
            "https://subs/subtitles/movie/tt0133093/filename=The%20Matrix%201999.mkv.json"
        );
        assert_eq!(
            subtitles_url("https://subs", Kind::Movie, "tt0133093", Some("")),
            "https://subs/subtitles/movie/tt0133093.json"
        );
    }

    #[cfg(feature = "json")]
    #[test]
    fn parses_replies() {
        let streams = parse_streams(
            r#"{"streams":[
                {"name":"[RD+] 4K","description":"Matrix.2160p","url":"https://rd/1",
                 "behaviorHints":{"bingeGroup":"aio|rd|4k","filename":"Matrix.mkv"}},
                {"name":"P2P","title":"old style","infoHash":"abc"},
                {"name":"Proxy","url":"https://x/2","behaviorHints":{"proxyHeaders":{"request":{}}}}
            ]}"#,
        )
        .unwrap();
        assert_eq!(streams.len(), 3);
        assert_eq!(streams[0].binge_group.as_deref(), Some("aio|rd|4k"));
        assert_eq!(streams[0].filename.as_deref(), Some("Matrix.mkv"));
        assert_eq!(streams[1].description, "old style");
        assert!(streams[1].url.is_empty());
        assert!(streams[2].needs_headers);
        assert!(parse_streams(r#"{"error":"x"}"#).is_err());

        let subs = parse_subtitles(
            r#"{"subtitles":[{"id":"1","url":"https://s/1.srt","lang":"eng"},{"id":"2","lang":"fre"}]}"#,
        )
        .unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].lang, "eng");
    }
}
