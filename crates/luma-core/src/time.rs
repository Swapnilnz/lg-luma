//! Clock labels and Trakt's timestamps.

/// `1:12:30`, or `4:05` under an hour.
pub fn clock(seconds: f64) -> String {
    let total = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `1h 12m` or `45m`, for "time left" labels.
pub fn short(seconds: f64) -> String {
    let minutes = if seconds.is_finite() {
        (seconds.max(0.) / 60.).round() as u64
    } else {
        0
    };
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{}m", m.max(1)),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// A time of day the Australian way: `10:32 pm`, `12:05 am`.
pub fn wall_clock(hours: u32, minutes: u32) -> String {
    let suffix = if hours % 24 < 12 { "am" } else { "pm" };
    let hour = match hours % 12 {
        0 => 12,
        h => h,
    };
    format!("{hour}:{:02} {suffix}", minutes % 60)
}

/// Whether an episode aired on `aired` (`YYYY-MM-DD`) is out by `today`. No date means TMDB
/// lists it before it has one: not out.
pub fn released(aired: &str, today: &str) -> bool {
    !aired.is_empty() && aired <= today
}

/// When an episode not out yet comes: "Out 13 Nov", "Out 13 Nov 2027", or "Not out yet".
pub fn coming_label(aired: &str, today: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = aired.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return "Not out yet".into();
    };
    let (Some(month), Ok(day)) = (
        month
            .parse::<usize>()
            .ok()
            .and_then(|m| MONTHS.get(m.wrapping_sub(1))),
        day.parse::<u32>(),
    ) else {
        return "Not out yet".into();
    };
    if today.starts_with(year) {
        format!("Out {day} {month}")
    } else {
        format!("Out {day} {month} {year}")
    }
}

/// How fast a held Left or Right scrubs, in seconds of video per second held: it speeds up
/// the longer the button is down, as Apple TV's does.
pub fn scrub_speed(held_ms: f64) -> f64 {
    match held_ms {
        ms if ms < 1_000. => 30.,
        ms if ms < 2_000. => 60.,
        ms if ms < 3_500. => 120.,
        ms if ms < 5_000. => 240.,
        _ => 480.,
    }
}

/// Milliseconds since the epoch from an RFC 3339 UTC time such as `2026-10-27T09:12:00.000Z`.
/// Offsets other than `Z` are applied; anything else is None.
pub fn parse_rfc3339_ms(text: &str) -> Option<f64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', 't', ' '])?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (clock_text, offset_minutes) = if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
        (clock, 0i64)
    } else {
        let at = rest.rfind(['+', '-'])?;
        let (clock, offset) = rest.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (oh, om) = offset[1..].split_once(':')?;
        (
            clock,
            sign * (oh.parse::<i64>().ok()? * 60 + om.parse::<i64>().ok()?),
        )
    };
    let mut clock_parts = clock_text.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    let second: f64 = clock_parts.next().unwrap_or("0").parse().ok()?;
    if hour > 23 || minute > 59 || !(0. ..61.).contains(&second) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 - offset_minutes * 60;
    Some(seconds as f64 * 1000. + second * 1000.)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_episodes_not_out_yet() {
        assert!(released("2026-10-01", "2026-10-28"));
        assert!(released("2026-10-28", "2026-10-28"));
        assert!(!released("2026-11-13", "2026-10-28"));
        assert!(!released("", "2026-10-28"));
        assert_eq!(coming_label("2026-11-13", "2026-10-28"), "Out 13 Nov");
        assert_eq!(coming_label("2027-01-05", "2026-10-28"), "Out 5 Jan 2027");
        assert_eq!(coming_label("", "2026-10-28"), "Not out yet");
    }

    #[test]
    fn scrubbing_speeds_up_while_held() {
        assert_eq!(scrub_speed(0.), 30.);
        assert!(scrub_speed(1_500.) > scrub_speed(500.));
        assert!(scrub_speed(4_000.) > scrub_speed(2_500.));
        assert_eq!(scrub_speed(60_000.), 480.);
    }

    #[test]
    fn formats_clocks() {
        assert_eq!(clock(4352.9), "1:12:32");
        assert_eq!(clock(245.), "4:05");
        assert_eq!(clock(-3.), "0:00");
        assert_eq!(clock(f64::NAN), "0:00");
        assert_eq!(short(4320.), "1h 12m");
        assert_eq!(short(3600.), "1h");
        assert_eq!(short(20.), "1m");
    }

    #[test]
    fn formats_wall_clocks() {
        assert_eq!(wall_clock(22, 32), "10:32 pm");
        assert_eq!(wall_clock(0, 5), "12:05 am");
        assert_eq!(wall_clock(12, 0), "12:00 pm");
        assert_eq!(wall_clock(9, 7), "9:07 am");
    }

    #[test]
    fn parses_trakt_times() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00.000Z"), Some(0.));
        assert_eq!(
            parse_rfc3339_ms("2000-03-01T00:00:01Z"),
            Some(951_868_801_000.)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-10-27T20:12:00+11:00"),
            parse_rfc3339_ms("2026-10-27T09:12:00.000Z")
        );
        assert!(parse_rfc3339_ms("2026-13-01T00:00:00Z").is_none());
        assert!(parse_rfc3339_ms("yesterday").is_none());
    }
}
