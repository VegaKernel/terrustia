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

# Everything CI runs: Rust format, clippy (both feature sets), supply-chain, the web build, tests
check: check-rust check-web test
    @echo "All checks passed ✓"

# Rust-only checks (faster). No tests: `just check` adds those, this is the lint pass on its own.
check-rust:
    @echo "── Rust format ──"
    cargo fmt --all --check
    @echo "── Rust clippy (0 warnings) ──"
    cargo clippy --workspace --all-targets -- -D warnings
    @# `embed-web` is default-on, so the pass above never compiles the panel's disk-serving branch
    @# (`panel/mod.rs::load_static_asset`'s `#[cfg(not(feature = "embed-web"))]` arm) or the `..`
    @# traversal guard inside it. CI lints it separately (ci.yml) and so does this.
    @echo "── Rust clippy, embed-web off (the disk-serving panel branch) ──"
    cargo clippy -p terrustia --all-targets --no-default-features -- -D warnings
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

# The README's comparison table, measured: startup, idle CPU, idle RAM and idle bandwidth against
# the real TerrariaServer on one shared world. Needs the real server and a QUIET machine; it warns
# and refuses to call its own output publishable if the load average says otherwise.
compare SECONDS="300":
    COMPARE_WINDOW={{SECONDS}} ./tools/compare_vanilla.sh

# Play the game toward a player's goals and report the ones that could not be reached.
# Owns the server's lifecycle, because two of the goals are about surviving a save and reload.
playbot:
    cargo build --release -p terrustia -p terrustia-client --bins --examples
    ./tools/playbot.sh

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

# `dead_code` cannot see these, because a read inside `#[cfg(test)]` counts as a read: a field the
# tests assert on and production ignores is invisible to it. Needs no decompiled tree; it is here
# because it belongs to the same qualification pass, not because it needs the game.
#
# Report struct fields written in production and read only by the tests
check-dead-writes:
    cargo run -q -p terrustia-codegen --bin deadwrite

# AGENTS.md rule 2 makes every transcription cite its source, so the tree holds ~1900
# `NPC.cs:12345` references. `docs/parity-index.tsv` is derived from them and is never hand-edited:
# a hand-maintained parity ledger that rots does not say "unknown", it says "verified". Each entry
# carries a hash of the cited vanilla lines and a hash of our own item's body, so a claim expires on
# its own the moment either side moves, and this says which side that was. It answers "is this still
# the code it was checked against" and "what is cited by nothing", never "is this right".
#
# Rebuild the index with `parity-update` and review the diff, exactly as with the generated tables.
# A regenerated decompiled tree moves every line number at once; that is reported as one sentence
# rather than 1900 drifts.
#
# Check every vanilla citation against the decompiled tree, and report what is cited by nothing
check-parity:
    python3 tools/parity_index.py --self-test
    python3 tools/parity_index.py {{DECOMPILED}}

# What fraction of each vanilla source file anything this project wrote has actually cited
parity-coverage:
    python3 tools/parity_index.py {{DECOMPILED}} --coverage

# Rebuild docs/parity-index.tsv from the citations in the source. Review the diff before committing
parity-update:
    python3 tools/parity_index.py {{DECOMPILED}} --update

# A surviving mutant is a blind spot in a checker, which is how a missing drop stays missing. Pass
# `--rust` to also measure the proto test suite, at a rebuild per mutant.
#
# Corrupt the generated tables and prove the checkers above actually fail
check-mutants *ARGS:
    python3 tools/mutate_tables.py {{DECOMPILED}} {{ARGS}}

# Part of release-candidate qualification, run locally against the decompiled tree: CI can never
# hold decompiled game source, so these deliberately never run there.
#
# `check-dead-writes` runs last on purpose: it is the one that currently fails (21 fields written in
# production and read by nothing), and `just` stops a chain at the first failure. Putting it at the
# end means the three that pass still report before it does.
#
# Every data cross-check in one go: the tables, the citations, the dead writes, and the checkers
check-data: check-drops check-recipes check-parity check-mutants check-dead-writes

# Regenerate every transcribed data table from a decompiled tree, then format
regen:
    cargo run -q -p terrustia-codegen -- recipes {{DECOMPILED}} crates/terrustia-proto/src/recipes.rs
    cargo run -q -p terrustia-codegen -- drops       {{DECOMPILED}} crates/terrustia-proto/src/npc_drops.rs
    cargo run -q -p terrustia-codegen -- projectiles {{DECOMPILED}} crates/terrustia-proto/src/projectile_data.rs
    cargo run -q -p terrustia-codegen -- banners     {{DECOMPILED}} crates/terrustia-proto/src/banners.rs
    cargo run -q -p terrustia-codegen -- buffs       {{DECOMPILED}} crates/terrustia-proto/src/buffs.rs
    cargo run -q -p terrustia-codegen -- angler      {{DECOMPILED}} crates/terrustia-proto/src/angler.rs
    cargo run -q -p terrustia-codegen -- shimmer     {{DECOMPILED}} crates/terrustia-proto/src/shimmer.rs
    cargo run -q -p terrustia-codegen -- hurt_tiles {{DECOMPILED}} crates/terrustia-proto/src/hurt_tiles.rs
    cargo run -q -p terrustia-codegen -- town_names {{DECOMPILED}} crates/terrustia-proto/src/town_names.rs
    cargo run -q -p terrustia-codegen -- travel_shop {{DECOMPILED}} crates/terrustia-proto/src/travel_shop.rs
    cargo run -q -p terrustia-codegen -- tile_death  {{DECOMPILED}} crates/terrustia-proto/src/tile_death.rs
    cargo fmt --all
    @echo "Regenerated the data tables. Review the diff before committing."

# ─────────────────────────────────────────
# PACKAGING
# ─────────────────────────────────────────

# Build the Docker image locally (expects a prebuilt musl binary in dist/, see Dockerfile)
docker-build:
    docker build -t terrustia:dev .

# ─────────────────────────────────────────
# PUBLISH  (crates.io)
# ─────────────────────────────────────────
# Only terrustia-proto is published to crates.io: it is the MIT wire-format library, free for any
# Terraria tool to build on. The server crate is AGPL and application-shaped, not a library, so it
# is not published here.

# List exactly what would go into the terrustia-proto package — catches a stray or missing file
publish-proto-list:
    cargo package -p terrustia-proto --locked --list

# Dry run: build and pack terrustia-proto the way crates.io will, without uploading anything
publish-proto-dry:
    cargo publish -p terrustia-proto --locked --dry-run
    @echo "Dry run OK. Publish for real with: just publish-proto"

# Publish terrustia-proto to crates.io (needs a prior `cargo login`). Dry-runs first as a guard.
publish-proto: publish-proto-dry
    cargo publish -p terrustia-proto --locked
    @echo "Published terrustia-proto → https://crates.io/crates/terrustia-proto"

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
