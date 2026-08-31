//! The audit log: an append-only record of every moderation and permission-affecting action.
//!
//! One JSON object per line (JSONL), beside the world — the same directory convention
//! [`super::store::Admin`] uses (`Admin::load`'s own doc comment). JSONL rather than a hand-rolled
//! delimited format because a reason string is free text an operator typed and could contain almost
//! anything, a tab or an embedded newline included; JSON already escapes that correctly, and
//! `serde_json` is already a dependency (the panel's own JSON API, `config.rs`), so this adds
//! nothing new. Each line stands alone and parses independently, which is also what makes tailing
//! and rotation both trivial: nothing here needs to understand the file as a whole.
//!
//! **Rotation** is size-based: once the live file would exceed `max_bytes` on its next write, it is
//! renamed to `<file>.1` (existing `.1..N-1` are bumped up by one first; whatever was at `.N` is
//! dropped) and a fresh file is started. Old segments are plain renamed files — never compressed,
//! parsed, or otherwise touched by this module again. [`AuditLog::tail`] only ever reads the live
//! file; a full history lives across the segment files on disk for an operator to open directly.
//!
//! **Write failures never crash or block moderation.** A ban, a kick, a mute all happen and take
//! effect regardless of whether the line describing them could be written; a failure is reported at
//! `warn!` and swallowed. A server that refused to ban somebody because its own bookkeeping file
//! could not be written would be worse than a server with one missing audit line — this is the same
//! discipline `Admin::ban`/`Admin::unban` already apply to the admin file itself.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

/// A generous default: an operator running this server for months between restarts should still
/// see occasional rotation, not a many-gigabyte file, but a fresh server should not rotate away its
/// first day's history either.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// How many rotated segments are kept beyond the live file (`<file>.1` through `<file>.5` by
/// default) before the oldest is dropped.
pub const DEFAULT_KEEP_SEGMENTS: usize = 5;

/// One audit-log entry, and the whole of the on-disk line format: a `serde_json`-serialized value
/// of this struct, one per line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// UTC seconds since the epoch (the same clock [`super::ban::now`] uses).
    pub when: u64,
    /// Who did it: an account name, `"console"` (a line typed at the server's own terminal, which
    /// is always fully privileged — see `server/console.rs`'s module doc), or `"system"` (the
    /// server acting on nobody's particular instruction). Nothing emits `"system"` today; the
    /// vocabulary exists so a future automatic action has somewhere to attribute itself rather than
    /// inventing a fourth convention.
    pub issuer: String,
    pub action: AuditAction,
    /// Who or what the action was aimed at: a player or account name, a group name, or empty for an
    /// action with no single target.
    #[serde(default)]
    pub target: String,
    /// A reason string, or other free-text detail. Empty when there is none.
    #[serde(default)]
    pub detail: String,
}

/// The kinds of thing worth a permanent record. Every one of these already changes something an
/// operator would want an accountable trail for — the world file's ordinary autosave is not one of
/// these, but a ban that outlives the person who placed it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditAction {
    Ban,
    Unban,
    Kick,
    Mute,
    Unmute,
    Register,
    /// An account removed outright (currently only reachable from the web panel; the console has
    /// no equivalent command). Distinct from [`Self::GroupChange`], which only ever moves an
    /// account between groups.
    DeleteAccount,
    GroupChange,
    PermissionChange,
    Claim,
    /// A whitelist (guest-list) entry added or removed, from either the console or the web panel.
    /// `target` is the affected name; `detail` says `"added"` or `"removed"`.
    Whitelist,
    /// A login-style attempt refused by `admin::throttle` while a per-IP or per-account backoff
    /// window was open. One line per summarised window, not one per refusal: see
    /// `throttle::Verdict::Refused`'s own doc comment. `issuer` is always `"system"` (nobody
    /// signed in caused this; the server's own throttle did), `target` names which key tripped it
    /// (`ip:<address>` or `account:<name>`), and `detail` carries the refusal count and window.
    Throttled,
}

impl AuditAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ban => "ban",
            Self::Unban => "unban",
            Self::Kick => "kick",
            Self::Mute => "mute",
            Self::Unmute => "unmute",
            Self::Register => "register",
            Self::DeleteAccount => "delete-account",
            Self::GroupChange => "group-change",
            Self::PermissionChange => "permission-change",
            Self::Claim => "claim",
            Self::Whitelist => "whitelist",
            Self::Throttled => "throttled",
        }
    }
}

/// The audit log itself: where it lives (or that it does not), and the rotation caps.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: Option<PathBuf>,
    max_bytes: u64,
    keep_segments: usize,
}

impl AuditLog {
    /// An audit log with nowhere to write itself — for a world that is not being saved, matching
    /// [`super::store::Admin::in_memory`]'s own reasoning: an ephemeral server (tests, most often)
    /// should not scatter a log file into whatever directory it happened to be started from.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            max_bytes: DEFAULT_MAX_BYTES,
            keep_segments: DEFAULT_KEEP_SEGMENTS,
        }
    }

    /// An audit log that writes beside the world at `path`.
    pub fn new(path: PathBuf, max_bytes: u64, keep_segments: usize) -> Self {
        Self {
            path: Some(path),
            max_bytes: max_bytes.max(1),
            keep_segments,
        }
    }

    /// Record one event. Never fails outwardly: a write failure is a `warn!` and nothing more — see
    /// this module's own doc comment for why.
    pub fn record(&self, issuer: &str, action: AuditAction, target: &str, detail: &str) {
        let Some(path) = &self.path else {
            return;
        };
        let event = AuditEvent {
            when: super::ban::now(),
            issuer: issuer.to_string(),
            action,
            target: target.to_string(),
            detail: detail.to_string(),
        };
        if let Err(e) = self.append(path, &event) {
            warn!(
                error = %e,
                issuer,
                action = action.as_str(),
                target,
                "could not write the audit log; the action itself still happened",
            );
        }
    }

    fn append(&self, path: &Path, event: &AuditEvent) -> std::io::Result<()> {
        // A rotation that cannot happen must not also silence the log. Rotation exists to stop the
        // file growing without bound, which is a housekeeping concern; losing the record of a ban
        // is an accountability one, and the second is worse. So this warns and carries on writing
        // to whatever file is there, rather than returning early and dropping the event.
        if let Err(e) = self.rotate_if_needed(path) {
            warn!(
                error = %e,
                "could not rotate the audit log; still appending to the live file"
            );
        }
        // Built as one buffer, newline included, and written with a single `write_all`. `writeln!`
        // goes through `write_fmt`, which is free to reach the kernel more than once, and a second
        // call that fails after the first succeeded leaves a line with no terminator: the next
        // event then lands on the same line and both are unreadable. One append-mode write of a
        // short buffer is the closest a portable program gets to an atomic line.
        let mut line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| crate::safe_write::explain("writing the audit log", path, &e))?;
        file.write_all(line.as_bytes())
            .map_err(|e| crate::safe_write::explain("writing the audit log", path, &e))
    }

    /// One segment's path: `<file>.<n>` — appended to the whole file name, not swapped in as an
    /// extension, so `world.audit.jsonl` rotates to `world.audit.jsonl.1` rather than clobbering its
    /// own `.jsonl` suffix.
    fn segment_path(path: &Path, n: usize) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    /// Roll the live file to `.1` (bumping any existing numbered segments up by one, dropping
    /// whatever falls off the end) if it is already at or past the size cap. A missing live file —
    /// the ordinary case, most writes — is not an error; there is nothing to rotate yet.
    fn rotate_if_needed(&self, path: &Path) -> std::io::Result<()> {
        let Ok(meta) = std::fs::metadata(path) else {
            return Ok(());
        };
        if meta.len() < self.max_bytes || self.keep_segments == 0 {
            return Ok(());
        }
        let oldest = Self::segment_path(path, self.keep_segments);
        let _ = std::fs::remove_file(&oldest);
        // Renames throughout, so a failure part-way leaves a shorter chain rather than a segment
        // that is half of two different logs. Nothing here ever copies bytes.
        for n in (1..self.keep_segments).rev() {
            let from = Self::segment_path(path, n);
            let to = Self::segment_path(path, n + 1);
            if from.exists() {
                std::fs::rename(&from, &to)
                    .map_err(|e| crate::safe_write::explain("rotating the audit log", &to, &e))?;
            }
        }
        let first = Self::segment_path(path, 1);
        std::fs::rename(path, &first)
            .map_err(|e| crate::safe_write::explain("rotating the audit log", &first, &e))
    }

    /// The most recent `n` events from the live file, oldest first (so it reads top-to-bottom the
    /// way it was written). Deliberately only the live file — see this module's own doc comment for
    /// why a full history is a matter of opening the rotated segments directly rather than
    /// something this reads back in. A line that fails to parse (hand-edited, or truncated by a
    /// crash mid-write) is skipped rather than aborting the whole read.
    pub fn tail(&self, n: usize) -> Vec<AuditEvent> {
        self.path
            .as_deref()
            .map(|path| tail_file(path, n))
            .unwrap_or_default()
    }

    /// Where this log writes, if it writes anywhere. The panel's `/api/audit` asks the game task
    /// for this and then reads the file itself, off the game task, rather than having the game
    /// task read it inline: see [`tail_file`].
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// [`AuditLog::tail`]'s body, as a free function over the path alone.
///
/// Split out because the caller that matters is the web panel, and the panel's own rule is "off the
/// game task, always" (`panel/mod.rs`'s module doc). `/api/audit` polls every five seconds; running
/// this inline in the game task's `select!` put a synchronous whole-file read of a file whose
/// default cap is 8 MB (and which `audit_log_max_bytes` can raise) against a 16.67 ms tick budget.
/// The panel now asks the game task only for [`AuditLog::path`], a `PathBuf` clone, and calls this
/// from a `spawn_blocking`.
///
/// Still reads the whole live file: the parse is already bounded by `n`, and a bounded seek-from-
/// the-end read would have to deal with a partial first line and a UTF-8 boundary for a saving that
/// no longer lands anywhere near the tick.
pub fn tail_file(path: &Path, n: usize) -> Vec<AuditEvent> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut events: Vec<AuditEvent> = text
        .lines()
        .rev()
        .take(n)
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    events.reverse();
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("terrustia-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        for n in 1..=10 {
            let _ = std::fs::remove_file(AuditLog::segment_path(&path, n));
        }
        path
    }

    /// An in-memory log (no save path) records nothing and never panics doing it.
    #[test]
    fn an_in_memory_log_writes_nothing_and_tails_empty() {
        let log = AuditLog::in_memory();
        log.record("brook", AuditAction::Ban, "griefer", "wrecked spawn");
        assert!(log.tail(10).is_empty());
    }

    /// A recorded event round-trips through the file, in order, oldest first.
    #[test]
    fn events_round_trip_in_order() {
        let path = temp("roundtrip.jsonl");
        let log = AuditLog::new(path, DEFAULT_MAX_BYTES, DEFAULT_KEEP_SEGMENTS);
        log.record("brook", AuditAction::Ban, "griefer", "wrecked spawn");
        log.record("brook", AuditAction::Unban, "griefer", "");
        let tail = log.tail(10);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].action, AuditAction::Ban);
        assert_eq!(tail[0].issuer, "brook");
        assert_eq!(tail[0].target, "griefer");
        assert_eq!(tail[0].detail, "wrecked spawn");
        assert_eq!(tail[1].action, AuditAction::Unban);
    }

    /// `tail` returns only the last `n`, still oldest-first within that window.
    #[test]
    fn tail_returns_only_the_most_recent_n() {
        let path = temp("tail-n.jsonl");
        let log = AuditLog::new(path, DEFAULT_MAX_BYTES, DEFAULT_KEEP_SEGMENTS);
        for i in 0..5 {
            log.record("brook", AuditAction::Kick, &format!("player{i}"), "");
        }
        let tail = log.tail(2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].target, "player3");
        assert_eq!(tail[1].target, "player4");
    }

    /// A write failure (here: the "file" is actually a directory, so opening it for append fails)
    /// is swallowed rather than propagated — the whole point of `record`'s signature having no
    /// `Result` at all.
    #[test]
    fn a_write_failure_does_not_panic() {
        let path = temp("a-directory-not-a-file");
        std::fs::create_dir_all(&path).unwrap();
        let log = AuditLog::new(path.clone(), DEFAULT_MAX_BYTES, DEFAULT_KEEP_SEGMENTS);
        log.record("brook", AuditAction::Kick, "someone", "");
        let _ = std::fs::remove_dir(&path);
    }

    /// Fail-then-pass for rotation: past the size cap, the live file becomes `.1` and a fresh one
    /// starts — before `rotate_if_needed` was wired into `append`, this test's `tail` after the
    /// second batch of writes would have kept growing the same file forever and the `.1` segment
    /// would never have appeared.
    #[test]
    fn rotation_moves_the_live_file_to_segment_one_past_the_cap() {
        let path = temp("rotate.jsonl");
        // A tiny cap so a handful of short lines is already "past" it.
        let log = AuditLog::new(path.clone(), 40, DEFAULT_KEEP_SEGMENTS);
        for i in 0..20 {
            log.record("brook", AuditAction::Kick, &format!("p{i}"), "");
        }
        assert!(
            AuditLog::segment_path(&path, 1).exists(),
            "the file should have rotated at least once"
        );
        assert!(
            path.exists(),
            "a fresh live file should exist after rotating"
        );
    }

    /// Rotation keeps at most `keep_segments`, dropping the oldest rather than growing forever.
    #[test]
    fn rotation_drops_the_oldest_segment_once_the_cap_is_reached() {
        let path = temp("rotate-cap.jsonl");
        let log = AuditLog::new(path.clone(), 10, 2);
        // Enough writes to rotate several times over.
        for i in 0..60 {
            log.record("brook", AuditAction::Kick, &format!("p{i}"), "");
        }
        assert!(AuditLog::segment_path(&path, 1).exists());
        assert!(AuditLog::segment_path(&path, 2).exists());
        assert!(
            !AuditLog::segment_path(&path, 3).exists(),
            "only 2 segments were asked to be kept"
        );
    }

    /// A rotation that cannot happen must not also silence the log.
    ///
    /// Rotation is housekeeping: it stops the file growing without bound. The record of a ban is
    /// accountability. `rotate_if_needed`'s failure used to propagate out of `append` through a
    /// `?`, so a segment that could not be replaced meant every later event was dropped without a
    /// word - the second concern sacrificed for the first. Restoring that `?` in place of the
    /// `if let Err(..)` arm makes this test fail on its final assertion.
    ///
    /// The blocker is a directory sitting where the live file wants to rotate to, with
    /// `keep_segments = 1` so nothing shuffles it out of the way first: renaming a file onto a
    /// directory is refused by the kernel, while the live file itself stays perfectly writable.
    #[test]
    fn a_rotation_that_cannot_happen_still_records_the_event() {
        let dir = crate::safe_write::tests::temp_dir("audit-blocked-rotation");
        let path = dir.join("world.audit.jsonl");
        let log = AuditLog::new(path.clone(), 10, 1);
        log.record("brook", AuditAction::Ban, "griefer", "the first line");

        let blocker = AuditLog::segment_path(&path, 1);
        std::fs::create_dir_all(&blocker).expect("blocking directory");
        std::fs::write(blocker.join("occupant"), b"in the way").expect("occupant");

        log.record("brook", AuditAction::Kick, "someone", "the second line");
        assert!(
            blocker.is_dir(),
            "the blocker should still be in the way, or this test proved nothing"
        );
        assert_eq!(
            log.tail(10).len(),
            2,
            "a failed rotation must not swallow the event it was rotating for"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A read-only directory stops rotation and nothing else.
    ///
    /// Worth pinning because it is the opposite of the intuition: POSIX permits appending to an
    /// existing writable file inside a directory that permits no renames at all, so the log keeps
    /// its accountability even while its housekeeping is blocked. Under the old propagating `?`
    /// this same situation lost every event from the moment the file first reached its size cap.
    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_blocks_rotation_but_not_the_record() {
        let dir = crate::safe_write::tests::temp_dir("audit-readonly-dir");
        let path = dir.join("world.audit.jsonl");
        // A tiny cap, so the next write is one that wants to rotate.
        let log = AuditLog::new(path.clone(), 10, DEFAULT_KEEP_SEGMENTS);
        log.record("brook", AuditAction::Ban, "griefer", "wrecked spawn");

        let Some(_guard) = crate::safe_write::tests::ReadOnlyDir::new(&dir) else {
            eprintln!("skipping: this environment cannot make a directory read-only");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        log.record("brook", AuditAction::Kick, "someone", "");
        let rotated = AuditLog::segment_path(&path, 1).exists();
        let recorded = log.tail(10).len();
        drop(_guard);

        assert!(!rotated, "a read-only directory cannot rotate");
        assert_eq!(recorded, 2, "and must still record");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A log file that genuinely cannot be written costs the line and nothing else.
    ///
    /// `record` has no `Result` by design (this module's own doc comment says why): a ban must
    /// happen whether or not the line describing it could be written. What must not happen is a
    /// panic, or damage to what is already in the file.
    #[cfg(unix)]
    #[test]
    fn an_unwritable_log_file_costs_the_line_and_nothing_else() {
        let dir = crate::safe_write::tests::temp_dir("audit-readonly-file");
        let path = dir.join("world.audit.jsonl");
        let log = AuditLog::new(path.clone(), DEFAULT_MAX_BYTES, DEFAULT_KEEP_SEGMENTS);
        log.record("brook", AuditAction::Ban, "griefer", "wrecked spawn");
        let before = std::fs::read(&path).expect("reading the log back");
        assert!(!before.is_empty(), "there should be a line to protect");

        let Some(_guard) = crate::safe_write::tests::ReadOnlyFile::new(&path) else {
            eprintln!("skipping: this environment cannot make a file read-only");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        log.record("brook", AuditAction::Kick, "someone", "");
        let after = std::fs::read(&path).expect("reading the log back");
        drop(_guard);

        assert_eq!(
            after, before,
            "a refused write must leave the existing log byte-identical"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory that has gone away entirely: no file to append to, and none can be created.
    #[test]
    fn a_vanished_directory_costs_the_line_and_does_not_panic() {
        let dir = crate::safe_write::tests::temp_dir("audit-vanished");
        let path = dir.join("world.audit.jsonl");
        let log = AuditLog::new(path.clone(), DEFAULT_MAX_BYTES, DEFAULT_KEEP_SEGMENTS);
        std::fs::remove_dir_all(&dir).expect("removing the directory out from under it");

        log.record("brook", AuditAction::Ban, "griefer", "wrecked spawn");
        assert!(!path.exists(), "nothing should have been conjured up");
        assert!(log.tail(10).is_empty());
    }

    /// A malformed line is skipped, not fatal to the rest of the read.
    #[test]
    fn a_malformed_line_is_skipped_not_fatal() {
        let path = temp("malformed.jsonl");
        std::fs::write(&path, "not json at all\n{\"broken\n").unwrap();
        let log = AuditLog::new(path.clone(), DEFAULT_MAX_BYTES, DEFAULT_KEEP_SEGMENTS);
        log.record("brook", AuditAction::Kick, "someone", "");
        let tail = log.tail(10);
        assert_eq!(tail.len(), 1, "the two malformed lines should be skipped");
        assert_eq!(tail[0].target, "someone");
    }
}
