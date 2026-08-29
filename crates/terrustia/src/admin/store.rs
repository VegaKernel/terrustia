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

use super::{Account, Ban, BanKind, Group, Mute, Permission, ban::now, group, group::defaults};

/// Everything the server knows about who may do what.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Admin {
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub bans: Vec<Ban>,
    /// Active (or not-yet-cleaned-up expired) mutes. `#[serde(default)]` so a store written before
    /// this field existed still loads with an empty list rather than refusing to parse.
    #[serde(default)]
    pub mutes: Vec<Mute>,
    /// Who is allowed in, when the list is not empty.
    ///
    /// Bans are reactive: they keep out somebody who has already been and done something. A
    /// whitelist is the thing that actually keeps a private server private, and it was the one
    /// piece of moderation this server had no version of at all.
    ///
    /// Empty means off, deliberately. A whitelist that defaults to on would lock the operator out
    /// of their own server the first time they enabled it, which is how people end up leaving it
    /// off for good.
    #[serde(default)]
    pub whitelist: Vec<String>,

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

        // A store written before permissions were namespaced still has the old coarse words
        // (`look`/`world`/`players`/`admin`) in one or more groups. Map them across once — see
        // `group::migrate`'s own doc comment for exactly what each word becomes and why — and save
        // immediately so the rewrite happens exactly once rather than on every boot.
        let mut migrated = false;
        for group in &mut admin.groups {
            if group::migrate(&mut group.permissions) {
                migrated = true;
            }
        }
        if migrated {
            info!(
                path = %path.display(),
                "migrated legacy coarse group permissions to the namespaced vocabulary",
            );
            if let Err(e) = admin.save() {
                warn!(
                    error = %e,
                    "could not persist the migrated admin file; migration will be retried next boot",
                );
            }
        }
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

    /// Whether the named group grants a permission. An unknown group name grants nothing — an
    /// account tagged with a group that no longer exists must not be treated as privileged. Used by
    /// the web panel to decide, off the game task, whether an account may drive it at all.
    pub fn group_grants(&self, group_name: &str, permission: Permission) -> bool {
        self.groups
            .iter()
            .any(|g| g.name == group_name && g.may(permission))
    }

    /// The raw-string form of [`Self::group_grants`], for a permission name that arrived over the
    /// wire (the panel's per-route `PanelAuthorize` check) rather than as a compile-time constant.
    pub fn group_grants_str(&self, group_name: &str, permission: &str) -> bool {
        self.groups
            .iter()
            .any(|g| g.name == group_name && g.grants_str(permission))
    }

    /// The group an account belongs to, by name, if the account exists. Used alongside
    /// [`Self::group_within_reach`] wherever an actor's own group needs to be found first.
    pub fn account_group(&self, name: &str) -> Option<&str> {
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .map(|a| a.group.as_str())
    }

    /// Whether everything `target_group` may do is already something `actor_group` may do — the
    /// general rule that stops any account/group change from ever handing out more power than the
    /// person making the change already holds themselves.
    ///
    /// This is what actually keeps `admin.accounts` safe to grant to the `admin` tier even though
    /// `admin` lacks `admin.groups`: without it, an `admin`-tier account could reassign *itself* into
    /// `owner` through the ordinary account-group-change route, which is exactly the
    /// self-escalation the ladder in `group::defaults` is trying to rule out. Both an unknown actor
    /// group and an unknown target group return `false` — nothing is "within reach" of a group that
    /// does not exist, and a target that does not exist cannot be reached either.
    ///
    /// Used for two different edits, both of which are really "does this change grant power the
    /// actor doesn't have": moving an account into a different group (every permission the *target*
    /// group holds must already be within the *actor*'s reach), and adding a permission to a group
    /// (the single permission being added must be within the actor's reach).
    pub fn group_within_reach(&self, actor_group: &str, target_group: &str) -> bool {
        let Some(actor) = self.groups.iter().find(|g| g.name == actor_group) else {
            return false;
        };
        let Some(target) = self.groups.iter().find(|g| g.name == target_group) else {
            return false;
        };
        target.permissions.iter().all(|p| actor.grants_str(p))
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

    /// An account's hash and group in one lookup, for a caller (the web panel) that needs both to
    /// verify a login off the game task and then know what the signed-in account may do.
    pub fn account_hash_and_group(&self, name: &str) -> Option<(String, String)> {
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
            .map(|a| (a.hash.clone(), a.group.clone()))
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

    /// Whether the whitelist is in use at all.
    pub fn whitelist_on(&self) -> bool {
        !self.whitelist.is_empty()
    }

    /// Whether this name is allowed in.
    ///
    /// Case-insensitive, because a name is typed by a person and "Brooklyn" and "brooklyn" are
    /// the same guest.
    pub fn welcome(&self, name: &str) -> bool {
        !self.whitelist_on() || self.whitelist.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    /// Add somebody to the guest list. Returns whether they were not already on it.
    pub fn add_to_whitelist(&mut self, name: &str) -> bool {
        if self.whitelist.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return false;
        }
        self.whitelist.push(name.to_string());
        true
    }

    /// Take somebody off it. Returns whether they were on it.
    pub fn remove_from_whitelist(&mut self, name: &str) -> bool {
        let before = self.whitelist.len();
        self.whitelist.retain(|n| !n.eq_ignore_ascii_case(name));
        self.whitelist.len() != before
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
    pub fn ban(&mut self, kind: BanKind, value: &str, reason: &str, issuer: &str) {
        self.bans.push(Ban::permanent(kind, value, reason, issuer));
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

    /// Mute a name, replacing any mute already in force for it (a second `/mute` on an already-
    /// muted name updates the reason and duration rather than stacking a second entry). `duration`
    /// is `None` for permanent, or seconds from now.
    pub fn mute(&mut self, name: &str, reason: &str, duration: Option<u64>, issuer: &str) {
        let until = duration.map(|secs| now().saturating_add(secs));
        self.mutes.retain(|m| !m.name.eq_ignore_ascii_case(name));
        self.mutes.push(Mute::new(name, reason, until, issuer));
        if let Err(e) = self.save() {
            warn!(error = %e, "could not write the admin file; the mute is only in memory");
        }
    }

    /// Lift every mute on this name (ordinarily just one — `mute` itself never stacks them, but an
    /// old hand-edited file could). Returns whether anything was actually removed.
    pub fn unmute(&mut self, name: &str) -> bool {
        let before = self.mutes.len();
        self.mutes.retain(|m| !m.name.eq_ignore_ascii_case(name));
        let removed = self.mutes.len() != before;
        if removed && let Err(e) = self.save() {
            warn!(error = %e, "could not write the admin file after unmuting");
        }
        removed
    }

    /// Whether this name is currently muted (an expired mute does not count — it is left in the
    /// list rather than swept, the same way a lapsed ban is).
    pub fn is_muted(&self, name: &str) -> bool {
        let when = now();
        self.mutes
            .iter()
            .any(|m| m.name.eq_ignore_ascii_case(name) && m.in_force(when))
    }

    /// The reason a currently-in-force mute gives, if this name is muted at all.
    pub fn mute_reason(&self, name: &str) -> Option<&str> {
        let when = now();
        self.mutes
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name) && m.in_force(when))
            .map(|m| m.reason.as_str())
    }

    /// Extend an already-active, non-permanent mute by `extra_secs`, capped so it never reaches
    /// more than `cap_secs` from now (`0` means no cap) — the mechanism behind
    /// `Config::mute_escalation_enabled`. Returns the new expiry, or `None` if the name is not
    /// currently muted, or its mute is already permanent (nothing to extend: it cannot get any
    /// "longer" than forever).
    pub fn extend_mute(&mut self, name: &str, extra_secs: u64, cap_secs: u64) -> Option<u64> {
        let when = now();
        let mute = self
            .mutes
            .iter_mut()
            .find(|m| m.name.eq_ignore_ascii_case(name) && m.in_force(when))?;
        let current_until = mute.until?;
        let mut extended = current_until.max(when).saturating_add(extra_secs);
        if cap_secs > 0 {
            extended = extended.min(when.saturating_add(cap_secs));
        }
        mute.until = Some(extended);
        if let Err(e) = self.save() {
            warn!(error = %e, "could not write the admin file after extending a mute");
        }
        Some(extended)
    }
}

#[cfg(test)]
mod tests {
    use super::group::perm;
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
        assert!(admin.may(0, perm::WORLD_TIME));
        assert!(admin.may(0, perm::ADMIN_GROUPS));
    }

    /// Only `owner` may edit who is allowed to do what (`admin.groups`) — not even `admin`, since
    /// that permission is exactly the one that would let a group grant itself anything, up to and
    /// including `owner` itself. This also pins that the bogus `"everything"` group name grants
    /// nothing, since it is not a real group (the owner group is named `"owner"`).
    #[test]
    fn only_owner_may_edit_group_permissions() {
        let admin = Admin::load(&temp("group-edit-authz.toml"));
        assert!(admin.group_grants("owner", perm::ADMIN_GROUPS), "owner may");
        assert!(
            !admin.group_grants("admin", perm::ADMIN_GROUPS),
            "admin must not be able to grant itself anything"
        );
        assert!(
            !admin.group_grants("moderator", perm::ADMIN_GROUPS),
            "a moderator is not an admin"
        );
        assert!(
            !admin.group_grants("default", perm::ADMIN_GROUPS),
            "a look-only default account must not administer permissions"
        );
        assert!(
            !admin.group_grants("everything", perm::ADMIN_GROUPS),
            "\"everything\" is not a real group name, so it grants nothing"
        );
    }

    /// The escalation guard: an `admin`-tier account cannot reach `owner` through account/group
    /// reassignment, because `owner` holds `*` and `admin` does not. It *can* reach `moderator` and
    /// its own tier, since everything those groups hold is already within its own reach.
    #[test]
    fn group_within_reach_blocks_reassignment_above_ones_own_ceiling() {
        let admin = Admin::load(&temp("within-reach.toml"));
        assert!(
            !admin.group_within_reach("admin", "owner"),
            "admin must not be able to promote anyone to owner"
        );
        assert!(admin.group_within_reach("admin", "moderator"));
        assert!(admin.group_within_reach("admin", "admin"));
        assert!(
            admin.group_within_reach("owner", "owner"),
            "owner reaches anything"
        );
        assert!(
            !admin.group_within_reach("moderator", "admin"),
            "moderator cannot promote anyone into admin either"
        );
        assert!(
            !admin.group_within_reach("nonsense", "moderator"),
            "an unknown actor group reaches nothing"
        );
        assert!(
            !admin.group_within_reach("admin", "nonsense"),
            "an unknown target group cannot be reached"
        );
    }

    /// The first account claims the server, and everyone else drops to the default group.
    #[test]
    fn registering_claims_the_server() {
        let mut admin = Admin::load(&temp("claim.toml"));
        admin
            .register("brook", "a good password", "owner")
            .expect("register");

        assert!(!admin.unclaimed());
        assert!(admin.may(0, perm::SERVER_LOOK), "looking is always allowed");
        assert!(
            !admin.may(0, perm::WORLD_TIME),
            "once claimed, a stranger cannot reshape the world",
        );
        assert!(!admin.may(0, perm::ADMIN_GROUPS));
    }

    /// Signing in with the right password moves you into your group; a wrong one changes nothing.
    #[test]
    fn signing_in_grants_the_group() {
        let mut admin = Admin::load(&temp("signin.toml"));
        admin
            .register("brook", "a good password", "owner")
            .expect("register");

        assert!(!admin.may(3, perm::ADMIN_GROUPS), "not until signed in");
        assert!(
            !admin.sign_in(3, "brook", "wrong"),
            "a wrong password fails"
        );
        assert!(!admin.may(3, perm::ADMIN_GROUPS), "and grants nothing");

        assert!(admin.sign_in(3, "brook", "a good password"));
        assert!(admin.may(3, perm::ADMIN_GROUPS));

        // And leaving gives it up, so whoever reuses the slot does not inherit it.
        admin.sign_out(3);
        assert!(!admin.may(3, perm::ADMIN_GROUPS));
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
        admin.ban(BanKind::Uuid, "abc-123", "griefing", "brook");

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

    /// A mute applies, survives being re-checked, and lifts on `/unmute` — the same shape as bans,
    /// and it persists across an `Admin` reload, which is the whole point of storing it rather than
    /// keeping it in memory on the connection.
    #[test]
    fn a_mute_applies_persists_and_lifts() {
        let path = temp("mute.toml");
        {
            let mut admin = Admin::load(&path);
            assert!(!admin.is_muted("chatty"));
            admin.mute("chatty", "spamming caps", None, "brook");
            assert!(admin.is_muted("chatty"));
            assert!(
                admin.is_muted("CHATTY"),
                "case-insensitive, like a name ban"
            );
            assert_eq!(admin.mute_reason("chatty"), Some("spamming caps"));
        }

        // Reloading finds the same mute — it was written to disk, not only held in memory.
        let mut reloaded = Admin::load(&path);
        assert!(
            reloaded.is_muted("chatty"),
            "a mute must survive a reconnect/restart, which is what persisting it is for"
        );
        assert!(reloaded.unmute("chatty"));
        assert!(!reloaded.is_muted("chatty"));
        assert!(
            !reloaded.unmute("chatty"),
            "nothing left to lift a second time"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A timed mute lapses on its own, exactly like a timed ban.
    #[test]
    fn a_timed_mute_expires_on_its_own() {
        let mut admin = Admin::load(&temp("mute-expiry.toml"));
        admin.mute("chatty", "cooldown", Some(0), "brook");
        // `duration = Some(0)` means "until right now" — already expired by the time this reads it.
        assert!(!admin.is_muted("chatty"));
    }

    /// Re-muting an already-muted name replaces the entry rather than stacking a second one.
    #[test]
    fn re_muting_replaces_rather_than_stacks() {
        let mut admin = Admin::load(&temp("mute-restack.toml"));
        admin.mute("chatty", "first reason", None, "brook");
        admin.mute("chatty", "second reason", None, "brook");
        assert_eq!(
            admin.mutes.iter().filter(|m| m.name == "chatty").count(),
            1,
            "muting twice must not leave two entries"
        );
        assert_eq!(admin.mute_reason("chatty"), Some("second reason"));
    }

    /// The escalation mechanism: extending a mute pushes its expiry out, capped so it never grows
    /// past `cap_secs` from now, and does nothing to a mute that is already permanent.
    #[test]
    fn extending_a_mute_pushes_its_expiry_out_and_respects_the_cap() {
        let mut admin = Admin::load(&temp("mute-extend.toml"));
        admin.mute("chatty", "spam", Some(60), "brook");
        let extended = admin
            .extend_mute("chatty", 300, 0)
            .expect("an active timed mute should extend");
        let now = now();
        assert!(
            extended >= now + 300,
            "extending by 300s should push the expiry at least 300s out"
        );

        // A cap of 100s from now must never be exceeded, even though the raw extension would.
        let capped = admin
            .extend_mute("chatty", 10_000, 100)
            .expect("still active");
        assert!(capped <= now + 100, "the cap must be respected: {capped}");

        admin.mute("permanent", "no expiry", None, "brook");
        assert!(
            admin.extend_mute("permanent", 60, 0).is_none(),
            "a permanent mute has nothing to extend"
        );

        assert!(
            admin.extend_mute("nobody", 60, 0).is_none(),
            "extending a name that is not muted at all does nothing"
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
            admin.ban(BanKind::Name, "griefer", "wrecked spawn", "brook");
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

    /// A store written before permissions were namespaced (a literal, hand-written coarse-word
    /// TOML file — no `Admin` in this test build ever writes that format any more) loads with the
    /// old words transparently mapped to their namespaced equivalents, and the mapping is persisted
    /// so a second load sees the namespaced form already on disk. This is the fail-then-pass for
    /// `group::migrate`: before the migration call was wired into `Admin::load`, `admin.may(0,
    /// perm::WORLD_TIME)` here returned `false` for a group whose file said `"world"`, because
    /// nothing recognised the old word as the new permission.
    #[test]
    fn an_old_coarse_store_migrates_transparently_and_the_migration_persists() {
        let path = temp("legacy-migrate.toml");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            r#"
            [[groups]]
            name = "default"
            permissions = ["look"]

            [[groups]]
            name = "moderator"
            permissions = ["look", "world", "players"]
            "#,
        )
        .expect("write legacy admin file");

        let admin = Admin::load(&path);
        let moderator = admin
            .groups
            .iter()
            .find(|g| g.name == "moderator")
            .expect("moderator group");
        assert_eq!(
            moderator.permissions,
            std::collections::BTreeSet::from([
                "server.look".to_string(),
                "world.*".to_string(),
                "server.*".to_string(),
            ]),
            "the old words must be rewritten to their namespaced equivalents",
        );
        assert!(
            admin.may(0, perm::WORLD_TIME),
            "world -> world.* keeps world access"
        );
        assert!(
            admin.group_grants("moderator", perm::SERVER_KICK),
            "players -> server.* keeps kick access",
        );

        // And the file on disk now says the namespaced form, so a second load finds nothing left to
        // migrate — the on-disk text itself changed, not just the in-memory value.
        let text = std::fs::read_to_string(&path).expect("read back");
        assert!(
            !text.contains("\"world\""),
            "the raw legacy word must be gone: {text}"
        );
        assert!(text.contains("world.*"), "{text}");

        let _ = std::fs::remove_file(&path);
    }

    /// An empty guest list lets everyone in; a non-empty one lets only those on it in.
    ///
    /// Empty-means-off is deliberate. A whitelist that defaulted to on would lock the operator out
    /// of their own server the moment the feature landed, which is how people end up leaving it
    /// off permanently.
    #[test]
    fn an_empty_guest_list_is_no_guest_list() {
        let mut admin = Admin::in_memory();
        assert!(!admin.whitelist_on());
        assert!(admin.welcome("anybody at all"));

        assert!(admin.add_to_whitelist("Brooklyn"));
        assert!(admin.whitelist_on());
        assert!(admin.welcome("Brooklyn"));
        assert!(!admin.welcome("somebody else"));
    }

    /// A name is typed by a person, so case is not part of who they are.
    #[test]
    fn the_guest_list_does_not_care_about_case() {
        let mut admin = Admin::in_memory();
        admin.add_to_whitelist("Brooklyn");
        assert!(admin.welcome("brooklyn"));
        assert!(admin.welcome("BROOKLYN"));
        assert!(!admin.add_to_whitelist("bRoOkLyN"), "no duplicate entries");
        assert_eq!(admin.whitelist.len(), 1);
    }

    #[test]
    fn removing_the_last_guest_turns_the_list_off_again() {
        let mut admin = Admin::in_memory();
        admin.add_to_whitelist("Brooklyn");
        assert!(admin.remove_from_whitelist("brooklyn"));
        assert!(
            !admin.whitelist_on(),
            "an emptied list stops shutting people out"
        );
        assert!(admin.welcome("anybody"));
        assert!(
            !admin.remove_from_whitelist("nobody"),
            "and says when it did nothing"
        );
    }
}
