//! Navigation sounds, Apple TV style: a soft tick as focus moves, a two-note chime on OK, a
//! falling note on Back. They are made here (no sound files), quiet, and a few milliseconds
//! long. One SDL audio queue plays them, opened at the first sound and closed after a few
//! seconds of quiet or as soon as Luma leaves the screen, so Luma never holds the TV's sound
//! output for long. Settings > Interface > Navigation sounds turns them off, and Sound volume
//! sets how loud they are.
use crate::state::Sound;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

/// The quietest step of Settings > Sound volume, in percent; the steps are 10 apart.
pub const VOLUME_MIN: u8 = 10;
/// The gain at 100%: about one and a half times the sounds' designed level, so the default
/// 70% plays them at that level.
const FULL_GAIN: f32 = 1.4;

/// The queue closes after this long without a sound.
const IDLE: Duration = Duration::from_secs(6);
/// After the audio output fails to open, it is tried again after this long.
const RETRY: Duration = Duration::from_secs(30);

pub struct Sounds {
    queue: Option<(sdl2::audio::AudioQueue<i16>, Instant)>,
    /// Each sound's samples, made once for the output's rate, channels and volume.
    made: Vec<(Sound, i32, u8, u8, Vec<i16>)>,
    failed: Option<Instant>,
}

impl Sounds {
    pub fn new() -> Self {
        Self {
            queue: None,
            made: Vec::new(),
            failed: None,
        }
    }

    /// Once a frame: plays the sound the last key asked for, and closes the queue when idle.
    pub fn tick(
        &mut self,
        sound: Option<Sound>,
        enabled: bool,
        volume: u8,
        audio: Option<&sdl2::AudioSubsystem>,
        visible: bool,
    ) {
        if !visible
            || !enabled
            || self
                .queue
                .as_ref()
                .is_some_and(|(_, last)| last.elapsed() >= IDLE)
        {
            self.queue = None;
        }
        let (Some(sound), true, true, Some(audio)) = (sound, enabled, visible, audio) else {
            return;
        };
        if self.queue.is_none() {
            if self.failed.is_some_and(|at| at.elapsed() < RETRY) {
                return;
            }
            let spec = sdl2::audio::AudioSpecDesired {
                freq: Some(48_000),
                channels: Some(2),
                // Small buffers: the tick lands with the key, not a moment after.
                samples: Some(512),
            };
            match audio.open_queue::<i16, _>(None, &spec) {
                Ok(queue) => {
                    self.failed = None;
                    self.queue = Some((queue, Instant::now()));
                }
                Err(error) => {
                    eprintln!("Luma sounds: no audio output: {error}");
                    self.failed = Some(Instant::now());
                    return;
                }
            }
        }
        let Some((queue, last)) = self.queue.as_mut() else {
            return;
        };
        let (rate, channels) = (queue.spec().freq, queue.spec().channels);
        let volume = volume.clamp(VOLUME_MIN, 100);
        let index = match self.made.iter().position(|(made, r, c, v, _)| {
            *made == sound && *r == rate && *c == channels && *v == volume
        }) {
            Some(index) => index,
            None => {
                // Only the sounds at the current volume are kept.
                self.made.retain(|(_, _, _, v, _)| *v == volume);
                self.made.push((
                    sound,
                    rate,
                    channels,
                    volume,
                    make(sound, rate, channels, volume),
                ));
                self.made.len() - 1
            }
        };
        // A new sound cuts the last one short, so holding a key never queues a backlog.
        queue.clear();
        if let Err(error) = queue.queue_audio(&self.made[index].4) {
            eprintln!("Luma sounds: {error}");
            return;
        }
        queue.resume();
        *last = Instant::now();
    }
}

/// One note: a sine gliding from `from` to `to` Hz over `glide` seconds, with a touch of its
/// octave, starting `at` seconds in, rising over 2 ms and dying away with time constant
/// `decay`.
struct Note {
    at: f32,
    from: f32,
    to: f32,
    glide: f32,
    decay: f32,
    level: f32,
}

fn make(sound: Sound, rate: i32, channels: u8, volume: u8) -> Vec<i16> {
    let gain = f32::from(volume.min(100)) / 100. * FULL_GAIN;
    if let Some((left, right)) = cue(sound, rate.max(8000) as f32) {
        return interleave(&left, &right, channels, gain);
    }
    // Pitched low and kept soft: a TV's speakers or a soundbar make high, bright tones shrill.
    let notes: &[Note] = match sound {
        // A soft, low tick.
        Sound::Move => &[Note {
            at: 0.,
            from: 880.,
            to: 820.,
            glide: 0.012,
            decay: 0.009,
            level: 0.15,
        }],
        // Two rising notes, a fifth apart (G4 to D5).
        Sound::Select => &[
            Note {
                at: 0.,
                from: 392.,
                to: 392.,
                glide: 0.,
                decay: 0.05,
                level: 0.16,
            },
            Note {
                at: 0.045,
                from: 587.,
                to: 587.,
                glide: 0.,
                decay: 0.075,
                level: 0.14,
            },
        ],
        // A short falling note.
        Sound::Back => &[Note {
            at: 0.,
            from: 440.,
            to: 300.,
            glide: 0.06,
            decay: 0.04,
            level: 0.15,
        }],
        Sound::Launch | Sound::Waiting | Sound::Ready | Sound::Whoosh => &[],
    };
    let rate = rate.max(8000) as f32;
    let length = notes
        .iter()
        .map(|note| note.at + note.decay * 6.)
        .fold(0., f32::max);
    let frames = (rate * length) as usize;
    let channels = usize::from(channels.max(1));
    let mut mix = vec![0f32; frames];
    for note in notes {
        let mut phase = 0f32;
        let start = (note.at * rate) as usize;
        for (i, sample) in mix.iter_mut().enumerate().skip(start) {
            let t = (i - start) as f32 / rate;
            let glide = if note.glide > 0. {
                (t / note.glide).min(1.)
            } else {
                1.
            };
            let hz = note.from + (note.to - note.from) * glide;
            phase = (phase + hz / rate) % 1.;
            let attack = (t / 0.002).min(1.);
            let envelope = attack * (-t / note.decay).exp();
            *sample += ((phase * TAU).sin() + 0.12 * (phase * 2. * TAU).sin())
                * envelope
                * note.level
                * gain;
        }
    }
    // The last few milliseconds fade out, so it never ends on a click.
    let tail = (rate * 0.004) as usize;
    let mut samples = Vec::with_capacity(frames * channels);
    for (i, value) in mix.iter().enumerate() {
        let fade = (frames - i).min(tail) as f32 / tail.max(1) as f32;
        let value = (value * fade).clamp(-1., 1.);
        let sample = (value * f32::from(i16::MAX)) as i16;
        for _ in 0..channels {
            samples.push(sample);
        }
    }
    samples
}

// ---- The console launch's cues ----
//
// Longer than the navigation sounds and in stereo: a swell as the card opens, a soft pulse
// while the PC wakes, a chime (and the whoosh into black) when it is ready.

/// A voice of a chord: `hz` reached from `hz * from` over `glide` seconds, its own envelope.
fn voice(
    out: &mut [f32],
    rate: f32,
    at: f32,
    hz: f32,
    from: f32,
    glide: f32,
    env: impl Fn(f32) -> f32,
) {
    let start = (at * rate) as usize;
    let mut phase = 0f32;
    for (i, sample) in out.iter_mut().enumerate().skip(start) {
        let t = (i - start) as f32 / rate;
        let g = if glide > 0. { (t / glide).min(1.) } else { 1. };
        // Eased glide: most of the rise early, settling into the note.
        let g = 1. - (1. - g).powi(3);
        let f = hz * (from + (1. - from) * g);
        phase = (phase + f / rate) % 1.;
        *sample += ((phase * TAU).sin() + 0.18 * (phase * 2. * TAU).sin()) * env(t);
    }
}

/// A bell-like note: quick attack, long ring.
fn bell(out: &mut [f32], rate: f32, at: f32, hz: f32, level: f32, decay: f32) {
    voice(out, rate, at, hz, 1., 0., |t| {
        (t / 0.003).min(1.) * (-t / decay).exp() * level
    });
}

/// Filtered noise sweeping down (and across, left to right).
fn whoosh(left: &mut [f32], right: &mut [f32], rate: f32, at: f32, length: f32, level: f32) {
    let start = (at * rate) as usize;
    let mut seed = 0x1234_5678u32;
    let (mut low_l, mut low_r) = (0f32, 0f32);
    for i in start..left.len().min(right.len()) {
        let t = (i - start) as f32 / rate;
        if t > length {
            break;
        }
        let p = t / length;
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 * 2. - 1.;
        // One-pole low-pass, its cutoff falling from about 3.5 kHz to 250 Hz.
        let cutoff = 3500. * (250f32 / 3500.).powf(p);
        let k = 1. - (-TAU * cutoff / rate).exp();
        low_l += k * (noise - low_l);
        low_r += k * (noise - low_r);
        let env = (t / 0.08).min(1.) * (1. - p).powf(1.6) * level;
        left[i] += low_l * env * (1. - 0.6 * p);
        right[i] += low_r * env * (0.4 + 0.6 * p);
    }
}

fn cue(sound: Sound, rate: f32) -> Option<(Vec<f32>, Vec<f32>)> {
    let length = match sound {
        Sound::Launch => 1.25,
        Sound::Waiting => 0.45,
        Sound::Ready => 1.5,
        Sound::Whoosh => 0.6,
        Sound::Move | Sound::Select | Sound::Back => return None,
    };
    let frames = (rate * length) as usize;
    let mut left = vec![0f32; frames];
    let mut right = vec![0f32; frames];
    match sound {
        Sound::Launch => {
            // D major, rising a fourth into place, each side slightly detuned for width; a
            // slow swell in, a soft release out.
            let swell = |level: f32| {
                move |t: f32| {
                    let attack = (t / 0.75).min(1.).powi(2);
                    let release = ((1.2 - t) / 0.45).clamp(0., 1.);
                    attack * release * level
                }
            };
            for (hz, level) in [
                (146.83, 0.07),
                (220., 0.06),
                (293.66, 0.055),
                (369.99, 0.04),
            ] {
                voice(&mut left, rate, 0., hz * 0.997, 0.75, 0.9, swell(level));
                voice(&mut right, rate, 0., hz * 1.003, 0.75, 0.9, swell(level));
            }
            // A sub underneath and a high shimmer arriving late.
            for out in [&mut left, &mut right] {
                voice(out, rate, 0., 73.42, 0.8, 0.9, swell(0.06));
                voice(out, rate, 0.55, 587.33, 1., 0., |t| {
                    (t / 0.25).min(1.) * ((0.65 - t) / 0.3).clamp(0., 1.) * 0.025
                });
            }
        }
        Sound::Waiting => {
            // A soft double pulse, like a heartbeat.
            for out in [&mut left, &mut right] {
                for (at, level) in [(0., 0.16), (0.17, 0.1)] {
                    voice(out, rate, at, 82., 1.35, 0.06, move |t| {
                        (t / 0.004).min(1.) * (-t / 0.07).exp() * level
                    });
                }
            }
        }
        Sound::Ready => {
            // An A major arpeggio rising, with a high shimmer, then the whoosh into black as
            // the screen fades (READY_HOLD later).
            for (index, hz) in [440., 554.37, 659.26, 880.].into_iter().enumerate() {
                let at = index as f32 * 0.07;
                bell(&mut left, rate, at, hz * 0.998, 0.11, 0.32);
                bell(&mut right, rate, at, hz * 1.002, 0.11, 0.32);
            }
            bell(&mut left, rate, 0.28, 1760., 0.025, 0.5);
            bell(&mut right, rate, 0.3, 2217.46, 0.02, 0.5);
            let hold = crate::console::READY_HOLD.as_secs_f32();
            whoosh(&mut left, &mut right, rate, hold - 0.05, 0.6, 0.28);
        }
        Sound::Whoosh => whoosh(&mut left, &mut right, rate, 0., 0.55, 0.28),
        Sound::Move | Sound::Select | Sound::Back => {}
    }
    Some((left, right))
}

/// Stereo floats to the output's samples, at the volume, with a click-free tail.
fn interleave(left: &[f32], right: &[f32], channels: u8, gain: f32) -> Vec<i16> {
    let channels = usize::from(channels.max(1));
    let frames = left.len().min(right.len());
    let tail = (frames / 50).max(1);
    let mut samples = Vec::with_capacity(frames * channels);
    for i in 0..frames {
        let fade = (frames - i).min(tail) as f32 / tail as f32;
        let sample =
            |value: f32| ((value * gain * fade).clamp(-1., 1.) * f32::from(i16::MAX)) as i16;
        let (l, r) = (sample(left[i]), sample(right[i]));
        let mono = ((i32::from(l) + i32::from(r)) / 2) as i16;
        if channels == 1 {
            samples.push(mono);
        } else {
            samples.push(l);
            samples.push(r);
            for _ in 2..channels {
                samples.push(mono);
            }
        }
    }
    samples
}

#[cfg(test)]
mod cue_tests {
    use super::*;

    #[test]
    fn launch_cues_are_stereo_bounded_and_end_quietly() {
        for sound in [Sound::Launch, Sound::Waiting, Sound::Ready, Sound::Whoosh] {
            let samples = make(sound, 48_000, 2, 70);
            assert!(samples.len() > 48_000 / 4, "{sound:?} is too short");
            assert_eq!(samples.len() % 2, 0);
            let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
            assert!(peak > 500, "{sound:?} is silent");
            assert!(peak < i16::MAX as u16, "{sound:?} clips");
            assert!(
                samples[samples.len() - 2].unsigned_abs() < 200,
                "{sound:?} ends on a click"
            );
        }
    }
}
