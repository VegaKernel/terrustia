# terrustia — Justfile
# https://github.com/casey/just
#
# Install: cargo install just  |  brew install just  |  pacman -S just

set shell := ["bash", "-euo", "pipefail", "-c"]

WEB := "crates/terrustia/web-panel"

# ─────────────────────────────────────────
# Default — list all recipes
# ─────────────────────────────────────────

[private]
default:
    @just --list

# ─────────────────────────────────────────
# DEVELOPMENT
# ─────────────────────────────────────────

# Install the dependencies needed for local development
install:
    @command -v cargo >/dev/null || { echo "error: cargo (Rust) is required"; exit 1; }
    @echo "── Rust dependencies ──"
    cargo fetch --locked
    @if command -v bun >/dev/null; then \
        echo "── Web panel dependencies ──"; \
        cd {{WEB}} && bun install --frozen-lockfile; \
    else \
        echo "warning: bun is not installed; the web panel (embed-web) will not build."; \
        echo "  install from https://bun.sh — the server itself runs fine without it."; \
    fi
    @echo "Ready. Next: just run   (or  just dev  to build the panel and embed it first)"

# Run a release server. Extra args pass through, e.g. `just run --new "My World"`
run *ARGS:
    cargo run --release -p terrustia -- {{ARGS}}

# Build the web panel and run a debug server with it embedded — the local iteration loop
dev: web-build
    cargo run -p terrustia --features embed-web

# Web panel dev server with hot reload (serve the panel from disk, not embedded)
web:
    cd {{WEB}} && bun install --frozen-lockfile && bun run dev

# ─────────────────────────────────────────
# BUILD
# ─────────────────────────────────────────

# Full release build — web panel first, then the whole Rust workspace
build: web-build
    cargo build --release --workspace
    @echo "Done → target/release/terrustia"

# Build the web panel assets only (→ crates/terrustia/web-panel/dist)
web-build:
    cd {{WEB}} && bun install --frozen-lockfile && bun run build

# Build the Rust workspace only (assumes web-panel/dist already exists)
rust-build:
    cargo build --release --workspace

# ─────────────────────────────────────────
# CHECKS — mirrors CI
# ─────────────────────────────────────────

# Everything CI runs: Rust format, clippy, supply-chain, tests, and the web build
check: check-rust check-web
    @echo "All checks passed ✓"

# Rust-only checks (faster)
check-rust:
    @echo "── Rust format ──"
    cargo fmt --all --check
    @echo "── Rust clippy (0 warnings) ──"
    cargo clippy --workspace --all-targets -- -D warnings
    @echo "── Supply chain (cargo-deny) ──"
    cargo deny check

# Web panel typecheck + build
check-web:
    cd {{WEB}} && bun install --frozen-lockfile && bun run build

# Format all Rust code
fmt:
    cargo fmt --all

# ─────────────────────────────────────────
# TESTS
# ─────────────────────────────────────────

# Run the whole workspace test suite
test:
    cargo test --workspace

# Run tests matching a filter, with output shown (e.g. `just test-filter fighter`)
test-filter FILTER:
    cargo test --workspace {{FILTER}} -- --nocapture

# The CI soak: a minute (default) of a real server with three real clients
soak SECONDS="60":
    ./tools/soak_ci.sh {{SECONDS}}

# Fuzz a decoder target for a while (needs nightly + `cargo install cargo-fuzz`)
fuzz TARGET="packet_decoders" SECONDS="60":
    cargo +nightly fuzz run {{TARGET}} -- -max_total_time={{SECONDS}}

# ─────────────────────────────────────────
# VERIFY AGAINST REAL TERRARIA
# ─────────────────────────────────────────
# These point at a live server — ours, or a real `TerrariaServer` — to prove the
# protocol and world format against something this project did not write.

# Round-trip a .wld through our loader and saver and report any difference
roundtrip WLD OUT="/tmp/terrustia-roundtrip.wld":
    cargo run --release -p terrustia --example roundtrip_wld -- {{WLD}} {{OUT}}

# Check our decoding against a server's bytes (ours: 7777, real Terraria: its port)
conform ADDR="127.0.0.1:7777" CAPTURE="/tmp/terrustia-conform.trcap":
    cargo run --release -p terrustia-client --example conform -- {{ADDR}} {{CAPTURE}}

# ─────────────────────────────────────────
# DATA TABLES  (dev-only — need a decompiled Terraria tree, not in the repo)
# ─────────────────────────────────────────
# The big data files (recipes.rs, npc_drops.rs, projectile_data.rs, …) are
# *committed* precisely so an ordinary build needs nothing but Rust. They are only
# regenerated from a decompiled Terraria source tree when the game version changes.
# DECOMPILED defaults to where this repo keeps its (gitignored) decompile.

DECOMPILED := ".scratch/decompiled"

# Cross-check the checked-in drop table against the decompiled game
check-drops:
    python3 tools/check_drops.py {{DECOMPILED}}

# Cross-check the checked-in shimmer-decraft recipes against the decompiled game
check-recipes:
    python3 tools/check_recipes.py {{DECOMPILED}} crates/terrustia-proto/src/recipes.rs

# Regenerate every transcribed data table from a decompiled tree, then format
regen:
    python3 tools/gen_recipes.py     {{DECOMPILED}} crates/terrustia-proto/src/recipes.rs
    python3 tools/gen_drops.py       {{DECOMPILED}} crates/terrustia-proto/src/npc_drops.rs
    python3 tools/gen_projectiles.py {{DECOMPILED}} crates/terrustia-proto/src/projectile_data.rs
    python3 tools/gen_banners.py     {{DECOMPILED}} crates/terrustia-proto/src/banners.rs
    python3 tools/gen_buffs.py       {{DECOMPILED}} crates/terrustia-proto/src/buffs.rs
    python3 tools/gen_angler.py      {{DECOMPILED}} crates/terrustia-proto/src/angler.rs
    python3 tools/gen_shimmer.py     {{DECOMPILED}} crates/terrustia-proto/src/shimmer.rs
    cargo run -q -p terrustia-codegen -- hurt_tiles {{DECOMPILED}} crates/terrustia-proto/src/hurt_tiles.rs
    python3 tools/gen_town_names.py  {{DECOMPILED}} crates/terrustia-proto/src/town_names.rs
    python3 tools/gen_travel_shop.py {{DECOMPILED}} crates/terrustia-proto/src/travel_shop.rs
    cargo fmt --all
    @echo "Regenerated the data tables. Review the diff before committing."

# ─────────────────────────────────────────
# PACKAGING
# ─────────────────────────────────────────

# Build the Docker image locally (expects a prebuilt musl binary in dist/, see Dockerfile)
docker-build:
    docker build -t terrustia:dev .

# ─────────────────────────────────────────
# UTILITIES
# ─────────────────────────────────────────

# Remove build and generated artifacts
clean:
    cargo clean
    rm -rf {{WEB}}/dist {{WEB}}/node_modules

# Count lines of source
loc:
    @echo "── Rust ──"
    @find crates -name '*.rs' -not -path '*/target/*' | xargs wc -l | tail -1
    @echo "── Web panel ──"
    @find {{WEB}}/src -type f \( -name '*.ts' -o -name '*.svelte' \) | xargs wc -l | tail -1
