//! Where the server keeps its worlds, and how to find one somebody already has.
//!
//! The server owns a `worlds/` directory in its working directory, the way a Minecraft server lays
//! out a `world/` folder wherever it is run. New worlds, whether from `--new`, the setup wizard or
//! the panel, land there rather than in the player's own Terraria save folder, which is not the
//! server's to write into.
//!
//! Serving a world you already own still works. `--world <path>` opens any file, and `--world
//! <name>` looks the name up in `worlds/` first and then in the platform Terraria folder, so a name
//! you type finds the world whether it belongs to the server or to the game.

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

/// The directory the server keeps its own worlds in: `worlds/`, relative to the working directory.
/// This is where a new world is created and where a running world saves, so a server run from a
/// folder lays out its own state there the way a Minecraft server does, rather than in the player's
/// Terraria save folder.
pub fn worlds_dir() -> PathBuf {
    PathBuf::from("worlds")
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

    // The server's own worlds/ directory first, then the player's Terraria folder, so a bare name
    // finds a world the server made or one the game did.
    if let Some(found) = find_by_name(&worlds_dir(), given) {
        return found;
    }
    if let Some(dir) = directory()
        && let Some(found) = find_by_name(&dir, given)
    {
        return found;
    }
    // Nothing matched. Hand back what was asked for so the error names it rather than naming a
    // guess the person never made.
    PathBuf::from(given)
}

/// Find a world called `given` inside `dir`, trying the exact name, then Terraria's own
/// space-to-underscore filename, then a case-insensitive scan. A person types "The Successful
/// Excrement", not the underscored filename they never see.
fn find_by_name(dir: &Path, given: &str) -> Option<PathBuf> {
    for candidate in [format!("{given}.wld"), filename_for(given)] {
        let path = dir.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let matches = path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| {
                stem.eq_ignore_ascii_case(given)
                    || stem.replace('_', " ").eq_ignore_ascii_case(given)
            });
        if matches {
            return Some(path);
        }
    }
    None
}

/// The filename a world called `name` gets, following Terraria's own convention of turning spaces
/// into underscores (see [`resolve`]'s doc comment) — a world created here should look, on disk,
/// exactly like one the game itself made.
fn filename_for(name: &str) -> String {
    format!("{}.wld", name.replace(' ', "_"))
}

/// Where a brand-new world called `name` should be written: into the server's own [`worlds_dir`].
///
/// Refuses a `name` that is not a plain world name (one containing a path separator or a `..`
/// segment) rather than silently joining it onto the directory, since a name that reaches outside
/// `worlds/` is a mistake worth stopping on. Anyone who wants an exact file elsewhere has `--save
/// <path>` for that. The caller creates the directory; this only decides the path.
pub fn new_world_path(name: &str) -> Result<PathBuf, String> {
    if name.trim().is_empty() {
        return Err("a world needs a name".into());
    }
    if name.contains('/') || name.contains('\\') || name.split(['/', '\\']).any(|s| s == "..") {
        return Err(format!(
            "\"{name}\" is not a plain world name; pick a name with no path in it, or use \
             --save <path> for an exact location"
        ));
    }
    Ok(worlds_dir().join(filename_for(name)))
}

/// Every `.wld` the server owns, in [`worlds_dir`], sorted by name.
///
/// Backups are excluded: `.bak1` and friends are ours, and a `.terrustia.wld` copy is not a world
/// somebody means to serve on its own.
pub fn list() -> Vec<PathBuf> {
    let dir = worlds_dir();
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

    #[test]
    fn a_new_world_name_gets_terrarias_own_underscored_filename() {
        assert_eq!(
            filename_for("The Successful Excrement"),
            "The_Successful_Excrement.wld"
        );
        assert_eq!(filename_for("NoSpaces"), "NoSpaces.wld");
    }

    #[test]
    fn an_empty_new_world_name_is_refused() {
        assert!(new_world_path("").is_err());
        assert!(new_world_path("   ").is_err());
    }

    /// `--new` writes into the world directory itself; a name carrying its own path would either
    /// escape it (an absolute-looking segment silently replaces the base in `Path::join`) or land
    /// somewhere inside it nobody asked for. Both are refused rather than followed.
    #[test]
    fn a_new_world_name_with_a_path_in_it_is_refused() {
        for name in [
            "a/b",
            "a\\b",
            "../escape",
            "sub/../../etc/passwd",
            "/etc/passwd",
        ] {
            assert!(new_world_path(name).is_err(), "{name} should be refused");
        }
    }

    #[test]
    fn a_new_world_lands_in_the_servers_worlds_dir() {
        let path = new_world_path("My Fork World").unwrap();
        assert_eq!(path, worlds_dir().join("My_Fork_World.wld"));
        assert!(
            path.starts_with("worlds"),
            "a new world belongs to the server, not the Terraria folder: {path:?}"
        );
    }
}
