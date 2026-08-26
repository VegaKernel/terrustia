//! `--new <name>` end to end: a real invocation of the compiled binary, not a unit test of the
//! path arithmetic — `worlds::new_world_path` already has those. What isn't proven anywhere else
//! is that `--new` actually reaches the filesystem: that the world it generates is written into
//! wherever this platform's own Terraria keeps its worlds, under the name given, and that asking
//! twice for the same name refuses the second time rather than clobbering the first.
//!
//! Runs the real `terrustia` binary as a subprocess with `HOME`/`XDG_DATA_HOME`/`USERPROFILE` all
//! redirected at a scratch directory — never the machine's real Terraria world directory — so
//! this both proves the CLI wiring and stays entirely inside a directory this test owns and
//! deletes when it is done.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

fn scratch_home() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "terrustia-new-world-cli-{}-{nanos}",
        std::process::id()
    ))
}

/// Every file under `dir` named exactly `name`, found by walking recursively — sidesteps needing
/// to know, in the test itself, which of `worlds::directory`'s three platform branches applies.
fn find_named(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(find_named(&path, name));
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            found.push(path);
        }
    }
    found
}

/// Poll for `name` to show up under `dir` as a non-empty file, rather than sleeping a fixed amount
/// and hoping. A real subprocess's first autosave landing inside a set wall-clock window is
/// inherently load-sensitive — both tests in this file spawn a real OS process each and, by
/// default, run concurrently in the same binary — so a fixed sleep is exactly the kind of "usually
/// enough" timing assumption this project's own testing discipline avoids elsewhere (see
/// `gameplay.rs`'s `deadline`-loop convention, reused here rather than reinvented).
fn wait_for_file(dir: &Path, name: &str, timeout: Duration) -> Vec<PathBuf> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let found = find_named(dir, name);
        if found
            .iter()
            .any(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
        {
            return found;
        }
        if std::time::Instant::now() >= deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Run `terrustia --new <name>` against a scratch home, with autosave fast enough that a short
/// fixed wait is enough to see the file land, then kill it — a clean shutdown's own save path is
/// already covered by the game-level save/reload tests; this only needs the file to exist once.
fn run_new(home: &Path, name: &str, listen: &str) -> std::process::Child {
    // The smallest size `Config::validate` accepts, generated much faster than the default —
    // this test needs the file to land, not a world worth playing in.
    std::fs::write(
        home.join("terrustia.toml"),
        "autosave_secs = 1\nworld_width = 400\nworld_height = 300\n",
    )
    .expect("write config");
    Command::new(env!("CARGO_BIN_EXE_terrustia"))
        .args(["--new", name, "--listen", listen])
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("xdg"))
        .env("USERPROFILE", home)
        .env_remove("TERRUSTIA_LOG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn terrustia")
}

#[test]
fn new_generates_a_world_into_the_platforms_terraria_world_directory() {
    let home = scratch_home();
    std::fs::create_dir_all(&home).expect("scratch home");

    let mut child = run_new(&home, "Fork Test World", "127.0.0.1:17779");
    let found = wait_for_file(&home, "Fork_Test_World.wld", Duration::from_secs(30));
    let _ = child.kill();
    let _ = child.wait();

    assert_eq!(
        found.len(),
        1,
        "expected exactly one Fork_Test_World.wld under {}, found {:?}",
        home.display(),
        found
    );
    assert!(
        std::fs::metadata(&found[0]).is_ok_and(|m| m.len() > 0),
        "the world file should not be empty"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// `--new` must generate a fresh world even when `terrustia.toml` already sets `world_file` to a
/// different, existing world — that config-file value is layered in (`Config::load`, then
/// `apply_env`) before any CLI flag is read, and `--new` used to only redirect where the result is
/// *saved* without ever clearing `world_file`, so the server would silently load and re-save the
/// stale world under the new name instead of generating one. Proven here by size, not just file
/// existence: the stale world and the freshly-requested one are given different dimensions, and
/// the resulting file is loaded back and checked against the *new* config's width, not the stale
/// file's.
#[test]
fn new_ignores_a_stale_world_file_left_in_the_config() {
    let home = scratch_home();
    std::fs::create_dir_all(&home).expect("scratch home");

    // First, generate the "stale" world `terrustia.toml` will point at. A generous timeout: this
    // is the only test in this file that generates two worlds in sequence, each waiting on top of
    // whatever the other tests' own concurrently-running server subprocesses are costing it.
    let mut stale = run_new(&home, "Stale World", "127.0.0.1:17782");
    let stale_found = wait_for_file(&home, "Stale_World.wld", Duration::from_secs(60));
    let _ = stale.kill();
    let _ = stale.wait();
    assert_eq!(
        stale_found.len(),
        1,
        "the stale world should have been written first"
    );
    let stale_path = stale_found[0]
        .to_str()
        .expect("utf8 path")
        .replace('\\', "\\\\");

    // Now point the config at it directly, with a different width, and ask for a new world.
    std::fs::write(
        home.join("terrustia.toml"),
        format!(
            "autosave_secs = 1\nworld_width = 600\nworld_height = 300\nworld_file = \"{stale_path}\"\n"
        ),
    )
    .expect("write config");
    let mut fresh = Command::new(env!("CARGO_BIN_EXE_terrustia"))
        .args(["--new", "Fresh World", "--listen", "127.0.0.1:17783"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("xdg"))
        .env("USERPROFILE", &home)
        .env_remove("TERRUSTIA_LOG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn terrustia");
    let fresh_found = wait_for_file(&home, "Fresh_World.wld", Duration::from_secs(60));
    let _ = fresh.kill();
    let _ = fresh.wait();
    assert_eq!(
        fresh_found.len(),
        1,
        "the fresh world should have been written"
    );

    let loaded = terrustia::world::wld::load(&fresh_found[0]).expect("load the generated world");
    assert_eq!(
        loaded.width(),
        600,
        "--new should generate at the new config's width, not silently load+re-save the stale \
         world_file (which was generated at the old, smaller default width)"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// `Config::validate` skips its own width/height/section-alignment checks entirely whenever
/// `world_file.is_some()` — correct for `--world`, where a loaded world brings its own dimensions,
/// but `Config::load` runs `validate` once *before* `--new` can clear `world_file`, so an
/// out-of-range `world_width`/`world_height` sitting in a config file that also sets `world_file`
/// used to reach real generation completely unvalidated. Proven by giving `--new` a config whose
/// `world_file` would have suppressed the check and whose dimensions are genuinely invalid (below
/// the documented 400x300 floor): this must fail fast with a clear message, not attempt
/// generation at an unvalidated size.
#[test]
fn new_still_validates_dimensions_even_with_a_world_file_set() {
    let home = scratch_home();
    std::fs::create_dir_all(&home).expect("scratch home");
    std::fs::write(
        home.join("terrustia.toml"),
        "world_width = 50\nworld_height = 20\nworld_file = \"anything.wld\"\n",
    )
    .expect("write config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_terrustia"))
        .args(["--new", "Too Small World", "--listen", "127.0.0.1:17784"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("xdg"))
        .env("USERPROFILE", &home)
        .env_remove("TERRUSTIA_LOG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn terrustia");
    let status = child.wait().expect("wait for terrustia");
    assert!(
        !status.success(),
        "an out-of-range world_width/world_height must be refused, not silently generated"
    );
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("captured stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    assert!(
        stdout.contains("must be at least 400x300"),
        "expected a clear size-refusal message on stdout, got: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn new_refuses_a_name_that_already_exists() {
    let home = scratch_home();
    std::fs::create_dir_all(&home).expect("scratch home");

    let mut first = run_new(&home, "Collision World", "127.0.0.1:17780");
    let found = wait_for_file(&home, "Collision_World.wld", Duration::from_secs(30));
    let _ = first.kill();
    let _ = first.wait();
    assert_eq!(
        found.len(),
        1,
        "the first run should have written the world before the second one is tried"
    );

    // A second `--new` under the same name must refuse rather than silently overwrite the first
    // server's world out from under it — the whole reason `--new` checks first.
    let mut second = Command::new(env!("CARGO_BIN_EXE_terrustia"))
        .args(["--new", "Collision World", "--listen", "127.0.0.1:17781"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join("xdg"))
        .env("USERPROFILE", &home)
        .env_remove("TERRUSTIA_LOG")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn terrustia");
    let status = second.wait().expect("wait for the second run");
    assert!(
        !status.success(),
        "a second --new with the same name should fail rather than clobber the first world"
    );
    // `error!()` goes through the same `TermLayer` as every other log line, onto stdout — not
    // stderr, which this process never writes to at all.
    let mut stdout = String::new();
    second
        .stdout
        .take()
        .expect("captured stdout")
        .read_to_string(&mut stdout)
        .expect("read stdout");
    assert!(
        stdout.contains("already exists"),
        "expected a clear refusal on stdout, got: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&home);
}
