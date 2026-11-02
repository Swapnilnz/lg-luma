//! What a stream is, read from AIOStreams' name, description and filename: resolution,
//! Dolby Vision or HDR, source (Remux, Blu-ray, WEB-DL), codec, audio, channels, size, and the
//! debrid service it comes from. Luma Player shows these as badges while the stream loads.

use crate::stremio::Stream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    Resolution,
    DolbyVision,
    Hdr,
    Source,
    Codec,
    BitDepth,
    Atmos,
    Audio,
    Channels,
    Language,
    /// A cinema recording (CAM, telesync): worth a warning.
    Cam,
}

impl BadgeKind {
    /// The badge's CSS class.
    pub fn class(self) -> &'static str {
        match self {
            BadgeKind::Resolution => "res",
            BadgeKind::DolbyVision => "dv",
            BadgeKind::Hdr => "hdr",
            BadgeKind::Source => "src",
            BadgeKind::Codec => "codec",
            BadgeKind::BitDepth => "codec",
            BadgeKind::Atmos => "atmos",
            BadgeKind::Audio => "audio",
            BadgeKind::Channels => "ch",
            BadgeKind::Language => "lang",
            BadgeKind::Cam => "cam",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub kind: BadgeKind,
    pub label: String,
}

/// Everything shown about a stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo {
    pub badges: Vec<Badge>,
    /// The debrid service, e.g. `Real-Debrid`, and whether it has the file ready.
    pub service: Option<String>,
    pub cached: Option<bool>,
    /// `14.2 GB`.
    pub size: Option<String>,
    /// The release's file name, when known.
    pub release: Option<String>,
}

/// Words of the stream's text, upper case, split at anything but letters, digits and `+`.
fn words(text: &str) -> Vec<String> {
    text.to_uppercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '+'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

struct Text {
    upper: String,
    words: Vec<String>,
}

impl Text {
    fn word(&self, word: &str) -> bool {
        self.words.iter().any(|w| w == word)
    }

    fn any(&self, words: &[&str]) -> bool {
        words.iter().any(|word| self.word(word))
    }

    fn prefix(&self, prefix: &str) -> bool {
        self.words.iter().any(|w| w.starts_with(prefix))
    }

    /// The words in a row, e.g. `WEB DL` for `WEB-DL`.
    fn run(&self, run: &[&str]) -> bool {
        self.words
            .windows(run.len())
            .any(|window| window.iter().zip(run).all(|(a, b)| a == b))
    }
}

/// The audio's channels, from `7.1`, `5.1` or `2.0` (not a size such as `5.1 GB`).
fn channels(upper: &str) -> Option<&'static str> {
    let bytes = upper.as_bytes();
    let mut best: Option<(u8, &'static str)> = None;
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] != b'.' || !bytes[i - 1].is_ascii_digit() || !bytes[i + 1].is_ascii_digit() {
            continue;
        }
        // One digit either side: not part of a longer number such as 12.1.
        let digit = |j: Option<usize>| {
            j.and_then(|j| bytes.get(j))
                .is_some_and(|b| b.is_ascii_digit())
        };
        if digit(i.checked_sub(2)) || digit(Some(i + 2)) {
            continue;
        }
        let rest = upper[i + 2..].trim_start();
        if ["GB", "GIB", "MB", "MIB", "TB", "G ", "M "]
            .iter()
            .any(|unit| rest.starts_with(unit))
        {
            continue;
        }
        let found = match (bytes[i - 1], bytes[i + 1]) {
            (b'7', b'1') => (3, "7.1"),
            (b'5', b'1') => (2, "5.1"),
            (b'6', b'1') => (2, "6.1"),
            (b'2', b'0') => (1, "Stereo"),
            _ => continue,
        };
        if best.is_none_or(|(rank, _)| found.0 > rank) {
            best = Some(found);
        }
    }
    best.map(|(_, label)| label)
}

/// The file's size, e.g. `14.2 GB`.
fn size(text: &str) -> Option<String> {
    let upper = text.to_uppercase();
    let bytes = upper.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || (i > 0 && (bytes[i - 1].is_ascii_digit())) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b',')
        {
            i += 1;
        }
        let number = upper[start..i].replace(',', ".");
        let rest = upper[i..].trim_start();
        let unit = ["TIB", "GIB", "MIB", "TB", "GB", "MB"]
            .into_iter()
            .find(|unit| rest.starts_with(unit))
            .filter(|unit| {
                // A whole word: `GB`, not the start of `GBP`.
                rest[unit.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_ascii_alphabetic())
            });
        if let (Some(unit), Ok(value)) = (unit, number.trim_end_matches('.').parse::<f64>()) {
            let unit = match unit {
                "TIB" | "TB" => "TB",
                "GIB" | "GB" => "GB",
                _ => "MB",
            };
            return Some(if value >= 100. || unit == "MB" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            });
        }
    }
    None
}

/// AIOStreams' service tag at the start of the name, e.g. `[RD+]` or `[TB⚡]`.
fn service(name: &str) -> (Option<String>, Option<bool>) {
    let tag = name
        .split(['[', ']'])
        .nth(1)
        .filter(|_| name.trim_start().starts_with('['))
        .unwrap_or_else(|| name.split_whitespace().next().unwrap_or(""));
    let code: String = tag
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_uppercase();
    let cached = if tag.contains('+') || tag.contains('⚡') {
        Some(true)
    } else if tag.contains('⏳') || tag.contains('↓') {
        Some(false)
    } else {
        None
    };
    let service = match code.as_str() {
        "RD" => "Real-Debrid",
        "AD" => "AllDebrid",
        "PM" => "Premiumize",
        "TB" | "TRB" => "TorBox",
        "DL" | "DLL" => "Debrid-Link",
        "ED" => "EasyDebrid",
        "OC" => "Offcloud",
        "PKP" | "PP" => "PikPak",
        "SDL" => "Seedr",
        "P2P" => "Torrent",
        _ => return (None, None),
    };
    (Some(service.into()), cached)
}

/// The badges and details of a stream, in the order they're shown.
pub fn stream_info(stream: &Stream) -> StreamInfo {
    let all = format!(
        "{}\n{}\n{}",
        stream.name,
        stream.description,
        stream.filename.as_deref().unwrap_or("")
    );
    let text = Text {
        upper: all.to_uppercase(),
        words: words(&all),
    };
    let mut badges = Vec::new();
    let mut push = |kind: BadgeKind, label: &str| {
        badges.push(Badge {
            kind,
            label: label.into(),
        })
    };

    let resolution = if text.any(&["2160P", "4K", "UHD", "2160"]) {
        Some("4K")
    } else if text.word("1440P") {
        Some("1440p")
    } else if text.any(&["1080P", "1080I", "FHD"]) {
        Some("1080p")
    } else if text.word("720P") {
        Some("720p")
    } else if text.any(&["576P", "480P", "SD"]) {
        Some("SD")
    } else {
        None
    };
    if let Some(resolution) = resolution {
        push(BadgeKind::Resolution, resolution);
    }

    if text.any(&["DV", "DOVI", "DOLBYVISION"]) || text.run(&["DOLBY", "VISION"]) {
        push(BadgeKind::DolbyVision, "Dolby Vision");
    }
    if text.any(&["HDR10+", "HDR10PLUS"]) || text.run(&["HDR10", "PLUS"]) {
        push(BadgeKind::Hdr, "HDR10+");
    } else if text.word("HDR10") {
        push(BadgeKind::Hdr, "HDR10");
    } else if text.any(&["HDR", "HDR+"]) {
        push(BadgeKind::Hdr, "HDR");
    } else if text.word("HLG") {
        push(BadgeKind::Hdr, "HLG");
    }

    if text.any(&[
        "CAM", "HDCAM", "CAMRIP", "TELESYNC", "HDTS", "TS", "TELECINE",
    ]) {
        push(BadgeKind::Cam, "CAM");
    } else if text.any(&["REMUX", "BDREMUX"]) {
        push(BadgeKind::Source, "Remux");
    } else if text.any(&["BLURAY", "BDRIP", "BRRIP", "UHDBLURAY"]) || text.run(&["BLU", "RAY"]) {
        push(BadgeKind::Source, "Blu-ray");
    } else if text.word("WEBDL") || text.run(&["WEB", "DL"]) {
        push(BadgeKind::Source, "WEB-DL");
    } else if text.word("WEBRIP") || text.run(&["WEB", "RIP"]) {
        push(BadgeKind::Source, "WEBRip");
    } else if text.word("WEB") {
        push(BadgeKind::Source, "WEB");
    } else if text.word("HDTV") {
        push(BadgeKind::Source, "HDTV");
    } else if text.any(&["DVDRIP", "DVD"]) {
        push(BadgeKind::Source, "DVD");
    }

    if text.any(&["HEVC", "X265", "H265"]) || text.run(&["H", "265"]) {
        push(BadgeKind::Codec, "HEVC");
    } else if text.word("AV1") {
        push(BadgeKind::Codec, "AV1");
    } else if text.any(&["X264", "H264", "AVC"]) || text.run(&["H", "264"]) {
        push(BadgeKind::Codec, "H.264");
    } else if text.word("VP9") {
        push(BadgeKind::Codec, "VP9");
    }
    if text.any(&["10BIT", "10BITS"]) || text.run(&["10", "BIT"]) {
        push(BadgeKind::BitDepth, "10-bit");
    }

    if text.word("ATMOS") || text.prefix("DDPA") {
        push(BadgeKind::Atmos, "Dolby Atmos");
    }
    let audio = if text.word("TRUEHD") || text.run(&["TRUE", "HD"]) {
        Some("TrueHD")
    } else if text.word("DTSX") || text.run(&["DTS", "X"]) {
        Some("DTS:X")
    } else if text.run(&["DTS", "HD", "MA"]) || text.run(&["DTSHD", "MA"]) {
        Some("DTS-HD MA")
    } else if text.word("DTSHD") || text.run(&["DTS", "HD"]) {
        Some("DTS-HD")
    } else if text.prefix("DTS") {
        Some("DTS")
    } else if text.prefix("DDP")
        || text.prefix("DD+")
        || text.prefix("EAC3")
        || text.run(&["E", "AC", "3"])
    {
        Some("Dolby Digital+")
    } else if text.word("DD") || text.prefix("DD5") || text.prefix("DD2") || text.prefix("AC3") {
        Some("Dolby Digital")
    } else if text.word("FLAC") {
        Some("FLAC")
    } else if text.any(&["LPCM", "PCM"]) {
        Some("PCM")
    } else if text.prefix("AAC") {
        Some("AAC")
    } else if text.prefix("OPUS") {
        Some("Opus")
    } else {
        None
    };
    if let Some(audio) = audio {
        push(BadgeKind::Audio, audio);
    }
    if let Some(channels) = channels(&text.upper) {
        push(BadgeKind::Channels, channels);
    }
    if text.word("MULTI") {
        push(BadgeKind::Language, "Multi-audio");
    } else if text.word("DUAL") {
        push(BadgeKind::Language, "Dual audio");
    }

    let (service, cached) = service(&stream.name);
    let release = stream
        .filename
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            // Else a line of the description that reads like a release (dots, no spaces).
            stream
                .description
                .lines()
                .map(str::trim)
                .find(|line| line.contains('.') && !line.contains(' ') && line.len() > 12)
                .map(str::to_owned)
        });
    StreamInfo {
        badges,
        service,
        cached,
        size: size(&format!("{}\n{}", stream.description, stream.name)),
        release,
    }
}

/// A TMDB genre's name, from its ID.
pub fn genre_name(id: u64) -> Option<&'static str> {
    Some(match id {
        28 | 10759 => "Action",
        12 => "Adventure",
        16 => "Animation",
        35 => "Comedy",
        80 => "Crime",
        99 => "Documentary",
        18 => "Drama",
        10751 => "Family",
        14 => "Fantasy",
        36 => "History",
        27 => "Horror",
        10402 => "Music",
        9648 => "Mystery",
        10749 => "Romance",
        878 | 10765 => "Sci-Fi",
        53 => "Thriller",
        10752 | 10768 => "War",
        37 => "Western",
        10762 => "Kids",
        10763 => "News",
        10764 => "Reality",
        10766 => "Soap",
        10767 => "Talk",
        10770 => "TV Movie",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, description: &str, filename: Option<&str>) -> StreamInfo {
        stream_info(&Stream {
            name: name.into(),
            description: description.into(),
            filename: filename.map(str::to_owned),
            ..Stream::default()
        })
    }

    fn labels(info: &StreamInfo) -> Vec<&str> {
        info.badges.iter().map(|b| b.label.as_str()).collect()
    }

    #[test]
    fn a_uhd_remux_reads_in_full() {
        let info = info(
            "[RD+] AIOStreams 4K DV",
            "Dune Part Two 2024\n📦 64.21 GB  👤 120\n🎞 BluRay REMUX HEVC 10bit\n🎧 TrueHD Atmos 7.1",
            Some(
                "Dune.Part.Two.2024.2160p.UHD.BluRay.REMUX.DV.HDR10.HEVC.TrueHD.7.1.Atmos-FGT.mkv",
            ),
        );
        assert_eq!(
            labels(&info),
            vec![
                "4K",
                "Dolby Vision",
                "HDR10",
                "Remux",
                "HEVC",
                "10-bit",
                "Dolby Atmos",
                "TrueHD",
                "7.1"
            ]
        );
        assert_eq!(info.service.as_deref(), Some("Real-Debrid"));
        assert_eq!(info.cached, Some(true));
        assert_eq!(info.size.as_deref(), Some("64.2 GB"));
        assert!(info.release.unwrap().ends_with("-FGT.mkv"));
    }

    #[test]
    fn a_web_release_with_dd_plus() {
        let info = info(
            "[TB⚡] 1080p",
            "The.Bear.S03E01.1080p.WEB-DL.DDP5.1.H.264-FLUX\n3.2 GB",
            None,
        );
        assert_eq!(
            labels(&info),
            vec!["1080p", "WEB-DL", "H.264", "Dolby Digital+", "5.1"]
        );
        assert_eq!(info.service.as_deref(), Some("TorBox"));
        assert_eq!(info.size.as_deref(), Some("3.2 GB"));
        assert_eq!(
            info.release.as_deref(),
            Some("The.Bear.S03E01.1080p.WEB-DL.DDP5.1.H.264-FLUX")
        );
    }

    #[test]
    fn a_size_is_not_taken_for_channels() {
        assert_eq!(channels("SIZE 5.1 GB"), None);
        assert_eq!(channels("AAC 2.0 5.1 GB"), Some("Stereo"));
        assert_eq!(channels("DTS-HD MA 5.1"), Some("5.1"));
        assert_eq!(channels("VERSION 12.1"), None);
    }

    #[test]
    fn hdr10_plus_and_dts_x() {
        let info = info(
            "4K HDR10+",
            "Movie.2023.2160p.WEB-DL.DTS-X.7.1.HDR10+.AV1",
            None,
        );
        assert_eq!(
            labels(&info),
            vec!["4K", "HDR10+", "WEB-DL", "AV1", "DTS:X", "7.1"]
        );
        assert_eq!(info.service, None);
    }

    #[test]
    fn sizes_read_in_several_forms() {
        assert_eq!(size("💾 980 MB").as_deref(), Some("980 MB"));
        assert_eq!(size("1,5 GiB").as_deref(), Some("1.5 GB"));
        assert_eq!(size("120.4GB").as_deref(), Some("120 GB"));
        assert_eq!(size("Season 2 GBP").as_deref(), None);
    }
}
