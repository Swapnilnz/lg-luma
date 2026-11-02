//! Home themes: one Settings row restyles Home, its Top Shelf, feed rows, Search, title pages
//! and screensaver. A theme is data: `view.rs` asks it how to style a card, never which theme
//! is active, apart from three layouts that change a row's card shapes (Bento, Video shelf,
//! Letterbox). "Classic" is Luma as it always looked and leaves Home untouched.
//!
//! The visual spec is the Luma Restyle Previews page (1280 × 720); its sizes are scaled by 1.5
//! here. Everything is drawn with the renderer's shapes and `Draw::Quad` (per-corner radii,
//! rotation, perspective, image filters, procedural patterns and blending), so no theme adds
//! textures: poster filters run in the shader, with no second copy of any poster in memory.
use crate::renderer::{Blend, Color, Draw, Filter, Pattern, Quad, QuadImage, TextStyle};
use crate::state::Rect;
use std::sync::Arc;

/// Theme font files, `assets/fonts/themes/theme-<stem>-<400|700>.ttf`. A `Face`'s family is
/// its index here plus one (0 is Inter).
pub const FONT_FAMILIES: [&str; 35] = [
    "figtree",
    "archivo-black",
    "jetbrains-mono",
    "manrope",
    "archivo",
    "oswald",
    "fraunces",
    "federo",
    "cormorant-garamond",
    "anton",
    "barlow",
    "league-spartan",
    "bangers",
    "righteous",
    "rubik",
    "roboto",
    "cinzel",
    "fredoka",
    "nunito",
    "ibm-plex-mono",
    "vt323",
    "limelight",
    "poiret-one",
    "dm-serif-display",
    "syne",
    "barlow-condensed",
    "monoton",
    "bodoni-moda",
    "special-elite",
    "share-tech-mono",
    "unbounded",
    "courier-prime",
    "press-start-2p",
    "audiowide",
    "orbitron",
];

// Family numbers, in FONT_FAMILIES order.
const INTER: u8 = 0;
const FIGTREE: u8 = 1;
const ARCHIVO_BLACK: u8 = 2;
const JETBRAINS_MONO: u8 = 3;
const MANROPE: u8 = 4;
const ARCHIVO: u8 = 5;
const OSWALD: u8 = 6;
const FRAUNCES: u8 = 7;
const FEDERO: u8 = 8;
const CORMORANT: u8 = 9;
const ANTON: u8 = 10;
const BARLOW: u8 = 11;
const LEAGUE_SPARTAN: u8 = 12;
const BANGERS: u8 = 13;
const RIGHTEOUS: u8 = 14;
const RUBIK: u8 = 15;
const ROBOTO: u8 = 16;
const CINZEL: u8 = 17;
const FREDOKA: u8 = 18;
const NUNITO: u8 = 19;
const PLEX_MONO: u8 = 20;
const VT323: u8 = 21;
const LIMELIGHT: u8 = 22;
const POIRET_ONE: u8 = 23;
const DM_SERIF: u8 = 24;
const SYNE: u8 = 25;
const BARLOW_CONDENSED: u8 = 26;
const MONOTON: u8 = 27;
const BODONI: u8 = 28;
const SPECIAL_ELITE: u8 = 29;
const SHARE_TECH_MONO: u8 = 30;
const UNBOUNDED: u8 = 31;
const COURIER_PRIME: u8 = 32;
const PRESS_START: u8 = 33;
const AUDIOWIDE: u8 = 34;
const ORBITRON: u8 = 35;

/// How a theme lays out a feed row's cards.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Layout {
    /// Posters in a row, as Luma always drew them.
    #[default]
    Rows,
    /// Rows boxed as tiles, every fifth card a double-width tile.
    Bento,
    /// Narrow spines on a shelf; the focused title slides out face-first.
    Shelf,
    /// Wide 2.39:1 stills between black scope bars.
    Scope,
}

/// A typeface and the weight its text is set in (theme faces have a 400 and, mostly, a 700).
#[derive(Clone, Copy)]
pub struct Face {
    pub family: u8,
    pub weight: u16,
}

/// Behind or around a card.
#[derive(Clone, Copy)]
pub enum Shadow {
    /// Luma's soft drop shadow.
    Soft,
    /// A solid copy offset by x, y, in a colour (neubrutalism, Pop Art).
    Hard(f32, f32, u32),
    /// A coloured soft glow.
    Glow(u32, f32),
    None,
}

/// How focus shows on a card or control, besides its growth.
#[derive(Clone, Copy)]
pub enum Focus {
    /// A ring: colour, width, gap outside the card.
    Ring(u32, f32, f32),
    /// A ring with a glow of its colour.
    Glow(u32, f32),
    /// The card lifts off a hard shadow this far, ringed in the colour.
    Hard(u32, f32),
    /// A bar under the card (Swiss).
    Underline(u32),
}

/// What a title page's or Top Shelf's big title gets (when drawn in words).
#[derive(Clone, Copy)]
pub enum TitleFx {
    Plain,
    /// Outlined in a colour, like comic lettering.
    Outline(u32, f32),
    /// A solid copy offset by x, y.
    Offset(u32, f32, f32),
    /// A soft glow of a colour.
    Glow(u32),
    /// Red and cyan copies either side (chromatic aberration).
    Split(u32, u32),
    /// Pastel chrome: a pink drop and a cyan glow.
    Chrome,
}

/// Behind the Top Shelf's words.
#[derive(Clone, Copy)]
pub struct HeroPanel {
    pub fill: (u32, f32),
    pub border: (u32, f32),
    pub radius: [f32; 4],
    pub shadow: Shadow,
    /// Degrees; a taped card or a tilted headline.
    pub angle: f32,
    /// Turned away in perspective, like a floating visionOS panel.
    pub perspective: bool,
    /// A ring of marquee bulbs around it.
    pub bulbs: bool,
}

/// Shapes and patterns on a themed Home's background, or over it (`Theme::overlay`). Sizes in
/// logical pixels on the 1920 × 1080 screen.
#[derive(Clone, Copy)]
pub enum Decor {
    /// A soft radial glow centred at x, y, about w × h.
    Glow(f32, f32, f32, f32, u32, f32),
    /// A filled rectangle: rect, colour, alpha, radii, degrees.
    Fill(Rect, u32, f32, [f32; 4], f32),
    /// A vertical gradient over a rect.
    Gradient(Rect, u32, u32),
    /// A ring: centre x, y, radius, width, colour.
    Ring(f32, f32, f32, f32, u32),
    /// A pattern over a rect: pattern, period, ink width, colour, alpha, blend.
    Pattern(Rect, Pattern, f32, f32, u32, f32, Blend),
    /// A triangle: three points and a colour.
    Triangle([f32; 2], [f32; 2], [f32; 2], u32),
    /// A stroke through a cubic curve: four control points, width, colour, alpha.
    Curve([[f32; 2]; 4], f32, u32, f32),
    /// A zigzag from x, y: steps, step width, height, line width, colour.
    Zigzag(f32, f32, u32, f32, f32, f32, u32),
    /// A four-pointed sparkle: centre, size, colour.
    Sparkle(f32, f32, f32, u32),
}

/// A Home look. `light` themes have dark ink on light paper; the view's light-on-dark colours
/// are mapped onto `fg`, `panel` and `line` accordingly.
#[derive(Clone, Copy)]
pub struct Theme {
    pub key: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    /// Build phase in the plan (1 is the first release); shown in the picker.
    pub phase: u8,
    pub light: bool,
    /// Home's background in place of the wallpaper (top, bottom); None keeps the wallpaper.
    pub background: Option<(u32, u32)>,
    pub decor: &'static [Decor],
    /// Over everything on Home (grain, scanlines, scope bars).
    pub overlay: &'static [Decor],
    pub fg: u32,
    pub muted: u32,
    pub accent: u32,
    /// Surfaces: placeholders, panels, unfocused pills (colour and opacity).
    pub panel: (u32, f32),
    /// Focused surfaces and the ink on them.
    pub focus_fill: u32,
    pub focus_ink: u32,
    /// Outlines on surfaces (width 0 keeps Luma's hairlines) and their colour.
    pub outline: (f32, u32),
    /// A hard shadow under surfaces (x, y, colour), or none.
    pub surface_shadow: Option<(f32, f32, u32)>,
    /// Corners never rounder than this, on surfaces and pills.
    pub max_radius: f32,
    pub display: Face,
    pub body: Face,
    /// Labels and captions (mono or condensed).
    pub label: Face,
    /// Headings and labels in capitals.
    pub caps: bool,
    pub card_corners: [f32; 4],
    /// The card's frame: width and colour (a photo-print border, an outline).
    pub card_border: (f32, u32),
    pub card_shadow: Shadow,
    pub focus: Focus,
    /// Posters' filter while unfocused, and their brightness.
    pub poster: Filter,
    pub poster_light: f32,
    /// The focused poster's filter (colour comes back on focus in several looks).
    pub poster_focused: Filter,
    /// Large images: Top Shelf, previews, title pages, screensaver.
    pub backdrop: Filter,
    pub layout: Layout,
    /// Cards tilt a little each, by a seed from their title (collage, Pop Art).
    pub tilt: f32,
    pub hero_panel: Option<HeroPanel>,
    pub title_fx: TitleFx,
    /// Progress bars' fill.
    pub progress: u32,
}

const fn face(family: u8, weight: u16) -> Face {
    Face { family, weight }
}
const fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}
const SCREEN: Rect = Rect::new(0., 0., 1920., 1080.);
const NONE: &[Decor] = &[];

/// Luma as it always looked: nothing is restyled.
const CLASSIC: Theme = Theme {
    key: "classic",
    name: "Classic",
    group: "Luma",
    phase: 0,
    light: false,
    background: None,
    decor: NONE,
    overlay: NONE,
    fg: 0xF5F5F5,
    muted: 0xB8BCC2,
    accent: 0xFFFFFF,
    panel: (0x171C20, 0.72),
    focus_fill: 0xFAFAFA,
    focus_ink: 0x1D1D1F,
    outline: (0., 0xFFFFFF),
    surface_shadow: None,
    max_radius: 999.,
    display: face(INTER, 650),
    body: face(INTER, 400),
    label: face(INTER, 500),
    caps: false,
    card_corners: [14.4; 4],
    card_border: (1., 0xFFFFFF),
    card_shadow: Shadow::Soft,
    focus: Focus::Ring(0xF0F0F0, 3.2, 0.),
    poster: Filter::None,
    poster_light: 1.,
    poster_focused: Filter::None,
    backdrop: Filter::None,
    layout: Layout::Rows,
    tilt: 0.,
    hero_panel: None,
    title_fx: TitleFx::Plain,
    progress: 0xFFFFFF,
};

const fn panel(fill: (u32, f32), border: (u32, f32), radius: f32, shadow: Shadow) -> HeroPanel {
    HeroPanel {
        fill,
        border,
        radius: [radius; 4],
        shadow,
        angle: 0.,
        perspective: false,
        bulbs: false,
    }
}

const GLASS: Theme = Theme {
    key: "glass",
    name: "Liquid glass",
    group: "2026 trends",
    phase: 1,
    background: Some((0x0A1230, 0x070B1A)),
    decor: &[
        Decor::Glow(1498., 324., 1560., 1140., 0x40A0BE, 0.55),
        Decor::Glow(1824., 918., 1260., 960., 0xAA46C8, 0.45),
        Decor::Glow(768., 108., 1800., 1260., 0x2846A0, 0.6),
        Decor::Gradient(r(0., 0., 1100., 1080.), 0x070B1A, 0x070B1A),
    ],
    fg: 0xF4F6FF,
    muted: 0xBFC6DA,
    accent: 0xFFFFFF,
    panel: (0xFFFFFF, 0.14),
    focus_fill: 0xFFFFFF,
    focus_ink: 0x0A1230,
    outline: (1.5, 0xFFFFFF),
    display: face(FIGTREE, 700),
    body: face(FIGTREE, 400),
    label: face(FIGTREE, 700),
    card_corners: [21.; 4],
    card_border: (0., 0xFFFFFF),
    focus: Focus::Ring(0xFFFFFF, 4.5, 0.),
    backdrop: Filter::Blur(18.),
    hero_panel: None,
    ..CLASSIC
};

const BRUTAL: Theme = Theme {
    key: "brutal",
    name: "Neubrutalism",
    group: "2026 trends",
    phase: 1,
    light: true,
    background: Some((0xC8B6FF, 0xC8B6FF)),
    decor: &[Decor::Pattern(
        SCREEN,
        Pattern::Grid,
        60.,
        1.5,
        0x0B0B0B,
        0.08,
        Blend::Normal,
    )],
    fg: 0x0B0B0B,
    muted: 0x2E2A3A,
    accent: 0xFFD400,
    panel: (0xFFFFFF, 1.),
    focus_fill: 0xFFD400,
    focus_ink: 0x0B0B0B,
    outline: (4.5, 0x0B0B0B),
    surface_shadow: Some((6., 6., 0x0B0B0B)),
    max_radius: 0.,
    display: face(ARCHIVO_BLACK, 400),
    body: face(JETBRAINS_MONO, 400),
    label: face(JETBRAINS_MONO, 700),
    caps: true,
    card_corners: [0.; 4],
    card_border: (4.5, 0x0B0B0B),
    card_shadow: Shadow::Hard(7.5, 7.5, 0x0B0B0B),
    focus: Focus::Hard(0xFFD400, 6.),
    hero_panel: Some(panel(
        (0xFFFFFF, 1.),
        (0x0B0B0B, 4.5),
        0.,
        Shadow::Hard(12., 12., 0x0B0B0B),
    )),
    title_fx: TitleFx::Plain,
    progress: 0xFF5EA8,
    ..CLASSIC
};

const BENTO: Theme = Theme {
    key: "bento",
    name: "Bento",
    group: "2026 trends",
    phase: 1,
    background: Some((0x0D0F12, 0x0D0F12)),
    fg: 0xEEF0F3,
    muted: 0x9AA1AC,
    accent: 0xF5C451,
    panel: (0x181B20, 1.),
    focus_fill: 0xEEF0F3,
    focus_ink: 0x0D0F12,
    max_radius: 33.,
    display: face(MANROPE, 700),
    body: face(MANROPE, 400),
    label: face(MANROPE, 700),
    card_corners: [18.; 4],
    card_border: (0., 0xFFFFFF),
    focus: Focus::Ring(0xEEF0F3, 4.5, 0.),
    layout: Layout::Bento,
    progress: 0xF5C451,
    ..CLASSIC
};

const RETRO: Theme = Theme {
    key: "retro",
    name: "Retro grain",
    group: "2026 trends",
    phase: 3,
    background: Some((0x17120D, 0x17120D)),
    decor: &[
        Decor::Glow(1536., 432., 2100., 1440., 0xB45A28, 0.55),
        Decor::Glow(1152., 972., 1500., 1080., 0x6E281E, 0.6),
        Decor::Gradient(r(0., 0., 1150., 1080.), 0x17120D, 0x17120D),
    ],
    overlay: &[
        Decor::Pattern(
            SCREEN,
            Pattern::Grain,
            1.,
            0.18,
            0xFFF2D9,
            1.,
            Blend::Normal,
        ),
        Decor::Pattern(
            SCREEN,
            Pattern::Lines,
            4.5,
            1.5,
            0x000000,
            0.12,
            Blend::Normal,
        ),
    ],
    fg: 0xF1E6D2,
    muted: 0xCDBB9B,
    accent: 0xE8A33D,
    panel: (0x2A2018, 0.85),
    focus_fill: 0xE8A33D,
    focus_ink: 0x17120D,
    outline: (1.5, 0xF1E6D2),
    max_radius: 4.,
    display: face(FRAUNCES, 400),
    body: face(BARLOW, 400),
    label: face(PLEX_MONO, 400),
    caps: true,
    card_corners: [4.5; 4],
    card_border: (0., 0xFFFFFF),
    focus: Focus::Glow(0xE8A33D, 3.),
    poster: Filter::Sepia,
    poster_light: 0.95,
    backdrop: Filter::Sepia,
    progress: 0xE8A33D,
    ..CLASSIC
};

const MINIMAL: Theme = Theme {
    key: "minimal",
    name: "Minimal, bold type",
    group: "2026 trends",
    phase: 2,
    background: Some((0x050505, 0x050505)),
    decor: &[Decor::Glow(1440., 270., 2700., 1500., 0x3C505F, 0.55)],
    fg: 0xF2F2F2,
    muted: 0x8B8B8B,
    accent: 0xF2F2F2,
    panel: (0x111111, 0.9),
    focus_fill: 0xF2F2F2,
    focus_ink: 0x050505,
    max_radius: 3.,
    display: face(ANTON, 400),
    body: face(BARLOW, 400),
    label: face(BARLOW, 400),
    caps: true,
    card_corners: [3.; 4],
    card_border: (0., 0xFFFFFF),
    card_shadow: Shadow::None,
    focus: Focus::Underline(0xF2F2F2),
    poster: Filter::Grey,
    poster_light: 0.55,
    progress: 0xFFFFFF,
    ..CLASSIC
};

const SWISS: Theme = Theme {
    key: "swiss",
    name: "Swiss grid",
    group: "Bold directions",
    phase: 1,
    light: true,
    background: Some((0xFFFFFF, 0xFFFFFF)),
    decor: &[
        Decor::Pattern(
            SCREEN,
            Pattern::Columns,
            160.,
            1.5,
            0x111111,
            0.07,
            Blend::Normal,
        ),
        Decor::Fill(r(1290., 0., 630., 1080.), 0xE3061B, 1., [0.; 4], 0.),
    ],
    fg: 0x111111,
    muted: 0x4A4A4A,
    accent: 0xE3061B,
    panel: (0xFFFFFF, 1.),
    focus_fill: 0x111111,
    focus_ink: 0xFFFFFF,
    outline: (3., 0x111111),
    max_radius: 0.,
    display: face(ARCHIVO, 700),
    body: face(ARCHIVO, 400),
    label: face(ARCHIVO, 700),
    card_corners: [0.; 4],
    card_border: (1.5, 0x111111),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0xE3061B, 6., 4.5),
    progress: 0xE3061B,
    ..CLASSIC
};

const Y2K: Theme = Theme {
    key: "y2k",
    name: "Y2K chrome",
    group: "Bold directions",
    phase: 4,
    background: Some((0x0B0620, 0x0B0620)),
    decor: &[
        Decor::Glow(1632., 270., 1260., 960., 0xFF5CCD, 0.7),
        Decor::Glow(1248., 756., 1140., 900., 0x3CDCFF, 0.55),
        Decor::Glow(1920., 864., 900., 780., 0xC6FF3D, 0.45),
        Decor::Glow(384., 0., 1500., 1200., 0x6E3CFF, 0.6),
        Decor::Sparkle(1470., 220., 70., 0xFFFFFF),
        Decor::Sparkle(1740., 650., 46., 0xC6FF3D),
    ],
    fg: 0xFFFFFF,
    muted: 0xE9E3FF,
    accent: 0xC6FF3D,
    panel: (0xFFFFFF, 0.1),
    focus_fill: 0xF2E6FF,
    focus_ink: 0x1A0D3D,
    outline: (2.25, 0xFFFFFF),
    display: face(SYNE, 700),
    body: face(SYNE, 400),
    label: face(SYNE, 700),
    card_corners: [39.; 4],
    card_border: (4.5, 0xFFFFFF),
    card_shadow: Shadow::Glow(0xFF5CCD, 0.35),
    focus: Focus::Glow(0xC6FF3D, 6.),
    title_fx: TitleFx::Chrome,
    progress: 0xC6FF3D,
    ..CLASSIC
};

const TERM: Theme = Theme {
    key: "term",
    name: "Terminal",
    group: "Bold directions",
    phase: 3,
    background: Some((0x030A04, 0x030A04)),
    overlay: &[Decor::Pattern(
        SCREEN,
        Pattern::Lines,
        4.5,
        1.5,
        0x000000,
        0.28,
        Blend::Normal,
    )],
    fg: 0x7CFF8A,
    muted: 0x52B85D,
    accent: 0x7CFF8A,
    panel: (0x030A04, 1.),
    focus_fill: 0x7CFF8A,
    focus_ink: 0x030A04,
    outline: (3., 0x7CFF8A),
    max_radius: 0.,
    display: face(VT323, 400),
    body: face(VT323, 400),
    label: face(VT323, 400),
    caps: true,
    card_corners: [0.; 4],
    card_border: (3., 0x7CFF8A),
    card_shadow: Shadow::None,
    focus: Focus::Glow(0x7CFF8A, 4.5),
    poster: Filter::Duotone(0x021404, 0x7CFF8A),
    poster_light: 0.75,
    poster_focused: Filter::Duotone(0x031A06, 0xB8FFC0),
    backdrop: Filter::Duotone(0x010802, 0x2F7A3A),
    title_fx: TitleFx::Offset(0x0F4D17, 7.5, 7.5),
    progress: 0x7CFF8A,
    ..CLASSIC
};

const DECO: Theme = Theme {
    key: "deco",
    name: "Art Deco marquee",
    group: "Bold directions",
    phase: 3,
    background: Some((0x0C0A08, 0x0C0A08)),
    decor: &[
        Decor::Pattern(
            r(-1040., -830., 4000., 3000.),
            Pattern::Rays,
            60.,
            0.33,
            0xD4AF37,
            0.09,
            Blend::Normal,
        ),
        Decor::Glow(960., 432., 1800., 1200., 0x781E1E, 0.5),
    ],
    fg: 0xF4E6C1,
    muted: 0xD4AF37,
    accent: 0xD4AF37,
    panel: (0x0C0A08, 0.88),
    focus_fill: 0xD4AF37,
    focus_ink: 0x0C0A08,
    outline: (1.5, 0xD4AF37),
    max_radius: 0.,
    display: face(LIMELIGHT, 400),
    body: face(POIRET_ONE, 400),
    label: face(POIRET_ONE, 400),
    caps: true,
    card_corners: [108., 108., 0., 0.],
    card_border: (1.5, 0xD4AF37),
    card_shadow: Shadow::Soft,
    focus: Focus::Glow(0xD4AF37, 3.),
    poster: Filter::Sepia,
    poster_light: 0.9,
    poster_focused: Filter::None,
    hero_panel: Some(HeroPanel {
        fill: (0x0C0A08, 0.88),
        border: (0xD4AF37, 3.),
        radius: [0.; 4],
        shadow: Shadow::None,
        angle: 0.,
        perspective: false,
        bulbs: true,
    }),
    title_fx: TitleFx::Glow(0xF4D78A),
    progress: 0xD4AF37,
    ..CLASSIC
};

const COLLAGE: Theme = Theme {
    key: "collage",
    name: "Cut-up collage",
    group: "Bold directions",
    phase: 3,
    light: true,
    background: Some((0xD8D6D0, 0xD8D6D0)),
    decor: &[Decor::Pattern(
        SCREEN,
        Pattern::Dots,
        9.,
        3.,
        0x000000,
        0.13,
        Blend::Normal,
    )],
    fg: 0x111111,
    muted: 0x3A3A3A,
    accent: 0xE8442E,
    panel: (0xF2F0EA, 1.),
    focus_fill: 0xE8442E,
    focus_ink: 0xFFFFFF,
    surface_shadow: Some((4.5, 4.5, 0x111111)),
    max_radius: 0.,
    display: face(DM_SERIF, 400),
    body: face(SYNE, 400),
    label: face(SYNE, 700),
    card_corners: [0.; 4],
    card_border: (9., 0xF2F0EA),
    card_shadow: Shadow::Soft,
    focus: Focus::Hard(0xE8442E, 15.),
    tilt: 3.,
    title_fx: TitleFx::Offset(0xE8442E, 12., 12.),
    progress: 0xE8442E,
    ..CLASSIC
};

const CLAY: Theme = Theme {
    key: "clay",
    name: "Claymorphism",
    group: "Bold directions",
    phase: 3,
    light: true,
    background: Some((0xE7E1F7, 0xE7E1F7)),
    decor: &[
        Decor::Glow(1632., 324., 1500., 1080., 0xFFD9CC, 1.),
        Decor::Glow(1344., 972., 1200., 900., 0xC9F2E3, 1.),
    ],
    fg: 0x2B2540,
    muted: 0x6B5FA8,
    accent: 0x7C6CF2,
    panel: (0xF7F4FF, 1.),
    focus_fill: 0x7C6CF2,
    focus_ink: 0xFFFFFF,
    display: face(NUNITO, 700),
    body: face(NUNITO, 400),
    label: face(NUNITO, 700),
    card_corners: [42.; 4],
    card_border: (7.5, 0xF7F4FF),
    card_shadow: Shadow::Glow(0x6E5AB4, 0.32),
    focus: Focus::Ring(0x7C6CF2, 7.5, -7.5),
    hero_panel: Some(panel(
        (0xF7F4FF, 1.),
        (0xFFFFFF, 0.),
        51.,
        Shadow::Glow(0x6E5AB4, 0.32),
    )),
    progress: 0x7C6CF2,
    ..CLASSIC
};

const BAUHAUS: Theme = Theme {
    key: "bauhaus",
    name: "Bauhaus",
    group: "Bold directions",
    phase: 2,
    light: true,
    background: Some((0xEEEDE9, 0xEEEDE9)),
    decor: &[
        Decor::Fill(r(1230., 90., 570., 570.), 0xF2C200, 1., [285.; 4], 0.),
        Decor::Fill(r(1725., 0., 195., 600.), 0x1C4DA1, 1., [0.; 4], 0.),
        Decor::Fill(
            r(1140., 450., 330., 255.),
            0xD7262B,
            1.,
            [0., 0., 0., 255.],
            0.,
        ),
    ],
    fg: 0x111111,
    muted: 0x444444,
    accent: 0xD7262B,
    panel: (0xEEEDE9, 1.),
    focus_fill: 0xD7262B,
    focus_ink: 0xFFFFFF,
    outline: (3., 0x111111),
    max_radius: 999.,
    display: face(LEAGUE_SPARTAN, 700),
    body: face(LEAGUE_SPARTAN, 400),
    label: face(LEAGUE_SPARTAN, 700),
    caps: true,
    card_corners: [0.; 4],
    card_border: (4.5, 0x111111),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0xF2C200, 9., 4.5),
    progress: 0xF2C200,
    ..CLASSIC
};

const SPATIAL: Theme = Theme {
    key: "spatial",
    name: "Spatial depth",
    group: "Bold directions",
    phase: 4,
    background: Some((0x0B1220, 0x05060A)),
    decor: &[
        Decor::Glow(1344., 216., 2100., 1260., 0x28788C, 0.55),
        Decor::Glow(576., 324., 1500., 900., 0xC86E32, 0.35),
        Decor::Fill(r(0., 670., 1920., 2.), 0xFFFFFF, 0.06, [0.; 4], 0.),
    ],
    fg: 0xF2F5FA,
    muted: 0xB0B6C2,
    accent: 0xF2F5FA,
    panel: (0xFFFFFF, 0.08),
    focus_fill: 0xF2F5FA,
    focus_ink: 0x05060A,
    outline: (1.5, 0xFFFFFF),
    display: face(FIGTREE, 700),
    body: face(FIGTREE, 400),
    label: face(FIGTREE, 700),
    card_corners: [27.; 4],
    card_border: (0., 0xFFFFFF),
    focus: Focus::Ring(0xFFFFFF, 3., 0.),
    backdrop: Filter::Blur(22.),
    hero_panel: Some(HeroPanel {
        fill: (0xFFFFFF, 0.07),
        border: (0xFFFFFF, 1.5),
        radius: [48.; 4],
        shadow: Shadow::Soft,
        angle: 0.,
        perspective: true,
        bulbs: false,
    }),
    ..CLASSIC
};

const SHELF: Theme = Theme {
    key: "shelf",
    name: "Video shelf",
    group: "Bold directions",
    phase: 1,
    background: Some((0x1A1411, 0x1A1411)),
    decor: &[
        Decor::Glow(960., 0., 2400., 1140., 0xFFC88C, 0.22),
        Decor::Pattern(
            SCREEN,
            Pattern::Columns,
            16.5,
            4.5,
            0xFFFFFF,
            0.015,
            Blend::Normal,
        ),
    ],
    fg: 0xF6F3EE,
    muted: 0xE9D9C4,
    accent: 0xC8102E,
    panel: (0x2A211C, 0.9),
    focus_fill: 0xC8102E,
    focus_ink: 0xFFFFFF,
    outline: (1.5, 0xE9D9C4),
    max_radius: 6.,
    display: face(FRAUNCES, 700),
    body: face(OSWALD, 400),
    label: face(OSWALD, 400),
    caps: true,
    card_corners: [3., 3., 0., 0.],
    card_border: (0., 0xFFFFFF),
    focus: Focus::Ring(0xF6F3EE, 3., 0.),
    layout: Layout::Shelf,
    hero_panel: Some(HeroPanel {
        fill: (0xF6F3EE, 1.),
        border: (0xF6F3EE, 0.),
        radius: [0.; 4],
        shadow: Shadow::Soft,
        angle: -1.2,
        perspective: false,
        bulbs: false,
    }),
    progress: 0xC8102E,
    ..CLASSIC
};

const BLUEPRINT: Theme = Theme {
    key: "blueprint",
    name: "Blueprint",
    group: "Art movements",
    phase: 3,
    background: Some((0x0E3A6E, 0x0E3A6E)),
    decor: &[
        Decor::Pattern(
            SCREEN,
            Pattern::Grid,
            150.,
            1.5,
            0xEAF2FF,
            0.14,
            Blend::Normal,
        ),
        Decor::Pattern(
            SCREEN,
            Pattern::Grid,
            30.,
            1.,
            0xEAF2FF,
            0.05,
            Blend::Normal,
        ),
    ],
    fg: 0xEAF2FF,
    muted: 0xA9C3E6,
    accent: 0xFFC857,
    panel: (0x0E3A6E, 1.),
    focus_fill: 0xFFC857,
    focus_ink: 0x0E3A6E,
    outline: (1.5, 0xEAF2FF),
    max_radius: 0.,
    display: face(BARLOW_CONDENSED, 700),
    body: face(BARLOW, 400),
    label: face(PLEX_MONO, 400),
    caps: true,
    card_corners: [0.; 4],
    card_border: (1.5, 0xEAF2FF),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0xFFC857, 1.5, 10.5),
    poster: Filter::Grey,
    poster_light: 0.45,
    title_fx: TitleFx::Outline(0xEAF2FF, 2.),
    progress: 0xFFC857,
    ..CLASSIC
};

const POP: Theme = Theme {
    key: "pop",
    name: "Pop Art",
    group: "Art movements",
    phase: 2,
    light: true,
    background: Some((0xFFE600, 0xFFE600)),
    decor: &[Decor::Pattern(
        SCREEN,
        Pattern::Dots,
        18.,
        7.8,
        0xE3262E,
        0.35,
        Blend::Normal,
    )],
    fg: 0x111111,
    muted: 0x333333,
    accent: 0x1E5BD8,
    panel: (0xFFFFFF, 1.),
    focus_fill: 0xE3262E,
    focus_ink: 0xFFFFFF,
    outline: (4.5, 0x111111),
    surface_shadow: Some((6., 6., 0x111111)),
    max_radius: 0.,
    display: face(BANGERS, 400),
    body: face(BARLOW, 400),
    label: face(BANGERS, 400),
    card_corners: [0.; 4],
    card_border: (6., 0x111111),
    card_shadow: Shadow::Hard(6., 6., 0x111111),
    focus: Focus::Hard(0x111111, 13.5),
    tilt: 2.,
    title_fx: TitleFx::Outline(0x111111, 4.5),
    progress: 0xE3262E,
    ..CLASSIC
};

const DESTIJL: Theme = Theme {
    key: "destijl",
    name: "De Stijl",
    group: "Art movements",
    phase: 2,
    light: true,
    background: Some((0xFBFBF8, 0xFBFBF8)),
    decor: &[
        Decor::Fill(r(1524., 0., 396., 294.), 0xDD0100, 1., [0.; 4], 0.),
        Decor::Fill(r(1524., 318., 396., 342.), 0x225095, 1., [0.; 4], 0.),
        Decor::Fill(r(1074., 489., 426., 171.), 0xFAC901, 1., [0.; 4], 0.),
        Decor::Fill(r(1050., 0., 24., 660.), 0x111111, 1., [0.; 4], 0.),
        Decor::Fill(r(1500., 0., 24., 660.), 0x111111, 1., [0.; 4], 0.),
        Decor::Fill(r(1524., 294., 396., 24.), 0x111111, 1., [0.; 4], 0.),
        Decor::Fill(r(1074., 465., 426., 24.), 0x111111, 1., [0.; 4], 0.),
        Decor::Fill(r(0., 660., 1920., 24.), 0x111111, 1., [0.; 4], 0.),
    ],
    fg: 0x111111,
    muted: 0x333333,
    accent: 0xDD0100,
    panel: (0xFFFFFF, 1.),
    focus_fill: 0xDD0100,
    focus_ink: 0xFFFFFF,
    outline: (6., 0x111111),
    max_radius: 0.,
    display: face(ARCHIVO, 700),
    body: face(ARCHIVO, 400),
    label: face(ARCHIVO, 700),
    caps: true,
    card_corners: [0.; 4],
    card_border: (12., 0x111111),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0xDD0100, 12., 0.),
    progress: 0xDD0100,
    ..CLASSIC
};

const MEMPHIS: Theme = Theme {
    key: "memphis",
    name: "Memphis",
    group: "Art movements",
    phase: 2,
    light: true,
    background: Some((0xFFC8DD, 0xFFC8DD)),
    decor: &[
        Decor::Curve(
            [[1110., 180.], [1250., 60.], [1350., 300.], [1470., 180.]],
            10.5,
            0x111111,
            1.,
        ),
        Decor::Triangle([1725., 135.], [1845., 345.], [1605., 345.], 0xFFE14D),
        Decor::Ring(1770., 570., 81., 24., 0x3D5AFE),
        Decor::Zigzag(1140., 645., 6, 45., 45., 13.5, 0xFF5A36),
        Decor::Fill(r(1470., 420., 60., 60.), 0x3D5AFE, 1., [0.; 4], 20.),
        Decor::Fill(r(1290., 760., 120., 18.), 0x111111, 1., [9.; 4], -25.),
        Decor::Fill(r(1660., 820., 36., 36.), 0xFF5A36, 1., [18.; 4], 0.),
    ],
    fg: 0x111111,
    muted: 0x3A2A33,
    accent: 0x3D5AFE,
    panel: (0xFFFFFF, 1.),
    focus_fill: 0xFFE14D,
    focus_ink: 0x111111,
    outline: (4.5, 0x111111),
    surface_shadow: Some((6., 6., 0x3D5AFE)),
    max_radius: 24.,
    display: face(RIGHTEOUS, 400),
    body: face(RUBIK, 400),
    label: face(RUBIK, 700),
    card_corners: [24.; 4],
    card_border: (4.5, 0x111111),
    card_shadow: Shadow::Hard(9., 9., 0x3D5AFE),
    focus: Focus::Hard(0x111111, 15.),
    title_fx: TitleFx::Offset(0x3D5AFE, 10.5, 10.5),
    progress: 0xFF5A36,
    ..CLASSIC
};

const CONSTRUCT: Theme = Theme {
    key: "construct",
    name: "Constructivism",
    group: "Art movements",
    phase: 3,
    light: true,
    background: Some((0xE5DCC6, 0xE5DCC6)),
    decor: &[
        Decor::Fill(r(1395., 105., 450., 450.), 0x111111, 1., [225.; 4], 0.),
        Decor::Fill(r(-300., 660., 2700., 120.), 0xC1121F, 1., [0.; 4], -12.),
        Decor::Fill(r(-300., 600., 2700., 6.), 0x111111, 1., [0.; 4], -12.),
    ],
    fg: 0x111111,
    muted: 0x3A342A,
    accent: 0xC1121F,
    panel: (0x111111, 1.),
    focus_fill: 0xC1121F,
    focus_ink: 0xE5DCC6,
    max_radius: 0.,
    display: face(OSWALD, 700),
    body: face(OSWALD, 400),
    label: face(OSWALD, 700),
    caps: true,
    card_corners: [0.; 4],
    card_border: (0., 0x111111),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0x111111, 7.5, 4.5),
    poster: Filter::Duotone(0x111111, 0xF0D8D0),
    poster_focused: Filter::None,
    tilt: 2.,
    hero_panel: None,
    progress: 0xC1121F,
    ..CLASSIC
};

const OPART: Theme = Theme {
    key: "opart",
    name: "Op Art",
    group: "Art movements",
    phase: 3,
    light: true,
    background: Some((0xF5F5F5, 0xF5F5F5)),
    decor: &[
        Decor::Pattern(
            r(240., -1035., 2400., 2610.),
            Pattern::Rings,
            27.,
            13.5,
            0x111111,
            1.,
            Blend::Normal,
        ),
        Decor::Gradient(r(0., 0., 1000., 1080.), 0xF5F5F5, 0xF5F5F5),
        Decor::Gradient(r(0., 580., 1920., 500.), 0xF5F5F5, 0xF5F5F5),
    ],
    fg: 0x111111,
    muted: 0x444444,
    accent: 0x111111,
    panel: (0xF5F5F5, 1.),
    focus_fill: 0x111111,
    focus_ink: 0xF5F5F5,
    outline: (3., 0x111111),
    max_radius: 0.,
    display: face(MONOTON, 400),
    body: face(BARLOW, 400),
    label: face(BARLOW, 700),
    caps: true,
    card_corners: [0.; 4],
    card_border: (10.5, 0x111111),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0x111111, 4.5, 4.5),
    poster: Filter::Grey,
    poster_focused: Filter::None,
    progress: 0x111111,
    ..CLASSIC
};

const NOUVEAU: Theme = Theme {
    key: "nouveau",
    name: "Art Nouveau",
    group: "Art movements",
    phase: 1,
    background: Some((0x1C362B, 0x1C362B)),
    decor: &[
        Decor::Curve(
            [[1875., 1080.], [1755., 840.], [1950., 705.], [1785., 540.]],
            3.75,
            0xC9A45C,
            0.75,
        ),
        Decor::Curve(
            [[1785., 540.], [1620., 375.], [1575., 255.], [1755., 45.]],
            3.75,
            0xC9A45C,
            0.75,
        ),
        Decor::Curve(
            [[1785., 540.], [1680., 480.], [1650., 420.], [1680., 330.]],
            3.,
            0xC9A45C,
            0.6,
        ),
        Decor::Curve(
            [[1830., 780.], [1740., 760.], [1700., 700.], [1720., 640.]],
            3.,
            0xC9A45C,
            0.6,
        ),
    ],
    fg: 0xF1E8D2,
    muted: 0xE7C98A,
    accent: 0xC9A45C,
    panel: (0x1C362B, 0.78),
    focus_fill: 0xC9A45C,
    focus_ink: 0x1C362B,
    outline: (1.5, 0xC9A45C),
    display: face(FEDERO, 400),
    body: face(CORMORANT, 400),
    label: face(CORMORANT, 700),
    card_corners: [108., 108., 12., 12.],
    card_border: (3., 0xC9A45C),
    focus: Focus::Glow(0xC9A45C, 3.),
    poster: Filter::Sepia,
    poster_light: 0.92,
    poster_focused: Filter::None,
    hero_panel: Some(HeroPanel {
        fill: (0x1C362B, 0.78),
        border: (0xC9A45C, 2.25),
        radius: [165., 165., 21., 21.],
        shadow: Shadow::None,
        angle: 0.,
        perspective: false,
        bulbs: false,
    }),
    progress: 0xC9A45C,
    ..CLASSIC
};

const RAWWEB: Theme = Theme {
    key: "rawweb",
    name: "Raw web brutalism",
    group: "Wildcards",
    phase: 2,
    light: true,
    background: Some((0xFFFFFF, 0xFFFFFF)),
    fg: 0x000000,
    muted: 0x333333,
    accent: 0x0000EE,
    panel: (0xC0C0C0, 1.),
    focus_fill: 0xFFFF00,
    focus_ink: 0x000000,
    outline: (1.5, 0x000000),
    max_radius: 0.,
    display: face(CORMORANT, 700),
    body: face(CORMORANT, 400),
    label: face(CORMORANT, 700),
    card_corners: [0.; 4],
    card_border: (1.5, 0x000000),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0x000000, 3., 4.5),
    progress: 0x0000EE,
    ..CLASSIC
};

const VAPOR: Theme = Theme {
    key: "vapor",
    name: "Vaporwave",
    group: "Wildcards",
    phase: 4,
    background: Some((0x1B0838, 0x12052A)),
    decor: &[
        Decor::Gradient(r(0., 0., 1920., 324.), 0x1B0838, 0x4A1470),
        Decor::Gradient(r(0., 324., 1920., 314.), 0x4A1470, 0xFF8FCF),
        Decor::Fill(r(1230., 160., 480., 480.), 0xFFB347, 1., [240.; 4], 0.),
        Decor::Pattern(
            r(1230., 420., 480., 220.),
            Pattern::Lines,
            30.,
            9.,
            0xC73C9E,
            1.,
            Blend::Normal,
        ),
        Decor::Fill(r(0., 638., 1920., 442.), 0x12052A, 1., [0.; 4], 0.),
        Decor::Pattern(
            r(-480., 638., 2880., 442.),
            Pattern::Floor,
            28.,
            2.25,
            0x00E5FF,
            0.85,
            Blend::Normal,
        ),
    ],
    fg: 0xFFFFFF,
    muted: 0x00E5FF,
    accent: 0xFF2975,
    panel: (0x2A0E4A, 0.85),
    focus_fill: 0xFF2975,
    focus_ink: 0xFFFFFF,
    outline: (3., 0x00E5FF),
    max_radius: 0.,
    display: face(AUDIOWIDE, 400),
    body: face(INTER, 400),
    label: face(AUDIOWIDE, 400),
    caps: true,
    card_corners: [0.; 4],
    card_border: (3., 0x00E5FF),
    card_shadow: Shadow::Glow(0x00E5FF, 0.45),
    focus: Focus::Glow(0xFF2975, 3.),
    poster: Filter::Duotone(0x1B0838, 0xFF8FCF),
    poster_focused: Filter::None,
    hero_panel: Some(panel(
        (0xC0C0C0, 1.),
        (0xFFFFFF, 3.),
        0.,
        Shadow::Hard(3., 3., 0x000000),
    )),
    title_fx: TitleFx::Offset(0x00C8E8, 4.5, 4.5),
    progress: 0xFF2975,
    ..CLASSIC
};

const CYBER: Theme = Theme {
    key: "cyber",
    name: "Cyberpunk glitch",
    group: "Wildcards",
    phase: 4,
    background: Some((0x0A0A12, 0x0A0A12)),
    decor: &[
        Decor::Glow(1536., 324., 1800., 1200., 0xFF003C, 0.35),
        Decor::Glow(1824., 864., 1500., 1080., 0x00F0FF, 0.25),
        Decor::Pattern(
            r(0., 1032., 1920., 48.),
            Pattern::Diagonal,
            42.,
            18.,
            0xF3E600,
            0.8,
            Blend::Normal,
        ),
    ],
    overlay: &[Decor::Pattern(
        SCREEN,
        Pattern::Lines,
        6.,
        1.5,
        0xFFFFFF,
        0.03,
        Blend::Normal,
    )],
    fg: 0xE6F7FF,
    muted: 0x9ACBE0,
    accent: 0xF3E600,
    panel: (0xF3E600, 0.06),
    focus_fill: 0xF3E600,
    focus_ink: 0x0A0A12,
    outline: (1.5, 0xF3E600),
    max_radius: 0.,
    display: face(ORBITRON, 700),
    body: face(BARLOW, 400),
    label: face(ORBITRON, 700),
    caps: true,
    card_corners: [0., 27., 0., 0.],
    card_border: (0., 0xF3E600),
    card_shadow: Shadow::None,
    focus: Focus::Glow(0x00F0FF, 4.5),
    hero_panel: Some(HeroPanel {
        fill: (0xF3E600, 0.06),
        border: (0xF3E600, 1.5),
        radius: [0., 45., 0., 45.],
        shadow: Shadow::None,
        angle: 0.,
        perspective: false,
        bulbs: false,
    }),
    title_fx: TitleFx::Split(0xFF003C, 0x00F0FF),
    progress: 0xF3E600,
    ..CLASSIC
};

const NOIR: Theme = Theme {
    key: "noir",
    name: "Film noir",
    group: "Wildcards",
    phase: 3,
    background: Some((0x0B0B0B, 0x0B0B0B)),
    decor: &[
        Decor::Glow(1498., 216., 2100., 1440., 0xFFFFFF, 0.16),
        Decor::Pattern(
            SCREEN,
            Pattern::Diagonal,
            66.,
            24.,
            0x000000,
            0.35,
            Blend::Normal,
        ),
    ],
    fg: 0xEDEDED,
    muted: 0xA8A8A8,
    accent: 0xC0161B,
    panel: (0x161616, 0.9),
    focus_fill: 0xEDEDED,
    focus_ink: 0x0B0B0B,
    max_radius: 3.,
    display: face(BODONI, 400),
    body: face(SPECIAL_ELITE, 400),
    label: face(SPECIAL_ELITE, 400),
    card_corners: [0.; 4],
    card_border: (6., 0xEDEDED),
    focus: Focus::Glow(0xEDEDED, 1.5),
    poster: Filter::Grey,
    poster_light: 0.8,
    poster_focused: Filter::Grey,
    backdrop: Filter::Grey,
    progress: 0xC0161B,
    ..CLASSIC
};

const MATERIAL: Theme = Theme {
    key: "material",
    name: "Material You",
    group: "Wildcards",
    phase: 2,
    background: Some((0x16263D, 0x101418)),
    fg: 0xE1E2E8,
    muted: 0xC3C6CF,
    accent: 0xA6C8FF,
    panel: (0x1D2024, 1.),
    focus_fill: 0xA6C8FF,
    focus_ink: 0x00315F,
    display: face(ROBOTO, 400),
    body: face(ROBOTO, 400),
    label: face(ROBOTO, 700),
    card_corners: [24.; 4],
    card_border: (0., 0xFFFFFF),
    focus: Focus::Ring(0xA6C8FF, 4.5, 4.5),
    hero_panel: Some(panel((0x1D2024, 1.), (0x1D2024, 0.), 48., Shadow::None)),
    progress: 0xA6C8FF,
    ..CLASSIC
};

const SKEUO: Theme = Theme {
    key: "skeuo",
    name: "Skeuomorphic deck",
    group: "Wildcards",
    phase: 3,
    light: true,
    background: Some((0xDDE0E3, 0xAEB2B6)),
    decor: &[Decor::Pattern(
        SCREEN,
        Pattern::Columns,
        7.5,
        1.5,
        0xFFFFFF,
        0.18,
        Blend::Normal,
    )],
    fg: 0x2A2D31,
    muted: 0x4A4D52,
    accent: 0x6CB4FF,
    panel: (0xF2F2F2, 1.),
    focus_fill: 0x6CB4FF,
    focus_ink: 0x0B2340,
    outline: (1.5, 0x55585C),
    max_radius: 12.,
    display: face(SHARE_TECH_MONO, 400),
    body: face(INTER, 400),
    label: face(INTER, 650),
    caps: true,
    card_corners: [10.5; 4],
    card_border: (1.5, 0x2A2D31),
    focus: Focus::Glow(0x6CB4FF, 4.5),
    hero_panel: Some(panel((0x0E1A10, 1.), (0x55585C, 3.), 12., Shadow::Soft)),
    title_fx: TitleFx::Glow(0x8CFF8C),
    progress: 0x6CB4FF,
    ..CLASSIC
};

const RISO: Theme = Theme {
    key: "riso",
    name: "Risograph",
    group: "Wildcards",
    phase: 3,
    light: true,
    background: Some((0xF2EFE9, 0xF2EFE9)),
    overlay: &[Decor::Pattern(
        SCREEN,
        Pattern::Grain,
        1.,
        0.3,
        0x1A1A33,
        1.,
        Blend::Multiply,
    )],
    fg: 0x0078BF,
    muted: 0x3A8CC4,
    accent: 0xFF48B0,
    panel: (0xF2EFE9, 1.),
    focus_fill: 0xFF48B0,
    focus_ink: 0xF2EFE9,
    outline: (2.25, 0x0078BF),
    max_radius: 0.,
    display: face(UNBOUNDED, 700),
    body: face(COURIER_PRIME, 400),
    label: face(UNBOUNDED, 700),
    caps: true,
    card_corners: [0.; 4],
    card_border: (0., 0x0078BF),
    card_shadow: Shadow::Hard(7.5, 6., 0x0078BF),
    focus: Focus::Hard(0xFF48B0, 12.),
    poster: Filter::Duotone(0x0078BF, 0xFFE3F2),
    poster_focused: Filter::None,
    backdrop: Filter::Duotone(0x0078BF, 0xF2EFE9),
    title_fx: TitleFx::Offset(0xFF48B0, 7.5, 4.5),
    progress: 0xFF48B0,
    ..CLASSIC
};

const PIXEL: Theme = Theme {
    key: "pixel",
    name: "8-bit pixel",
    group: "Wildcards",
    phase: 3,
    background: Some((0x1D2B53, 0x1D2B53)),
    decor: &[Decor::Fill(
        r(0., 670., 1920., 30.),
        0x7E2553,
        1.,
        [0.; 4],
        0.,
    )],
    fg: 0xFFF1E8,
    muted: 0xC2C3C7,
    accent: 0xFFEC27,
    panel: (0x000000, 1.),
    focus_fill: 0xFFEC27,
    focus_ink: 0x000000,
    outline: (6., 0xFFF1E8),
    max_radius: 0.,
    display: face(PRESS_START, 400),
    body: face(VT323, 400),
    label: face(PRESS_START, 400),
    card_corners: [0.; 4],
    card_border: (6., 0xFFF1E8),
    card_shadow: Shadow::Hard(0., 18., 0x000000),
    focus: Focus::Ring(0xFFEC27, 6., 0.),
    poster: Filter::Pixel(9.),
    poster_focused: Filter::Pixel(6.),
    backdrop: Filter::Pixel(16.),
    hero_panel: Some(panel((0x000000, 1.), (0xFFF1E8, 6.), 0., Shadow::None)),
    title_fx: TitleFx::Offset(0x7E2553, 6., 6.),
    progress: 0x00E436,
    ..CLASSIC
};

const LETTERBOX: Theme = Theme {
    key: "letterbox",
    name: "Letterbox",
    group: "Wildcards",
    phase: 2,
    background: Some((0x0E0C0A, 0x0E0C0A)),
    decor: &[
        Decor::Glow(1344., 432., 2400., 1260., 0xD27832, 0.45),
        Decor::Glow(384., 648., 2100., 1200., 0x145A64, 0.5),
    ],
    fg: 0xF4EFE6,
    muted: 0xCFC6B8,
    accent: 0xF4EFE6,
    panel: (0x1A1612, 0.9),
    focus_fill: 0xF4EFE6,
    focus_ink: 0x0E0C0A,
    max_radius: 2.,
    display: face(CINZEL, 700),
    body: face(BARLOW, 400),
    label: face(CINZEL, 400),
    caps: true,
    card_corners: [0.; 4],
    card_border: (0., 0xFFFFFF),
    card_shadow: Shadow::None,
    focus: Focus::Ring(0xF4EFE6, 1.5, 0.),
    poster: Filter::None,
    poster_light: 0.6,
    layout: Layout::Scope,
    progress: 0xF4EFE6,
    ..CLASSIC
};

const PAPERCUT: Theme = Theme {
    key: "papercut",
    name: "Paper cut",
    group: "Wildcards",
    phase: 2,
    light: true,
    background: Some((0xF6D5B8, 0xF6D5B8)),
    decor: &[
        Decor::Fill(r(1404., 219., 222., 222.), 0xFFF4E0, 1., [111.; 4], 0.),
        Decor::Fill(r(840., 600., 2400., 1500.), 0xE99A7E, 1., [750.; 4], 0.),
        Decor::Fill(r(450., 760., 2400., 1500.), 0xC9677A, 1., [750.; 4], 0.),
        Decor::Fill(r(-600., 870., 2400., 1500.), 0x8C5A8C, 1., [750.; 4], 0.),
        Decor::Fill(r(-1300., 960., 2400., 1500.), 0x4B3B6E, 1., [750.; 4], 0.),
    ],
    fg: 0x2C2A4A,
    muted: 0x4B3B6E,
    accent: 0xE99A7E,
    panel: (0xFFF8EE, 1.),
    focus_fill: 0x4B3B6E,
    focus_ink: 0xFFF8EE,
    surface_shadow: Some((0., 9., 0xC9A88E)),
    display: face(FREDOKA, 700),
    body: face(NUNITO, 400),
    label: face(FREDOKA, 700),
    card_corners: [21.; 4],
    card_border: (6., 0xFFF8EE),
    focus: Focus::Hard(0xE99A7E, 12.),
    hero_panel: Some(panel(
        (0xFFF8EE, 1.),
        (0xFFF8EE, 0.),
        33.,
        Shadow::Hard(-18., 18., 0xE99A7E),
    )),
    progress: 0xE99A7E,
    ..CLASSIC
};

/// Every look, Classic first, then the plan's groups in order.
pub const THEMES: [&Theme; 32] = [
    &CLASSIC, &GLASS, &BRUTAL, &BENTO, &RETRO, &MINIMAL, &SWISS, &Y2K, &TERM, &DECO, &COLLAGE,
    &CLAY, &BAUHAUS, &SPATIAL, &SHELF, &BLUEPRINT, &POP, &DESTIJL, &MEMPHIS, &CONSTRUCT, &OPART,
    &NOUVEAU, &RAWWEB, &VAPOR, &CYBER, &NOIR, &MATERIAL, &SKEUO, &RISO, &PIXEL, &LETTERBOX,
    &PAPERCUT,
];

/// The theme saved under `key`; Classic for an unknown one.
pub fn get(key: &str) -> &'static Theme {
    THEMES
        .iter()
        .copied()
        .find(|theme| theme.key == key)
        .unwrap_or(&CLASSIC)
}

/// The theme to restyle Home with, or None for Classic.
pub fn active(key: &str) -> Option<&'static Theme> {
    let theme = get(key);
    (theme.key != CLASSIC.key).then_some(theme)
}

pub fn index_of(key: &str) -> usize {
    THEMES
        .iter()
        .position(|theme| theme.key == key)
        .unwrap_or(0)
}

/// 0xRRGGBB and an alpha as a colour.
pub fn color(rgb: u32, alpha: f32) -> Color {
    [
        ((rgb >> 16) & 255) as f32 / 255.,
        ((rgb >> 8) & 255) as f32 / 255.,
        (rgb & 255) as f32 / 255.,
        alpha,
    ]
}

/// Effects a TV may find heavy or that move (blur, grain, scanlines, tilts, perspective, glows)
/// are left out when `reduced` (Settings > Reduced effects, or Reduce motion).
#[derive(Clone, Copy)]
pub struct Style {
    pub theme: &'static Theme,
    pub reduced: bool,
}

impl Style {
    pub fn new(theme: &'static Theme, reduced: bool) -> Self {
        Self { theme, reduced }
    }

    fn keeps(&self, decor: &Decor) -> bool {
        !self.reduced
            || !matches!(
                decor,
                Decor::Pattern(_, Pattern::Grain | Pattern::Lines, ..)
            )
    }

    fn filter(&self, filter: Filter) -> Filter {
        match filter {
            Filter::Blur(_) if self.reduced => Filter::None,
            other => other,
        }
    }

    /// Home's background in place of the wallpaper, with its decorations. Empty for themes that
    /// keep the wallpaper.
    pub fn background(&self) -> Vec<Draw> {
        let theme = self.theme;
        let Some((top, bottom)) = theme.background else {
            return vec![];
        };
        let mut out = vec![Draw::RoundedGradient(
            SCREEN,
            0.,
            color(top, 1.),
            color(bottom, 1.),
        )];
        for decor in theme.decor.iter().filter(|decor| self.keeps(decor)) {
            decor_draws(&mut out, decor, theme);
        }
        out
    }

    /// Drawn over Home: grain, scanlines.
    pub fn overlay(&self) -> Vec<Draw> {
        let mut out = vec![];
        for decor in self.theme.overlay.iter().filter(|decor| self.keeps(decor)) {
            decor_draws(&mut out, decor, self.theme);
        }
        out
    }

    /// Restyles draws made for Classic: type into the theme's faces (and capitals), Luma's
    /// light-on-dark colours onto the theme's, corners within its limit, surfaces outlined and
    /// shadowed as it asks. Images and coloured accents are left alone; posters are drawn by
    /// `card` instead. Big words get the theme's title treatment.
    pub fn restyle(&self, draws: Vec<Draw>) -> Vec<Draw> {
        let theme = self.theme;
        let mut out = Vec::with_capacity(draws.len() + draws.len() / 4);
        for draw in draws {
            match draw {
                Draw::Text(text, rect, style, ink, centered) => {
                    let (text, style) = self.type_style(text, style);
                    let ink = self.ink(ink);
                    if style.size >= 54. {
                        self.title_effect(&mut out, &text, rect, style, ink, centered);
                    }
                    out.push(Draw::Text(text, rect, style, ink, centered));
                }
                Draw::Shape(rect, radius, border, fill) => {
                    let radius = radius.min(theme.max_radius);
                    if border > 0. {
                        let (width, line) = self.line(fill, border);
                        out.push(Draw::Shape(rect, radius, width, line));
                    } else {
                        self.surface(&mut out, rect, radius, [fill, fill]);
                    }
                }
                Draw::RoundedGradient(rect, radius, top, bottom) => {
                    let radius = radius.min(theme.max_radius);
                    self.surface(&mut out, rect, radius, [top, bottom]);
                }
                Draw::Shadow(rect, radius, blur, shade) => {
                    // Drop shadows read as dirt on paper: lighter on light themes.
                    let shade = if theme.light {
                        [shade[0], shade[1], shade[2], shade[3] * 0.5]
                    } else {
                        shade
                    };
                    if theme.surface_shadow.is_none() {
                        out.push(Draw::Shadow(
                            rect,
                            radius.min(theme.max_radius),
                            blur,
                            shade,
                        ));
                    }
                }
                // Luma's icons are white vectors: they take the theme's ink.
                Draw::TintedImage(path, rect, radius, contain, tint)
                    if path.ends_with(".lvec") && neutral(tint) =>
                {
                    out.push(Draw::TintedImage(
                        path,
                        rect,
                        radius,
                        contain,
                        self.ink(tint),
                    ));
                }
                Draw::AppImage(image, rect, tint) if rect.w >= 960. => {
                    let mut quad = Quad::new(rect, tint);
                    quad.image = Some(QuadImage::App(image));
                    quad.filter = self.filter(theme.backdrop);
                    out.push(Draw::Quad(Box::new(quad)));
                }
                other => out.push(other),
            }
        }
        out
    }

    fn type_style(&self, text: String, style: TextStyle) -> (String, TextStyle) {
        let theme = self.theme;
        let face = if style.size >= 30. {
            theme.display
        } else if style.size <= 15.5 || style.tracking >= 0.08 {
            theme.label
        } else {
            theme.body
        };
        // Pixel and terminal faces run wide or small: keep their sizes in step with Inter.
        let scale = match face.family {
            PRESS_START => 0.62,
            VT323 => 1.3,
            ANTON | BARLOW_CONDENSED | OSWALD | BANGERS => 1.08,
            MONOTON | UNBOUNDED | ORBITRON | AUDIOWIDE => 0.84,
            _ => 1.,
        };
        let text = if theme.caps && (style.size >= 30. || style.tracking >= 0.08) {
            text.to_uppercase()
        } else {
            text
        };
        let weight = if face.family == INTER {
            style.weight
        } else if style.weight >= 600 {
            face.weight.max(700)
        } else {
            face.weight
        };
        let mut style = style.family(face.family);
        style.weight = weight;
        style.size *= scale;
        (text, style)
    }

    /// Text colour: light ink becomes the theme's ink (muted when Luma dimmed it); dark ink, on
    /// a focused surface, the theme's focus ink.
    fn ink(&self, ink: Color) -> Color {
        let theme = self.theme;
        if !neutral(ink) {
            return ink;
        }
        if luminance(ink) >= 0.6 {
            // Luma dims secondary words with alpha; the theme has a muted ink for them. Alpha
            // is kept as it is, so fades that ran before the restyle still fade.
            let rgb = if ink[3] < 0.62 { theme.muted } else { theme.fg };
            color(rgb, ink[3])
        } else {
            color(theme.focus_ink, ink[3])
        }
    }

    fn line(&self, ink: Color, width: f32) -> (f32, Color) {
        let theme = self.theme;
        if !neutral(ink) {
            return (width, ink);
        }
        let (outline, rgb) = theme.outline;
        if outline > 0. {
            (
                outline.max(width),
                color(rgb, outline_alpha(outline, ink[3])),
            )
        } else {
            (width, color(theme.fg, ink[3]))
        }
    }

    /// A fill: Luma's dark translucent panels become the theme's surface (with its outline and
    /// hard shadow), its white focused ones the focus colour, faint white the theme's ink.
    fn surface(&self, out: &mut Vec<Draw>, rect: Rect, radius: f32, colors: [Color; 2]) {
        let theme = self.theme;
        let [top, bottom] = colors;
        let full = rect.w >= 1900. && rect.h >= 1000.;
        if !neutral(top) || !neutral(bottom) {
            out.push(Draw::RoundedGradient(rect, radius, top, bottom));
            return;
        }
        // A fade (a scrim under words, a band's edge) is not a surface: it takes the
        // background's colour, with no outline or shadow.
        if top[3] != bottom[3] || rect.w >= 1900. {
            let rgb = if luminance(top) >= 0.7 {
                theme.fg
            } else {
                theme.background.map_or(theme.panel.0, |(paper, _)| paper)
            };
            out.push(Draw::RoundedGradient(
                rect,
                radius,
                color(rgb, top[3]),
                color(rgb, bottom[3]),
            ));
            return;
        }
        let light = luminance(top) >= 0.7;
        if light && top[3] < 0.5 {
            // Hairlines, skeletons and frosted chips: the theme's ink, as faint.
            let rgb = theme.fg;
            out.push(Draw::RoundedGradient(
                rect,
                radius,
                color(rgb, top[3]),
                color(rgb, bottom[3]),
            ));
            return;
        }
        if light {
            if let Some((x, y, rgb)) = theme.surface_shadow {
                out.push(Draw::Shape(
                    Rect::new(rect.x + x, rect.y + y, rect.w, rect.h),
                    radius,
                    0.,
                    color(rgb, top[3]),
                ));
            }
            out.push(Draw::Shape(
                rect,
                radius,
                0.,
                color(theme.focus_fill, top[3]),
            ));
            return;
        }
        // Full-screen scrims (Search, title pages, menus) take the background's colour.
        if full {
            let (rgb, _) = theme.background.unwrap_or((theme.panel.0, theme.panel.0));
            out.push(Draw::RoundedGradient(
                rect,
                radius,
                color(rgb, top[3]),
                color(rgb, bottom[3]),
            ));
            return;
        }
        // Luma's panels are about 0.6 to 0.8 opaque; less is a panel fading in or out, and the
        // theme's surface (with its shadow and outline) fades by as much.
        let shown = (top[3] / 0.6).min(1.);
        if let Some((x, y, rgb)) = theme.surface_shadow
            && top[3] > 0.3
        {
            out.push(Draw::Shape(
                Rect::new(rect.x + x, rect.y + y, rect.w, rect.h),
                radius,
                0.,
                color(rgb, shown),
            ));
        }
        let (rgb, opacity) = theme.panel;
        let alpha = |a: f32| opacity * (a / 0.6).min(1.);
        out.push(Draw::RoundedGradient(
            rect,
            radius,
            color(rgb, alpha(top[3])),
            color(rgb, alpha(bottom[3])),
        ));
        let (outline, line) = theme.outline;
        if outline > 0. && top[3] > 0.3 && rect.h >= 24. {
            out.push(Draw::Shape(
                rect,
                radius,
                outline,
                color(line, outline_alpha(outline, 1.) * shown),
            ));
        }
    }

    /// Copies behind a big title: an outline, an offset shadow, a glow, a split.
    fn title_effect(
        &self,
        out: &mut Vec<Draw>,
        text: &str,
        rect: Rect,
        style: TextStyle,
        ink: Color,
        centered: bool,
    ) {
        let copy = |out: &mut Vec<Draw>, dx: f32, dy: f32, rgb: u32, alpha: f32| {
            out.push(Draw::Text(
                text.into(),
                Rect::new(rect.x + dx, rect.y + dy, rect.w, rect.h),
                style,
                color(rgb, alpha * ink[3]),
                centered,
            ));
        };
        match self.theme.title_fx {
            TitleFx::Plain => {}
            TitleFx::Outline(rgb, width) => {
                for (dx, dy) in [
                    (-1., -1.),
                    (0., -1.),
                    (1., -1.),
                    (-1., 0.),
                    (1., 0.),
                    (-1., 1.),
                    (0., 1.),
                    (1., 1.),
                ] {
                    copy(out, dx * width, dy * width, rgb, 1.);
                }
            }
            TitleFx::Offset(rgb, x, y) => copy(out, x, y, rgb, 1.),
            TitleFx::Glow(rgb) if !self.reduced => {
                for (dx, dy) in [(-3., 0.), (3., 0.), (0., -3.), (0., 3.)] {
                    copy(out, dx, dy, rgb, 0.22);
                }
            }
            TitleFx::Split(left, right) => {
                copy(out, -4.5, 0., left, 0.9);
                copy(out, 4.5, 0., right, 0.9);
            }
            TitleFx::Chrome => {
                copy(out, 0., 6., 0xFF5CCD, 0.55);
                if !self.reduced {
                    copy(out, -2., -2., 0xB9F0FF, 0.5);
                    copy(out, 2., 2., 0x5B3CC4, 0.5);
                }
            }
            TitleFx::Glow(_) => {}
        }
    }

    /// A feed card in the theme: shadow, image (filtered, dimmed unfocused), frame and focus.
    /// `seed` tilts it in looks that tilt. Returns the rect the card occupies (as drawn level).
    #[allow(clippy::too_many_arguments)]
    pub fn card(
        &self,
        out: &mut Vec<Draw>,
        rect: Rect,
        image: Option<(Arc<crate::app_artwork::Artwork>, f32)>,
        focused: bool,
        seed: u32,
        layout: Layout,
    ) {
        let theme = self.theme;
        let angle = if theme.tilt > 0. && !self.reduced && !focused {
            ((seed % 5) as f32 - 2.) / 2. * theme.tilt
        } else {
            0.
        };
        let corners = match layout {
            // Bauhaus: shapes cycle by slot (square, arch, leaf).
            _ if theme.key == BAUHAUS.key => match seed % 3 {
                1 => [90., 90., 0., 0.],
                2 => [0., 0., 90., 0.],
                _ => [0.; 4],
            },
            Layout::Shelf if !focused => [3., 3., 0., 0.],
            _ => theme.card_corners,
        };
        let quad = |rect: Rect, rgb: u32, alpha: f32, border: f32| {
            let mut quad = Quad::new(rect, color(rgb, alpha));
            quad.corners = corners;
            quad.border = border;
            quad.angle = angle;
            quad
        };
        // Shadow, or the hard block a focused card lifts off.
        match theme.card_shadow {
            Shadow::Soft => {
                let mut shadow = quad(
                    Rect::new(
                        rect.x,
                        rect.y + if focused { 14. } else { 9. },
                        rect.w,
                        rect.h,
                    ),
                    0,
                    if focused { 0.45 } else { 0.2 },
                    if focused { -34. } else { -22. },
                );
                shadow.colors = [[0., 0., 0., shadow.colors[0][3]]; 2];
                out.push(Draw::Quad(Box::new(shadow)));
            }
            Shadow::Hard(x, y, rgb) => {
                let extra = if focused { 1.6 } else { 1. };
                out.push(Draw::Quad(Box::new(quad(
                    Rect::new(rect.x + x * extra, rect.y + y * extra, rect.w, rect.h),
                    rgb,
                    1.,
                    0.,
                ))));
            }
            Shadow::Glow(rgb, alpha) if !self.reduced => {
                out.push(Draw::Quad(Box::new(quad(
                    Rect::new(rect.x, rect.y + 12., rect.w, rect.h),
                    rgb,
                    alpha,
                    -30.,
                ))));
            }
            Shadow::Glow(..) | Shadow::None => {}
        }
        if let Focus::Hard(rgb, offset) = theme.focus
            && focused
        {
            out.push(Draw::Quad(Box::new(quad(
                Rect::new(rect.x + offset, rect.y + offset, rect.w, rect.h),
                rgb,
                1.,
                0.,
            ))));
        }
        let (frame, frame_rgb) = theme.card_border;
        // The paper or frame colour fills behind the image (a photo print's border).
        if frame >= 6. {
            out.push(Draw::Quad(Box::new(quad(rect, frame_rgb, 1., 0.))));
        }
        let picture = if frame >= 6. { rect.inset(frame) } else { rect };
        // The title card beneath a poster still on its way (the view writes the title on it).
        if image.as_ref().is_none_or(|(_, reveal)| *reveal < 1.) && frame < 6. {
            let (rgb, _) = theme.panel;
            let mut blank = quad(picture, rgb, 1., 0.);
            blank.corners = corners;
            out.push(Draw::Quad(Box::new(blank)));
        }
        if let Some((image, reveal)) = image {
            let light = if focused { 1. } else { theme.poster_light };
            let light = light * (0.55 + 0.45 * reveal);
            let mut art = quad(picture, 0, 1., 0.);
            art.colors = [[light, light, light, reveal]; 2];
            art.corners = corners.map(|r| (r - frame.max(0.)).max(0.));
            art.image = Some(QuadImage::App(image));
            art.filter = self.filter(if focused {
                theme.poster_focused
            } else {
                theme.poster
            });
            out.push(Draw::Quad(Box::new(art)));
            if layout == Layout::Scope {
                // Black scope bars: the still in a 2.39:1 band.
                let band = picture.w / 2.39;
                let bar = ((picture.h - band) / 2.).max(0.);
                out.push(Draw::Quad(Box::new(quad(
                    Rect::new(picture.x, picture.y, picture.w, bar),
                    0x000000,
                    1.,
                    0.,
                ))));
                out.push(Draw::Quad(Box::new(quad(
                    Rect::new(picture.x, picture.y + picture.h - bar, picture.w, bar),
                    0x000000,
                    1.,
                    0.,
                ))));
            }
        }
        if frame > 0. && frame < 6. {
            out.push(Draw::Quad(Box::new(quad(rect, frame_rgb, 1., frame))));
        }
        if focused {
            self.focus_ring(out, rect, corners, angle);
        }
    }

    /// The theme's focus mark around a rect.
    pub fn focus_ring(&self, out: &mut Vec<Draw>, rect: Rect, corners: [f32; 4], angle: f32) {
        let ring = |rect: Rect, rgb: u32, alpha: f32, border: f32, grow: f32| {
            let mut quad = Quad::new(rect.inset(-grow), color(rgb, alpha));
            quad.corners = corners.map(|r| if r > 0. { r + grow } else { 0. });
            quad.border = border;
            quad.angle = angle;
            Draw::Quad(Box::new(quad))
        };
        match self.theme.focus {
            Focus::Ring(rgb, width, gap) => {
                out.push(ring(rect, rgb, 1., width, gap + width));
            }
            Focus::Glow(rgb, width) => {
                if !self.reduced {
                    out.push(ring(rect, rgb, 0.45, -24., 0.));
                }
                out.push(ring(rect, rgb, 1., width, width));
            }
            Focus::Hard(rgb, _) => {
                out.push(ring(rect, rgb, 1., 4.5, 0.));
            }
            Focus::Underline(rgb) => {
                out.push(Draw::Shape(
                    Rect::new(rect.x, rect.y + rect.h + 9., rect.w, 3.),
                    0.,
                    0.,
                    color(rgb, 1.),
                ));
            }
        }
    }

    /// A focus ring for controls drawn with Luma's ring (buttons, tiles): the theme's mark in
    /// the theme's corners.
    pub fn control_ring(&self, out: &mut Vec<Draw>, rect: Rect, radius: f32) {
        let radius = radius.min(self.theme.max_radius);
        self.focus_ring(out, rect, [radius; 4], 0.);
    }

    /// The panel behind the Top Shelf's words, when the theme has one.
    pub fn hero_panel(&self, out: &mut Vec<Draw>, rect: Rect) {
        let Some(panel) = self.theme.hero_panel else {
            return;
        };
        let angle = if self.reduced { 0. } else { panel.angle };
        // visionOS: the panel turns away from its left edge. Each layer (shadow, panel,
        // border) is turned by itself, its corners from its own rect grown by its blur.
        let turned = panel.perspective && !self.reduced;
        let base = |rect: Rect, rgb: u32, alpha: f32, border: f32| {
            let mut quad = Quad::new(rect, color(rgb, alpha));
            quad.corners = panel.radius;
            quad.border = border;
            quad.angle = angle;
            if turned {
                let outer = rect.inset(-(-border).max(0.));
                let shrink = outer.h * 0.08;
                quad.points = Some([
                    [outer.x, outer.y],
                    [outer.x + outer.w * 0.96, outer.y + shrink],
                    [outer.x + outer.w * 0.96, outer.y + outer.h - shrink],
                    [outer.x, outer.y + outer.h],
                ]);
            }
            quad
        };
        if panel.bulbs && !self.reduced {
            // Marquee bulbs in a ring around the frame: a dotted band, the panel over it.
            let mut bulbs = base(rect.inset(-24.), 0xFFE9A8, 1., 0.);
            bulbs.filter = Filter::Pattern(Pattern::Dots, 33., 9.);
            out.push(Draw::Quad(Box::new(base(
                rect.inset(-33.),
                0xFFD278,
                0.25,
                -18.,
            ))));
            out.push(Draw::Quad(Box::new(bulbs)));
            out.push(Draw::Quad(Box::new(base(
                rect.inset(-6.),
                0x0C0A08,
                1.,
                0.,
            ))));
        }
        match panel.shadow {
            Shadow::Hard(x, y, rgb) => out.push(Draw::Quad(Box::new(base(
                Rect::new(rect.x + x, rect.y + y, rect.w, rect.h),
                rgb,
                1.,
                0.,
            )))),
            Shadow::Soft => {
                let mut shadow = base(
                    Rect::new(rect.x, rect.y + 30., rect.w, rect.h),
                    0,
                    0.5,
                    -45.,
                );
                shadow.colors = [[0., 0., 0., 0.5]; 2];
                out.push(Draw::Quad(Box::new(shadow)));
            }
            Shadow::Glow(rgb, alpha) if !self.reduced => out.push(Draw::Quad(Box::new(base(
                Rect::new(rect.x + 15., rect.y + 15., rect.w, rect.h),
                rgb,
                alpha,
                -36.,
            )))),
            Shadow::Glow(..) | Shadow::None => {}
        }
        let (fill, opacity) = panel.fill;
        out.push(Draw::Quad(Box::new(base(rect, fill, opacity, 0.))));
        let (line, width) = panel.border;
        if width > 0. {
            out.push(Draw::Quad(Box::new(base(rect, line, 1., width))));
        }
        if self.theme.key == SHELF.key && !self.reduced {
            // A strip of tape across the card's top.
            let mut tape = Quad::new(
                Rect::new(rect.x + 60., rect.y - 15., 135., 33.),
                [1., 1., 1., 0.55],
            );
            tape.angle = -3.;
            out.push(Draw::Quad(Box::new(tape)));
        }
    }

    /// The colour the Top Shelf's words take over the hero panel (dark on light panels).
    pub fn hero_ink(&self) -> Option<u32> {
        let panel = self.theme.hero_panel?;
        let (fill, opacity) = panel.fill;
        (opacity > 0.6 && luminance(color(fill, 1.)) > 0.6).then_some(match self.theme.key {
            "shelf" => 0x1A1411,
            "vapor" => 0x000000,
            _ => self.theme.fg,
        })
    }
}

/// Bold outlines (3 px and up) are solid; hairlines stay faint, as on frosted glass.
/// `alpha` is the line's own: a faded line (alpha toward 0) fades its replacement too.
fn outline_alpha(width: f32, alpha: f32) -> f32 {
    let shown = (alpha / 0.06).min(1.);
    if width >= 3. {
        shown
    } else {
        alpha.max(0.28 * shown)
    }
}

/// Whether a colour is a grey (Luma's own neutrals, which a theme recolours).
fn neutral(color: Color) -> bool {
    let max = color[0].max(color[1]).max(color[2]);
    let min = color[0].min(color[1]).min(color[2]);
    max - min < 0.09
}

fn luminance(color: Color) -> f32 {
    0.299 * color[0] + 0.587 * color[1] + 0.114 * color[2]
}

fn decor_draws(out: &mut Vec<Draw>, decor: &Decor, theme: &Theme) {
    let quad = |rect: Rect, rgb: u32, alpha: f32| Quad::new(rect, color(rgb, alpha));
    match *decor {
        Decor::Glow(x, y, w, h, rgb, alpha) => {
            // A soft blob: a small core whose blur reaches the glow's size.
            let core = Rect::new(x - w * 0.12, y - h * 0.12, w * 0.24, h * 0.24);
            let mut glow = quad(core, rgb, alpha * 0.9);
            glow.border = -(w.min(h) * 0.38);
            glow.corners = [w.min(h) * 0.12; 4];
            out.push(Draw::Quad(Box::new(glow)));
        }
        Decor::Fill(rect, rgb, alpha, corners, angle) => {
            let mut fill = quad(rect, rgb, alpha);
            fill.corners = corners;
            fill.angle = angle;
            out.push(Draw::Quad(Box::new(fill)));
        }
        Decor::Gradient(rect, top, bottom) => {
            // Over a background of `top`: fades from it, so a glow's spill is calmed toward
            // the left (where Home's words are) without a hard edge.
            let (from, _) = theme.background.unwrap_or((top, bottom));
            if top == bottom && top == from {
                let mut veil = quad(rect, top, 0.85);
                veil.colors = [color(top, 0.85), color(top, 0.85)];
                let fade = Rect::new(rect.x + rect.w, rect.y, rect.w * 0.35, rect.h);
                out.push(Draw::Quad(Box::new(veil)));
                // A horizontal fade has no gradient primitive: three steps of falling opacity.
                for (index, alpha) in [0.6, 0.35, 0.15].into_iter().enumerate() {
                    let step = fade.w / 3.;
                    out.push(Draw::Quad(Box::new(quad(
                        Rect::new(fade.x + index as f32 * step, fade.y, step, fade.h),
                        top,
                        alpha,
                    ))));
                }
            } else {
                let mut band = quad(rect, top, 1.);
                band.colors = [color(top, 1.), color(bottom, 1.)];
                out.push(Draw::Quad(Box::new(band)));
            }
        }
        Decor::Ring(x, y, radius, width, rgb) => {
            let mut ring = quad(
                Rect::new(x - radius, y - radius, radius * 2., radius * 2.),
                rgb,
                1.,
            );
            ring.corners = [radius; 4];
            ring.border = width;
            out.push(Draw::Quad(Box::new(ring)));
        }
        Decor::Pattern(rect, pattern, period, width, rgb, alpha, blend) => {
            let mut field = quad(rect, rgb, alpha);
            field.filter = Filter::Pattern(pattern, period, width);
            field.blend = blend;
            out.push(Draw::Quad(Box::new(field)));
        }
        Decor::Triangle(a, b, c, rgb) => {
            let mut triangle = quad(Rect::new(a[0], a[1], 1., 1.), rgb, 1.);
            triangle.points = Some([a, a, b, c]);
            out.push(Draw::Quad(Box::new(triangle)));
        }
        Decor::Curve(points, width, rgb, alpha) => {
            let at = |t: f32| {
                let u = 1. - t;
                let w = [u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t];
                [
                    points.iter().zip(w).map(|(p, w)| p[0] * w).sum::<f32>(),
                    points.iter().zip(w).map(|(p, w)| p[1] * w).sum::<f32>(),
                ]
            };
            segments(out, (0..=28).map(|i| at(i as f32 / 28.)), width, rgb, alpha);
        }
        Decor::Zigzag(x, y, steps, step, height, width, rgb) => {
            let points = (0..=steps * 2).map(|i| {
                [
                    x + i as f32 * step / 2.,
                    if i % 2 == 0 { y } else { y - height },
                ]
            });
            segments(out, points, width, rgb, 1.);
        }
        Decor::Sparkle(x, y, size, rgb) => {
            for (w, h) in [(size * 0.22, size), (size, size * 0.22)] {
                let mut ray = quad(Rect::new(x - w / 2., y - h / 2., w, h), rgb, 1.);
                ray.corners = [w.min(h) / 2.; 4];
                // Light added to the glow beneath, as a sparkle would be.
                ray.blend = Blend::Screen;
                out.push(Draw::Quad(Box::new(ray)));
            }
        }
    }
}

/// A polyline as rounded quads from point to point.
fn segments(
    out: &mut Vec<Draw>,
    points: impl Iterator<Item = [f32; 2]>,
    width: f32,
    rgb: u32,
    alpha: f32,
) {
    let mut previous: Option<[f32; 2]> = None;
    for point in points {
        if let Some(from) = previous {
            let (dx, dy) = (point[0] - from[0], point[1] - from[1]);
            let length = (dx * dx + dy * dy).sqrt() + width;
            let (cx, cy) = ((from[0] + point[0]) / 2., (from[1] + point[1]) / 2.);
            let mut segment = Quad::new(
                Rect::new(cx - length / 2., cy - width / 2., length, width),
                color(rgb, alpha),
            );
            segment.corners = [width / 2.; 4];
            segment.angle = dy.atan2(dx).to_degrees();
            out.push(Draw::Quad(Box::new(segment)));
        }
        previous = Some(point);
    }
}

/// A small, stable number from a title: which way a tilting card leans.
pub fn seed(text: &str) -> u32 {
    text.bytes()
        .fold(2166136261u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16777619)
        })
        .rotate_right(7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_is_unique_and_found_by_key() {
        for (index, theme) in THEMES.iter().enumerate() {
            assert_eq!(get(theme.key).key, theme.key);
            assert_eq!(index_of(theme.key), index);
            assert!(
                THEMES[index + 1..]
                    .iter()
                    .all(|other| other.key != theme.key)
            );
        }
        assert_eq!(get("nonsense").key, "classic");
        assert!(active("classic").is_none());
        assert_eq!(THEMES.len(), 32);
    }

    #[test]
    fn every_face_has_a_font_file() {
        for theme in THEMES {
            for face in [theme.display, theme.body, theme.label] {
                assert!(usize::from(face.family) <= FONT_FAMILIES.len());
            }
        }
    }

    #[test]
    fn restyling_maps_luma_ink_onto_a_light_theme() {
        let style = Style::new(get("swiss"), false);
        let draws = style.restyle(vec![Draw::Text(
            "Up next".into(),
            Rect::new(0., 0., 100., 20.),
            TextStyle::new(20.),
            [0.96, 0.96, 0.96, 1.],
            false,
        )]);
        let Draw::Text(_, _, text_style, ink, _) = &draws[0] else {
            panic!("text stays text");
        };
        assert_eq!(*ink, color(0x111111, 1.));
        assert_eq!(text_style.family, ARCHIVO);
    }
}
