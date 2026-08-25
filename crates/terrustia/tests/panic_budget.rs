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
const ALLOWED: usize = 7;

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

/// Count panic sites in one file, stopping at the first `#[cfg(test)]`.
///
/// Test modules are conventionally last in this codebase, so truncating there is enough and
/// avoids needing to parse Rust to find where the module ends.
fn panic_sites(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let body = text.split("#[cfg(test)]").next().unwrap_or_default();
    body.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .filter(|line| {
            line.contains(".unwrap()")
                || line.contains(".expect(")
                || line.contains("panic!(")
                || line.contains("unreachable!(")
        })
        .map(|line| {
            format!(
                "{}: {line}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        })
        .collect()
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
