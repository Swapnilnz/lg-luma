//! Photos from the PC: the console agent's `GET /photo` hands out random photos from a folder
//! there (already upright and no larger than 1920x1080). They become the screensaver's slides
//! and Home's background while the PC answers; nothing is stored on the TV, and when the PC
//! sleeps everything goes back to the usual screensaver and wallpaper.

use crate::app_artwork::Artwork;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Home's background changes this often (the screensaver uses its own slide length).
pub const HOME_SLIDE: Duration = Duration::from_secs(45);
/// The crossfade from one photo to the next.
pub const FADE: Duration = Duration::from_millis(1400);
/// After this many failed requests in a row, photos stop until the PC answers again or
/// `RETRY` has passed.
const FAILURES: u8 = 2;
const RETRY: Duration = Duration::from_secs(5 * 60);
/// A portrait photo's blurred fill: small, so drawn full screen it is soft for free.
pub const FILL: (u32, u32) = (192, 108);

/// One photo, and for a portrait one, the blurred copy of itself that fills the screen.
pub struct Photo {
    pub image: Arc<Artwork>,
    pub fill: Option<Arc<Artwork>>,
}

impl Photo {
    /// Decodes a photo; `id` keeps its textures apart from every other photo's.
    pub fn decode(bytes: &[u8], id: u64) -> Option<Self> {
        let probe = crate::feeds_service::decode_as(String::new(), bytes, 1920, 1080)?;
        let portrait = probe.pixels.height() > probe.pixels.width();
        // A landscape photo fills the screen (cropped); a portrait one is shown whole.
        let key = if portrait {
            format!("photo:{id}")
        } else {
            format!("backdrop:photo:{id}")
        };
        let fill = if portrait {
            Some(Arc::new(fill(&probe.pixels, id)))
        } else {
            None
        };
        let mut image = probe;
        image.key = key;
        Some(Self {
            image: Arc::new(image),
            fill,
        })
    }
}

/// The photo cropped to the screen's shape, shrunk, blurred and darkened.
fn fill(pixels: &image::RgbaImage, id: u64) -> Artwork {
    let (width, height) = (pixels.width().max(1), pixels.height().max(1));
    let (fw, fh) = FILL;
    // The largest 16:9 crop around the middle.
    let crop_h = (width * fh / fw).min(height).max(1);
    let crop_w = (crop_h * fw / fh).min(width).max(1);
    let x = (width - crop_w) / 2;
    let y = (height - crop_h) / 2;
    let cropped = image::imageops::crop_imm(pixels, x, y, crop_w, crop_h).to_image();
    let small = image::imageops::resize(&cropped, fw, fh, image::imageops::FilterType::Triangle);
    let mut soft = image::imageops::blur(&small, 3.);
    for pixel in soft.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            *channel = (f32::from(*channel) * 0.65) as u8;
        }
        pixel.0[3] = 255;
    }
    Artwork {
        key: format!("backdrop:photo-fill:{id}"),
        pixels: soft,
        color: [20, 20, 20],
    }
}

/// The photos in memory: the one showing, the one fading out, and the next one, ready.
#[derive(Default)]
pub struct Photos {
    /// The photo showing and when it came in.
    pub current: Option<(Arc<Photo>, Instant)>,
    /// The photo before, while the current one fades in over it.
    pub previous: Option<Arc<Photo>>,
    pub next: Option<Arc<Photo>>,
    /// A request is on its way.
    pub pending: bool,
    failures: u8,
    failed_at: Option<Instant>,
    count: u64,
}

impl Photos {
    /// Whether photos can be asked for: the last requests did not all fail.
    pub fn working(&self) -> bool {
        self.failures < FAILURES || self.failed_at.is_some_and(|at| at.elapsed() >= RETRY)
    }

    /// Whether there is a photo to show (or one ready to be shown).
    pub fn ready(&self) -> bool {
        self.working() && (self.current.is_some() || self.next.is_some())
    }

    /// Whether another photo is wanted now.
    pub fn wanted(&self) -> bool {
        self.next.is_none() && !self.pending && self.working()
    }

    /// The id for the next request's textures.
    pub fn request(&mut self) -> u64 {
        self.pending = true;
        self.count += 1;
        self.count
    }

    /// A photo arrived, or (None) the PC did not give one.
    pub fn received(&mut self, photo: Option<Photo>) {
        self.pending = false;
        match photo {
            Some(photo) => {
                self.failures = 0;
                self.failed_at = None;
                self.next = Some(Arc::new(photo));
            }
            None => {
                self.failures = self.failures.saturating_add(1);
                if self.failures >= FAILURES {
                    self.failed_at = Some(Instant::now());
                }
            }
        }
    }

    /// Shows the next photo when the current one has had `slide` (or there is none yet).
    /// True when something changed.
    pub fn tick(&mut self, slide: Duration, now: Instant) -> bool {
        let mut changed = false;
        let faded = self
            .current
            .as_ref()
            .is_some_and(|(_, since)| now.duration_since(*since) >= FADE);
        if faded && self.previous.is_some() {
            self.previous = None;
            changed = true;
        }
        let due = self
            .current
            .as_ref()
            .is_none_or(|(_, since)| now.duration_since(*since) >= slide);
        if due && self.next.is_some() {
            self.advance(now);
            changed = true;
        }
        changed
    }

    /// The screensaver starts: the next photo if one is ready, else the current one again,
    /// fading in from black.
    pub fn begin(&mut self, now: Instant) {
        if self.next.is_some() {
            self.advance(now);
        } else if let Some((photo, _)) = self.current.take() {
            self.current = Some((photo, now));
            self.previous = None;
        }
    }

    /// The next photo comes in now, fading in over the current one.
    pub fn advance(&mut self, now: Instant) {
        if let Some(next) = self.next.take() {
            self.previous = self.current.take().map(|(photo, _)| photo);
            self.current = Some((next, now));
        }
    }

    /// The current photo's fade-in, 0 to 1.
    pub fn fade(&self, now: Instant) -> f32 {
        self.current.as_ref().map_or(0., |(_, since)| {
            (now.duration_since(*since).as_secs_f32() / FADE.as_secs_f32()).min(1.)
        })
    }

    pub fn fading(&self, now: Instant) -> bool {
        self.current
            .as_ref()
            .is_some_and(|(_, since)| now.duration_since(*since) < FADE + Duration::from_millis(40))
    }

    /// The PC answered again: try again at once.
    pub fn reset_failures(&mut self) {
        self.failures = 0;
        self.failed_at = None;
    }
}

/// The desktop preview's stand-in for the PC: a random image from `LUMA_PHOTOS_DIR`.
pub fn local_photo() -> Option<Vec<u8>> {
    let dir = std::env::var_os("LUMA_PHOTOS_DIR")?;
    let files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                matches!(
                    e.to_ascii_lowercase().as_str(),
                    "jpg" | "jpeg" | "png" | "webp"
                )
            })
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as usize);
    let path = &files[seed.wrapping_mul(2_654_435_761) % files.len()];
    let bytes = std::fs::read(path).ok()?;
    // Big camera files: shrink here as the PC would.
    let image = image::load_from_memory(&bytes).ok()?;
    let image = image.thumbnail(1920, 1080);
    let mut out = Vec::new();
    image
        .into_rgb8()
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .ok()?;
    Some(out)
}

impl crate::state::State {
    /// Whether the PC is answering and photos are coming: the console is set up, its agent
    /// answered the last probe, and the last photo requests did not all fail.
    pub fn photos_live(&self) -> bool {
        self.preferences.console.is_some()
            && self.console_awake == Some(true)
            && self.photos.working()
    }

    /// Whether Home's background is a photo now.
    pub fn home_photo(&self) -> bool {
        self.page == crate::state::Page::Home
            && self.preferences.photo_background
            && self.photos_live()
            && self.photos.current.is_some()
    }

    /// Whether a photo is on screen: Home's background or the screensaver's.
    pub fn photos_showing(&self) -> bool {
        self.home_photo()
            || self
                .feeds
                .ambient
                .as_ref()
                .is_some_and(|ambient| ambient.photos)
    }

    /// Whether to ask the PC for another photo now.
    pub fn photo_wanted(&self) -> bool {
        self.photos_live()
            && self.photos.wanted()
            && ((self.preferences.photo_background && self.page == crate::state::Page::Home)
                || (self.preferences.photo_screensaver && self.preferences.feed_ambient > 0)
                || self
                    .feeds
                    .ambient
                    .as_ref()
                    .is_some_and(|ambient| ambient.photos))
    }

    /// Home's slideshow: the next photo every `HOME_SLIDE` while Home shows it. (The
    /// screensaver turns its own slides, in `ambient_tick`.)
    pub fn photos_tick(&mut self, visible: bool) {
        if !visible || self.feeds.ambient.is_some() || self.overlay.is_some() || self.recents_open()
        {
            return;
        }
        if self.page == crate::state::Page::Home
            && self.preferences.photo_background
            && self.photos_live()
            && self.photos.tick(HOME_SLIDE, Instant::now())
        {
            self.dirty = true;
        }
    }

    /// A reply to `Request::Photo`.
    pub fn photo_received(&mut self, photo: Option<Photo>) {
        if photo.is_none() {
            eprintln!("Luma photos: the PC gave no photo");
        }
        self.photos.received(photo);
        self.dirty = true;
    }
}
