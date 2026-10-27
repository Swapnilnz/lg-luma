//! Luma Player: a webOS web app, Rust compiled to WebAssembly. Luma launches it with IDs
//! (see `luma_core::launch`); it finds streams on AIOStreams, plays them on LG's hardware
//! pipeline, and keeps Trakt in sync.

mod app;
mod config;
mod devlog;
mod dom;
mod net;
mod store;
mod ui;
mod video;

fn main() {
    app::start();
}
