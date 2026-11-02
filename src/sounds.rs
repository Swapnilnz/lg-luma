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
    };
    let gain = f32::from(volume.min(100)) / 100. * FULL_GAIN;
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
