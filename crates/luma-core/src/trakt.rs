//! Trakt: device sign-in, token refresh, scrobbling, playback progress and next episode.
//!
//! New Trakt apps have no client secret (October 2026); it is sent only when configured.

use crate::http::Request;
use crate::resume::Point;
use crate::scrobble::Action;
use crate::time::parse_rfc3339_ms;
use serde_json::{Value, json};

pub const API: &str = "https://api.trakt.tv";
const REDIRECT: &str = "urn:ietf:wg:oauth:2.0:oob";

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl App {
    fn headers(&self, request: Request, token: Option<&str>) -> Request {
        let request = request
            .header("trakt-api-version", "2")
            .header("trakt-api-key", self.client_id.clone());
        match token {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        }
    }

    fn with_secret(&self, mut body: Value) -> Value {
        if let Some(secret) = self
            .client_secret
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.starts_with("YOUR_"))
            && let Some(fields) = body.as_object_mut()
        {
            fields.insert("client_secret".into(), json!(secret));
        }
        body
    }

    pub fn device_code(&self) -> Request {
        self.headers(
            Request::post(
                format!("{API}/oauth/device/code"),
                json!({"client_id": self.client_id}).to_string(),
            ),
            None,
        )
    }

    pub fn device_token(&self, device_code: &str) -> Request {
        let body = self.with_secret(json!({"code": device_code, "client_id": self.client_id}));
        self.headers(
            Request::post(format!("{API}/oauth/device/token"), body.to_string()),
            None,
        )
    }

    pub fn refresh(&self, refresh_token: &str) -> Request {
        let body = self.with_secret(json!({
            "refresh_token": refresh_token,
            "client_id": self.client_id,
            "redirect_uri": REDIRECT,
            "grant_type": "refresh_token",
        }));
        self.headers(
            Request::post(format!("{API}/oauth/token"), body.to_string()),
            None,
        )
    }

    pub fn revoke(&self, token: &str) -> Request {
        let body = self.with_secret(json!({"token": token, "client_id": self.client_id}));
        self.headers(
            Request::post(format!("{API}/oauth/revoke"), body.to_string()),
            None,
        )
    }

    pub fn scrobble(&self, token: &str, action: Action, media: &Media, progress: f64) -> Request {
        self.headers(
            Request::post(
                format!("{API}/scrobble/{}", action.path()),
                scrobble_body(media, progress).to_string(),
            ),
            Some(token),
        )
    }

    /// A queued call replayed with today's token.
    pub fn replay(&self, token: &str, path: &str, body: &str) -> Request {
        self.headers(
            Request::post(format!("{API}{path}"), body.to_owned()),
            Some(token),
        )
    }

    /// Paused titles: `movies` or `episodes`.
    pub fn playback(&self, token: &str, episodes: bool) -> Request {
        let kind = if episodes { "episodes" } else { "movies" };
        self.headers(
            Request::get(format!("{API}/sync/playback/{kind}?limit=100")),
            Some(token),
        )
    }

    /// A show's watched progress, with its next episode. `show` is an IMDb or Trakt ID.
    pub fn progress(&self, token: &str, show: &str) -> Request {
        self.headers(
            Request::get(format!(
                "{API}/shows/{show}/progress/watched?hidden=false&specials=false&count_specials=false"
            )),
            Some(token),
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ids {
    pub imdb: Option<String>,
    pub tmdb: Option<u64>,
}

impl Ids {
    fn value(&self) -> Value {
        let mut ids = serde_json::Map::new();
        if let Some(imdb) = &self.imdb {
            ids.insert("imdb".into(), json!(imdb));
        }
        if let Some(tmdb) = self.tmdb {
            ids.insert("tmdb".into(), json!(tmdb));
        }
        Value::Object(ids)
    }

    fn matches(&self, ids: Option<&Value>) -> bool {
        let Some(ids) = ids else { return false };
        let imdb = ids.get("imdb").and_then(Value::as_str);
        let tmdb = ids.get("tmdb").and_then(Value::as_u64);
        (self.imdb.is_some() && imdb == self.imdb.as_deref())
            || (self.tmdb.is_some() && tmdb == self.tmdb)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Media {
    Movie(Ids),
    Episode { show: Ids, season: u32, number: u32 },
}

pub fn scrobble_body(media: &Media, progress: f64) -> Value {
    let progress = (progress * 100.).round() / 100.;
    match media {
        Media::Movie(ids) => json!({"movie": {"ids": ids.value()}, "progress": progress}),
        Media::Episode {
            show,
            season,
            number,
        } => json!({
            "show": {"ids": show.value()},
            "episode": {"season": season, "number": number},
            "progress": progress,
        }),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: f64,
    /// Seconds between polls.
    pub interval: f64,
}

pub fn parse_device_code(text: &str) -> Result<DeviceCode, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("Trakt sign-in: {e}"))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(DeviceCode {
        device_code: s("device_code").ok_or("Trakt sent no device code")?,
        user_code: s("user_code").ok_or("Trakt sent no user code")?,
        verification_url: s("verification_url")
            .unwrap_or_else(|| "https://trakt.tv/activate".into()),
        expires_in: v.get("expires_in").and_then(Value::as_f64).unwrap_or(600.),
        interval: v
            .get("interval")
            .and_then(Value::as_f64)
            .unwrap_or(5.)
            .max(1.),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub access: String,
    pub refresh: String,
    /// Seconds since the epoch.
    pub expires_at: f64,
}

impl Token {
    pub fn from_reply(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        let created = v.get("created_at").and_then(Value::as_f64)?;
        let expires = v
            .get("expires_in")
            .and_then(Value::as_f64)
            .unwrap_or(86_400.);
        Some(Token {
            access: v.get("access_token")?.as_str()?.to_owned(),
            refresh: v.get("refresh_token")?.as_str()?.to_owned(),
            expires_at: created + expires,
        })
    }

    pub fn to_json(&self) -> String {
        json!({"access": self.access, "refresh": self.refresh, "expires_at": self.expires_at})
            .to_string()
    }

    pub fn from_json(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        Some(Token {
            access: v.get("access")?.as_str()?.to_owned(),
            refresh: v.get("refresh")?.as_str()?.to_owned(),
            expires_at: v.get("expires_at")?.as_f64()?,
        })
    }

    /// Refresh a day early, so a long film never meets an expired token mid-way.
    pub fn due(&self, now_s: f64) -> bool {
        now_s >= self.expires_at - 86_400.
    }
}

/// What a device-token poll said.
#[derive(Clone, Debug, PartialEq)]
pub enum Poll {
    Pending,
    SlowDown,
    Signed(Token),
    Expired,
    Denied,
    Failed(String),
}

pub fn poll_reply(status: u16, text: &str) -> Poll {
    match status {
        200 => {
            Token::from_reply(text).map_or(Poll::Failed("Trakt sent no token".into()), Poll::Signed)
        }
        400 => Poll::Pending,
        429 => Poll::SlowDown,
        404 | 409 | 410 => Poll::Expired,
        418 => Poll::Denied,
        other => Poll::Failed(format!("Trakt answered {other}")),
    }
}

/// The newest paused record for this media in a `/sync/playback` reply.
pub fn find_playback(text: &str, media: &Media) -> Option<Point> {
    let list: Value = serde_json::from_str(text).ok()?;
    list.as_array()?
        .iter()
        .filter(|entry| match media {
            Media::Movie(ids) => ids.matches(entry.pointer("/movie/ids")),
            Media::Episode {
                show,
                season,
                number,
            } => {
                show.matches(entry.pointer("/show/ids"))
                    && entry.pointer("/episode/season").and_then(Value::as_u64)
                        == Some(u64::from(*season))
                    && entry.pointer("/episode/number").and_then(Value::as_u64)
                        == Some(u64::from(*number))
            }
        })
        .filter_map(|entry| {
            Some(Point {
                position: None,
                percent: entry.get("progress")?.as_f64()?,
                at_ms: entry
                    .get("paused_at")
                    .and_then(Value::as_str)
                    .and_then(parse_rfc3339_ms)
                    .unwrap_or(0.),
            })
        })
        .max_by(|a, b| a.at_ms.total_cmp(&b.at_ms))
}

/// The next episode in a `/shows/{id}/progress/watched` reply; None when there is none.
pub fn parse_next_episode(text: &str) -> Option<(u32, u32)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let next = v.get("next_episode")?;
    Some((
        next.get("season")?.as_u64()? as u32,
        next.get("number")?.as_u64()? as u32,
    ))
}

/// Episodes marked watched, from a `/shows/{id}/progress/watched` reply.
pub fn parse_watched(text: &str) -> Vec<(u32, u32)> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return vec![];
    };
    let Some(seasons) = v.get("seasons").and_then(Value::as_array) else {
        return vec![];
    };
    seasons
        .iter()
        .filter_map(|season| {
            let number = season.get("number")?.as_u64()? as u32;
            let episodes = season.get("episodes")?.as_array()?;
            Some(episodes.iter().filter_map(move |e| {
                let completed = e.get("completed").and_then(Value::as_bool).unwrap_or(false);
                completed.then_some((number, e.get("number")?.as_u64()? as u32))
            }))
        })
        .flatten()
        .collect()
}

/// This show's paused episodes and how far in (percent), from `/sync/playback/episodes`.
pub fn parse_show_playback(text: &str, show: &Ids) -> Vec<((u32, u32), f64)> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else {
        return vec![];
    };
    list.iter()
        .filter(|entry| show.matches(entry.pointer("/show/ids")))
        .filter_map(|entry| {
            Some((
                (
                    entry.pointer("/episode/season")?.as_u64()? as u32,
                    entry.pointer("/episode/number")?.as_u64()? as u32,
                ),
                entry.get("progress")?.as_f64()?,
            ))
        })
        .collect()
}

/// Whether a reply means the call can be forgotten: done, or a duplicate (409).
pub fn settled(status: u16) -> bool {
    (200..300).contains(&status) || status == 409 || status == 404 || status == 422
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(secret: Option<&str>) -> App {
        App {
            client_id: "cid".into(),
            client_secret: secret.map(Into::into),
        }
    }

    #[test]
    fn sends_the_secret_only_when_set() {
        let body = app(None).device_token("dc").body.unwrap();
        assert!(!body.contains("client_secret"));
        let body = app(Some("YOUR_SECRET")).refresh("r").body.unwrap();
        assert!(!body.contains("client_secret"));
        let body = app(Some("s3")).refresh("r").body.unwrap();
        assert!(body.contains("\"client_secret\":\"s3\""));
    }

    #[test]
    fn scrobbles_episodes_with_show_ids() {
        let media = Media::Episode {
            show: Ids {
                imdb: Some("tt0944947".into()),
                tmdb: Some(1399),
            },
            season: 1,
            number: 2,
        };
        let request = app(None).scrobble("tok", Action::Pause, &media, 42.123);
        assert_eq!(request.url, "https://api.trakt.tv/scrobble/pause");
        assert!(
            request
                .headers
                .contains(&("Authorization".into(), "Bearer tok".into()))
        );
        let body: Value = serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["show"]["ids"]["imdb"], "tt0944947");
        assert_eq!(body["episode"]["number"], 2);
        assert_eq!(body["progress"], 42.12);
    }

    #[test]
    fn polls_and_tokens() {
        assert_eq!(poll_reply(400, ""), Poll::Pending);
        assert_eq!(poll_reply(429, ""), Poll::SlowDown);
        assert_eq!(poll_reply(410, ""), Poll::Expired);
        let reply =
            r#"{"access_token":"a","refresh_token":"r","expires_in":7776000,"created_at":1000}"#;
        let Poll::Signed(token) = poll_reply(200, reply) else {
            panic!("no token")
        };
        assert_eq!(token.expires_at, 7_777_000.);
        assert_eq!(Token::from_json(&token.to_json()), Some(token.clone()));
        assert!(!token.due(1_000.));
        assert!(token.due(7_700_000.));
        let code = parse_device_code(
            r#"{"device_code":"d","user_code":"ABCD1234","verification_url":"https://trakt.tv/activate","expires_in":600,"interval":5}"#,
        )
        .unwrap();
        assert_eq!(code.user_code, "ABCD1234");
    }

    #[test]
    fn finds_the_newest_playback_record() {
        let reply = r#"[
            {"progress":10.0,"paused_at":"2026-10-01T10:00:00.000Z","type":"movie","movie":{"ids":{"imdb":"tt0133093","tmdb":603}}},
            {"progress":55.5,"paused_at":"2026-10-20T10:00:00.000Z","type":"movie","movie":{"ids":{"imdb":"tt0133093","tmdb":603}}},
            {"progress":90.0,"paused_at":"2026-10-25T10:00:00.000Z","type":"movie","movie":{"ids":{"imdb":"tt0000001"}}}
        ]"#;
        let media = Media::Movie(Ids {
            imdb: None,
            tmdb: Some(603),
        });
        assert_eq!(find_playback(reply, &media).unwrap().percent, 55.5);
        let episodes = r#"[{"progress":30,"paused_at":"2026-10-20T10:00:00Z","episode":{"season":1,"number":2},"show":{"ids":{"imdb":"tt0944947"}}}]"#;
        let media = Media::Episode {
            show: Ids {
                imdb: Some("tt0944947".into()),
                tmdb: None,
            },
            season: 1,
            number: 2,
        };
        assert_eq!(find_playback(episodes, &media).unwrap().percent, 30.);
        let other = Media::Episode {
            show: Ids {
                imdb: Some("tt0944947".into()),
                tmdb: None,
            },
            season: 1,
            number: 3,
        };
        assert!(find_playback(episodes, &other).is_none());
    }

    #[test]
    fn watched_episodes_and_show_playback() {
        let progress = r#"{"seasons":[
            {"number":1,"episodes":[{"number":1,"completed":true},{"number":2,"completed":false}]},
            {"number":2,"episodes":[{"number":1,"completed":true}]}]}"#;
        assert_eq!(parse_watched(progress), vec![(1, 1), (2, 1)]);
        assert!(parse_watched("{}").is_empty());
        let playback = r#"[
            {"progress":42.0,"episode":{"season":1,"number":3},"show":{"ids":{"imdb":"tt0944947"}}},
            {"progress":10.0,"episode":{"season":1,"number":1},"show":{"ids":{"imdb":"tt0000001"}}}]"#;
        let show = Ids {
            imdb: Some("tt0944947".into()),
            tmdb: None,
        };
        assert_eq!(parse_show_playback(playback, &show), vec![((1, 3), 42.)]);
    }

    #[test]
    fn next_episode_and_settled_calls() {
        assert_eq!(
            parse_next_episode(r#"{"next_episode":{"season":2,"number":1,"title":"x"}}"#),
            Some((2, 1))
        );
        assert_eq!(parse_next_episode(r#"{"next_episode":null}"#), None);
        assert!(settled(201));
        assert!(settled(409));
        assert!(!settled(503));
        assert!(!settled(401));
    }
}
