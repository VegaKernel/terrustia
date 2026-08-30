//! Who is allowed to do what, and who is not allowed in at all.
//!
//! There was none of this. The comment above the command dispatcher said so outright — "there is
//! no permission model: this is aimed at a server among friends" — and the command list it sat
//! above was not as harmless as that suggests. Any connected player could set the world to night,
//! summon a boss beside somebody, or delete every NPC in the world with `/butcher`.
//!
//! Five pieces, deliberately small:
//!
//! * **Groups** carry permissions, dotted and namespaced (`server.kick`, `world.time`, ...) — see
//!   [`group`] for the vocabulary and the wildcard rule. A player belongs to one. The default group
//!   can register and sign in, and nothing else.
//! * **Accounts** bind a name to a password so a group survives a reconnect. Hashed with argon2 —
//!   the one place in this workspace where taking a dependency beats writing it, because a KDF
//!   written from memory is a security bug with a long fuse.
//! * **Bans** by name, address or the client UUID the server already receives and, until now,
//!   never read.
//! * **Mutes** ([`mute`]) — chat suppression by name, persisted the same way a ban is, off by
//!   default in the sense that nothing does anything until an operator actually mutes somebody.
//! * **The audit log** ([`audit`]) — an append-only, rotating record of who did what: every ban,
//!   kick, mute, group change and permission edit, attributed to an account, `"console"`, or (never
//!   yet, but the vocabulary is there) `"system"`.
//!
//! Stored as TOML beside the world, written atomically through the same temp-file-and-rename the
//! world save uses. Admin data is hundreds of rows; a database would be machinery for its own sake.
//!
//! **Convention: a password or a claim token is never logged, at any level, in any field.** Every
//! `tracing::` call and every [`audit::AuditLog::record`] on a login, register or claim path names
//! only outcomes (an account name, a group, a slot, whether a credential was `correct`), never
//! the credential itself, not even at `trace` and not inside an error branch. `Account`'s own
//! stored form is a PHC hash, checked here; the plain-text value never has a reason to reach a log
//! line at all. The one-time claim token is likewise never printed anywhere but the console's own
//! stdout ([`crate::game::server::GameServer::announce_claim_token`]), which is the same trust
//! boundary the world file itself already sits behind.

pub mod audit;
pub mod ban;
pub mod group;
pub mod mute;
pub mod store;
pub mod throttle;

pub use audit::{AuditAction, AuditLog};
pub use ban::{Ban, BanKind};
pub use group::{Group, Permission, perm};
pub use mute::Mute;
pub use store::Admin;
pub use throttle::{REFUSAL_MESSAGE, Throttle, Verdict};

/// An account: a name, a hashed password, and the group it belongs to.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    pub name: String,
    /// A PHC-format argon2 string. Never a password, and never reversible.
    pub hash: String,
    pub group: String,
}

impl Account {
    /// Hash a password for storage.
    ///
    /// Fails only if the hasher itself refuses, which in practice means the salt source did.
    pub fn new(name: &str, password: &str, group: &str) -> Result<Self, String> {
        use argon2::{
            Argon2,
            password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
        };

        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| format!("could not hash that password: {e}"))?
            .to_string();
        Ok(Self {
            name: name.to_string(),
            hash,
            group: group.to_string(),
        })
    }

    /// Check a password against a stored PHC string, without needing the account.
    ///
    /// The free function exists so the comparison can run on a worker thread while the game task
    /// gets on with the tick. See [`crate::admin::Admin::account_hash`].
    pub fn verify_hash(hash: &str, password: &str) -> bool {
        use argon2::{
            Argon2,
            password_hash::{PasswordHash, PasswordVerifier},
        };

        let Ok(parsed) = PasswordHash::new(hash) else {
            return false;
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }

    /// Whether this password is the one the account was made with.
    ///
    /// Argon2's own verifier does the comparison, which is constant-time — a hand-rolled `==` on
    /// hashes is the classic way to leak one a byte at a time.
    pub fn verify(&self, password: &str) -> bool {
        Self::verify_hash(&self.hash, password)
    }
}

/// Compare two byte strings without leaking their contents through timing.
///
/// The one shared primitive behind every plain-secret compare in this crate: the server join
/// password (`game::server::dispatch::on_password`) and the claim token, checked both from the
/// console (`game::server::console::run_admin_command`'s `"register"` arm) and from the web panel
/// (`panel::mod::login`). None of these is a high-value secret (the claim token in particular is
/// spent once and lives for a single process), but a length-independent compare costs nothing and
/// avoids having to argue about it three separate times. (Argon2's own verifier already handles
/// this for password hashes; this helper is only for the plain-string cases that never go through
/// it.)
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A password verifies against its own hash and nothing else.
    #[test]
    fn a_password_round_trips() {
        let account = Account::new("brook", "correct horse", "admin").expect("hashing");
        assert!(account.verify("correct horse"));
        assert!(!account.verify("correct hors"));
        assert!(!account.verify(""));
    }

    /// The password itself is never stored, in any form anybody could read back.
    #[test]
    fn the_password_is_not_in_the_record() {
        let account = Account::new("brook", "hunter2", "default").expect("hashing");
        assert!(
            !account.hash.contains("hunter2"),
            "the stored hash must not contain the password",
        );
        assert!(account.hash.starts_with("$argon2"), "{}", account.hash);
    }

    /// Two accounts with the same password get different hashes, because each has its own salt.
    #[test]
    fn identical_passwords_hash_differently() {
        let one = Account::new("a", "same", "default").expect("hashing");
        let two = Account::new("b", "same", "default").expect("hashing");
        assert_ne!(
            one.hash, two.hash,
            "equal hashes would mean no salt, and a stolen file would rank passwords by frequency",
        );
    }

    /// The primitive every claim-token and join-password compare site now shares. Timing itself
    /// is not something a unit test can observe; what is checked here is the ordinary correctness
    /// every one of those call sites depends on: right accepted, wrong (whether shorter, longer,
    /// or the same length) refused.
    #[test]
    fn constant_time_eq_accepts_the_right_value_and_refuses_every_wrong_one() {
        assert!(constant_time_eq(b"correct-token", b"correct-token"));
        assert!(
            !constant_time_eq(b"correct-token", b"wrong-token12"),
            "same length, wrong bytes"
        );
        assert!(
            !constant_time_eq(b"correct-token", b"correct-toke"),
            "shorter than the real one"
        );
        assert!(
            !constant_time_eq(b"correct-token", b"correct-tokenX"),
            "longer than the real one"
        );
        assert!(constant_time_eq(b"", b""), "two empty strings still match");
        assert!(!constant_time_eq(b"", b"x"));
    }
}
