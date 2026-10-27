//! Trakt scrobbling as a state machine: player events in, Trakt calls out.
//!
//! - First frame, or playing again after a pause or seek: `start`.
//! - Pause: `pause` (Trakt keeps the resume point).
//! - Exit before 80%: `pause`; exit at 80% or later, or the end: `stop` (marks it watched).
//! - At most one call every 10 s; a later state replaces one still waiting. The end and
//!   exit skip the wait, since the app may be gone a moment later.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Pause,
    Stop,
}

impl Action {
    pub fn path(self) -> &'static str {
        match self {
            Action::Start => "start",
            Action::Pause => "pause",
            Action::Stop => "stop",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Call {
    pub action: Action,
    /// Percent watched, 0 to 100.
    pub progress: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Frames are moving: the first frame, or after a pause, seek or stall.
    Playing,
    Paused,
    /// Playback reached the end.
    Ended,
    /// The viewer left: Back, Stop, Home, or a switch to another title.
    Exit,
}

pub const THROTTLE_MS: f64 = 10_000.;
/// From here on, leaving counts as finished.
pub const WATCHED_PERCENT: f64 = 80.;

#[derive(Clone, Debug, Default)]
pub struct Scrobbler {
    last_sent_ms: Option<f64>,
    last: Option<Call>,
    pending: Option<Action>,
    finished: bool,
}

impl Scrobbler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `stop` has gone out: this title is done.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The call to send now for this event, if any.
    pub fn event(&mut self, event: Event, progress: f64, now_ms: f64) -> Option<Call> {
        if self.finished {
            return None;
        }
        let progress = clamp(progress);
        match event {
            Event::Ended => self.send(Action::Stop, progress.max(99.9), now_ms),
            Event::Exit => {
                let action = if progress >= WATCHED_PERCENT {
                    Action::Stop
                } else {
                    Action::Pause
                };
                // Nothing went out yet and nothing is owed: a title left at once leaves no trace.
                if self.last.is_none() && self.pending.is_none() && progress < 1. {
                    return None;
                }
                self.send(action, progress, now_ms)
            }
            Event::Playing | Event::Paused => {
                let action = if event == Event::Playing {
                    Action::Start
                } else {
                    Action::Pause
                };
                // A pause after a pause, or a start barely moved from the last, adds nothing.
                if self.pending.is_none()
                    && let Some(last) = self.last
                    && last.action == action
                    && (action == Action::Pause || (progress - last.progress).abs() < 0.5)
                {
                    return None;
                }
                if self
                    .last_sent_ms
                    .is_some_and(|sent| now_ms - sent < THROTTLE_MS)
                {
                    self.pending = Some(action);
                    return None;
                }
                self.send(action, progress, now_ms)
            }
        }
    }

    /// Call often (every second): sends a call held back by the throttle once it may go.
    pub fn tick(&mut self, progress: f64, now_ms: f64) -> Option<Call> {
        if self.finished {
            return None;
        }
        let action = self.pending?;
        if self
            .last_sent_ms
            .is_some_and(|sent| now_ms - sent < THROTTLE_MS)
        {
            return None;
        }
        if self
            .last
            .is_some_and(|last| last.action == action && action == Action::Pause)
        {
            self.pending = None;
            return None;
        }
        self.send(action, clamp(progress), now_ms)
    }

    fn send(&mut self, action: Action, progress: f64, now_ms: f64) -> Option<Call> {
        let call = Call { action, progress };
        self.pending = None;
        self.last = Some(call);
        self.last_sent_ms = Some(now_ms);
        if action == Action::Stop {
            self.finished = true;
        }
        Some(call)
    }
}

fn clamp(progress: f64) -> f64 {
    if progress.is_finite() {
        (progress * 100.).round().clamp(0., 10_000.) / 100.
    } else {
        0.
    }
}

/// Percent watched from a position and duration (0 when the duration is unknown).
pub fn percent(position: f64, duration: f64) -> f64 {
    if duration.is_finite() && duration > 0. && position.is_finite() {
        (position / duration * 100.).clamp(0., 100.)
    } else {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(call: Option<Call>) -> Option<Action> {
        call.map(|call| call.action)
    }

    #[test]
    fn start_pause_resume_and_finish() {
        let mut s = Scrobbler::new();
        assert_eq!(action(s.event(Event::Playing, 0., 0.)), Some(Action::Start));
        assert_eq!(
            action(s.event(Event::Paused, 20., 60_000.)),
            Some(Action::Pause)
        );
        assert_eq!(action(s.event(Event::Paused, 20., 70_000.)), None);
        assert_eq!(
            action(s.event(Event::Playing, 20., 80_000.)),
            Some(Action::Start)
        );
        let end = s.event(Event::Ended, 98., 90_000.).unwrap();
        assert_eq!(end.action, Action::Stop);
        assert!(end.progress >= 99.9);
        assert!(s.finished());
        assert_eq!(s.event(Event::Exit, 100., 91_000.), None);
    }

    #[test]
    fn exit_at_79_pauses_and_at_81_stops() {
        let mut s = Scrobbler::new();
        s.event(Event::Playing, 10., 0.);
        assert_eq!(
            action(s.event(Event::Exit, 79., 1_000.)),
            Some(Action::Pause)
        );
        let mut s = Scrobbler::new();
        s.event(Event::Playing, 10., 0.);
        assert_eq!(
            action(s.event(Event::Exit, 81., 1_000.)),
            Some(Action::Stop)
        );
    }

    #[test]
    fn throttles_to_one_call_every_ten_seconds() {
        let mut s = Scrobbler::new();
        s.event(Event::Playing, 5., 0.);
        // Pause then play within the window: only the last state is owed.
        assert_eq!(s.event(Event::Paused, 6., 2_000.), None);
        assert_eq!(s.event(Event::Playing, 30., 3_000.), None);
        assert_eq!(s.tick(30., 5_000.), None);
        let call = s.tick(31., 10_000.).unwrap();
        assert_eq!(call.action, Action::Start);
        assert_eq!(call.progress, 31.);
        assert_eq!(s.tick(31., 30_000.), None);
    }

    #[test]
    fn a_waiting_pause_is_dropped_if_already_paused() {
        let mut s = Scrobbler::new();
        s.event(Event::Playing, 5., 0.);
        s.event(Event::Paused, 5., 11_000.);
        assert_eq!(s.event(Event::Playing, 5.2, 12_000.), None);
        assert_eq!(s.event(Event::Paused, 5.2, 13_000.), None);
        // The owed pause matches the last sent: nothing to send.
        assert_eq!(s.tick(5.2, 30_000.), None);
    }

    #[test]
    fn exit_skips_the_throttle() {
        let mut s = Scrobbler::new();
        s.event(Event::Playing, 50., 0.);
        assert_eq!(
            action(s.event(Event::Exit, 50., 1_000.)),
            Some(Action::Pause)
        );
    }

    #[test]
    fn leaving_before_anything_played_sends_nothing() {
        let mut s = Scrobbler::new();
        assert_eq!(s.event(Event::Exit, 0., 0.), None);
    }

    #[test]
    fn percent_handles_unknown_duration() {
        assert_eq!(percent(30., 60.), 50.);
        assert_eq!(percent(30., f64::NAN), 0.);
        assert_eq!(percent(90., 60.), 100.);
    }
}
