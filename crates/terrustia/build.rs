//! Ensures `web-panel/dist/` exists before `rust-embed`'s `#[derive(RustEmbed)]` looks for it.
//!
//! That macro fails the whole crate's compile — not just the panel — if the folder it names is
//! missing entirely, and `dist/` is gitignored (it's build output, not source): a fresh checkout
//! that hasn't run `web-panel`'s own `npm run build` yet has no such directory. CI/release builds
//! it for real first (see the workflows under `.github/workflows/`); this is the fallback so a
//! local `cargo build` — someone who hasn't touched the frontend at all — doesn't hard-fail over a
//! subsystem they may not even be using (the panel is opt-in and off by default). An empty `dist/`
//! embeds zero real files, so the panel would serve nothing useful until it's actually built, but
//! the rest of the server compiles and runs fine either way.
//!
//! Same shape as `../alchemist`'s own `build.rs`, which solves the identical problem for the
//! identical `rust-embed` pattern.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=web-panel/dist");
    if env::var_os("CARGO_FEATURE_EMBED_WEB").is_none() {
        return;
    }
    let dist_dir = Path::new("web-panel/dist");
    if let Err(err) = fs::create_dir_all(dist_dir) {
        panic!("failed to create web-panel/dist for the embed-web feature: {err}");
    }
}
