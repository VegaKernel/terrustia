//! Stats for the projectiles NPCs throw, transcribed from `Projectile.SetDefaults`.
//!
//! Only the types the pre-hardmode roster actually fires are here. The game defines over a
//! thousand; the rest belong to player weapons, which this server does not yet simulate.

/// Everything the server needs to know about a projectile type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileStats {
    /// The `ProjectileID` constant name, for logs.
    pub name: &'static str,
    pub width: i32,
    pub height: i32,
    /// Which behaviour routine drives it.
    pub ai_style: i32,
    /// How many things it can hit before it dies. -1 means no limit.
    pub penetrate: i32,
    /// Ticks it lives for.
    pub time_left: i32,
    /// Whether terrain stops it.
    pub tile_collide: bool,
    /// Extra movement steps per tick, which is how the fast ones stay accurate.
    pub extra_updates: i32,
    pub knockback: f32,
}

/// Stats for a projectile type, or `None` for one this server does not model.
pub fn projectile_stats(projectile_type: u16) -> Option<ProjectileStats> {
    let stats = match projectile_type {
        31 => ProjectileStats {
            name: "SandBall",
            width: 10,
            height: 10,
            ai_style: 10,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 6.0,
        },
        38 => ProjectileStats {
            name: "HarpyFeather",
            width: 14,
            height: 14,
            ai_style: 1,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        44 => ProjectileStats {
            name: "DemonSickle",
            width: 48,
            height: 48,
            ai_style: 18,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        55 => ProjectileStats {
            name: "Stinger",
            width: 10,
            height: 10,
            ai_style: 1,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        83 => ProjectileStats {
            name: "EyeLaser",
            width: 4,
            height: 4,
            ai_style: 1,
            penetrate: 3,
            time_left: 600,
            tile_collide: true,
            extra_updates: 2,
            knockback: 0.0,
        },
        96 => ProjectileStats {
            name: "Flamelash",
            width: 16,
            height: 16,
            ai_style: 8,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        109 => ProjectileStats {
            name: "SnowBallHostile",
            width: 10,
            height: 10,
            ai_style: 10,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 6.0,
        },
        110 => ProjectileStats {
            name: "SnowBallFriendly",
            width: 4,
            height: 4,
            ai_style: 1,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 1,
            knockback: 0.0,
        },
        115 => ProjectileStats {
            name: "DeathLaser",
            width: 16,
            height: 16,
            ai_style: 27,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        128 => ProjectileStats {
            name: "FrostBoltHostile",
            width: 14,
            height: 14,
            ai_style: 28,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        270 => ProjectileStats {
            name: "BoneJavelin",
            width: 26,
            height: 26,
            ai_style: 1,
            penetrate: 3,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        288 => ProjectileStats {
            name: "NebulaLaser",
            width: 32,
            height: 32,
            ai_style: 12,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 2,
            knockback: 0.0,
        },
        299 => ProjectileStats {
            name: "SkullSpirit",
            width: 6,
            height: 6,
            ai_style: 1,
            penetrate: -1,
            time_left: 600,
            tile_collide: false,
            extra_updates: 2,
            knockback: 0.0,
        },
        719 => ProjectileStats {
            name: "QueenBeeStinger",
            width: 10,
            height: 10,
            ai_style: 1,
            penetrate: -1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        836 => ProjectileStats {
            name: "Seed",
            width: 4,
            height: 4,
            ai_style: 112,
            penetrate: 1,
            time_left: 600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        961 => ProjectileStats {
            name: "DeerclopsIceSpike",
            width: 32,
            height: 32,
            ai_style: 157,
            penetrate: 1,
            time_left: 600,
            tile_collide: false,
            extra_updates: 0,
            knockback: 0.0,
        },
        962 => ProjectileStats {
            name: "DeerclopsRangedProjectile",
            width: 32,
            height: 32,
            ai_style: 1,
            penetrate: 1,
            time_left: 220,
            tile_collide: false,
            extra_updates: 0,
            knockback: 0.0,
        },
        965 => ProjectileStats {
            name: "InsanityShadowHostile",
            width: 40,
            height: 40,
            ai_style: 187,
            penetrate: 1,
            time_left: 300,
            tile_collide: false,
            extra_updates: 0,
            knockback: 0.0,
        },
        1092 => ProjectileStats {
            name: "BookOfSkullsSkull",
            width: 16,
            height: 16,
            ai_style: 18,
            penetrate: -1,
            time_left: 420,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        // What the wired traps throw. These belong to the world rather than to any NPC, which is
        // why they are grouped: a dart trap in a dungeon fires the same dart whatever is standing
        // on the plate.
        98 => ProjectileStats {
            name: "Dart",
            width: 10,
            height: 10,
            ai_style: 1,
            penetrate: -1,
            time_left: 3600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        184 => ProjectileStats {
            name: "PoisonDartTrap",
            width: 10,
            height: 10,
            ai_style: 1,
            penetrate: -1,
            time_left: 3600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        185 => ProjectileStats {
            name: "SpikyBallTrap",
            width: 14,
            height: 14,
            ai_style: 14,
            penetrate: -1,
            time_left: 900,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        186 => ProjectileStats {
            name: "SpearTrap",
            width: 10,
            height: 14,
            ai_style: 37,
            penetrate: -1,
            time_left: 300,
            // A spear is anchored to the trap and slides through whatever it is set into, so it
            // has to ignore terrain rather than die on the block it grew out of.
            tile_collide: false,
            extra_updates: 0,
            knockback: 0.0,
        },
        187 => ProjectileStats {
            name: "FlamethrowerTrap",
            width: 6,
            height: 6,
            ai_style: 38,
            penetrate: 1,
            time_left: 60,
            tile_collide: false,
            extra_updates: 0,
            knockback: 0.0,
        },
        188 => ProjectileStats {
            name: "Flames",
            width: 6,
            height: 6,
            ai_style: 23,
            penetrate: -1,
            time_left: 3600,
            tile_collide: true,
            extra_updates: 2,
            knockback: 0.0,
        },
        654 => ProjectileStats {
            name: "GeyserTrap",
            width: 30,
            height: 30,
            ai_style: 126,
            penetrate: -1,
            time_left: 120,
            tile_collide: false,
            extra_updates: 0,
            knockback: 0.0,
        },
        980 => ProjectileStats {
            name: "VenomDartTrap",
            width: 10,
            height: 10,
            ai_style: 1,
            penetrate: -1,
            time_left: 3600,
            tile_collide: true,
            extra_updates: 0,
            knockback: 0.0,
        },
        _ => return None,
    };
    Some(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_projectiles_the_roster_fires_are_all_here() {
        for t in [
            31u16, 38, 44, 55, 83, 109, 110, 299, 719, 836, 961, 962, 965, 1092,
        ] {
            assert!(projectile_stats(t).is_some(), "missing {t}");
        }
    }

    #[test]
    fn a_harpy_feather_arcs_and_a_skull_does_not() {
        // Style 1 is the arcing one; the skull ignores terrain entirely.
        assert_eq!(projectile_stats(38).unwrap().ai_style, 1);
        assert!(projectile_stats(38).unwrap().tile_collide);
        assert!(!projectile_stats(299).unwrap().tile_collide);
    }

    #[test]
    fn a_demon_scythe_passes_through_everything_it_hits() {
        assert_eq!(projectile_stats(44).unwrap().penetrate, -1);
    }
}
