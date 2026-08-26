//! A basic, interactive first-run setup wizard.
//!
//! Not the zero-flag default path — that stays exactly what it always was ("point it at nothing
//! and it generates a world," non-interactive, no prompts) — this is a *separate*, opt-in-
//! triggered flow for someone who wants a little more hand-holding: `terrustia --setup`
//! explicitly, or a first-run launch this module judges likely to be someone who just downloaded
//! the raw binary and ran it right where it landed (see [`should_auto_trigger`]).
//!
//! What it produces is nothing more exotic than an ordinary `terrustia.toml` — the wizard is a
//! way of writing one without knowing the file format, not a second configuration mechanism.
//! Once it has written that file, `main` proceeds exactly as if `--config <the file it wrote>`
//! had been passed, so every other precedence rule (environment, CLI flags) keeps working
//! unchanged from that point on.
//!
//! **The install-guard property this exists to provide**: double-clicking the raw binary must
//! never scatter a world file and a `terrustia.toml` into wherever it happens to sit (`~/Downloads`,
//! most likely). The wizard's dedicated directory holds the config; the world itself is generated
//! through [`crate::worlds::new_world_path`] — Terraria's own real world directory, the same place
//! `--new` already writes to — so *neither* file lands beside the executable. The dedicated
//! directory is refused outright if it already has anything in it, rather than writing into
//! whatever is already there.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::config::Config;

/// Whether a plain, zero-flag `terrustia` invocation should offer the wizard instead of falling
/// through to the ordinary non-interactive default (generate an ephemeral world, serve it).
///
/// The signal: the current directory is the same directory the running executable itself lives
/// in, and nothing terrustia-shaped is in it yet (no `terrustia.toml`, no `.wld` file). That
/// combination is what "downloaded a release archive, extracted it, and ran `./terrustia` from
/// right there" actually looks like at the process level — genuinely different from "installed
/// via a package manager to `/usr/local/bin`, then `cd`'d into `~/my-server` and ran `terrustia`
/// with no flags," which is the existing, legitimate zero-config path and must keep working
/// exactly as it always has. A double-clicked binary on Windows/Linux desktops lands in this same
/// shape (its own directory becomes the process's working directory), which is the actual thing
/// this project's own plan calls out by name.
pub fn should_auto_trigger(args_are_empty: bool) -> bool {
    if !args_are_empty {
        return false;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(exe_dir) = exe.parent() else {
        return false;
    };
    // Canonicalize both sides: `current_exe()` on some platforms returns a path through a symlink
    // (Homebrew's Cellar, for one), and a naive `==` would then never match even when the two
    // really are the same directory.
    let same_dir = match (cwd.canonicalize(), exe_dir.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => cwd == exe_dir,
    };
    if !same_dir {
        return false;
    }
    if cwd.join("terrustia.toml").exists() {
        return false;
    }
    has_no_world_files(&cwd)
}

fn has_no_world_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    !entries.flatten().any(|e| {
        e.path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wld"))
    })
}

#[derive(Debug, thiserror::Error)]
pub enum WizardError {
    #[error("{0}")]
    Cancelled(String),
    #[error("reading input: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Invalid(String),
}

/// Runs the interactive prompts on stdin/stdout and returns the path to the `terrustia.toml` it
/// wrote. Blocking by design — there is nothing else running yet for it to block, this is the
/// very first thing `main` does when it triggers — callers on an async runtime should still run
/// it inside `spawn_blocking` regardless, so a future caller earlier in the startup sequence does
/// not accidentally stall a worker thread other tasks depend on.
pub fn run_wizard() -> Result<PathBuf, WizardError> {
    println!("terrustia setup");
    println!("===============");
    println!(
        "A few questions, then this writes a terrustia.toml and gets out of your way. Press \
         Enter to accept the default shown in [brackets]."
    );
    println!();

    let default_dir = default_dedicated_dir();
    let dir = prompt(
        &format!(
            "Directory for terrustia's own config (must be empty or not yet exist) [{}]",
            default_dir.display()
        ),
        &default_dir.display().to_string(),
    )?;
    let dir = PathBuf::from(dir);
    ensure_empty_or_new(&dir)?;

    let world_name = prompt("World name", "Terrustia")?;
    let max_players = prompt_number("Max players", 8, 1, crate::config::MAX_PLAYERS)?;
    let panel_enabled = prompt_yes_no(
        "Enable the web admin panel (browser-based start/stop, player list, whitelist, \
         settings)? It only ever listens on this machine, never the network",
        true,
    )?;

    let world_path = crate::worlds::new_world_path(&world_name).map_err(WizardError::Invalid)?;
    if let Some(parent) = world_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if world_path.exists() {
        return Err(WizardError::Invalid(format!(
            "a world named \"{world_name}\" already exists at {} — run terrustia --setup again \
             with a different name, or --world \"{world_name}\" to serve the one you already have",
            world_path.display()
        )));
    }

    std::fs::create_dir_all(&dir)?;
    let config = Config {
        world_name: world_name.clone(),
        save_file: Some(world_path.clone()),
        max_players,
        panel_enabled,
        ..Config::default()
    };
    let config_path = dir.join("terrustia.toml");
    write_config(&config_path, &config)?;

    println!();
    println!("Written to {}", config_path.display());
    println!(
        "The world \"{world_name}\" will be generated at {} on first start.",
        world_path.display()
    );
    println!("Starting terrustia now — Ctrl-C to stop it, same as any other run.");
    println!();

    Ok(config_path)
}

fn default_dedicated_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.unwrap_or_else(|| PathBuf::from("."))
        .join("terrustia-server")
}

/// Refuses outright if `dir` exists and already has anything in it — the whole point of a
/// *dedicated* directory is that it is not also wherever a stray download or an old install left
/// something behind. Creating it is left to the caller, once every other prompt has succeeded
/// too, so an early cancellation or a later mistake never leaves a half-set-up empty directory.
fn ensure_empty_or_new(dir: &Path) -> Result<(), WizardError> {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(WizardError::Invalid(format!(
                    "{} already exists and is not empty — pick an empty or new directory, so \
                     nothing already there is at risk of being overwritten",
                    dir.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(WizardError::Io(e)),
    }
}

fn write_config(path: &Path, config: &Config) -> Result<(), WizardError> {
    // Only the fields the wizard actually asked about, written explicitly — a full
    // `toml::to_string(config)` would also freeze every other field's current default into the
    // file forever, silently opting this install out of any default this project changes later.
    let mut out = String::new();
    out.push_str(
        "# Written by `terrustia --setup`. Every key is optional — see \
                   terrustia.toml.example for the rest.\n\n",
    );
    out.push_str(&format!("world_name = {:?}\n", config.world_name));
    out.push_str(&format!(
        "save_file = {:?}\n",
        config
            .save_file
            .as_deref()
            .unwrap_or(Path::new(""))
            .display()
    ));
    out.push_str(&format!("max_players = {}\n", config.max_players));
    out.push_str(&format!("panel_enabled = {}\n", config.panel_enabled));
    std::fs::write(path, out)?;
    Ok(())
}

fn prompt(question: &str, default: &str) -> Result<String, WizardError> {
    print!("{question} [{default}]: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_number(
    question: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, WizardError> {
    loop {
        let value = prompt(question, &default.to_string())?;
        match value.parse::<usize>() {
            Ok(n) if n >= min && n <= max => return Ok(n),
            _ => println!("please enter a number between {min} and {max}"),
        }
    }
}

fn prompt_yes_no(question: &str, default: bool) -> Result<bool, WizardError> {
    let default_str = if default { "Y/n" } else { "y/N" };
    loop {
        let value = prompt(question, default_str)?.to_lowercase();
        match value.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            v if v == default_str.to_lowercase() => return Ok(default),
            _ => println!("please answer y or n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_that_does_not_match_the_executables_own_never_auto_triggers() {
        // `should_auto_trigger` reads the *real* `current_exe`/`current_dir` — in a `cargo test`
        // run those are never the same directory (the test binary lives under `target/debug/
        // deps/`, while the working directory `cargo test` sets is the crate root), so this
        // exercises the real function against its real environment rather than a mock of it: the
        // "cwd == exe's own directory" guard must read false here, and nothing after it should
        // ever run (in particular, it must not touch the filesystem looking for `.wld` files).
        assert!(!should_auto_trigger(true));
    }

    #[test]
    fn args_present_always_skips_auto_trigger_without_touching_the_filesystem() {
        assert!(!should_auto_trigger(false));
    }

    #[test]
    fn an_empty_directory_is_accepted_and_a_nonexistent_one_too() {
        let dir =
            std::env::temp_dir().join(format!("terrustia-setup-test-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Does not exist yet:
        ensure_empty_or_new(&dir).expect("a directory that does not exist yet must be accepted");
        // Exists and is empty:
        std::fs::create_dir_all(&dir).unwrap();
        ensure_empty_or_new(&dir).expect("an existing empty directory must be accepted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_nonempty_directory_is_refused_rather_than_written_into() {
        let dir = std::env::temp_dir().join(format!(
            "terrustia-setup-test-nonempty-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("something-already-here.txt"), b"do not touch").unwrap();
        let result = ensure_empty_or_new(&dir);
        assert!(
            result.is_err(),
            "a directory with something already in it must be refused"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("something-already-here.txt")).unwrap(),
            "do not touch",
            "the pre-existing file must be completely untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn has_no_world_files_is_true_only_when_none_exist() {
        let dir =
            std::env::temp_dir().join(format!("terrustia-setup-test-wld-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(has_no_world_files(&dir), "a fresh empty directory has none");
        std::fs::write(dir.join("SomeWorld.wld"), b"").unwrap();
        assert!(
            !has_no_world_files(&dir),
            "a directory with a .wld file must be detected"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_written_config_round_trips_through_the_real_toml_parser() {
        let dir =
            std::env::temp_dir().join(format!("terrustia-setup-test-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("terrustia.toml");
        let config = Config {
            world_name: "A Test World".to_string(),
            save_file: Some(dir.join("A_Test_World.wld")),
            max_players: 16,
            panel_enabled: true,
            ..Config::default()
        };
        write_config(&config_path, &config).unwrap();

        let loaded = Config::load(&config_path).expect("the file this wizard wrote must load");
        assert_eq!(loaded.world_name, "A Test World");
        assert_eq!(loaded.save_file, config.save_file);
        assert_eq!(loaded.max_players, 16);
        assert!(loaded.panel_enabled);
        // Everything the wizard did not ask about keeps its ordinary default.
        assert_eq!(loaded.autosave_secs, Config::default().autosave_secs);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
