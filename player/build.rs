//! Bakes `config.local.json` (gitignored) into the bundle, or `config.example.json` when there
//! is none yet, so a fresh checkout still builds.

use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let local = dir.join("config.local.json");
    let example = dir.join("config.example.json");
    println!("cargo:rerun-if-changed={}", local.display());
    println!("cargo:rerun-if-changed={}", example.display());
    let source = if local.exists() { &local } else { &example };
    let text = std::fs::read_to_string(source).expect("read the player config");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("config.json");
    std::fs::write(out, text).expect("write the player config");
    if !local.exists() {
        println!(
            "cargo:warning=player/config.local.json is missing: building with config.example.json"
        );
    }
}
