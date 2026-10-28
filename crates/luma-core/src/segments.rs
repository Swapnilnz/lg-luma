//! Skip Intro, Skip Recap and Skip to the post-credits scene, from IntroDB's community
//! timestamps (https://introdb.app/docs):
//! `GET {base}/segments?imdb_id=tt…&season=1&episode=2`, no key needed for reads.

use crate::http::{Request, encode};

pub const API: &str = "https://api.introdb.app";

/// A span of the episode, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: f64,
    pub end: f64,
}

impl Span {
    /// Whether the skip button shows at `seconds`: from the start until 2 s before the end.
    pub fn offers_skip(&self, seconds: f64) -> bool {
        seconds >= self.start && seconds < self.end - 2.
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Segments {
    pub intro: Option<Span>,
    pub recap: Option<Span>,
    /// The end credits: Next Up shows when they start.
    pub outro: Option<Span>,
    /// A scene after (or during) the credits: Next Up waits for it, and the credits offer a
    /// skip straight to it.
    pub post_credits: Option<Span>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Intro,
    Recap,
    /// From the credits to the scene after them.
    PostCredits,
}

impl Skip {
    pub fn label(self) -> &'static str {
        match self {
            Skip::Intro => "Skip Intro",
            Skip::Recap => "Skip Recap",
            Skip::PostCredits => "Skip to Post-Credits Scene",
        }
    }
}

impl Segments {
    /// What can be skipped at `seconds`, and where it ends.
    pub fn skip_at(&self, seconds: f64) -> Option<(Skip, f64)> {
        [(Skip::Recap, self.recap), (Skip::Intro, self.intro)]
            .into_iter()
            .find_map(|(kind, span)| {
                span.filter(|s| s.offers_skip(seconds))
                    .map(|s| (kind, s.end))
            })
            .or_else(|| self.credits_skip(seconds))
    }

    /// In the credits before a post-credits scene: a skip to where the scene starts.
    fn credits_skip(&self, seconds: f64) -> Option<(Skip, f64)> {
        let scene = self.post_credits?;
        let credits = self.outro?;
        (scene.start > credits.start + 5. && seconds >= credits.start && seconds < scene.start - 2.)
            .then_some((Skip::PostCredits, scene.start))
    }

    /// A post-credits scene still to come at `seconds`: Next Up waits until it has played.
    pub fn scene_ahead(&self, seconds: f64) -> bool {
        self.post_credits.is_some_and(|scene| seconds < scene.end)
    }
}

pub fn request(base: &str, imdb: &str, episode: Option<(u32, u32)>) -> Request {
    let base = base.trim_end_matches('/');
    let url = match episode {
        Some((season, number)) => format!(
            "{base}/segments?imdb_id={}&season={season}&episode={number}",
            encode(imdb)
        ),
        None => format!("{base}/segments?imdb_id={}&is_movie=true", encode(imdb)),
    };
    Request::get(url).header("Accept", "application/json")
}

#[cfg(feature = "json")]
pub fn parse(text: &str) -> Segments {
    use serde_json::Value;
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Segments::default();
    };
    let span = |key: &str| -> Option<Span> {
        let s = value.get(key).filter(|s| s.is_object())?;
        let seconds = |sec: &str, ms: &str| {
            s.get(sec)
                .and_then(Value::as_f64)
                .or_else(|| s.get(ms).and_then(Value::as_f64).map(|ms| ms / 1000.))
        };
        let start = seconds("start_sec", "start_ms")?;
        let end = seconds("end_sec", "end_ms")?;
        // Low-confidence guesses are left out rather than skipping the wrong part.
        let confident = s
            .get("confidence")
            .and_then(Value::as_f64)
            .is_none_or(|c| c >= 0.5);
        (start >= 0. && end > start + 3. && confident).then_some(Span { start, end })
    };
    Segments {
        intro: span("intro"),
        recap: span("recap"),
        outro: span("outro"),
        post_credits: span("post_credits").or_else(|| span("post-credits")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_requests() {
        assert_eq!(
            request(API, "tt0944947", Some((1, 2))).url,
            "https://api.introdb.app/segments?imdb_id=tt0944947&season=1&episode=2"
        );
        assert_eq!(
            request("https://x/", "tt0133093", None).url,
            "https://x/segments?imdb_id=tt0133093&is_movie=true"
        );
    }

    #[test]
    fn offers_the_skip_until_just_before_the_end() {
        let segments = Segments {
            intro: Some(Span {
                start: 60.,
                end: 120.,
            }),
            recap: Some(Span {
                start: 0.,
                end: 55.,
            }),
            outro: None,
            post_credits: None,
        };
        assert_eq!(segments.skip_at(10.), Some((Skip::Recap, 55.)));
        assert_eq!(segments.skip_at(57.), None);
        assert_eq!(segments.skip_at(60.), Some((Skip::Intro, 120.)));
        assert_eq!(segments.skip_at(118.5), None);
        assert_eq!(segments.skip_at(200.), None);
    }

    #[test]
    fn credits_offer_the_post_credits_scene() {
        let segments = Segments {
            outro: Some(Span {
                start: 2600.,
                end: 2700.,
            }),
            post_credits: Some(Span {
                start: 2705.,
                end: 2760.,
            }),
            ..Segments::default()
        };
        assert_eq!(segments.skip_at(2500.), None);
        assert_eq!(segments.skip_at(2610.), Some((Skip::PostCredits, 2705.)));
        assert_eq!(segments.skip_at(2704.), None);
        assert!(segments.scene_ahead(2710.));
        assert!(!segments.scene_ahead(2760.));
    }

    #[cfg(feature = "json")]
    #[test]
    fn parses_introdb_replies() {
        let reply = r#"{"imdb_id":"tt0903747","season":1,"episode":1,
            "intro":{"start_sec":2.5,"end_sec":58,"start_ms":2500,"end_ms":58000,"confidence":0.92,"submission_count":14},
            "recap":null,"no_recap":true,
            "outro":{"start_ms":2700000,"end_ms":2820000,"confidence":0.3}}"#;
        let segments = parse(reply);
        assert_eq!(
            segments.intro,
            Some(Span {
                start: 2.5,
                end: 58.
            })
        );
        assert_eq!(segments.recap, None);
        // Too unsure to act on.
        assert_eq!(segments.outro, None);
        assert_eq!(parse("not json"), Segments::default());
        let ms_only = parse(r#"{"outro":{"start_ms":2700000,"end_ms":2820000}}"#);
        assert_eq!(
            ms_only.outro,
            Some(Span {
                start: 2700.,
                end: 2820.
            })
        );
    }
}
