//! Where to resume: Trakt's playback record versus the player's own, whichever is newer.

/// A saved place in a title.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Seconds in, when known (local records). Trakt only keeps a percentage.
    pub position: Option<f64>,
    /// Percent watched, 0 to 100.
    pub percent: f64,
    /// When it was saved, in ms since the epoch.
    pub at_ms: f64,
}

/// Resume points under two minutes in are not worth asking about.
pub const MIN_SECONDS: f64 = 120.;
/// Past this, the title counts as finished and starts over.
pub const MAX_PERCENT: f64 = 95.;

/// The newer of the two records.
pub fn newer(trakt: Option<Point>, local: Option<Point>) -> Option<Point> {
    match (trakt, local) {
        (Some(t), Some(l)) => Some(if l.at_ms >= t.at_ms { l } else { t }),
        (t, l) => t.or(l),
    }
}

/// Where "Resume" would start, in seconds when that can be known now, given the title's
/// running time if known (TMDB's runtime before the stream loads, the real duration after).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offer {
    pub percent: f64,
    pub seconds: Option<f64>,
}

impl Offer {
    /// The seek target once the real duration is known.
    pub fn seconds_in(&self, duration: f64) -> f64 {
        if duration.is_finite() && duration > 0. {
            (self.percent / 100. * duration).clamp(0., duration)
        } else {
            self.seconds.unwrap_or(0.)
        }
    }
}

/// Some when the point is past two minutes and before 95%; otherwise start from the top.
pub fn offer(point: Option<Point>, duration: Option<f64>) -> Option<Offer> {
    let point = point?;
    let duration = duration.filter(|d| d.is_finite() && *d > 0.);
    let percent = match (point.position, duration) {
        (Some(position), Some(duration)) => position / duration * 100.,
        _ => point.percent,
    };
    let seconds = point
        .position
        .or_else(|| duration.map(|duration| percent / 100. * duration));
    if !(percent.is_finite() && percent < MAX_PERCENT && percent > 0.) {
        return None;
    }
    match seconds {
        Some(seconds) if seconds < MIN_SECONDS => None,
        // Without any duration, anything past 2% is worth offering.
        None if percent < 2. => None,
        _ => Some(Offer { percent, seconds }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(position: Option<f64>, percent: f64, at_ms: f64) -> Point {
        Point {
            position,
            percent,
            at_ms,
        }
    }

    #[test]
    fn newer_record_wins() {
        let trakt = point(None, 40., 1_000.);
        let local = point(Some(600.), 10., 2_000.);
        assert_eq!(newer(Some(trakt), Some(local)), Some(local));
        assert_eq!(newer(Some(trakt), Some(point(None, 1., 500.))), Some(trakt));
        assert_eq!(newer(None, Some(local)), Some(local));
        assert_eq!(newer(None, None), None);
    }

    #[test]
    fn offers_between_two_minutes_and_95_percent() {
        // Local: exact seconds.
        let o = offer(Some(point(Some(4350.), 0., 0.)), Some(7200.)).unwrap();
        assert_eq!(o.seconds, Some(4350.));
        assert!(offer(Some(point(Some(90.), 0., 0.)), Some(7200.)).is_none());
        assert!(offer(Some(point(Some(7000.), 0., 0.)), Some(7200.)).is_none());
        // Trakt: a percentage, seconds from the runtime.
        let o = offer(Some(point(None, 50., 0.)), Some(3600.)).unwrap();
        assert_eq!(o.seconds, Some(1800.));
        assert_eq!(o.seconds_in(4000.), 2000.);
        assert!(offer(Some(point(None, 1., 0.)), Some(3600.)).is_none());
        assert!(offer(Some(point(None, 96., 0.)), Some(3600.)).is_none());
        // No duration at all: by percentage only.
        let o = offer(Some(point(None, 30., 0.)), None).unwrap();
        assert_eq!(o.seconds, None);
        assert!(offer(Some(point(None, 1., 0.)), None).is_none());
        assert!(offer(None, Some(3600.)).is_none());
    }
}
