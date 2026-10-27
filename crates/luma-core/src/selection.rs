//! Which stream plays, what comes next when one fails, and which tracks to pick.
//!
//! AIOStreams' order is kept: your ranking rules live there, so nothing here re-sorts.

use crate::stremio::Stream;

/// After this many failed streams, the picker opens instead of trying another.
pub const MAX_TRIES: usize = 5;

/// Drops what `<video>` can't play: torrents (no URL) and streams that need custom headers.
pub fn playable(streams: Vec<Stream>) -> Vec<Stream> {
    streams
        .into_iter()
        .filter(|s| {
            (s.url.starts_with("https://") || s.url.starts_with("http://")) && !s.needs_headers
        })
        .collect()
}

/// The stream to start: the first in the same `bingeGroup` as the last episode's, else the top.
pub fn binge_pick(streams: &[Stream], group: Option<&str>) -> usize {
    group
        .and_then(|group| {
            streams
                .iter()
                .position(|s| s.binge_group.as_deref() == Some(group))
        })
        .unwrap_or(0)
}

/// Streams tried for one title, so fallback never repeats one and knows when to give up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attempts {
    failed: Vec<Option<String>>,
    tries: usize,
}

impl Attempts {
    pub fn new(count: usize) -> Self {
        Self {
            failed: vec![None; count],
            tries: 0,
        }
    }

    /// Marks `index` failed with a reason for the picker. True while fallback may go on.
    pub fn fail(&mut self, index: usize, reason: impl Into<String>) -> bool {
        if let Some(slot) = self.failed.get_mut(index)
            && slot.is_none()
        {
            *slot = Some(reason.into());
            self.tries += 1;
        }
        !self.exhausted()
    }

    pub fn failure(&self, index: usize) -> Option<&str> {
        self.failed.get(index).and_then(|reason| reason.as_deref())
    }

    /// A stream picked by hand gets another chance.
    pub fn clear(&mut self, index: usize) {
        if let Some(slot) = self.failed.get_mut(index) {
            *slot = None;
        }
    }

    pub fn exhausted(&self) -> bool {
        self.tries >= MAX_TRIES || self.failed.iter().all(Option::is_some)
    }

    /// The next untried stream after `current`, in AIOStreams' order, then from the top.
    pub fn next_after(&self, current: usize) -> Option<usize> {
        if self.exhausted() {
            return None;
        }
        let n = self.failed.len();
        (1..=n)
            .map(|step| (current + step) % n)
            .find(|&index| self.failed[index].is_none())
    }
}

/// ISO 639-1 codes with their 639-2 (B and T) codes and English names, for the languages a
/// TV in Australia is most likely to meet.
const LANGUAGES: &[(&str, &[&str])] = &[
    ("en", &["eng", "english"]),
    (
        "es",
        &[
            "spa",
            "spanish",
            "español",
            "espanol",
            "castellano",
            "latino",
        ],
    ),
    ("fr", &["fre", "fra", "french", "français", "francais"]),
    ("de", &["ger", "deu", "german", "deutsch"]),
    ("it", &["ita", "italian", "italiano"]),
    (
        "pt",
        &[
            "por",
            "portuguese",
            "português",
            "portugues",
            "pob",
            "pt-br",
        ],
    ),
    ("ja", &["jpn", "japanese"]),
    ("ko", &["kor", "korean"]),
    ("zh", &["chi", "zho", "chinese", "mandarin", "cantonese"]),
    ("hi", &["hin", "hindi"]),
    ("mr", &["mar", "marathi"]),
    ("ar", &["ara", "arabic"]),
    ("ru", &["rus", "russian"]),
    ("nl", &["dut", "nld", "dutch"]),
    ("sv", &["swe", "swedish"]),
    ("no", &["nor", "nob", "norwegian"]),
    ("da", &["dan", "danish"]),
    ("fi", &["fin", "finnish"]),
    ("pl", &["pol", "polish"]),
    ("tr", &["tur", "turkish"]),
    ("el", &["gre", "ell", "greek"]),
    ("he", &["heb", "hebrew"]),
    ("th", &["tha", "thai"]),
    ("vi", &["vie", "vietnamese"]),
    ("id", &["ind", "indonesian"]),
    ("ms", &["may", "msa", "malay"]),
    ("ta", &["tam", "tamil"]),
    ("te", &["tel", "telugu"]),
];

/// A language tag, code or name as its ISO 639-1 code (`eng`, `English`, `en-AU` → `en`), or
/// the lowercase input when unknown.
pub fn normalize_language(value: &str) -> String {
    let lower = value.trim().to_lowercase();
    if lower == "pt-br" || lower == "pob" {
        return "pt".into();
    }
    let primary = lower
        .split(['-', '_', ' ', '('])
        .next()
        .unwrap_or_default()
        .to_owned();
    for (code, others) in LANGUAGES {
        if primary == *code
            || others.contains(&primary.as_str())
            || others.contains(&lower.as_str())
        {
            return (*code).into();
        }
    }
    primary
}

pub fn same_language(a: &str, b: &str) -> bool {
    let (a, b) = (normalize_language(a), normalize_language(b));
    !a.is_empty() && a == b
}

/// The track to pick for the viewer's preferred languages (in order): the first track in
/// the first preferred language that has one. None when none match.
pub fn pick_track(languages: &[String], preferred: &[String]) -> Option<usize> {
    preferred
        .iter()
        .find_map(|want| languages.iter().position(|have| same_language(have, want)))
}

/// A language's display name for track lists: `en` → `English`; unknown codes upper-cased.
pub fn language_name(value: &str) -> String {
    let code = normalize_language(value);
    let name = match code.as_str() {
        "en" => "English",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        "hi" => "Hindi",
        "mr" => "Marathi",
        "ar" => "Arabic",
        "ru" => "Russian",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "no" => "Norwegian",
        "da" => "Danish",
        "fi" => "Finnish",
        "pl" => "Polish",
        "tr" => "Turkish",
        "el" => "Greek",
        "he" => "Hebrew",
        "th" => "Thai",
        "vi" => "Vietnamese",
        "id" => "Indonesian",
        "ms" => "Malay",
        "ta" => "Tamil",
        "te" => "Telugu",
        "" | "und" => return "Unknown".into(),
        other => return other.to_uppercase(),
    };
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(url: &str, group: Option<&str>) -> Stream {
        Stream {
            url: url.into(),
            binge_group: group.map(Into::into),
            ..Stream::default()
        }
    }

    #[test]
    fn keeps_only_playable_streams_in_order() {
        let mut headers = stream("https://c", None);
        headers.needs_headers = true;
        let kept = playable(vec![
            stream("https://a", None),
            stream("", None),
            headers,
            stream("magnet:?x", None),
            stream("http://b", None),
        ]);
        let urls: Vec<_> = kept.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(urls, ["https://a", "http://b"]);
    }

    #[test]
    fn binge_group_keeps_the_source() {
        let streams = [
            stream("https://a", Some("rd|1080")),
            stream("https://b", Some("rd|4k")),
        ];
        assert_eq!(binge_pick(&streams, Some("rd|4k")), 1);
        assert_eq!(binge_pick(&streams, Some("other")), 0);
        assert_eq!(binge_pick(&streams, None), 0);
    }

    #[test]
    fn falls_back_in_order_and_gives_up_after_five() {
        let mut attempts = Attempts::new(8);
        assert_eq!(attempts.next_after(0), Some(1));
        assert!(attempts.fail(0, "error"));
        assert!(attempts.fail(1, "no first frame"));
        assert_eq!(attempts.next_after(1), Some(2));
        // Failing the same stream twice counts once.
        assert!(attempts.fail(1, "again"));
        assert_eq!(attempts.failure(1), Some("no first frame"));
        assert!(attempts.fail(2, "x"));
        assert!(attempts.fail(3, "x"));
        assert!(!attempts.fail(4, "x"));
        assert!(attempts.exhausted());
        assert_eq!(attempts.next_after(4), None);
    }

    #[test]
    fn wraps_to_untried_streams_above() {
        let mut attempts = Attempts::new(3);
        attempts.fail(2, "x");
        assert_eq!(attempts.next_after(2), Some(0));
        attempts.fail(0, "x");
        attempts.fail(1, "x");
        assert!(attempts.exhausted());
    }

    #[test]
    fn picks_languages() {
        assert_eq!(normalize_language("eng"), "en");
        assert_eq!(normalize_language("English (SDH)"), "en");
        assert_eq!(normalize_language("en-AU"), "en");
        assert_eq!(normalize_language("pt-BR"), "pt");
        assert_eq!(normalize_language("xyz"), "xyz");
        let tracks: Vec<String> = ["jpn", "eng", "en"].map(String::from).to_vec();
        assert_eq!(pick_track(&tracks, &["en".into()]), Some(1));
        assert_eq!(pick_track(&tracks, &["fr".into(), "ja".into()]), Some(0));
        assert_eq!(pick_track(&tracks, &["fr".into()]), None);
        assert_eq!(language_name("ger"), "German");
        assert_eq!(language_name("und"), "Unknown");
        assert_eq!(language_name("xyz"), "XYZ");
    }
}
