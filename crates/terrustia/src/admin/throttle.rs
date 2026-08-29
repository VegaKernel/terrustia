//! Login-attempt throttling: exponential backoff with jitter, never a lockout.
//!
//! The design this implements (see `TODO.md`'s Lane F): per-IP **and** per-account backoff, in
//! memory, reset the moment a credential is right, with no state that ever refuses a key
//! permanently. That combination matters for two different attackers at once. Someone spraying
//! many account names from one address is slowed down by the per-IP window regardless of which
//! name they are currently trying; someone spraying one known account name from many addresses
//! (to grief its real owner, not to guess the password) is slowed down by the per-account window
//! regardless of where the attempt comes from. Neither window can be forced open forever by a
//! third party: the delay is capped ([`MAX_DELAY_SECS`]), so a real owner trying their own account
//! mid-attack waits at most that long, the same as the attacker does, and a single correct
//! credential clears the key's history immediately ([`Throttle::record_success`]). A process
//! restart drops every window too, which is fine — see `TODO.md` again: "in-memory ... so a
//! restart clears state harmlessly" is the point, not a gap. There is deliberately no notion of an
//! account becoming inaccessible until an operator intervenes; that would let the very attack this
//! guards against turn into a denial of service against the account's own owner.
//!
//! Two independent [`Throttle`]s are meant to be held side by side by a caller — one keyed by the
//! caller's address, one by the account name they typed — and a login attempt is refused if
//! *either* key's window is open. [`Throttle`] itself does not know or care which dimension a
//! given instance covers; it is the same map either way.
//!
//! The schedule ([`base_delay`]) and the jitter it is spread by ([`jittered`]) are plain functions
//! of their inputs, with no clock or randomness of their own, so they are unit-tested directly.
//! [`Throttle::check`], [`Throttle::record_failure`] and [`Throttle::record_success`] take `now`
//! as a parameter rather than reading `Instant::now()` themselves, for the same reason: a test
//! drives time by picking `Instant` values, never by actually sleeping.
//!
//! Every call site is meant to stay thin: call [`Throttle::check`] before touching the real
//! credential at all (a refused attempt must not pay for an Argon2 hash, or even a lookup that
//! would answer "does this account exist"), refuse with [`REFUSAL_MESSAGE`] if it says so, and
//! otherwise check the credential and call [`Throttle::record_success`] or
//! [`Throttle::record_failure`] with the outcome. [`Verdict::Refused`] carries `log_summary` for
//! exactly the refusals that should produce one audit-log line — see its own doc comment.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rand::{Rng, SeedableRng, rngs::SmallRng};

/// Attempts allowed before backoff engages at all. A slow typist or a fat-fingered password gets
/// this many free retries with no extra delay — the point of throttling is brute force, not an
/// honest mistake.
///
/// `pub` (unlike this module's other constants, which stay private) so a test elsewhere — in this
/// crate or in `tests/panel.rs`, a separate binary that only ever sees this crate's public surface
/// — can open a window in exactly as many real attempts as it takes, rather than duplicating the
/// number and silently drifting from it if the schedule ever changes. Not a secret either way: see
/// [`jittered`]'s own doc comment on why the schedule itself is not something worth hiding.
pub const FREE_ATTEMPTS: u32 = 3;
/// The delay before jitter after the first throttled failure (the `FREE_ATTEMPTS + 1`th).
const INITIAL_DELAY_SECS: u64 = 1;
/// Doubles every failure past `FREE_ATTEMPTS`, capped here. Five minutes between attempts makes
/// brute force impractically slow (a few hundred guesses a day, not a second) without ever
/// reading as "locked" to someone who genuinely forgot which password they used.
const MAX_DELAY_SECS: u64 = 300;
/// How far the exponent is allowed to climb before `.min(MAX_DELAY_SECS)` would clamp it anyway.
/// Bounded so the shift below never has to consider overflow.
const EXPONENT_CAP: u32 = 20;
/// How much of the base delay is randomised, each way. Not a secrecy measure — the schedule is
/// public — just enough to stop an attacker timing their next attempt to land the instant a
/// window closes.
const JITTER_FRACTION: f64 = 0.2;
/// A key already inside a refused window earns at most one audit-log line per this interval, no
/// matter how many refusals landed in it — seem this module's top doc and [`Verdict::Refused`].
const LOG_INTERVAL: Duration = Duration::from_secs(60);
/// A key that has not been touched in this long is forgotten outright the next time the map is
/// swept ([`Throttle::prune_stale`]), so a sustained flood of distinct keys (many IPs, many
/// account names tried once and abandoned) cannot grow this map without bound. Well past
/// `MAX_DELAY_SECS`, so pruning never interrupts an attack actually in progress.
const IDLE_EXPIRY: Duration = Duration::from_secs(3600);
/// The map is only swept once it holds more than this many keys — pruning is `O(n)`, so gating it
/// on size keeps the ordinary case (a handful of keys, most servers) from paying for a sweep on
/// every single failure.
const PRUNE_THRESHOLD: usize = 4096;

/// The message every throttled refusal answers with, everywhere this module is used. One shared
/// string rather than one per call site so the wording can never drift into accidentally
/// distinguishing "this account does not exist" from "this account exists and is being tried too
/// often" — the whole point of routing both through the same [`Verdict::Refused`] arm.
pub const REFUSAL_MESSAGE: &str = "too many attempts; wait a moment and try again.";

/// The base backoff for a key's `n`th consecutive failure (`n` counts every failure, including
/// the free ones from the start), before jitter. Pure and deterministic — see this module's top
/// doc for why that matters for testing.
pub fn base_delay(consecutive_failures: u32) -> Duration {
    if consecutive_failures <= FREE_ATTEMPTS {
        return Duration::ZERO;
    }
    let exponent = (consecutive_failures - FREE_ATTEMPTS - 1).min(EXPONENT_CAP);
    let secs = INITIAL_DELAY_SECS.saturating_mul(1u64 << exponent);
    Duration::from_secs(secs.min(MAX_DELAY_SECS))
}

/// Spread `base` by up to [`JITTER_FRACTION`] in either direction. `unit` is the caller's own
/// random draw in `[0, 1]` (a real one in production, a fixed value in a test) — kept as a plain
/// parameter rather than an RNG so this function itself needs nothing injected but the number.
/// A zero base (still inside the free attempts) is returned unchanged: there is no window to
/// jitter yet.
pub fn jittered(base: Duration, unit: f64) -> Duration {
    if base.is_zero() {
        return base;
    }
    let unit = unit.clamp(0.0, 1.0);
    let spread = base.as_secs_f64() * JITTER_FRACTION;
    let offset = spread * (2.0 * unit - 1.0); // unit in [0,1] -> offset in [-spread, spread]
    Duration::from_secs_f64((base.as_secs_f64() + offset).max(0.0))
}

/// One key's throttle state.
#[derive(Debug, Clone, Default)]
struct Entry {
    consecutive_failures: u32,
    /// When the current refusal window ends, or `None` if the key is not currently backed off
    /// (either it has never failed past `FREE_ATTEMPTS`, or its window has already closed and
    /// nothing has failed since).
    open_until: Option<Instant>,
    /// Refusals since the last summarising audit-log line for this key.
    refused_since_log: u32,
    last_logged: Option<Instant>,
    /// Last time this key was touched at all (a failure or a refused check), for
    /// [`Throttle::prune_stale`].
    touched_at: Option<Instant>,
}

/// What [`Throttle::check`] found for one key.
#[derive(Debug)]
pub enum Verdict {
    /// No window is open for this key; go ahead and check the real credential.
    Allowed,
    /// A window is open. `retry_after` is how much longer it has to run.
    ///
    /// `log_summary` is `Some(n)` exactly on the one refusal in every [`LOG_INTERVAL`] window that
    /// should produce a single audit-log line covering the `n` refusals (this one included) since
    /// the line before it — every other refusal in between carries `None` and should stay silent.
    /// This is the "not one line per spam attempt; summarise" requirement: an attacker retrying
    /// every few milliseconds produces one log line a minute, not one a millisecond.
    Refused {
        retry_after: Duration,
        log_summary: Option<u32>,
    },
}

/// Per-key exponential backoff, in memory, with jitter and no lockout. See this module's top doc
/// for the full design and how a caller is meant to use one of these.
pub struct Throttle {
    entries: HashMap<String, Entry>,
    rng: SmallRng,
}

impl Throttle {
    /// A fresh throttle with no history for any key.
    ///
    /// Jitter is seeded from the OS's own CSPRNG — the same `argon2::password_hash::rand_core`
    /// re-export of `rand_core::OsRng` this crate already pulls in for password salts and session
    /// tokens (`admin::Account::new`, `panel::mod::issue_session`), so this adds no new
    /// dependency — then advances deterministically from there with the same `SmallRng` the game
    /// task already uses for ordinary game randomness (`GameServer::rng`). Jitter only has to keep
    /// an attacker from timing a retry to the second; it is not a secret and does not need to
    /// resist prediction from a leaked seed, so a non-cryptographic generator is the right tool
    /// here exactly as it is everywhere else this workspace uses one.
    pub fn new() -> Self {
        use argon2::password_hash::rand_core::{OsRng, RngCore};
        let mut buf = [0u8; 8];
        OsRng.fill_bytes(&mut buf);
        Self::with_seed(u64::from_le_bytes(buf))
    }

    /// A throttle whose jitter is reproducible, for tests: real entropy would make a bounds
    /// assertion on `retry_after` correct but unrepeatable, and a fixed seed is not weaker for
    /// this purpose than a random one — see [`Self::new`]'s own doc comment for why jitter is not
    /// a secret in the first place.
    fn with_seed(seed: u64) -> Self {
        Self {
            entries: HashMap::new(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    /// Whether `key`'s window is currently open. Call this before touching the real credential at
    /// all — see this module's top doc.
    pub fn check(&mut self, key: &str, now: Instant) -> Verdict {
        let Some(entry) = self.entries.get_mut(key) else {
            return Verdict::Allowed;
        };
        entry.touched_at = Some(now);
        let Some(open_until) = entry.open_until else {
            return Verdict::Allowed;
        };
        if now >= open_until {
            return Verdict::Allowed;
        }
        entry.refused_since_log = entry.refused_since_log.saturating_add(1);
        let due = match entry.last_logged {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= LOG_INTERVAL,
        };
        let log_summary = due.then(|| {
            let count = entry.refused_since_log;
            entry.refused_since_log = 0;
            entry.last_logged = Some(now);
            count
        });
        Verdict::Refused {
            retry_after: open_until.saturating_duration_since(now),
            log_summary,
        }
    }

    /// Record a failed credential check for `key`, extending (or opening) its window from `now`.
    ///
    /// Only ever call this after [`Self::check`] returned [`Verdict::Allowed`] and the real
    /// credential then turned out to be wrong — a refusal from `check` itself must never also
    /// count as a failure, or a key already backed off would have its window pushed out further
    /// just by being hammered, punishing a confused legitimate user exactly as hard as an
    /// attacker.
    pub fn record_failure(&mut self, key: &str, now: Instant) {
        if self.entries.len() > PRUNE_THRESHOLD {
            self.prune_stale(now, IDLE_EXPIRY);
        }
        let unit = self.rng.random_range(0.0..=1.0);
        let entry = self.entries.entry(key.to_string()).or_default();
        entry.touched_at = Some(now);
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        let base = base_delay(entry.consecutive_failures);
        entry.open_until = if base.is_zero() {
            None
        } else {
            Some(now + jittered(base, unit))
        };
    }

    /// Clear `key`'s entire history on a correct credential. No lockout: a right answer always
    /// wins immediately, and the next failure (if any, from anyone) starts back at the beginning.
    pub fn record_success(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// Drop every key untouched for longer than `idle_expiry`, so a flood of distinct keys cannot
    /// grow this map without bound. Only called from [`Self::record_failure`] once the map has
    /// grown past [`PRUNE_THRESHOLD`].
    fn prune_stale(&mut self, now: Instant, idle_expiry: Duration) {
        self.entries.retain(|_, entry| {
            entry
                .touched_at
                .is_some_and(|touched| now.saturating_duration_since(touched) < idle_expiry)
        });
    }
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- base_delay -------------------------------------------------------------------------

    #[test]
    fn the_free_attempts_cost_nothing() {
        for n in 0..=FREE_ATTEMPTS {
            assert_eq!(
                base_delay(n),
                Duration::ZERO,
                "failure {n} should still be free"
            );
        }
    }

    #[test]
    fn the_delay_doubles_each_failure_past_the_free_attempts() {
        assert_eq!(base_delay(FREE_ATTEMPTS + 1), Duration::from_secs(1));
        assert_eq!(base_delay(FREE_ATTEMPTS + 2), Duration::from_secs(2));
        assert_eq!(base_delay(FREE_ATTEMPTS + 3), Duration::from_secs(4));
        assert_eq!(base_delay(FREE_ATTEMPTS + 4), Duration::from_secs(8));
    }

    #[test]
    fn the_delay_is_capped_rather_than_growing_forever() {
        assert_eq!(
            base_delay(FREE_ATTEMPTS + 20),
            Duration::from_secs(MAX_DELAY_SECS)
        );
        assert_eq!(base_delay(u32::MAX), Duration::from_secs(MAX_DELAY_SECS));
    }

    // ---- jittered -----------------------------------------------------------------------------

    #[test]
    fn a_zero_base_is_never_jittered() {
        assert_eq!(jittered(Duration::ZERO, 0.0), Duration::ZERO);
        assert_eq!(jittered(Duration::ZERO, 1.0), Duration::ZERO);
    }

    #[test]
    fn the_midpoint_draw_leaves_the_base_unchanged() {
        let base = Duration::from_secs(10);
        assert_eq!(jittered(base, 0.5), base);
    }

    #[test]
    fn jitter_stays_within_its_fraction_either_way() {
        let base = Duration::from_secs(10);
        let low = jittered(base, 0.0);
        let high = jittered(base, 1.0);
        let spread = base.as_secs_f64() * JITTER_FRACTION;
        assert!((low.as_secs_f64() - (base.as_secs_f64() - spread)).abs() < 1e-9);
        assert!((high.as_secs_f64() - (base.as_secs_f64() + spread)).abs() < 1e-9);
    }

    // ---- Throttle -------------------------------------------------------------------------------

    fn base_now() -> Instant {
        Instant::now()
    }

    #[test]
    fn free_attempts_are_always_allowed_immediately() {
        let mut throttle = Throttle::with_seed(1);
        let now = base_now();
        for _ in 0..FREE_ATTEMPTS {
            throttle.record_failure("1.2.3.4", now);
            assert!(matches!(throttle.check("1.2.3.4", now), Verdict::Allowed));
        }
    }

    /// Fail-then-pass for the core mechanism: before a window opens, a fourth attempt right after
    /// three failures goes through unthrottled; once the fourth failure itself lands, a further
    /// attempt inside the same instant is refused.
    #[test]
    fn a_throttled_attempt_inside_the_window_is_refused() {
        let mut throttle = Throttle::with_seed(2);
        let now = base_now();
        for _ in 0..FREE_ATTEMPTS {
            throttle.record_failure("acct", now);
        }
        assert!(
            matches!(throttle.check("acct", now), Verdict::Allowed),
            "still within the free attempts"
        );
        throttle.record_failure("acct", now); // the failure that opens the window
        match throttle.check("acct", now) {
            Verdict::Refused { retry_after, .. } => {
                assert!(retry_after > Duration::ZERO);
            }
            Verdict::Allowed => panic!("the window just opened; this must be refused"),
        }
    }

    #[test]
    fn the_window_closes_once_its_own_delay_has_passed() {
        let mut throttle = Throttle::with_seed(3);
        let now = base_now();
        for _ in 0..=FREE_ATTEMPTS {
            throttle.record_failure("acct", now);
        }
        let retry_after = match throttle.check("acct", now) {
            Verdict::Refused { retry_after, .. } => retry_after,
            Verdict::Allowed => panic!("expected a window to be open"),
        };
        let later = now + retry_after;
        assert!(
            matches!(throttle.check("acct", later), Verdict::Allowed),
            "the window should have closed by its own retry_after"
        );
    }

    /// The design's whole point: a correct credential always wins, however deep the backoff has
    /// gone, and the very next failure (from anyone) starts back at the beginning rather than
    /// carrying any memory of the attack.
    #[test]
    fn success_resets_a_key_completely_no_matter_how_far_backed_off_it_was() {
        let mut throttle = Throttle::with_seed(4);
        let now = base_now();
        for _ in 0..10 {
            throttle.record_failure("acct", now);
        }
        assert!(matches!(
            throttle.check("acct", now),
            Verdict::Refused { .. }
        ));

        throttle.record_success("acct");

        assert!(
            matches!(throttle.check("acct", now), Verdict::Allowed),
            "a correct credential must clear the window immediately"
        );
        // And the next failure is treated as the very first one again.
        throttle.record_failure("acct", now);
        assert!(matches!(throttle.check("acct", now), Verdict::Allowed));
    }

    #[test]
    fn an_untouched_key_is_always_allowed() {
        let mut throttle = Throttle::with_seed(5);
        assert!(matches!(
            throttle.check("nobody-has-ever-tried-this", base_now()),
            Verdict::Allowed
        ));
    }

    /// Distinct keys never affect each other — the whole reason a caller holds two `Throttle`s
    /// (one per-IP, one per-account) rather than mixing every attempt into one bucket.
    #[test]
    fn different_keys_are_independent() {
        let mut throttle = Throttle::with_seed(6);
        let now = base_now();
        for _ in 0..10 {
            throttle.record_failure("attacker-ip", now);
        }
        assert!(matches!(
            throttle.check("attacker-ip", now),
            Verdict::Refused { .. }
        ));
        assert!(matches!(
            throttle.check("someone-elses-ip", now),
            Verdict::Allowed
        ));
    }

    /// "Not one line per spam attempt": only the first refusal in a key's window and, after that,
    /// at most one more per `LOG_INTERVAL`, ever carries a summary to log.
    ///
    /// Enough failures to push the window's base delay all the way to `MAX_DELAY_SECS` (well past
    /// `LOG_INTERVAL` even after the worst-case `-JITTER_FRACTION` draw), so the window is still
    /// open sixty seconds later — a handful of failures would open a window shorter than
    /// `LOG_INTERVAL` itself, closing before there was ever a second refusal to fold in.
    #[test]
    fn refusals_are_summarised_at_most_once_per_log_interval() {
        let mut throttle = Throttle::with_seed(7);
        let now = base_now();
        for _ in 0..(FREE_ATTEMPTS + 20) {
            throttle.record_failure("acct", now);
        }

        let first = match throttle.check("acct", now) {
            Verdict::Refused { log_summary, .. } => log_summary,
            Verdict::Allowed => panic!("expected a window to be open"),
        };
        assert_eq!(first, Some(1), "the first refusal always logs");

        for _ in 0..50 {
            let repeat = match throttle.check("acct", now) {
                Verdict::Refused { log_summary, .. } => log_summary,
                Verdict::Allowed => panic!("still inside the window"),
            };
            assert_eq!(repeat, None, "must stay silent within the same interval");
        }

        let later = now + LOG_INTERVAL;
        let summary = match throttle.check("acct", later) {
            Verdict::Refused { log_summary, .. } => log_summary,
            Verdict::Allowed => None,
        };
        assert_eq!(
            summary,
            Some(51),
            "the next line past the interval should fold in every silent refusal since the last \
             one, this one included: 50 silent ones plus itself"
        );
    }

    #[test]
    fn stale_entries_are_pruned_once_idle_long_enough() {
        let mut throttle = Throttle::with_seed(8);
        let now = base_now();
        throttle.record_failure("gone-quiet", now);
        assert_eq!(throttle.entries.len(), 1);

        let much_later = now + IDLE_EXPIRY + Duration::from_secs(1);
        throttle.prune_stale(much_later, IDLE_EXPIRY);
        assert!(
            throttle.entries.is_empty(),
            "an entry idle past its expiry should be pruned"
        );
    }

    #[test]
    fn a_recently_touched_entry_survives_pruning() {
        let mut throttle = Throttle::with_seed(9);
        let now = base_now();
        throttle.record_failure("still-active", now);

        throttle.prune_stale(now + Duration::from_secs(1), IDLE_EXPIRY);
        assert_eq!(
            throttle.entries.len(),
            1,
            "an entry touched moments ago must not be pruned"
        );
    }
}
