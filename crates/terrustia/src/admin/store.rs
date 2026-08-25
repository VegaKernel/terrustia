//! The admin file: groups, accounts and bans, on disk beside the world.
//!
//! TOML, because `toml` is already a dependency and this is hundreds of rows rather than millions.
//! Written through the same temp-file-and-rename the world save uses, so a crash mid-write leaves
//! the old file rather than half of a new one.
//!
//! A missing file is not an error. A server that refuses to start because nobody has made an admin
//! file yet is a server that gets run with the checks turned off.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use tracing::{info, warn};

use super::{Account, Ban, BanKind, Group, Permission, ban::now, group::defaults};

/// Everything the server knows about who may do what.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Admin {
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub bans: Vec<Ban>,

    /// Where this came from, so it can be written back.
    #[serde(skip)]
    path: Option<PathBuf>,
    /// Who is logged in right now, by player slot. Never written: a session is not state.
    #[serde(skip)]
    signed_in: HashMap<u8, String>,
}

impl Admin {
    /// An admin store with nowhere to write itself.
    ///
    /// For a world that is not being saved: an ephemeral server should not leave an admin file in
    /// whatever directory it happened to be started from. Tests are the obvious case, and they
    /// found this the hard way — the file persisted between runs, so the second run started
    /// already claimed and the test that had just passed failed.
    pub fn in_memory() -> Self {
        Self {
            groups: defaults(),
            ..Self::default()
        }
    }

    /// Read the admin file, or start from sensible defaults if there is not one yet.
    pub fn load(path: &Path) -> Self {
        let mut admin = match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(admin) => {
                    info!(
                        path = %path.display(),
                        groups = admin.groups.len(),
                        accounts = admin.accounts.len(),
                        bans = admin.bans.len(),
                        "admin file loaded",
                    );
                    admin
                }
                Err(e) => {
                    // Loudly, and then carry on with the defaults: refusing to start would be
                    // worse, and running with *no* permissions at all would be worse still.
                    warn!(path = %path.display(), error = %e, "admin file is malformed; using defaults");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        };
        if admin.groups.is_empty() {
            admin.groups = defaults();
        }
        admin.path = Some(path.to_path_buf());
        admin
    }

    /// Write it back, atomically.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, path)
    }

    /// The group somebody belongs to, or the default group.
    pub fn group_of(&self, slot: u8) -> &Group {
        let name = self
            .signed_in
            .get(&slot)
            .and_then(|account| self.accounts.iter().find(|a| &a.name == account))
            .map_or("default", |a| a.group.as_str());
        self.groups
            .iter()
            .find(|g| g.name == name)
            .or_else(|| self.groups.iter().find(|g| g.name == "default"))
            .unwrap_or_else(|| &self.groups[0])
    }

    /// Whether anybody has claimed this server yet.
    ///
    /// Until somebody does, it behaves the way it always did — everyone may do everything. A
    /// server that locked its own commands away the moment this landed, before anyone could
    /// possibly have an account, would be a server people turn the checks off on. Registering the
    /// first account claims ownership and engages the gate in the same movement.
    pub fn unclaimed(&self) -> bool {
        self.accounts.is_empty()
    }

    /// Whether this player may do this.
    pub fn may(&self, slot: u8, permission: Permission) -> bool {
        self.unclaimed() || self.group_of(slot).may(permission)
    }

    /// Sign in, returning whether the password was right.
    pub fn sign_in(&mut self, slot: u8, name: &str, password: &str) -> bool {
        let Some(account) = self
            .accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        if !account.verify(password) {
            return false;
        }
        let account = account.name.clone();
        self.signed_in.insert(slot, account);
        true
    }

    /// The stored hash for an account, so it can be verified somewhere other than the game task.
    ///
    /// Argon2 is deliberately expensive — tens of milliseconds against a 16.67 ms tick — so a
    /// verification cannot happen inline without stalling the world for everyone. The hash comes
    /// out here, the comparison happens on a worker thread, and [`Self::complete_sign_in`] applies
    /// the answer. Handing out a PHC string is safe: it is a hash and a salt, not a password.
    pub fn account_hash(&self, name: &str) -> Option<String> {
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .map(|a| a.hash.clone())
    }

    /// Record a sign-in whose password was already checked off the game task.
    pub fn complete_sign_in(&mut self, slot: u8, name: &str) {
        let Some(account) = self
            .accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        else {
            return;
        };
        let account = account.name.clone();
        self.signed_in.insert(slot, account);
    }

    /// Whether a name is already taken, checked before paying for a hash.
    pub fn name_taken(&self, name: &str) -> bool {
        self.accounts
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Add an account whose password was hashed off the game task.
    pub fn insert_account(&mut self, account: Account) -> Result<(), String> {
        if self.name_taken(&account.name) {
            return Err(format!(
                "there is already an account called {}",
                account.name
            ));
        }
        self.accounts.push(account);
        Ok(())
    }

    /// Forget a session. Called when somebody leaves, so a reused slot does not inherit rights.
    pub fn sign_out(&mut self, slot: u8) {
        self.signed_in.remove(&slot);
    }

    /// Who this slot is signed in as.
    pub fn signed_in_as(&self, slot: u8) -> Option<&str> {
        self.signed_in.get(&slot).map(String::as_str)
    }

    /// Add an account. Refuses a duplicate name rather than shadowing one.
    pub fn register(&mut self, name: &str, password: &str, group: &str) -> Result<(), String> {
        if self
            .accounts
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("there is already an account called {name}"));
        }
        if password.len() < 6 {
            return Err("that password is too short; use at least six characters".into());
        }
        self.accounts.push(Account::new(name, password, group)?);
        Ok(())
    }

    /// The ban that keeps this person out, if one does.
    pub fn ban_for(&self, name: &str, address: &str, uuid: Option<&str>) -> Option<&Ban> {
        let when = now();
        self.bans.iter().find(|ban| {
            ban.in_force(when)
                && (ban.matches(&BanKind::Name, name)
                    || ban.matches(&BanKind::Address, address)
                    || uuid.is_some_and(|uuid| ban.matches(&BanKind::Uuid, uuid)))
        })
    }

    /// Add a ban and write the file.
    pub fn ban(&mut self, kind: BanKind, value: &str, reason: &str) {
        self.bans.push(Ban::permanent(kind, value, reason));
        if let Err(e) = self.save() {
            warn!(error = %e, "could not write the admin file; the ban is only in memory");
        }
    }

    /// Remove every ban naming this value. Returns how many went.
    pub fn unban(&mut self, value: &str) -> usize {
        let before = self.bans.len();
        self.bans
            .retain(|ban| !ban.value.eq_ignore_ascii_case(value));
        let removed = before - self.bans.len();
        if removed > 0
            && let Err(e) = self.save()
        {
            warn!(error = %e, "could not write the admin file after unbanning");
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("terrustia-admin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(name)
    }

    /// A server nobody has claimed behaves as it always did: everyone may do everything.
    ///
    /// The alternative — locking the commands away the moment permissions landed, before anybody
    /// could have an account — is how a security feature becomes a thing people disable.
    #[test]
    fn an_unclaimed_server_is_open() {
        let admin = Admin::load(&temp("does-not-exist.toml"));
        assert!(!admin.groups.is_empty());
        assert!(admin.unclaimed());
        assert!(admin.may(0, Permission::World));
        assert!(admin.may(0, Permission::Admin));
    }

    /// The first account claims the server, and everyone else drops to the default group.
    #[test]
    fn registering_claims_the_server() {
        let mut admin = Admin::load(&temp("claim.toml"));
        admin
            .register("brook", "a good password", "owner")
            .expect("register");

        assert!(!admin.unclaimed());
        assert!(admin.may(0, Permission::Look), "looking is always allowed");
        assert!(
            !admin.may(0, Permission::World),
            "once claimed, a stranger cannot reshape the world",
        );
        assert!(!admin.may(0, Permission::Admin));
    }

    /// Signing in with the right password moves you into your group; a wrong one changes nothing.
    #[test]
    fn signing_in_grants_the_group() {
        let mut admin = Admin::load(&temp("signin.toml"));
        admin
            .register("brook", "a good password", "owner")
            .expect("register");

        assert!(!admin.may(3, Permission::Admin), "not until signed in");
        assert!(
            !admin.sign_in(3, "brook", "wrong"),
            "a wrong password fails"
        );
        assert!(!admin.may(3, Permission::Admin), "and grants nothing");

        assert!(admin.sign_in(3, "brook", "a good password"));
        assert!(admin.may(3, Permission::Admin));

        // And leaving gives it up, so whoever reuses the slot does not inherit it.
        admin.sign_out(3);
        assert!(!admin.may(3, Permission::Admin));
    }

    /// A name cannot be registered twice, and a short password is refused.
    #[test]
    fn registration_is_fussy() {
        let mut admin = Admin::load(&temp("register.toml"));
        admin
            .register("brook", "long enough", "default")
            .expect("first");
        assert!(admin.register("BROOK", "long enough", "owner").is_err());
        assert!(admin.register("other", "short", "default").is_err());
    }

    /// A ban keeps somebody out by any of the three names, and lifting it lets them back.
    #[test]
    fn bans_apply_and_lift() {
        let mut admin = Admin::load(&temp("bans.toml"));
        admin.ban(BanKind::Uuid, "abc-123", "griefing");

        assert!(
            admin
                .ban_for("anyone", "1.2.3.4", Some("abc-123"))
                .is_some()
        );
        assert!(
            admin.ban_for("anyone", "1.2.3.4", Some("other")).is_none(),
            "a different uuid is a different person",
        );
        assert_eq!(admin.unban("abc-123"), 1);
        assert!(
            admin
                .ban_for("anyone", "1.2.3.4", Some("abc-123"))
                .is_none()
        );
    }

    /// It survives a write and a read, which is the whole point of having a file.
    #[test]
    fn the_file_round_trips() {
        let path = temp("roundtrip.toml");
        let _ = std::fs::remove_file(&path);
        {
            let mut admin = Admin::load(&path);
            admin
                .register("brook", "a good password", "owner")
                .expect("register");
            admin.ban(BanKind::Name, "griefer", "wrecked spawn");
            admin.save().expect("save");
        }

        let mut back = Admin::load(&path);
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.bans.len(), 1);
        assert!(
            back.sign_in(0, "brook", "a good password"),
            "the hash has to survive the round trip or nobody can log in again",
        );
        let _ = std::fs::remove_file(&path);
    }
}
