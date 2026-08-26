//! A real `SIGTERM` against a real running server actually stops it — not just logs "shutting
//! down" and keeps ticking forever.
//!
//! Found by hand while verifying `packaging/terrustia.service`'s `ExecStart` path actually works
//! end to end (not just that the unit file parses): `main.rs` spawns `panel::supervise` with a
//! bare `tokio::spawn`, never storing or aborting its `JoinHandle` — but that task holds its own
//! clone of `events_tx` for as long as it runs, so `main`'s own `drop(events_tx)` during shutdown
//! was never actually dropping the *last* sender. `GameServer::run`'s only clean-exit path is
//! `events.recv() => None => break` (`game/server.rs`), which needs every sender gone — so a real
//! SIGTERM logged "shutting down" and then the game loop kept ticking and autosaving forever,
//! never actually stopping, until `packaging/terrustia.service`'s own `TimeoutStopSec=90` would
//! eventually force a hard kill — defeating the graceful shutdown save that whole unit's hardening
//! is built around. Fixed by aborting `panel::supervise`'s handle alongside `accept`/`console` in
//! `main`'s existing shutdown sequence — see that call site's own comment for the full story.
//!
//! This test sends a real `SIGTERM` to a real running subprocess and asserts it actually exits
//! within a bounded window, with a real shutdown save on disk — the unfixed code hung
//! indefinitely at exactly this point (observed directly: 27+ seconds and counting, autosaving on
//! its ordinary 1-second interval throughout, before being killed by hand).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

fn scratch_home() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "terrustia-shutdown-signal-{}-{nanos}",
        std::process::id()
    ))
}

fn stream_stdout_lines(stdout: std::process::ChildStdout) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for_line(rx: &mpsc::Receiver<String>, needle: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match rx.recv_timeout(remaining) {
            Ok(line) if line.contains(needle) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

fn spawn_server(home: &std::path::Path, listen: &str) -> Child {
    let save_file = home.join("ShutdownSignalTest.wld");
    std::fs::write(
        home.join("terrustia.toml"),
        format!(
            "autosave_secs = 300\nworld_width = 400\nworld_height = 300\nlisten = \"{listen}\"\n\
             save_file = {save_file:?}\n"
        ),
    )
    .expect("write config");
    Command::new(env!("CARGO_BIN_EXE_terrustia"))
        .current_dir(home)
        .env_remove("TERRUSTIA_LOG")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn terrustia")
}

/// A 300-second `autosave_secs` (the real default) means the world file can only ever land on
/// disk two ways: the periodic autosave, decades from now as far as this test is concerned, or a
/// clean shutdown. That makes this the tightest possible pin against the exact bug: on the
/// unfixed code, nothing would ever have written this file at all within this test's window.
#[test]
fn sigterm_stops_the_server_and_saves_within_a_bounded_window() {
    let home = scratch_home();
    std::fs::create_dir_all(&home).expect("scratch home");

    let mut child = spawn_server(&home, "127.0.0.1:17796");
    let stdout_lines = stream_stdout_lines(child.stdout.take().expect("piped stdout"));

    assert!(
        wait_for_line(
            &stdout_lines,
            "accepting connections",
            Duration::from_secs(30)
        ),
        "the server should have reached its main loop by now"
    );

    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    // The unfixed code hung here indefinitely — this timeout is generous next to the sub-second
    // shutdown actually measured by hand (~150ms from "shutting down" to "game loop stopped"),
    // not a guess at how long a fixed version might reasonably take.
    #[cfg(unix)]
    {
        assert!(
            wait_for_line(&stdout_lines, "game loop stopped", Duration::from_secs(15)),
            "the server must actually stop after SIGTERM, not just log \"shutting down\" and \
             keep running"
        );
    }

    let status = child
        .wait_timeout_or_kill(Duration::from_secs(10))
        .expect("the process must exit on its own after a graceful SIGTERM shutdown");
    assert!(
        status.success(),
        "a graceful SIGTERM shutdown should exit 0, got {status:?}"
    );

    // `autosave_secs = 300` above means the only way this file can exist at all, this soon, is
    // the shutdown save that just ran — on the unfixed code, nothing would ever have written it
    // within this test's whole window.
    let save_file = home.join("ShutdownSignalTest.wld");
    assert!(
        save_file.is_file(),
        "the shutdown save should have written {} by now",
        save_file.display()
    );
    assert!(
        std::fs::metadata(&save_file).is_ok_and(|m| m.len() > 0),
        "the saved world file should not be empty"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// `std::process::Child` has no built-in bounded wait — this is the small, hand-rolled
/// equivalent, matching this project's own stated preference for a narrow helper over a crate for
/// something this small (the `wait-timeout` crate exists solely for this one function).
trait WaitTimeoutOrKill {
    fn wait_timeout_or_kill(
        &mut self,
        timeout: Duration,
    ) -> std::io::Result<std::process::ExitStatus>;
}

impl WaitTimeoutOrKill for Child {
    fn wait_timeout_or_kill(
        &mut self,
        timeout: Duration,
    ) -> std::io::Result<std::process::ExitStatus> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if std::time::Instant::now() >= deadline {
                let _ = self.kill();
                let _ = self.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "process did not exit within the timeout",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
