//! Logic shared by Luma (ARM, native) and Luma Player (wasm32): no I/O, no clocks.
//!
//! Requests are built as [`http::Request`] values and replies are parsed from JSON text; each
//! app sends them its own way (ureq on the TV, `fetch` in the player). Time comes in as
//! milliseconds since the epoch, so every state machine here runs in plain `cargo test`.

pub mod http;
pub mod ids;
pub mod launch;
pub mod next;
pub mod remote;
pub mod resume;
pub mod scrobble;
pub mod segments;
pub mod selection;
pub mod stremio;
pub mod subs;
pub mod time;
#[cfg(feature = "json")]
pub mod tmdb;
#[cfg(feature = "json")]
pub mod trakt;
