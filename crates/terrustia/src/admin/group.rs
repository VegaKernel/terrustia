//! What a group of players is allowed to do.
//!
//! Permissions are named rather than numbered so the config reads as English and an unknown one is
//! a typo rather than a silent grant. A group holds a set of them; `*` means everything, which is
//! what the owner's group has and nobody else's should.

use std::collections::BTreeSet;

/// One thing a command can require.
///
/// Deliberately coarse. Fine-grained permissions are a way of pretending a decision has been made
/// when it has not — these are the four kinds of thing the commands actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Ask the server about itself. Harmless, and the default group has it.
    Look,
    /// Change the world: the time, the weather, what is alive in it.
    World,
    /// Act on other players: kick, ban, mute.
    Players,
    /// Change who is allowed to do what.
    Admin,
}

impl Permission {
    /// The name this permission is written as in the config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Look => "look",
            Self::World => "world",
            Self::Players => "players",
            Self::Admin => "admin",
        }
    }

    /// Read one back, or `None` for a name nothing recognises.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "look" => Some(Self::Look),
            "world" => Some(Self::World),
            "players" => Some(Self::Players),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// A named set of permissions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Group {
    pub name: String,
    /// Permission names, or the single entry `*`.
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

    pub fn may(&self, permission: Permission) -> bool {
        self.permissions.contains("*") || self.permissions.contains(permission.as_str())
    }
}

/// The groups a server starts with: everyone can look, an owner can do anything.
pub fn defaults() -> Vec<Group> {
    vec![
        Group::of("default", &[Permission::Look]),
        Group::of(
            "moderator",
            &[Permission::Look, Permission::Players, Permission::World],
        ),
        Group::everything("owner"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default group can ask questions and nothing else.
    ///
    /// This is the whole point: `/butcher` deleted every NPC in the world and any player could run
    /// it, because there was no such thing as not being allowed.
    #[test]
    fn the_default_group_cannot_change_the_world() {
        let groups = defaults();
        let default = groups
            .iter()
            .find(|g| g.name == "default")
            .expect("default");
        assert!(default.may(Permission::Look));
        assert!(!default.may(Permission::World));
        assert!(!default.may(Permission::Players));
        assert!(!default.may(Permission::Admin));
    }

    /// A wildcard covers permissions that did not exist when it was written.
    #[test]
    fn the_owner_may_do_anything() {
        let owner = Group::everything("owner");
        for permission in [
            Permission::Look,
            Permission::World,
            Permission::Players,
            Permission::Admin,
        ] {
            assert!(owner.may(permission));
        }
    }

    /// A permission name survives a round trip, and nonsense is refused rather than granted.
    #[test]
    fn names_round_trip_and_typos_are_refused() {
        for permission in [
            Permission::Look,
            Permission::World,
            Permission::Players,
            Permission::Admin,
        ] {
            assert_eq!(Permission::parse(permission.as_str()), Some(permission));
        }
        assert_eq!(Permission::parse("wolrd"), None, "a typo must not grant");
        assert_eq!(Permission::parse(""), None);
    }
}
