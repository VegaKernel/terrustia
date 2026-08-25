//! Finding the worlds somebody already has.
//!
//! `--world` used to take a path and nothing else, so serving a world you own meant pasting
//! `/Users/you/Library/Application Support/Terraria/Worlds/My World.wld` onto a command line —
//! with a space in it. The worlds are always in the same place; the server may as well look.
//!
//! A path is still a path. This is a fallback for a value that plainly is not one.

use std::path::{Path, PathBuf};

/// Where Terraria keeps its worlds on this platform.
///
/// Matches `ReLogic`'s own `PathService`, which is what decides where the game itself saves.
/// Returns `None` when the environment does not say where home is — which happens in containers
/// and in some service managers, and is not an error, just nowhere to look.
pub fn directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(|home| PathBuf::from(home).join("Documents/My Games/Terraria/Worlds"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Terraria/Worlds"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // `$XDG_DATA_HOME` when it is set, and the specified default when it is not.
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|data| data.join("Terraria/Worlds"))
    }
}

/// Turn whatever `--world` was given into a path to open.
///
/// A value with a separator or a `.wld` on it is taken at its word. Anything else is looked for by
/// name in the platform world directory, with and without the extension, case-insensitively —
/// because "The Successful Excrement" is a name a person types, not a filename they remember.
pub fn resolve(given: &str) -> PathBuf {
    let looks_like_a_path = given.contains(std::path::MAIN_SEPARATOR)
        || given.contains('/')
        || given.ends_with(".wld")
        || Path::new(given).exists();
    if looks_like_a_path {
        return PathBuf::from(given);
    }

    let Some(dir) = directory() else {
        return PathBuf::from(given);
    };
    let exact = dir.join(format!("{given}.wld"));
    if exact.exists() {
        return exact;
    }
    // Terraria replaces spaces with underscores when it names the file, so a world called
    // "My World" is `My_World.wld` on disk and nobody remembers that.
    let underscored = dir.join(format!("{}.wld", given.replace(' ', "_")));
    if underscored.exists() {
        return underscored;
    }
    for world in list() {
        if world
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| {
                stem.eq_ignore_ascii_case(given)
                    || stem.replace('_', " ").eq_ignore_ascii_case(given)
            })
        {
            return world;
        }
    }
    // Nothing matched. Hand back what was asked for so the error names it rather than naming a
    // guess the person never made.
    PathBuf::from(given)
}

/// Every `.wld` in the platform world directory, sorted by name.
///
/// Backups are excluded: `.bak1` and friends are ours, and Terraria's own `.wld.bak` is not a
/// world somebody means to serve.
pub fn list() -> Vec<PathBuf> {
    let Some(dir) = directory() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut worlds: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wld")))
        .filter(|p| {
            // `.terrustia.wld` is a copy this server made; listing it beside the original is
            // confusing, and serving it by accident is worse.
            !p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.ends_with(".terrustia"))
        })
        .collect();
    worlds.sort();
    worlds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_taken_at_its_word() {
        let path = format!("some{}where{}World.wld", '/', '/');
        assert_eq!(resolve(&path), PathBuf::from(&path));
        assert_eq!(resolve("Relative.wld"), PathBuf::from("Relative.wld"));
    }

    /// A bare name that matches nothing comes back unchanged, so the error names what was asked
    /// for rather than a guess nobody made.
    #[test]
    fn an_unknown_name_is_returned_as_given() {
        assert_eq!(
            resolve("NoSuchWorldExistsAnywhere"),
            PathBuf::from("NoSuchWorldExistsAnywhere")
        );
    }

    /// The directory is platform-specific and may legitimately be unknown; neither is a panic.
    #[test]
    fn looking_for_the_directory_never_fails() {
        let _ = directory();
        let _ = list();
    }
}
