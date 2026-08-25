//! Keeping somebody out.
//!
//! Three ways to name a person, because each is defeated differently: a name is changed in a
//! second, an address by a router restart, and the client UUID by reinstalling. Together they are
//! enough friction for a server among friends, which is what this is for.
//!
//! `Player::uuid` was already received and stored by the server and read by nothing at all. This
//! is what it was for.

/// How a ban names somebody.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BanKind {
    Name,
    /// The address, without its port — a port changes every connection.
    Address,
    /// The client's own identifier, which it sends in packet 68.
    Uuid,
}

/// One entry in the ban list.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Ban {
    pub kind: BanKind,
    /// The name, address or UUID this applies to.
    pub value: String,
    pub reason: String,
    /// Unix seconds after which it lapses, or `None` for permanent.
    pub until: Option<u64>,
}

impl Ban {
    pub fn permanent(kind: BanKind, value: &str, reason: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
            reason: reason.to_string(),
            until: None,
        }
    }

    /// Whether this ban still applies at `now` (unix seconds).
    pub fn in_force(&self, now: u64) -> bool {
        self.until.is_none_or(|until| now < until)
    }

    /// Whether it names this person. Names and UUIDs compare case-insensitively; addresses do not
    /// need to, and comparing them loosely would be a way to widen a ban by accident.
    pub fn matches(&self, kind: &BanKind, value: &str) -> bool {
        if &self.kind != kind {
            return false;
        }
        match kind {
            BanKind::Address => self.value == value,
            _ => self.value.eq_ignore_ascii_case(value),
        }
    }
}

/// Seconds since the epoch, or zero if the clock is before it.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A permanent ban never lapses.
    #[test]
    fn a_permanent_ban_stays() {
        let ban = Ban::permanent(BanKind::Name, "griefer", "wrecked spawn");
        assert!(ban.in_force(0));
        assert!(ban.in_force(u64::MAX - 1));
    }

    /// A timed one lapses on its own rather than needing to be lifted.
    #[test]
    fn a_timed_ban_expires() {
        let ban = Ban {
            until: Some(1_000),
            ..Ban::permanent(BanKind::Name, "hothead", "language")
        };
        assert!(ban.in_force(999));
        assert!(!ban.in_force(1_000));
        assert!(!ban.in_force(2_000));
    }

    /// A name matches whatever its capitals, and does not reach across kinds.
    #[test]
    fn matching_is_by_kind_and_case_insensitive_for_names() {
        let ban = Ban::permanent(BanKind::Name, "Griefer", "");
        assert!(ban.matches(&BanKind::Name, "griefer"));
        assert!(ban.matches(&BanKind::Name, "GRIEFER"));
        assert!(
            !ban.matches(&BanKind::Uuid, "griefer"),
            "a name is not a uuid"
        );
    }

    /// Addresses match exactly, so a loose comparison cannot widen one.
    #[test]
    fn addresses_match_exactly() {
        let ban = Ban::permanent(BanKind::Address, "10.0.0.5", "");
        assert!(ban.matches(&BanKind::Address, "10.0.0.5"));
        assert!(!ban.matches(&BanKind::Address, "10.0.0.50"));
    }
}
