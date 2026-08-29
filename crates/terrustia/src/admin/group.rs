//! What a group of players is allowed to do.
//!
//! Permissions are dotted, namespaced strings — `server.kick`, `world.time`, `panel.console` — so
//! the config reads as English, an unknown one is a typo rather than a silent grant, and a family
//! of related commands can be granted at once with a trailing `*` (`server.*` grants every
//! `server.` permission, present and future). The bare `*` grants everything. A group holds a set
//! of permission strings; `Group::may` (and the free function [`grants`] it is built on) is the one
//! place that decides whether a held set satisfies a requested name.
//!
//! This replaces an earlier four-variant `Permission` enum (`Look`/`World`/`Players`/`Admin`) that
//! was deliberately coarse. It did not survive contact with a real moderation toolkit: there was no
//! way to hand somebody `kick` without also handing them `ban`, or to let the panel show a
//! read-only settings page without the account behind it also being able to run arbitrary console
//! commands. [`migrate`] carries an existing admin file's old coarse words forward into the new
//! vocabulary the first time it loads — see its own doc comment for the mapping and why each choice
//! was made.

use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

/// A permission name: a dotted string such as `server.kick`, or a wildcard (`*`, or `family.*`).
///
/// A thin newtype over `&'static str` rather than a bare string, so the built-in vocabulary in
/// [`perm`] is only ever spelled once and a typo in a call site (`peram::SERVER_KCIK`) is a compile
/// error rather than a permission nobody has that silently refuses everyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Permission(&'static str);

impl Permission {
    /// The name this permission is written as in the config and checked against on the wire.
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// The built-in permission vocabulary, one constant per leaf. Grouped by family so the dotted name
/// and the constant's place in this list agree. A command or panel route asks for one of these
/// (never a bare string), so a typo here is the only place one could ever hide.
pub mod perm {
    use super::Permission;

    // server.* — acting on other players: moderation, not world-shaping.
    /// Read-only questions about the server (`/players`, `/npcs`, `/where`, `/house`, `/help`).
    /// Nothing currently checks this — the old `Permission::Look` it replaces never gated anything
    /// either, see [`super::migrate`] — but it is real, granted vocabulary so a future command can
    /// adopt it without inventing a new name, and so `moderator`'s ladder entry (`kick/mute/look`)
    /// names something that exists.
    pub const SERVER_LOOK: Permission = Permission("server.look");
    pub const SERVER_KICK: Permission = Permission("server.kick");
    pub const SERVER_BAN: Permission = Permission("server.ban");
    pub const SERVER_UNBAN: Permission = Permission("server.unban");
    pub const SERVER_MUTE: Permission = Permission("server.mute");
    pub const SERVER_UNMUTE: Permission = Permission("server.unmute");
    /// `/world undo` — reverting a player's recent tile edits. Named under `server.` rather than
    /// `world.` because it is aimed at a player's actions, not the world's shape, matching the old
    /// coarse system (it needed `Permission::Players`, not `Permission::World`).
    pub const SERVER_UNDO: Permission = Permission("server.undo");
    /// The console/panel `whitelist add|remove` commands. Viewing the list needs only `panel.view`;
    /// changing it needs this.
    pub const SERVER_WHITELIST: Permission = Permission("server.whitelist");

    // world.* — shaping the world itself.
    pub const WORLD_TIME: Permission = Permission("world.time");
    pub const WORLD_SAVE: Permission = Permission("world.save");
    pub const WORLD_SPAWN: Permission = Permission("world.spawn");
    pub const WORLD_BUTCHER: Permission = Permission("world.butcher");
    /// Changing the message of the day from the panel.
    pub const WORLD_MOTD: Permission = Permission("world.motd");
    /// Restarting the process into a different world file.
    pub const WORLD_SWITCH: Permission = Permission("world.switch");
    /// Generating a brand-new world from the panel.
    pub const WORLD_NEW: Permission = Permission("world.new");
    /// Restoring a rotating backup — destructive, and distinct from `world.save`.
    pub const WORLD_ROLLBACK: Permission = Permission("world.rollback");

    // panel.* — the web admin panel itself.
    /// May sign in to the panel at all, and use its read-only views.
    pub const PANEL_VIEW: Permission = Permission("panel.view");
    /// May send a raw line down `/api/console` or `/api/chat` — the same unrestricted channel the
    /// server's own terminal uses (`run_console`). Holding this is equivalent to holding every
    /// other permission there is, since a console line can run `group <self> owner`.
    pub const PANEL_CONSOLE: Permission = Permission("panel.console");

    // admin.* — changing who is allowed to do what.
    /// Edit a group's own permission set, and move an account between groups. Deliberately powerful
    /// and deliberately not handed to `admin` by default — see [`super::defaults`].
    pub const ADMIN_GROUPS: Permission = Permission("admin.groups");
    /// Create, delete and list accounts, without touching what any group may do.
    pub const ADMIN_ACCOUNTS: Permission = Permission("admin.accounts");
    /// Read the audit log.
    pub const ADMIN_AUDIT: Permission = Permission("admin.audit");
}

/// Every leaf permission, for defaults, migration, and the built-in half of the vocabulary registry.
const BUILTIN_LEAVES: &[Permission] = &[
    perm::SERVER_LOOK,
    perm::SERVER_KICK,
    perm::SERVER_BAN,
    perm::SERVER_UNBAN,
    perm::SERVER_MUTE,
    perm::SERVER_UNMUTE,
    perm::SERVER_UNDO,
    perm::SERVER_WHITELIST,
    perm::WORLD_TIME,
    perm::WORLD_SAVE,
    perm::WORLD_SPAWN,
    perm::WORLD_BUTCHER,
    perm::WORLD_MOTD,
    perm::WORLD_SWITCH,
    perm::WORLD_NEW,
    perm::WORLD_ROLLBACK,
    perm::PANEL_VIEW,
    perm::PANEL_CONSOLE,
    perm::ADMIN_GROUPS,
    perm::ADMIN_ACCOUNTS,
    perm::ADMIN_AUDIT,
];

/// The family wildcards every leaf above belongs to, plus the bare `*`. Listed as their own known
/// vocabulary entries so the panel's group editor can offer "grant the whole family" as a single,
/// spelled-correctly choice rather than the operator having to type `server.*` from memory.
const BUILTIN_WILDCARDS: &[&str] = &["*", "server.*", "world.*", "panel.*", "admin.*"];

/// The known-vocabulary registry: every built-in name plus whatever a plugin has registered with
/// [`register`]. Behind a `Mutex` rather than anything lock-free because it is written rarely (once
/// per plugin at startup) and read by a human-facing list, never on the packet path.
fn registry() -> &'static Mutex<BTreeSet<String>> {
    static REGISTRY: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut set: BTreeSet<String> = BUILTIN_LEAVES
            .iter()
            .map(|p| p.as_str().to_string())
            .collect();
        set.extend(BUILTIN_WILDCARDS.iter().map(|s| (*s).to_string()));
        Mutex::new(set)
    })
}

/// Register a permission name into the known vocabulary, so it appears in [`known`] and validates
/// as recognised rather than a typo. This is the whole of the plugin surface this module offers: a
/// future plugin that wants its own namespace (`myplugin.frobnicate`) calls this once at load time
/// and is otherwise on its own — there is no broader plugin API here.
pub fn register(name: &str) {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.to_string());
}

/// Every known permission name, sorted, for validation and for listing (the panel's group editor's
/// picker, and `admin groups` style console output).
pub fn known() -> Vec<String> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .cloned()
        .collect()
}

/// Whether a name is recognised: built in, or registered by [`register`]. Used to validate a
/// group-permission edit before it is saved, rather than after — so the panel's group editor
/// refuses `sever.kick` (a typo) up front instead of silently granting nothing.
pub fn is_known(name: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(name)
}

/// Whether a held set of permission strings satisfies a requested name: an exact match, the bare
/// `*`, or a family wildcard on a segment boundary (`server.*` satisfies `server.kick`, but not
/// `serverish.kick` and not a bare `serverstuff`). Checked from the most specific ancestor down to
/// the least, so `a.b.c` is tested against `a.b.*` before `a.*`.
///
/// This is the one place wildcard matching happens; [`Group::may`] and the escalation guard in
/// [`crate::admin::store::Admin::group_within_reach`] both call through it rather than each keeping
/// their own copy.
pub fn grants(held: &BTreeSet<String>, requested: &str) -> bool {
    if held.contains("*") || held.contains(requested) {
        return true;
    }
    let mut end = requested.len();
    while let Some(dot) = requested[..end].rfind('.') {
        let mut candidate = String::with_capacity(dot + 2);
        candidate.push_str(&requested[..dot]);
        candidate.push_str(".*");
        if held.contains(candidate.as_str()) {
            return true;
        }
        end = dot;
    }
    false
}

/// A named set of permissions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Group {
    pub name: String,
    /// Permission names (leaves, family wildcards like `server.*`, or the single entry `*`).
    pub permissions: BTreeSet<String>,
}

impl Group {
    /// A group holding exactly these permissions.
    pub fn of(name: &str, permissions: &[Permission]) -> Self {
        Self {
            name: name.to_string(),
            permissions: permissions.iter().map(|p| p.as_str().to_string()).collect(),
        }
    }

    /// A group that may do anything, now and after any permission is added later.
    pub fn everything(name: &str) -> Self {
        Self {
            name: name.to_string(),
            permissions: BTreeSet::from(["*".to_string()]),
        }
    }

    /// Whether this group's held permissions grant a name, exactly or through a wildcard. See
    /// [`grants`] for the matching rule.
    pub fn may(&self, permission: Permission) -> bool {
        grants(&self.permissions, permission.as_str())
    }

    /// The raw-string form of [`Self::may`], for a caller checking a permission that came from
    /// outside this vocabulary — another group's own permission set, in
    /// [`crate::admin::store::Admin::group_within_reach`].
    pub fn grants_str(&self, permission: &str) -> bool {
        grants(&self.permissions, permission)
    }
}

/// The groups a server starts with: a four-tier ladder, each one strictly weaker than the next.
///
/// * `default` — self-service only. Registering, logging in, logging out and `/whoami` all need no
///   permission at all (see `server/console.rs`'s `run_command`), so this group's own permission set
///   is nearly vestigial; it carries `server.look` only to match what every group has always carried
///   (the old `Permission::Look`, which gated nothing either — see [`migrate`]'s own doc comment).
/// * `moderator` — day-to-day moderation: kick, mute/unmute, the read-only "look" commands, and
///   `panel.view` so it can do all of that from the panel too. No bans, no world-shaping, no
///   account/group administration.
/// * `admin` — everything moderator has, plus bans, whitelist management, world management (time,
///   save, spawn, butcher, motd, switch, new, rollback) and account administration. Deliberately
///   **not** `admin.groups` (editing what a group may do) and **not** `panel.console` (a raw,
///   unrestricted console/chat line) — either would let an `admin` account grant itself anything,
///   including `owner`. That is the one property this ladder exists to guarantee: nobody below
///   `owner` can promote themselves to `owner`.
/// * `owner` — `*`, everything, forever.
pub fn defaults() -> Vec<Group> {
    vec![
        Group::of("default", &[perm::SERVER_LOOK]),
        Group::of(
            "moderator",
            &[
                perm::SERVER_LOOK,
                perm::SERVER_KICK,
                perm::SERVER_MUTE,
                perm::SERVER_UNMUTE,
                perm::PANEL_VIEW,
            ],
        ),
        Group::of(
            "admin",
            &[
                perm::SERVER_LOOK,
                perm::SERVER_KICK,
                perm::SERVER_BAN,
                perm::SERVER_UNBAN,
                perm::SERVER_MUTE,
                perm::SERVER_UNMUTE,
                perm::SERVER_UNDO,
                perm::SERVER_WHITELIST,
                perm::WORLD_TIME,
                perm::WORLD_SAVE,
                perm::WORLD_SPAWN,
                perm::WORLD_BUTCHER,
                perm::WORLD_MOTD,
                perm::WORLD_SWITCH,
                perm::WORLD_NEW,
                perm::WORLD_ROLLBACK,
                perm::PANEL_VIEW,
                perm::ADMIN_ACCOUNTS,
                perm::ADMIN_AUDIT,
            ],
        ),
        Group::everything("owner"),
    ]
}

/// What a pre-namespace coarse permission word maps onto, and why.
///
/// Applied once, transparently, the first time an admin file written before this system existed is
/// loaded (see `Admin::load`), then saved back so the rewrite happens exactly once. Each mapping is
/// chosen to reproduce the *old permission's actual reach* rather than its name:
///
/// * `look` gated nothing at all in the command dispatcher (`Permission::Look` was checked by
///   nothing in `run_command`'s old coarse table) → `server.look`, which is equally inert today.
///   Carried forward anyway rather than dropped, so a group an operator deliberately gave `look` to
///   keeps whatever documentation value that had.
/// * `world` gated every world-shaping command (`time`, `save`, `spawn`, `butcher`) → `world.*`, the
///   whole family, so a group that could do all four before still can, and picks up any future
///   `world.` command too.
/// * `players` gated `kick`, `ban`, `unban`, and — easy to miss — the `world undo` command (it
///   needed `Permission::Players`, not `Permission::World`, in the old table) → `server.*`, which
///   reproduces all four and additionally includes `server.mute`/`server.unmute`, new in this
///   release. That is a strict widening for anyone who had `players`, chosen deliberately: granting
///   a little more moderation power on upgrade is a far smaller surprise than an admin file that
///   quietly stops being able to `/kick` after a routine update.
/// * `admin` gated the `group` command, and — separately, and more importantly — was the *only*
///   check standing between an account and full, unrestricted web-panel console access (the panel's
///   login used to require exactly `Permission::Admin`). Holding old `admin` was therefore already
///   equivalent to near-total control, so it maps to the bare `*` rather than the narrower
///   `admin.accounts`/`admin.groups`: anything less would silently downgrade an operator whose
///   group used to be able to do anything through the panel.
fn migrate_legacy_word(word: &str) -> Option<&'static [&'static str]> {
    match word {
        "look" => Some(&["server.look"]),
        "world" => Some(&["world.*"]),
        "players" => Some(&["server.*"]),
        "admin" => Some(&["*"]),
        _ => None,
    }
}

/// Rewrite a group's permission set in place if it contains any pre-namespace coarse word
/// (`look`/`world`/`players`/`admin`), replacing each with its mapping from
/// [`migrate_legacy_word`]. Returns whether anything changed, so the caller ([`Admin::load`]) knows
/// whether the file needs writing back.
///
/// Safe to call on an already-namespaced set: none of the four legacy words collides with any real
/// namespaced permission (every one of those contains a `.`), so a set with no legacy word in it is
/// returned unchanged and reports no change.
pub fn migrate(permissions: &mut BTreeSet<String>) -> bool {
    let mut changed = false;
    let mut next = BTreeSet::new();
    for entry in permissions.iter() {
        match migrate_legacy_word(entry) {
            Some(replacements) => {
                changed = true;
                for r in replacements {
                    next.insert((*r).to_string());
                }
            }
            None => {
                next.insert(entry.clone());
            }
        }
    }
    if changed {
        *permissions = next;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default group can register/log in (which need no permission at all — see
    /// `server/console.rs`) and its one nominal permission gates nothing, matching the old
    /// `Permission::Look`.
    #[test]
    fn the_default_group_cannot_moderate_or_administer() {
        let groups = defaults();
        let default = groups
            .iter()
            .find(|g| g.name == "default")
            .expect("default");
        assert!(!default.may(perm::SERVER_KICK));
        assert!(!default.may(perm::SERVER_BAN));
        assert!(!default.may(perm::WORLD_TIME));
        assert!(!default.may(perm::PANEL_VIEW));
        assert!(!default.may(perm::ADMIN_GROUPS));
    }

    /// Moderator gets exactly kick/mute/unmute/look plus panel access, nothing destructive.
    #[test]
    fn moderator_may_moderate_but_not_administer() {
        let groups = defaults();
        let moderator = groups.iter().find(|g| g.name == "moderator").unwrap();
        assert!(moderator.may(perm::SERVER_KICK));
        assert!(moderator.may(perm::SERVER_MUTE));
        assert!(moderator.may(perm::SERVER_UNMUTE));
        assert!(moderator.may(perm::PANEL_VIEW));
        assert!(!moderator.may(perm::SERVER_BAN), "no bans for a moderator");
        assert!(!moderator.may(perm::WORLD_TIME), "no world-shaping either");
        assert!(!moderator.may(perm::PANEL_CONSOLE));
        assert!(!moderator.may(perm::ADMIN_GROUPS));
    }

    /// Admin can do almost everything, but not edit groups and not touch the raw console — the two
    /// things that would let it promote itself to owner.
    #[test]
    fn admin_cannot_self_escalate() {
        let groups = defaults();
        let admin = groups.iter().find(|g| g.name == "admin").unwrap();
        assert!(admin.may(perm::SERVER_BAN));
        assert!(admin.may(perm::WORLD_BUTCHER));
        assert!(admin.may(perm::ADMIN_ACCOUNTS));
        assert!(
            !admin.may(perm::ADMIN_GROUPS),
            "editing group permissions would let admin grant itself anything"
        );
        assert!(
            !admin.may(perm::PANEL_CONSOLE),
            "a raw console line can run `group <self> owner`"
        );
        assert!(!admin.grants_str("*"));
    }

    /// A wildcard covers permissions that did not exist when it was written.
    #[test]
    fn the_owner_may_do_anything() {
        let owner = Group::everything("owner");
        for permission in [
            perm::SERVER_KICK,
            perm::WORLD_TIME,
            perm::PANEL_CONSOLE,
            perm::ADMIN_GROUPS,
        ] {
            assert!(owner.may(permission));
        }
    }

    /// A family wildcard grants every leaf in that family and nothing outside it.
    #[test]
    fn a_family_wildcard_is_scoped_to_its_own_segment() {
        let held = BTreeSet::from(["server.*".to_string()]);
        assert!(grants(&held, "server.kick"));
        assert!(grants(&held, "server.ban"));
        assert!(!grants(&held, "world.time"), "a different family");
        assert!(
            !grants(&held, "serverish.kick"),
            "must match on the segment boundary, not a text prefix"
        );
        assert!(
            !grants(&held, "server"),
            "the bare family name is not a leaf"
        );
    }

    /// Every built-in leaf and family wildcard is present in the vocabulary registry from the start.
    #[test]
    fn the_builtin_vocabulary_is_registered() {
        let known = known();
        assert!(known.contains(&"server.kick".to_string()));
        assert!(known.contains(&"panel.console".to_string()));
        assert!(known.contains(&"admin.groups".to_string()));
        assert!(known.contains(&"server.*".to_string()));
        assert!(known.contains(&"*".to_string()));
        assert!(!known.contains(&"nonsense.permission".to_string()));
    }

    /// A plugin registering its own namespace shows up in `known` and passes `is_known` afterward —
    /// the one piece of "plugin API" this module offers.
    #[test]
    fn a_registered_permission_becomes_known() {
        assert!(!is_known("exampleplugin.frobnicate"));
        register("exampleplugin.frobnicate");
        assert!(is_known("exampleplugin.frobnicate"));
    }

    /// Every legacy coarse word maps to something, and a set with none of them is left untouched.
    #[test]
    fn migration_maps_every_legacy_word_and_leaves_new_ones_alone() {
        let mut look = BTreeSet::from(["look".to_string()]);
        assert!(migrate(&mut look));
        assert_eq!(look, BTreeSet::from(["server.look".to_string()]));

        let mut world = BTreeSet::from(["world".to_string()]);
        assert!(migrate(&mut world));
        assert_eq!(world, BTreeSet::from(["world.*".to_string()]));

        let mut players = BTreeSet::from(["players".to_string()]);
        assert!(migrate(&mut players));
        assert_eq!(players, BTreeSet::from(["server.*".to_string()]));

        let mut admin = BTreeSet::from(["admin".to_string()]);
        assert!(migrate(&mut admin));
        assert_eq!(admin, BTreeSet::from(["*".to_string()]));

        let mut already_new = BTreeSet::from(["server.kick".to_string(), "panel.view".to_string()]);
        assert!(!migrate(&mut already_new), "nothing to migrate");
        assert_eq!(
            already_new,
            BTreeSet::from(["server.kick".to_string(), "panel.view".to_string()])
        );
    }

    /// A mixed set migrates only the legacy words and keeps the rest.
    #[test]
    fn migration_handles_a_mixed_set() {
        let mut mixed = BTreeSet::from(["look".to_string(), "panel.view".to_string()]);
        assert!(migrate(&mut mixed));
        assert_eq!(
            mixed,
            BTreeSet::from(["server.look".to_string(), "panel.view".to_string()])
        );
    }
}
