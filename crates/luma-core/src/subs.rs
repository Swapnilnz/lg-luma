//! SRT and WebVTT subtitles: decoding, parsing, and finding the cues on screen.
//!
//! Cue text comes out as safe HTML: everything is escaped except `<i>` and `</i>`, and lines
//! are separated by `\n` (render with `white-space: pre-line`).

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// Seconds.
    pub start: f64,
    pub end: f64,
    pub html: String,
}

/// Subtitle bytes as text: the BOM stripped, UTF-8, else Windows-1252.
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// Parses SRT or WebVTT (either timestamp style, with or without hours), sorted by start.
pub fn parse(text: &str) -> Vec<Cue> {
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut cues = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((start, end)) = timing(line) else {
            continue;
        };
        let mut body = Vec::new();
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                break;
            }
            // A cue with no blank line before the next timing (seen in broken SRTs).
            if timing(next).is_some() {
                break;
            }
            body.push(*next);
            lines.next();
        }
        let html = to_html(&body.join("\n"));
        if end > start && !html.trim().is_empty() {
            cues.push(Cue { start, end, html });
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// `00:01:02,345 --> 00:01:04.000 align:start` → (62.345, 64.0).
fn timing(line: &str) -> Option<(f64, f64)> {
    let (left, right) = line.split_once("-->")?;
    let start = timestamp(left.trim())?;
    let end = timestamp(right.split_whitespace().next()?)?;
    Some((start, end))
}

fn timestamp(text: &str) -> Option<f64> {
    let text = text.replace(',', ".");
    let parts: Vec<&str> = text.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (
            h.parse::<f64>().ok()?,
            m.parse::<f64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        [m, s] => (0., m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    let total = h * 3600. + m * 60. + s;
    total.is_finite().then_some(total)
}

/// Cue text to safe HTML, keeping only italics.
fn to_html(text: &str) -> String {
    let text = decode_entities(text);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while !rest.is_empty() {
        // ASS override blocks such as {\an8} or {\i1}.
        if rest.starts_with("{\\")
            && let Some(close) = rest.find('}')
        {
            let block = &rest[..close];
            if block.contains("\\i1") {
                out.push_str("<i>");
            } else if block.contains("\\i0") {
                out.push_str("</i>");
            }
            rest = &rest[close + 1..];
            continue;
        }
        if rest.starts_with('<')
            && let Some(close) = rest.find('>')
        {
            let tag = rest[1..close].trim().to_ascii_lowercase();
            match tag.as_str() {
                "i" => out.push_str("<i>"),
                "/i" => out.push_str("</i>"),
                _ => {}
            }
            rest = &rest[close + 1..];
            continue;
        }
        let ch = rest.chars().next().unwrap_or_default();
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\u{e000}' => out.push_str("&lt;"),
            '\u{e001}' => out.push_str("&gt;"),
            ch => out.push(ch),
        }
        rest = &rest[ch.len_utf8()..];
    }
    balance_italics(out.trim())
}

fn decode_entities(text: &str) -> String {
    // Escaped angle brackets become placeholders, so they stay text and never read as tags.
    text.replace("&nbsp;", "\u{a0}")
        .replace("&lt;", "\u{e000}")
        .replace("&gt;", "\u{e001}")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Closes italics left open, so one cue can't slant the next.
fn balance_italics(html: &str) -> String {
    let opens = html.matches("<i>").count();
    let closes = html.matches("</i>").count();
    let mut out = html.to_owned();
    for _ in closes..opens {
        out.push_str("</i>");
    }
    out
}

/// Parsed cues with a lookup by time.
#[derive(Clone, Debug, Default)]
pub struct Track {
    cues: Vec<Cue>,
    longest: f64,
}

impl Track {
    pub fn new(cues: Vec<Cue>) -> Self {
        let longest = cues.iter().map(|c| c.end - c.start).fold(0., f64::max);
        Self { cues, longest }
    }

    pub fn len(&self) -> usize {
        self.cues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// The cues on screen at `seconds` (video time plus the viewer's offset), in start order.
    /// Binary search to the last cue starting by then, then back over overlapping ones.
    pub fn at(&self, seconds: f64) -> Vec<&Cue> {
        let upto = self.cues.partition_point(|c| c.start <= seconds);
        let mut on: Vec<&Cue> = self.cues[..upto]
            .iter()
            .rev()
            .take_while(|c| c.start >= seconds - self.longest)
            .filter(|c| c.end > seconds)
            .collect();
        on.reverse();
        on
    }

    /// Where the dialogue stops near the end, a sign the credits start: the end of the last
    /// line, or of the last one before a gap of a minute or more in the final 15% (a scene
    /// after the credits). Lines that are only music (♪) don't count. None when the
    /// last line isn't in the final 15%, where it says little.
    pub fn dialogue_end(&self, duration: f64) -> Option<f64> {
        if duration <= 0. {
            return None;
        }
        let spoken: Vec<&Cue> = self
            .cues
            .iter()
            .filter(|c| c.end < duration - 5.)
            .filter(|c| {
                let text: String = c.html.chars().filter(|ch| ch.is_alphanumeric()).collect();
                !text.is_empty() && !c.html.contains('♪') && !c.html.contains('♫')
            })
            .collect();
        let last = spoken.last()?;
        let tail = duration * 0.85;
        // A long silence near the end, then more lines: the credits came between.
        let gap = spoken
            .windows(2)
            .filter(|pair| pair[0].end >= tail && pair[1].start - pair[0].end >= 60.)
            .map(|pair| pair[0].end)
            .next();
        let end = gap.unwrap_or(last.end);
        (end >= duration * 0.85).then_some(end)
    }

    /// The HTML to show at `seconds`, cues joined by line breaks; empty when none.
    pub fn html_at(&self, seconds: f64) -> String {
        self.at(seconds)
            .iter()
            .map(|c| c.html.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:03,500\r\nHello <b>there</b>\r\n<i>world</i>\r\n\r\n2\r\n00:00:03,000 --> 00:00:05,000\r\nOverlap & co\r\n\r\n3\r\n00:00:10,000 --> 00:00:09,000\r\nBackwards\r\n";

    #[test]
    fn finds_where_the_dialogue_stops() {
        let cue = |start: f64, end: f64, html: &str| Cue {
            start,
            end,
            html: html.into(),
        };
        let episode = Track::new(vec![
            cue(10., 12., "Hi"),
            cue(2500., 2510., "Bye"),
            cue(2560., 2600., "♪ theme ♪"),
        ]);
        assert_eq!(episode.dialogue_end(2700.), Some(2510.));
        // A scene after the credits: the dialogue before the long gap.
        let film = Track::new(vec![
            cue(10., 12., "Hi"),
            cue(6200., 6210., "The end"),
            cue(6650., 6660., "Post-credits"),
        ]);
        assert_eq!(film.dialogue_end(6800.), Some(6210.));
        // Lines stop too early to say anything.
        assert_eq!(
            Track::new(vec![cue(10., 12., "Hi")]).dialogue_end(2700.),
            None
        );
    }

    #[test]
    fn parses_srt() {
        let cues = parse(SRT);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start, 1.);
        assert_eq!(cues[0].end, 3.5);
        assert_eq!(cues[0].html, "Hello there\n<i>world</i>");
        assert_eq!(cues[1].html, "Overlap &amp; co");
    }

    #[test]
    fn parses_vtt_without_hours_and_with_settings() {
        let vtt = "WEBVTT\n\nNOTE x\n\n00:01.000 --> 00:02.000 align:start\n<v Bob><c.yellow>Hi</c> &lt;tag&gt;\n\n01:00:00.000 --> 01:00:01.000\n{\\an8}{\\i1}Top\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start, 1.);
        assert_eq!(cues[0].html, "Hi &lt;tag&gt;");
        assert_eq!(cues[1].start, 3600.);
        assert_eq!(cues[1].html, "<i>Top</i>");
    }

    #[test]
    fn escapes_scripts() {
        let cues = parse("00:00:01,000 --> 00:00:02,000\n<script>alert(1)</script>\"q\"\n");
        assert_eq!(cues[0].html, "alert(1)&quot;q&quot;");
    }

    #[test]
    fn decodes_bom_and_windows_1252() {
        assert_eq!(decode(b"\xEF\xBB\xBFcaf\xC3\xA9"), "café");
        assert_eq!(decode(b"caf\xE9 \x93hi\x94"), "café “hi”");
    }

    #[test]
    fn finds_cues_at_boundaries_and_overlaps() {
        let track = Track::new(parse(SRT));
        assert!(track.at(0.999).is_empty());
        assert_eq!(track.at(1.).len(), 1);
        assert_eq!(track.at(3.2).len(), 2);
        // End is exclusive.
        assert_eq!(track.at(3.5).len(), 1);
        assert!(track.at(5.).is_empty());
        assert_eq!(
            track.html_at(3.2),
            "Hello there\n<i>world</i>\nOverlap &amp; co"
        );
    }

    #[test]
    fn long_cue_is_found_behind_short_ones() {
        let cues = parse(
            "00:00:00,000 --> 00:00:30,000\nLong\n\n00:00:10,000 --> 00:00:11,000\nA\n\n00:00:20,000 --> 00:00:21,000\nB\n",
        );
        let track = Track::new(cues);
        let at = track.at(25.);
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].html, "Long");
    }
}
