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

/// The filename a world called `name` gets, following Terraria's own convention of turning spaces
/// into underscores (see [`resolve`]'s doc comment) — a world created here should look, on disk,
/// exactly like one the game itself made.
fn filename_for(name: &str) -> String {
    format!("{}.wld", name.replace(' ', "_"))
}

/// Where a brand-new world called `name` should be written, so `--new` lands it next to every
/// world Terraria already has rather than in some path nobody would think to look.
///
/// Refuses a `name` that is not a plain world name — one containing a path separator or a `..`
/// segment — rather than silently joining it onto the world directory: `--new` is one command for
/// one thing, "generate a fresh world here, called this," and a name that reaches outside the
/// world directory is a mistake worth stopping on rather than a path worth following. Anyone who
/// wants an exact file elsewhere already has `--save <path>` for that.
pub fn new_world_path(name: &str) -> Result<PathBuf, String> {
    if name.trim().is_empty() {
        return Err("a world needs a name".into());
    }
    if name.contains('/') || name.contains('\\') || name.split(['/', '\\']).any(|s| s == "..") {
        return Err(format!(
            "\"{name}\" is not a plain world name; --new writes into the Terraria world \
             directory itself, so pick a name with no path in it, or use --save <path> instead"
        ));
    }
    let dir = directory().ok_or_else(|| {
        "cannot find the Terraria world directory on this system; use --save <path> instead of \
         --new"
            .to_string()
    })?;
    Ok(dir.join(filename_for(name)))
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
    fn a_plain_new_world_name_is_accepted_whenever_the_directory_is_known() {
        // `directory()` depends on the environment, so this only asserts when it resolves —
        // matching `looking_for_the_directory_never_fails` above, which does the same.
        if let Some(dir) = directory() {
            let path = new_world_path("My Fork World").unwrap();
            assert_eq!(path, dir.join("My_Fork_World.wld"));
        }
    }
}
