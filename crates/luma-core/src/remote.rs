//! Magic Remote keycodes (and the laptop keys that stand in for them) to player keys.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Ok,
    Back,
    Up,
    Down,
    Left,
    Right,
    PlayPause,
    Play,
    Pause,
    Stop,
    FastForward,
    Rewind,
    Red,
    Green,
    Yellow,
    Blue,
}

/// A `keydown` event's `keyCode`. On a laptop: Enter = OK, Backspace or Escape = Back,
/// Space = Play/Pause, `[` and `]` = rewind and fast-forward, 1 to 4 = the colour keys.
pub fn key(code: u32) -> Option<Key> {
    Some(match code {
        13 => Key::Ok,
        461 | 8 | 27 => Key::Back,
        37 => Key::Left,
        38 => Key::Up,
        39 => Key::Right,
        40 => Key::Down,
        415 => Key::Play,
        19 => Key::Pause,
        32 | 179 => Key::PlayPause,
        413 | 178 => Key::Stop,
        417 | 228 | 221 => Key::FastForward,
        412 | 227 | 219 => Key::Rewind,
        403 | 49 => Key::Red,
        404 | 50 => Key::Green,
        405 | 51 => Key::Yellow,
        406 | 52 => Key::Blue,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_remote_and_the_keyboard() {
        assert_eq!(key(461), Some(Key::Back));
        assert_eq!(key(8), Some(Key::Back));
        assert_eq!(key(415), Some(Key::Play));
        assert_eq!(key(19), Some(Key::Pause));
        assert_eq!(key(32), Some(Key::PlayPause));
        assert_eq!(key(417), Some(Key::FastForward));
        assert_eq!(key(219), Some(Key::Rewind));
        assert_eq!(key(404), Some(Key::Green));
        assert_eq!(key(52), Some(Key::Blue));
        assert_eq!(key(65), None);
    }
}
