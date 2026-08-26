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
    std::thread::sleep(Duration::from_secs(12));
    let _ = child.kill();
    let _ = child.wait();

    let found = find_named(&home, "Fork_Test_World.wld");
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

#[test]
fn new_refuses_a_name_that_already_exists() {
    let home = scratch_home();
    std::fs::create_dir_all(&home).expect("scratch home");

    let mut first = run_new(&home, "Collision World", "127.0.0.1:17780");
    std::thread::sleep(Duration::from_secs(12));
    let _ = first.kill();
    let _ = first.wait();
    assert_eq!(
        find_named(&home, "Collision_World.wld").len(),
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
