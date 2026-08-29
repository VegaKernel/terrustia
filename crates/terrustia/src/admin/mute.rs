//! Chat suppression: muting a player by name.
//!
//! Named the same way a ban's [`BanKind::Name`](super::BanKind::Name) is — a display name, not an
//! account — because a mute is aimed at whoever is talking right now, signed in or not. Off by
//! default in the sense that matters: nothing here does anything until an operator (or a moderator
//! holding `server.mute`) actually mutes somebody. See `game/server/dispatch.rs`'s chat handling
//! for the shadow-mute itself — the muted player still sees their own line, staff see it flagged,
//! everyone else gets nothing.

/// One active (or expired-but-not-yet-cleaned-up) mute.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Mute {
    /// Case-insensitively matched, like a name ban.
    pub name: String,
    pub reason: String,
    /// Unix seconds after which it lapses, or `None` for permanent (until `/unmute`).
    pub until: Option<u64>,
    /// Who placed it: an account name, `"console"`, or `"system"` (mute escalation — see
    /// `Config::mute_escalation_enabled` — extends under this issuer, since the extension was not
    /// anyone's direct instruction).
    #[serde(default)]
    pub issuer: String,
    #[serde(default)]
    pub issued_at: u64,
}

impl Mute {
    pub fn new(name: &str, reason: &str, until: Option<u64>, issuer: &str) -> Self {
        Self {
            name: name.to_string(),
            reason: reason.to_string(),
            until,
            issuer: issuer.to_string(),
            issued_at: super::ban::now(),
        }
    }

    /// Whether this mute still applies at `now` (unix seconds).
    pub fn in_force(&self, now: u64) -> bool {
        self.until.is_none_or(|until| now < until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permanent_mute_stays() {
        let mute = Mute::new("chatty", "spam", None, "brook");
        assert!(mute.in_force(0));
        assert!(mute.in_force(u64::MAX - 1));
    }

    #[test]
    fn a_timed_mute_expires() {
        let mute = Mute::new("chatty", "spam", Some(1_000), "brook");
        assert!(mute.in_force(999));
        assert!(!mute.in_force(1_000));
    }
}
