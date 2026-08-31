//! Keep the count of things that can panic honest.
//!
//! `docs/performance.md` used to claim there were three `unwrap`/`expect` calls outside tests, and
//! named them. There were seven. Nobody had lied: the sentence was true when it was written and
//! nothing kept it true afterwards, which is the failure mode of every number written in prose.
//!
//! A panic is not a small thing here. The game is a single actor task with no `catch_unwind`, and
//! the shutdown save runs *inside* that task — so a panic does not degrade the server, it loses
//! the world back to the last autosave.
//!
//! This test does not forbid them. It pins the number, so adding one is a deliberate act with a
//! failing test attached rather than a line nobody notices.

use std::path::{Path, PathBuf};

/// Every site outside a `#[cfg(test)]` module that can panic on purpose.
///
/// Raise this only alongside a comment at the new site saying which invariant makes it safe.
///
/// 11, up from 10: `tile_cleanup.rs`'s `gravitating_sand_cleanup` gained one, joining the worker
/// threads its own column-band parallelization spawns — see that call site's own comment for the
/// invariant (`gravitating_sand_column_range` only reads `World::tile`, which never panics, and
/// pushes to a `Vec`, so the joined thread cannot actually have panicked).
///
/// 14, net up from 11 (2026-08-29): the audit wave's liquid L1 merge-semantics rewrite added
/// four sites in `liquid.rs` (three `unreachable!` match arms, each guarded by an explicit
/// `this_kind != X` check two lines above, and one `max()/min().unwrap()` over a slice whose
/// length starts at 1 and only grows; each carries its invariant at the site) and the worldgen
/// traps rewrite one (`kind_type` is rolled from a fixed range, the invariant is in the message),
/// while the same wave's cleanups removed two older sites. This surfaced only now because the
/// audit-wave branch's CI never ran as a whole before it merged to main.
///
/// 12, down from 14 (Lane B, error handling and data safety): `config.rs`'s two
/// `"0.0.0.0:7777".parse().expect("valid default address")` calls are gone, replaced by
/// `SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 7777)` and its loopback twin. Neither could
/// ever have fired - but "could never have fired" is a claim about a string literal that has to be
/// re-checked every time somebody edits it, and building the address instead removes the claim
/// rather than restating it. Nothing else in that lane's scope had a site to remove; `record.rs`'s
/// two and `reader.rs`'s one are in files it did not own.
///
/// 11, down from 12 (Fix lane B, liquid and world runtime): `liquid.rs`'s
/// `levels.iter().max().unwrap() - levels.iter().min().unwrap()` is gone with the "flat to within
/// a drop, leave it alone" tolerance it computed. That tolerance was standing in for a spare-unit
/// tie-break that `level` now does directly, so the whole expression, and its invariant claim
/// about a slice whose length starts at 1, went with it. The three `unreachable!` arms in
/// `merge_result` are untouched.
///
/// 12, up from 11 (2026-08-31): no site was added. `panic_sites` used to truncate each file at its
/// first `#[cfg(test)]`, which hid every production line below one, and one real site was hiding
/// there: `game/ai/mod.rs:1253`, `unreachable!("style {style} claims parity but has no routine
/// here")`, in a file whose first test module opens at line 391. That arm is defended - the roster
/// test at `ai/mod.rs:607-609` walks all 691 NPC types to prove no `ai_style` reaches it - but the
/// budget is supposed to count defended sites, not omit them. The checker is now brace-aware and
/// the number is the honest one; see `panic_sites` below for what the old truncation cost.
///
/// Lower this whenever a site genuinely goes away, in the same commit. Never raise it without a
/// comment at the new site saying which invariant makes it safe.
const ALLOWED: usize = 12;

fn crate_roots() -> Vec<PathBuf> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        here.join("src"),
        here.parent()
            .expect("crates/")
            .join("terrustia-proto")
            .join("src"),
    ]
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Count panic sites in one file, skipping each `#[cfg(test)]` item and resuming after it.
///
/// This used to truncate the file at the first `#[cfg(test)]`, on the stated grounds that test
/// modules are conventionally last here. 24 files do not follow that convention: `systems.rs`
/// carries ~2,775 production lines after its first test module, `npc_params.rs` ~3,656,
/// `server/mod.rs` ~2,045, `spawn.rs` ~1,983, `dispatch.rs` ~1,161, `ai/mod.rs` ~847. Roughly
/// 20,000 production lines were never scanned, so an `.unwrap()` added below any of those points
/// kept this test green, which is the exact opposite of what it exists to do.
///
/// Skipping needs no Rust parser, only the two shapes the attribute takes here: a braced item
/// (`mod tests { … }`, a `#[cfg(test)] fn`, an enum variant, a multi-line match arm), or one that
/// ends at a `;` or `,` with no brace at all (`mod liquid_faithful;`, a `const`, a single-line
/// match arm - `console.rs:44`'s `__panic_probe` is one, and it contains a literal `panic!`).
/// Track braces from the item's first `{` and resume once it closes; with no brace, resume after
/// the terminator. Braces inside string literals are counted too, and are balanced in every
/// literal in the tree today; if that ever stops being true the count moves and this test says so,
/// which is the failure direction to have.
fn panic_sites(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // `#![cfg(test)]` makes the whole file test-only (`world/liquid_faithful.rs`).
    if text.contains("#![cfg(test)]") {
        return Vec::new();
    }
    let mut sites = Vec::new();
    let mut skipping = false;
    let mut depth = 0i32;
    let mut opened = false;
    for line in text.lines().map(str::trim) {
        let item = match (skipping, line.split_once("#[cfg(test)]")) {
            (true, _) => Some(line),
            (false, Some((_, rest))) => {
                skipping = true;
                depth = 0;
                opened = false;
                Some(rest)
            }
            (false, None) => None,
        };
        if let Some(item) = item {
            let opens = item.matches('{').count() as i32;
            opened |= opens > 0;
            depth += opens - item.matches('}').count() as i32;
            skipping = if opened {
                depth > 0
            } else {
                !item.ends_with(';') && !item.ends_with(',')
            };
            continue;
        }
        if line.starts_with("//") {
            continue;
        }
        if line.contains(".unwrap()")
            || line.contains(".expect(")
            || line.contains("panic!(")
            || line.contains("unreachable!(")
        {
            sites.push(format!(
                "{}: {line}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
    }
    sites
}

#[test]
fn the_number_of_ways_to_panic_is_known() {
    let mut files = Vec::new();
    for root in crate_roots() {
        rust_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "found no source to scan");

    let mut sites: Vec<String> = files.iter().flat_map(|f| panic_sites(f)).collect();
    sites.sort();

    assert_eq!(
        sites.len(),
        ALLOWED,
        "the panic budget moved. A panic loses the world back to the last autosave, so each of \
         these needs an invariant that makes it unreachable — and a comment saying which.\n  {}",
        sites.join("\n  "),
    );
}
