//! Enemy spawning.
//!
//! The rate, cap and spawn area come from the game (`defaultSpawnRate` 600, `defaultMaxSpawns` 5,
//! an area 0.7 screens across with a 0.52-screen safe zone around the player). The pools are
//! transcribed from what `Spawner.SpawnAnNPC` can choose pre-hardmode for each depth, time and
//! biome.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::tile_solid::solid;

use super::{journey::JourneyPowers, npc::NpcStore, player::Player};
use crate::world::World;

/// Everything about the world that changes how fast things spawn.
///
/// The player-carried modifiers — water and peace candles, battle and calming potions, invisibility,
/// the sunflower, the angler set — are deliberately absent: the server does not model a player's
/// inventory or buffs, so it cannot know them. Everything here is world state the server owns.
///
/// Also absent, and for the same reason (the server has no notion of the thing they test):
/// `ZoneSandstorm`, `cloudAlpha`'s snow-in-a-storm bonus, the dual-dungeon seeds, `getGoodWorld`,
/// and the Wall of Flesh's underworld suppression (`NPC.cs:662-666`).
/// Journey mode's slider is real and is applied by the caller, where the power lives, rather than
/// threaded through here.
#[derive(Debug, Clone, Copy)]
pub struct Conditions {
    /// Which rate band the *player* is in, from [`rate_depth_at`] rather than [`depth_at`]: the
    /// game's rate bands carry a screen-height offset its pool tests do not.
    pub depth: Depth,
    /// The biome the player is standing in.
    ///
    /// `SetSpawnFlags` (`NPC.cs:382-397`) copies the player's own `Zone*` flags across, and
    /// `GetSpawnRate` reads them for a whole block of rate and cap modifiers (`NPC.cs:591-660`).
    /// This was missing entirely, so the underground desert was five times too quiet, the jungle
    /// two and a half times, and the corruption one and a half.
    pub biome: Biome,
    pub hard_mode: bool,
    pub day_time: bool,
    pub blood_moon: bool,
    pub eclipse: bool,
    /// A pumpkin or frost moon: `Main.pumpkinMoon || Main.snowMoon`, with no height test.
    ///
    /// The height test belongs to the two *rate* branches rather than to the flag: both carry
    /// `&& player.position.Y < Main.worldSurface * 16.0` (`NPC.cs:543` and `:772`), and the town
    /// gate that also reads the moon (`NPC.cs:800`) carries no such test, so folding it in here
    /// would quietly turn town suppression back on for anyone underground during a moon. See
    /// [`Self::above_surface_line`].
    pub event_moon: bool,
    /// `player.position.Y < Main.worldSurface * 16.0`: the height half of the two moon branches.
    ///
    /// Not derivable from [`Depth`], whose surface band sits a screen height lower
    /// (see [`rate_depth_at`]).
    pub above_surface_line: bool,
    /// Townsfolk living near the player.
    ///
    /// This is what makes a base safe, and it is the single most player-visible spawn rule in the
    /// game: with nobody home the wilderness comes to your door, and with three residents it stops.
    /// The game suppresses only when nothing else is going on — an invasion, a blood moon, an
    /// eclipse or a moon all overrule it, because an event that a town could turn off would not be
    /// much of an event.
    pub town_npcs: u32,
    /// `player.nearbyActiveNPCs`: the spawn weight already close to this player.
    ///
    /// This was read only as a hard cap gate. The game *also* ramps the rate down as the area
    /// empties (`NPC.cs:668-698`, two stacked ladders), so a cleared cave refills faster than a
    /// crowded one: up to 2.38x faster than we were managing.
    pub nearby_active_npcs: f32,
    /// Whether the player is below `(worldSurface + rockLayer) / 2`, which is the second emptiness
    /// ladder's own gate (`NPC.cs:686`). Not derivable from [`Depth`]: it is a midline between two
    /// of its boundaries.
    pub below_dirt_midline: bool,
    /// `downedBoss3`, for the dungeon's pre-Skeletron flat rate (`NPC.cs:787-790`).
    pub downed_boss3: bool,
    /// Whether the player is standing in front of a house wall.
    ///
    /// `NPC.cs:411`, `noWorms = WorldGen.InWorld(pX, pY) && Main.wallHouse[Main.tile[pX, pY].wall]`:
    /// the other half of the "walls keep things out" rule, and the half that stops burrowers rather
    /// than walkers.
    pub behind_a_house_wall: bool,
    /// `numberOfActivePlayers` (`NPC.cs:266`), which the moon override's cap is a function of.
    pub active_players: u32,
    /// `Player.ZoneGraveyard`: whether the player is standing among enough tombstones.
    ///
    /// `SetSpawnFlags` copies it across with the rest of the zones (`NPC.cs:389`), and it is the one
    /// player-carried zone the server *can* know, because the client computes it and sends it up
    /// (see [`crate::game::player::Player::in_graveyard`]). A graveyard is not a quiet place: it
    /// takes over the surface night roster and it loosens town suppression rather than tightening
    /// it (`NPC.cs:861`, `:884`, `:906`).
    pub graveyard: bool,
    /// Whether the player is standing inside a lunar pillar's zone.
    ///
    /// This is vanilla's `invaders`, not a flag of its own: `SetSpawnFlags` (`NPC.cs:404-409`)
    /// forces `invaders = true` and `ignoreSafeWalls = true` for any of the four `ZoneTower*`,
    /// which is what gives a pillar fight an invasion's rate and cap (`NPC.cs:782-786`) and lets
    /// its escort spawn through a walled-off arena. Named for the zone rather than for `invaders`
    /// because the server's own invasions never reach this function: `tick_spawning` returns into
    /// `spawn_invaders` before it.
    pub in_tower_zone: bool,
    /// `Player.ZoneMeteor`: whether enough meteorite is under the player's feet to count as a
    /// crater ([`Zones::meteor`]).
    ///
    /// It is not a [`Biome`], because a crater takes the biome it fell into with it, and it does two
    /// things here: the last arm of the zone chain that sets the rate (`NPC.cs:636-640`) and one of
    /// the exclusions on town suppression (`NPC.cs:800`). A meteor crater is meant to be dangerous
    /// however many people live beside it.
    pub meteor: bool,
    /// `Player.ZoneLihzhardTemple`: whether the player is standing in front of a Lihzahrd brick wall.
    ///
    /// Not a tile count and not a [`Biome`]: `SceneMetrics.cs:693` is the whole of it,
    /// `ZoneLihzhardTemple = tileSafely.wall == 87`, one wall read at the player's own tile, the
    /// same shape as `ZoneGranite`, `ZoneMarble` and `ZoneHive` beside it. The wall it reads is
    /// already read here for [`Self::behind_a_house_wall`], so the flag costs nothing.
    ///
    /// A temple also reads as [`Biome::Jungle`], because Lihzahrd brick is one of the game's own
    /// jungle zone tiles (`SceneMetrics.cs:613`), so this rate modifier stacks on the jungle's
    /// exactly as vanilla's does: `NPC.cs:641-650` is a separate `if` below the biome chain.
    pub lihzahrd_temple: bool,
}

/// Which rate band a *player* is in, which is not the same question [`depth_at`] answers.
///
/// `GetSpawnRate`'s boundaries carry a screen height on top of the layer they name
/// (`NPC.cs:487`: `position.Y > Main.rockLayer * 16.0 + sHeight`; `:508` the same for
/// `worldSurface`), where `sHeight => 1200` px (`NPC.cs:6793`), which is 75 tiles. The *pool*
/// tests do not: `underGround` and `deeperThanRockLayer` (`NPC.cs:1144`, `:1204`) compare the
/// chosen tile against the bare layer. So the two need different functions, and sharing one put
/// every rate band 75 tiles too shallow, roughly doubling the rate through the dirt-layer band.
///
/// The underworld boundary has no offset in the game either (`NPC.cs:485`,
/// `position.Y > Main.UnderworldLayer * 16`), so it is the same on both sides.
pub fn rate_depth_at(world: &World, y: i32) -> Depth {
    /// `NPC.sHeight` (1200 px) in tiles.
    const SCREEN_TILES: i32 = 75;

    if y >= world.height() - UNDERWORLD_DEPTH {
        Depth::Underworld
    } else if y > i32::from(world.rock_layer) + SCREEN_TILES {
        Depth::Cavern
    } else if y > i32::from(world.surface) + SCREEN_TILES {
        Depth::Underground
    } else {
        Depth::Surface
    }
}

/// The spawn rate and cap for a set of conditions, after `NPC.GetSpawnRate`.
///
/// A flat 600/5 — the game's *surface daytime default* — was being used everywhere, so caverns
/// were about two and a half times too quiet, the underworld half as busy as it should be, and
/// neither hardmode nor a blood moon made any difference at all.
///
/// Returns `(one_in_n_per_tick, cap, spawn_friendly)`. A *lower* rate means more spawning, which
/// is the game's own convention and reads backwards until you know it. `spawn_friendly` is real
/// vanilla's own `spawnFriendly` (`NPC.cs`): when true, this attempt should draw a friendly
/// critter instead of a monster rather than being throttled by the rate at all — see the town
/// suppression block below for where it comes from. `rng` is only ever consulted there; every
/// other modifier in this function is a deterministic fact about the world.
///
/// The same block's other output, `noWorms`, is [`no_worms`] instead of a fourth element here,
/// because for everything this server models it needs no roll.
pub fn rates(at: Conditions, rng: &mut SmallRng) -> (u32, f32, bool) {
    let mut rate = SPAWN_RATE as f32;
    let mut max = MAX_SPAWNS;

    if at.hard_mode {
        rate *= 0.9;
        max += 1.0;
    }

    match at.depth {
        Depth::Underworld => max *= 2.0,
        Depth::Cavern => {
            rate *= 0.4;
            max *= 1.9;
        }
        Depth::Underground => {
            let (r, m) = if at.hard_mode {
                (0.45, 1.8)
            } else {
                (0.5, 1.7)
            };
            rate *= r;
            max *= m;
        }
        Depth::Surface => {
            if !at.day_time {
                rate *= 0.6;
                max *= 1.3;
                if at.blood_moon {
                    rate *= 0.3;
                    max *= 1.8;
                }
                // NPC.cs:543, with its own `position.Y < worldSurface * 16` half.
                if at.event_moon && at.above_surface_line {
                    rate *= 0.2;
                    max *= 2.0;
                }
            } else if at.eclipse {
                rate *= 0.2;
                max *= 1.9;
            }
        }
    }

    // The biome block, `NPC.cs:591-660`. It is one `if`/`else if` chain in the game, so a dungeon
    // takes its own modifier and none of the others; the hallow's and the temple's are separate
    // `if`s below it. Three branches of that chain are absent here because this server has no
    // notion of the zone they test: `ZoneSandstorm`, `inDualDungeon` and
    // `tresspassingDualDungeon`. `ZoneUndergroundDesert` is real vanilla's
    // "desert + below the surface + a sandstone or hardened-sand wall that is not a house wall"
    // (`SceneMetrics.cs:699`), narrowed here to the first two, since the server does not track
    // which wall a player is standing in front of.
    match at.biome {
        // NPC.cs:591-595.
        Biome::Dungeon => {
            rate *= 0.3;
            max *= 1.8;
        }
        // NPC.cs:603-607.
        Biome::Desert if at.depth != Depth::Surface => {
            rate *= 0.2;
            max *= 3.0;
        }
        // NPC.cs:609-635: the jungle thins out as a town fills up, on its own ladder rather than
        // the general town suppression further down (which it does not replace: both apply).
        Biome::Jungle => {
            let (r, m) = match at.town_npcs {
                0 => (0.4, 1.5),
                1 => (0.55, 1.4),
                2 => (0.7, 1.3),
                _ => (0.85, 1.2),
            };
            rate *= r;
            max *= m;
        }
        // NPC.cs:637-641.
        Biome::Corruption | Biome::Crimson => {
            rate *= 0.65;
            max *= 1.3;
        }
        // NPC.cs:636-640, the last arm of the same chain and the only one keyed on a zone that is
        // not one of [`Biome`]'s winners, so it answers exactly where none of the arms above did.
        _ if at.meteor => {
            rate *= 0.4;
            max *= 1.1;
        }
        _ => {}
    }
    // NPC.cs:641-650, a separate `if` sitting between the biome chain and the hallow's: the temple
    // is busier than the jungle it is buried in, and it stacks on the jungle's own modifier rather
    // than replacing it, because Lihzahrd brick makes the place read as jungle too. The
    // `Main.remixWorld` half of the branch (`:645-649`) is not modelled anywhere in this server.
    if at.lihzahrd_temple {
        rate *= 0.8;
        max *= 1.2;
    }
    // NPC.cs:656-660, a separate `if`: the hallow is busier only below the rock layer.
    if at.biome == Biome::Hallow && matches!(at.depth, Depth::Cavern | Depth::Underworld) {
        rate *= 0.65;
        max *= 1.3;
    }

    // The emptiness ramp, `NPC.cs:668-698`. Two stacked ladders: everywhere, then again below the
    // dirt-layer midline or in either evil. An area that has been cleared refills faster than one
    // that is still full, which is what stops a farmed cave going quiet for minutes at a time.
    // Both read the *running* `maxSpawns`, before its own ceiling is applied, as the game does.
    let near = at.nearby_active_npcs;
    if near < max * 0.2 {
        rate *= 0.6;
    } else if near < max * 0.4 {
        rate *= 0.7;
    } else if near < max * 0.6 {
        rate *= 0.8;
    } else if near < max * 0.8 {
        rate *= 0.9;
    }
    if at.below_dirt_midline || matches!(at.biome, Biome::Corruption | Biome::Crimson) {
        if near < max * 0.2 {
            rate *= 0.7;
        } else if near < max * 0.4 {
            rate *= 0.9;
        }
    }

    // The game's own floor and ceiling, which stop a stack of modifiers running away
    // (`NPC.cs:738-745`). Everything below this point is an *override*: the game assigns rather
    // than multiplies, so a clamp placed after them would undo them. Putting the clamps last is
    // what made a pumpkin moon three and a half times too slow.
    rate = rate.max(SPAWN_RATE as f32 * 0.1);
    max = max.min(MAX_SPAWNS * 3.0);

    // `NPC.cs:772-776`, the moon override, absolute in both directions: it replaces the rate with a
    // flat 20 and the cap with a function of the party size, whatever the clamps just said. Reached
    // at 64 or 72 before this, against the game's 20.
    if at.event_moon && at.above_surface_line {
        max = MAX_SPAWNS * (2.0 + 0.3 * at.active_players as f32);
        rate = 20.0;
    }

    // `NPC.cs:782-786`, the invasion override, and the only rate a pillar fight ever runs at: a
    // tower zone sets `invaders` outright (`NPC.cs:404-409`), so the numbers are the moon
    // override's exactly. Without it a pillar's escort would arrive at the surrounding terrain's
    // ordinary rate, which is 30 times slower than the game's and would leave a hundred-kill
    // shield unbreakable in practice as well as in principle.
    if at.in_tower_zone {
        max = MAX_SPAWNS * (2.0 + 0.3 * at.active_players as f32);
        rate = 20.0;
    }

    // `NPC.cs:787-790`: below the dungeon before Skeletron falls, the rate is a flat 10, which is
    // the pressure that makes early-dungeon farming impractical. The Dungeon Guardian this pairs
    // with landed in PR #32; the rate did not, so it arrived every 240 to 600 ticks instead.
    if at.biome == Biome::Dungeon && !at.downed_boss3 {
        rate = 10.0;
    }

    // Townsfolk quiet the place down, but only when nothing else is happening: an event overrules
    // them, so a blood moon still comes to a full town. Real vanilla (`NPC.cs:795-924`) is not a
    // flat multiplier here: past the event gate, every attempt is a coin flip between throttling
    // `spawnRate` and leaving it alone while shrinking `maxSpawns` and forcing the spawn to be a
    // friendly critter instead of a monster (`spawnFriendly`). One thing is deliberately not
    // modelled: the underworld's own separate, simpler fork (`NPC.cs:802-855`) — a base built in
    // the underworld is the rare exception, not the case this quiets.
    //
    // Each branch's `ZoneGraveyard` sub-case *is* modelled now that a graveyard is a thing the
    // server knows about, and it is not a variation on the ordinary case: it throttles the rate by
    // a smaller factor and then rolls *separately* for the friendly fork, where the ordinary case
    // picks one or the other. A base built in a graveyard stays a dangerous place to live, which is
    // the point of building one. Vanilla's own condition is
    // `ZoneGraveyard && (!ZonePeaceCandle || Main.rand.Next(3) == 0)`; `ZonePeaceCandle` is a
    // player-carried effect this server cannot see (this struct's own doc comment), so it is taken
    // as absent, which makes the left-hand disjunct true and leaves the bare graveyard test.
    let mut spawn_friendly = false;
    if town_suppression_applies(at) {
        match at.town_npcs {
            0 => {}
            // NPC.cs:861-882.
            1 => {
                if at.graveyard {
                    // NPC.cs:863-869.
                    rate *= 1.66;
                    if rng.random_ratio(1, 9) {
                        spawn_friendly = true;
                        max *= 0.6;
                    }
                } else if rng.random_ratio(1, 3) {
                    // NPC.cs:870-878, the ordinary case: a one-in-three chance forces a friendly
                    // spawn and shrinks the cap; the other two-in-three simply double the rate.
                    spawn_friendly = true;
                    max *= 0.6;
                } else {
                    rate *= 2.0;
                }
            }
            // NPC.cs:884-900: the odds flip to two-in-three for the friendly fork, and the rate
            // triples on the remaining one-in-three.
            2 => {
                if at.graveyard {
                    // NPC.cs:886-892.
                    rate *= 2.33;
                    if rng.random_ratio(1, 6) {
                        spawn_friendly = true;
                        max *= 0.6;
                    }
                } else if !rng.random_ratio(1, 3) {
                    spawn_friendly = true;
                    max *= 0.6;
                } else {
                    rate *= 3.0;
                }
            }
            // NPC.cs:902-921. `!Main.expertMode` is unconditionally true in classic mode, so the
            // ordinary branch sets `spawnFriendly` on *every* attempt rather than rolling for it —
            // `spawnRate` is never assigned there at all. Expert mode's own further
            // `Main.rand.Next(30) != 0` (a 29-in-30 chance) is folded into the same unconditional
            // case rather than threading a whole difficulty flag through `Conditions` for a
            // 1-in-30 edge: friendly wins the overwhelming majority of the time in expert mode
            // too, and this module already accepts small, disclosed over-approximations like it
            // elsewhere.
            _ => {
                if at.graveyard {
                    // NPC.cs:906-913: a full town in a graveyard still only triples the rate, and
                    // still spawns monsters two attempts in three.
                    rate *= 3.0;
                    if rng.random_ratio(1, 3) {
                        spawn_friendly = true;
                        max *= 0.6;
                    }
                } else {
                    spawn_friendly = true;
                    max *= 0.6;
                }
            }
        }
    }

    // `NPC.cs:925-929` ends the function with a `RollOnlyBadLuckExtreme(50) == 0` bonus of
    // `rate * 0.85` and `cap * 1.15`. It is deliberately not transcribed, because it can never
    // fire here: `Luck.RollOnlyBadLuckExtreme` (`Terraria.GameContent/Luck.cs:53-60`) returns -1
    // unless `luck < 0`, and this server does not model player luck at all, so its players are at
    // luck 0 exactly as a vanilla player with no luck effects is. Vanilla skips it for them too.

    (rate as u32, max.max(1.0), spawn_friendly)
}

/// `noWorms`: whether burrowers are kept out of this attempt's draw.
///
/// Its own function rather than a fourth thing [`rates`] returns, because for everything this
/// server models it is decided without a roll. Two sources:
///
/// * the wall at the player's own back (`NPC.cs:411`,
///   `noWorms = WorldGen.InWorld(pX, pY) && Main.wallHouse[Main.tile[pX, pY].wall]`);
/// * the town, which sets it unconditionally in all three headcount branches of the ordinary
///   surface/underground fork (`NPC.cs:858`, `:883`, `:905`), behind the same event gate the rest of
///   town suppression sits behind (`NPC.cs:800`).
///
/// Only the underworld's own fork rolls for it (`NPC.cs:810` one in two, `:827` three in four,
/// `:843` nine in ten), and that fork is already disclosed in [`rates`] as not modelled.
/// `ZoneShadowCandle` clearing it again (`NPC.cs:420-424`) is a player-carried effect, out of
/// [`Conditions`]' scope like every other one.
pub fn no_worms(at: Conditions) -> bool {
    at.behind_a_house_wall || (town_suppression_applies(at) && at.town_npcs >= 1)
}

/// The gate the whole town-suppression block sits behind, `NPC.cs:800`:
///
/// ```csharp
/// if (!invaders && ((!Main.bloodMoon && !Main.pumpkinMoon && !Main.snowMoon) || Main.dayTime)
///     && (!Main.eclipse || !Main.dayTime) && !flag && !ZoneCrimson && !ZoneMeteor
///     && !ZoneOldOneArmy)
/// ```
///
/// where `flag` is `ZoneCorrupt || ZoneCrimson`. So an event overrules the town, and so does simply
/// standing in an evil: a corrupt base is never quiet, however many people live in it. That last
/// clause could not be modelled until `Conditions` carried a biome at all.
///
/// Note the moon here is tested with **no** height condition, unlike the two rate branches: a
/// player underground during a pumpkin moon still has town suppression switched off.
///
/// `invaders` is real here for one case only, and it is the pillar fight: a tower zone sets it
/// (`NPC.cs:404-409`), and this server's own invasions never reach this function at all, because
/// `tick_spawning` returns into `spawn_invaders` before it. One of the game's exclusions is still
/// dropped, because the thing it tests does not exist here: `ZoneOldOneArmy`. `Main.infectedSeed`,
/// which would clear `flag` again, is likewise unmodelled.
fn town_suppression_applies(at: Conditions) -> bool {
    !at.in_tower_zone
        && ((!at.blood_moon && !at.event_moon) || at.day_time)
        && (!at.eclipse || !at.day_time)
        && !matches!(at.biome, Biome::Corruption | Biome::Crimson)
        && !at.meteor
}

/// The burrowers whose spawn branch vanilla gates on `noWorms`, out of the types this server fields.
///
/// `NPC.cs:3704-3713` is the Devourer / World Feeder branch, and it is the only one of the game's
/// several `!noWorms` gates naming a type that appears in these pools. Deliberately *not* here: the
/// underworld's Bone Serpent, which the game spawns with no such gate at all (`NPC.cs:4885`,
/// `Main.rand.Next(40) == 0 && !AnyNPCs(39)`). The rest of the game's gates (`NPC.cs:1409` the
/// Wyvern, `:3973`, `:4062`, `:1698`) name hardmode worms with no pool here.
const NO_WORMS_GATES: [u16; 2] = [
    7,  // DevourerHead
    98, // SeekerHead, the World Feeder
];

#[cfg(test)]
mod rate_tests {
    use super::*;
    use rand::SeedableRng;

    /// A neutral world: plain forest surface, daytime, nothing running, nobody about.
    ///
    /// `nearby_active_npcs` is deliberately *not* zero. An empty area is the game's fastest case,
    /// not its neutral one (`NPC.cs:668`, rate x0.6), so pinning a modifier against an empty
    /// baseline would fold that ramp into every number here. This is far above the ramp's top rung
    /// (`maxSpawns * 0.8`) for any cap the modifiers can build, so it leaves the ramp off entirely
    /// and each pin measures the one modifier it names.
    fn plain() -> Conditions {
        Conditions {
            depth: Depth::Surface,
            biome: Biome::Forest,
            hard_mode: false,
            day_time: true,
            blood_moon: false,
            eclipse: false,
            event_moon: false,
            above_surface_line: true,
            town_npcs: 0,
            nearby_active_npcs: 1_000.0,
            below_dirt_midline: false,
            downed_boss3: true,
            behind_a_house_wall: false,
            active_players: 1,
            in_tower_zone: false,
            graveyard: false,
            meteor: false,
            lihzahrd_temple: false,
        }
    }

    /// A fresh RNG for a call that never touches the town-suppression roll (`town_npcs: 0`, or an
    /// event overruling it) and so does not care which one it gets.
    fn any_rng() -> SmallRng {
        SmallRng::seed_from_u64(0)
    }

    /// Going down makes the world busier, which is most of what depth is for.
    #[test]
    fn caverns_are_busier_than_the_surface() {
        let (surface, surface_cap, _) = rates(plain(), &mut any_rng());
        let (cavern, cavern_cap, _) = rates(
            Conditions {
                depth: Depth::Cavern,
                ..plain()
            },
            &mut any_rng(),
        );
        assert!(
            cavern < surface,
            "a lower rate means more spawning: {cavern} vs {surface}",
        );
        assert!(cavern_cap > surface_cap);
        // The game's figure is 0.4x the rate. A flat 600 everywhere made caves this much too quiet.
        assert_eq!(cavern, (surface as f32 * 0.4) as u32);
    }

    /// Night, a blood moon and an eclipse each raise the surface's rate.
    #[test]
    fn events_make_the_surface_dangerous() {
        let (day, _, _) = rates(plain(), &mut any_rng());
        let (night, _, _) = rates(
            Conditions {
                day_time: false,
                ..plain()
            },
            &mut any_rng(),
        );
        let (blood, blood_cap, _) = rates(
            Conditions {
                day_time: false,
                blood_moon: true,
                ..plain()
            },
            &mut any_rng(),
        );
        let (eclipse, _, _) = rates(
            Conditions {
                eclipse: true,
                ..plain()
            },
            &mut any_rng(),
        );

        assert!(night < day, "night is busier than day");
        assert!(
            blood < night,
            "a blood moon is busier than an ordinary night"
        );
        assert!(eclipse < day, "an eclipse is busier than a plain day");
        assert!(blood_cap > rates(plain(), &mut any_rng()).1);
    }

    /// A meteor crater is busier than the ground it fell on, and a town cannot quiet it.
    ///
    /// Two vanilla lines, and they pull the same way: `NPC.cs:636-640` is the last arm of the zone
    /// rate chain (`spawnRate * 0.4`, `maxSpawns * 1.1`), and `NPC.cs:800` names `!ZoneMeteor` among
    /// the exclusions on the whole town-suppression block, alongside the evils. Building a house on
    /// a crater is not meant to make it safe.
    ///
    /// Neutralised twice, each failing its own assertion: deleting the `_ if at.meteor` arm from the
    /// biome chain gives "a crater should be busier than the forest it fell on: 216 vs 216", and
    /// dropping `&& !at.meteor` from `town_suppression_applies` gives "a full town should not quiet
    /// a crater".
    #[test]
    fn a_crater_is_busier_than_the_ground_it_fell_on_and_a_town_cannot_quiet_it() {
        let (forest, forest_cap, _) = rates(plain(), &mut any_rng());
        let crater = Conditions {
            meteor: true,
            ..plain()
        };
        let (rate, cap, _) = rates(crater, &mut any_rng());
        assert!(
            rate < forest,
            "a crater should be busier than the forest it fell on: {rate} vs {forest}"
        );
        assert!(
            cap > forest_cap,
            "and hold more of them: {cap} vs {forest_cap}"
        );

        // The evils take their own arm first, so a corrupted crater is corruption's rate, not the
        // crater's: vanilla's chain is `else if`, and this is the arm order rather than a tie.
        let corrupt = Conditions {
            biome: Biome::Corruption,
            ..plain()
        };
        assert_eq!(
            rates(corrupt, &mut any_rng()).0,
            rates(
                Conditions {
                    meteor: true,
                    ..corrupt
                },
                &mut any_rng()
            )
            .0,
            "the corruption's arm answers before the crater's",
        );

        // ...and a full town, which quiets a forest outright, does nothing at all here.
        let mut rng = SmallRng::seed_from_u64(1);
        for _ in 0..200 {
            let (town_rate, town_cap, friendly) = rates(
                Conditions {
                    town_npcs: 3,
                    ..crater
                },
                &mut rng,
            );
            assert!(!friendly, "a full town should not quiet a crater");
            assert_eq!((town_rate, town_cap), (rate, cap));
        }
        assert!(
            rates(
                Conditions {
                    town_npcs: 3,
                    ..plain()
                },
                &mut rng
            )
            .2,
            "a full town does quiet an ordinary forest, which is what makes the above a difference",
        );
    }

    /// A town of one or two residents forks every attempt between throttling the rate and
    /// shrinking the cap while forcing a friendly spawn — real vanilla is not a flat multiplier
    /// here (`NPC.cs:870-901`), so this samples many attempts rather than asserting one.
    #[test]
    fn one_or_two_residents_fork_between_a_slower_rate_and_a_smaller_friendly_cap() {
        let (wild, wild_cap, wild_friendly) = rates(plain(), &mut any_rng());
        assert!(
            !wild_friendly,
            "no town at all never forces a friendly spawn"
        );

        let mut rng = SmallRng::seed_from_u64(1);
        for town_npcs in [1, 2] {
            let mut saw_slower_rate = false;
            let mut saw_smaller_friendly_cap = false;
            for _ in 0..200 {
                let (rate, cap, friendly) = rates(
                    Conditions {
                        town_npcs,
                        ..plain()
                    },
                    &mut rng,
                );
                if friendly {
                    assert_eq!(cap, wild_cap * 0.6, "the friendly fork shrinks the cap");
                    assert_eq!(rate, wild, "and leaves the rate exactly where it was");
                    saw_smaller_friendly_cap = true;
                } else {
                    assert!(rate > wild, "the other fork should slow the rate");
                    assert_eq!(cap, wild_cap, "and leaves the cap exactly where it was");
                    saw_slower_rate = true;
                }
            }
            assert!(
                saw_slower_rate,
                "town_npcs {town_npcs}: 200 trials never rolled the rate fork"
            );
            assert!(
                saw_smaller_friendly_cap,
                "town_npcs {town_npcs}: 200 trials never rolled the friendly fork"
            );
        }
    }

    /// Three or more residents is the one headcount classic mode makes fully deterministic:
    /// every attempt forces a friendly spawn and shrinks the cap, and the rate is never touched
    /// at all (`NPC.cs:917-921`) — not throttled further, the way a flat multiplier would.
    #[test]
    fn three_or_more_residents_always_forces_a_friendly_spawn_at_the_unchanged_rate() {
        let (wild, wild_cap, _) = rates(plain(), &mut any_rng());
        let mut rng = SmallRng::seed_from_u64(2);
        for town_npcs in 3..8 {
            let (rate, cap, friendly) = rates(
                Conditions {
                    town_npcs,
                    ..plain()
                },
                &mut rng,
            );
            assert_eq!(rate, wild, "townNPCs >= 3 never assigns spawnRate");
            assert!(friendly, "and always forces a friendly spawn");
            assert_eq!(cap, wild_cap * 0.6);
        }
    }

    /// Standing among tombstones does not make the place safe, which is the whole point of building
    /// there: the town-suppression block's graveyard sub-cases throttle the rate by a smaller factor
    /// and then roll *separately* for the friendly fork (`NPC.cs:861-869`, `:886-892`, `:906-913`),
    /// where the ordinary case picks one or the other.
    ///
    /// The measure is monsters per attempt, not `spawnRate`: at two residents a graveyard is
    /// actually the *slower* of the two on raw rate (2.33x flat, against a mean of 1.67x), and is
    /// still three times as dangerous, because five attempts in six there draw a monster where two
    /// in three of the ordinary ones draw a harmless critter instead. Comparing rates alone would
    /// have read that backwards.
    ///
    /// Neutralised by deleting the three `if at.graveyard` arms in `rates` so both sides take the
    /// ordinary path: the two figures come out identical and the assertion fails at every one of
    /// the three headcounts.
    #[test]
    fn a_town_does_not_quieten_a_graveyard_the_way_it_quietens_a_forest() {
        for town_npcs in [1u32, 2, 3] {
            // Both forks are rolls, so this is an average over many draws: the chance that one
            // attempt puts a *monster* on the field, which is `1/rate` on the attempts that are not
            // diverted into a critter.
            let monsters_per_attempt = |graveyard: bool| {
                let mut rng = SmallRng::seed_from_u64(4);
                let mut total = 0.0;
                for _ in 0..20_000 {
                    let (rate, _, friendly) = rates(
                        Conditions {
                            town_npcs,
                            graveyard,
                            ..plain()
                        },
                        &mut rng,
                    );
                    if !friendly {
                        total += 1.0 / f64::from(rate);
                    }
                }
                total / 20_000.0
            };
            let ordinary = monsters_per_attempt(false);
            let haunted = monsters_per_attempt(true);
            assert!(
                haunted > ordinary * 1.5,
                "{town_npcs} residents: a graveyard base ({haunted:.6} monsters an attempt) should \
                 stay far more dangerous than an ordinary one ({ordinary:.6})"
            );
        }
    }

    /// An event overrules the town: a blood moon still comes to a full street.
    #[test]
    fn an_event_ignores_the_town() {
        let quiet_night = rates(
            Conditions {
                day_time: false,
                town_npcs: 3,
                ..plain()
            },
            &mut any_rng(),
        );
        let blood_night = rates(
            Conditions {
                day_time: false,
                blood_moon: true,
                town_npcs: 3,
                ..plain()
            },
            &mut any_rng(),
        );
        assert!(
            blood_night.0 < quiet_night.0,
            "a town that could switch off a blood moon would not be much of an event",
        );
        assert!(
            !blood_night.2,
            "and an event never forces a friendly spawn either"
        );
    }

    /// The gate the whole town-suppression block sits behind (`NPC.cs:800`), which has two clauses
    /// worth their own pin.
    ///
    /// An evil is never quiet, however many people live in it: `!flag && !ZoneCrimson` where
    /// `flag = ZoneCorrupt || ZoneCrimson`. That clause could not be modelled at all until
    /// `Conditions` carried a biome.
    ///
    /// And the moon is tested there with no height condition, unlike the two rate branches, so a
    /// player underground during a pumpkin moon still has town suppression switched off.
    #[test]
    fn an_evil_is_never_quieted_by_a_town_and_a_moon_switches_it_off_at_any_depth() {
        let mut rng = SmallRng::seed_from_u64(3);
        for biome in [Biome::Corruption, Biome::Crimson] {
            for _ in 0..50 {
                let (_, cap, friendly) = rates(
                    Conditions {
                        biome,
                        town_npcs: 5,
                        ..plain()
                    },
                    &mut rng,
                );
                assert!(!friendly, "{biome:?} never draws a friendly for a town");
                assert_eq!(cap, 5.0 * 1.3, "and its cap is the evil's, not a town's");
            }
        }
        // A forest of the same headcount does get quieted, so the biome is what did it.
        assert!(
            rates(
                Conditions {
                    town_npcs: 5,
                    ..plain()
                },
                &mut rng,
            )
            .2
        );

        // Underground, during a moon, with a full town: no suppression, because the gate's moon
        // clause carries no height test.
        let deep_moon = Conditions {
            depth: Depth::Cavern,
            above_surface_line: false,
            event_moon: true,
            day_time: false,
            town_npcs: 5,
            ..plain()
        };
        assert!(!rates(deep_moon, &mut rng).2, "a moon overrules the town");
        assert!(!no_worms(deep_moon), "and its noWorms with it");
        // ...but the moon's own rate override does not reach down here.
        assert_ne!(rates(deep_moon, &mut rng).0, 20);
    }

    /// `noWorms` keeps burrowers out when there is a town or a wall at your back, and an event
    /// overrules the town half of that exactly as it overrules the rest of town suppression
    /// (`NPC.cs:411`, `:800`, `:858`, `:883`, `:905`).
    ///
    /// Fails before the fix, when `noWorms` was not modelled at all: Devourers came straight
    /// through a town and through a walled base's own walls.
    #[test]
    fn a_town_or_a_wall_keeps_the_burrowers_out() {
        assert!(!no_worms(plain()), "an empty wilderness has worms in it");
        for town in 1..5 {
            assert!(
                no_worms(Conditions {
                    town_npcs: town,
                    ..plain()
                }),
                "{town} residents should stop worms",
            );
        }
        assert!(
            no_worms(Conditions {
                behind_a_house_wall: true,
                ..plain()
            }),
            "so should a wall at your own back, with no town at all",
        );
        // An event overrules the town, but not the wall.
        for town in 0..5 {
            assert!(
                !no_worms(Conditions {
                    town_npcs: town,
                    blood_moon: true,
                    day_time: false,
                    ..plain()
                }),
                "a blood moon brings worms to a town of {town}",
            );
        }
        assert!(
            no_worms(Conditions {
                behind_a_house_wall: true,
                blood_moon: true,
                day_time: false,
                ..plain()
            }),
            "the wall is not part of the town's event gate",
        );

        // The gated set is the Devourer branch and nothing else this server fields.
        assert_eq!(NO_WORMS_GATES, [7, 98]);
        assert!(
            !NO_WORMS_GATES.contains(&39),
            "the underworld's Bone Serpent has no such gate in the game (NPC.cs:4885)",
        );
    }

    /// However the modifiers stack, they stay inside the game's own floor and ceiling.
    #[test]
    fn the_rate_is_bounded() {
        // No moon here: the moon override (`NPC.cs:772-776`) is *outside* the clamps by design and
        // sets a flat 20, so including it would be asking the clamps to bound something the game
        // deliberately puts beyond them. It has its own pin below.
        let worst = rates(
            Conditions {
                depth: Depth::Underworld,
                hard_mode: true,
                day_time: false,
                blood_moon: true,
                nearby_active_npcs: 0.0,
                below_dirt_midline: true,
                ..plain()
            },
            &mut any_rng(),
        );
        assert!(worst.0 >= (SPAWN_RATE as f32 * 0.1) as u32, "{worst:?}");
        assert!(worst.1 <= MAX_SPAWNS * 3.0, "{worst:?}");
    }

    /// The temple is busier than the jungle it is buried in, and it stacks (`NPC.cs:641-650` is a
    /// separate `if` below the biome chain, not another arm of it).
    ///
    /// Neutralised by turning the `if at.lihzahrd_temple` block in [`rates`] off: the first
    /// assertion fires, "spawnRate x0.8: left 300, right 240", the temple settling for the
    /// surrounding rock's own numbers.
    #[test]
    fn the_temple_quickens_the_jungle_it_is_buried_in() {
        // A plain underground band first, where neither clamp is anywhere near, so the two factors
        // can be read off exactly.
        let plain_band = Conditions {
            depth: Depth::Underground,
            ..plain()
        };
        let (base_rate, base_cap, _) = rates(plain_band, &mut any_rng());
        let (rate, cap, _) = rates(
            Conditions {
                lihzahrd_temple: true,
                ..plain_band
            },
            &mut any_rng(),
        );
        assert_eq!(rate, (base_rate as f32 * 0.8) as u32, "spawnRate x0.8");
        assert!(
            (cap - base_cap * 1.2).abs() < 1e-4,
            "maxSpawns x1.2: {base_cap} -> {cap}"
        );

        // ...and it stacks on the jungle rather than replacing it, which is what makes a temple
        // busier than the jungle it is buried in. The cap is already at its own ceiling down there
        // (`MAX_SPAWNS * 3`), so the rate is what shows it.
        let jungle = Conditions {
            depth: Depth::Cavern,
            biome: Biome::Jungle,
            ..plain()
        };
        let (jungle_rate, _, _) = rates(jungle, &mut any_rng());
        let (temple_rate, _, _) = rates(
            Conditions {
                lihzahrd_temple: true,
                ..jungle
            },
            &mut any_rng(),
        );
        assert!(
            temple_rate < jungle_rate,
            "the temple stacks on the jungle: {jungle_rate} -> {temple_rate}"
        );
    }
}

/// The baseline: `NPC.defaultSpawnRate`, before anything modifies it.
pub const SPAWN_RATE: u32 = 600;

/// Spawn slots a single player supports.
pub const MAX_SPAWNS: f32 = 5.0;

/// Spawn area around the player, in tiles: roughly 0.7 of a 1080p screen.
pub const SPAWN_RANGE_X: i32 = 84;
pub const SPAWN_RANGE_Y: i32 = 47;

/// Nothing spawns inside this box around the player.
pub const SAFE_RANGE_X: i32 = 62;
pub const SAFE_RANGE_Y: i32 = 35;

/// How deep below the world's bottom the underworld begins.
pub const UNDERWORLD_DEPTH: i32 = 200;

/// Where in the world column a spawn point sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Surface,
    Underground,
    Cavern,
    Underworld,
}

/// Everything one scan of the tiles around a point says about the place.
///
/// The game's zones are independent flags rather than one winner, and `SceneMetrics.CalculateZones`
/// (`SceneMetrics.cs:668-686`) sets all of them off the same pass of tile counts. [`Biome`] has to
/// pick one, so the three flags that have their own spawn branches and are not a `Biome` ride along
/// here rather than being thrown away: each is a counter the scan is already walking every tile for,
/// so none of them costs a second pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zones {
    /// The one biome that won, which is what most of the game's own spawn checks read first.
    pub biome: Biome,
    /// `ZoneDesert = EnoughTilesForDesert` (`SceneMetrics.cs:683`), true alongside whatever else the
    /// place also is: a corrupted desert really is both at once, and that is exactly where the three
    /// converted sandsharks live.
    pub desert: bool,
    /// `ZoneGlowshroom = EnoughTilesForGlowingMushroom` (`SceneMetrics.cs:684`), which is
    /// `MushroomTileCount >= 100` (`:52`, `:260`).
    pub glowshroom: bool,
    /// `ZoneMeteor = EnoughTilesForMeteor` (`SceneMetrics.cs:685`), which is
    /// `MeteorTileCount >= 75` (`:56`, `:268`).
    pub meteor: bool,
    /// `SceneMetrics.EnoughTilesForShimmer` (`:252`), which is `ShimmerTileCount >= 300`
    /// (`:24`) and `ShimmerTileCount = _liquidCounts[3]` (`:601`, `LiquidID.Shimmer = 3`).
    ///
    /// The only one of these counted off *liquid* rather than off blocks, which is why it is the
    /// one flag the scan's `!is_active()` arm has to look at rather than skip.
    ///
    /// Deliberately the raw tile-count flag and not `SceneMetrics.ZoneShimmer`, which is
    /// `EnoughTilesForShimmer && UndergroundForShimmering && !ZoneDungeon` (`:711-712`). The one
    /// thing that reads this today is the shimmer pylon, and vanilla's pylon check reads the raw
    /// flag (`TeleportPylonsSystem.cs:307-308`), not the zone. Anything wanting the full
    /// `ZoneShimmer` has to add the other two terms itself.
    pub shimmer: bool,
}

/// Which biome the surrounding tiles say we are in.
///
/// Deliberately without a glowing-mushroom or shimmer variant, and it is not an oversight to fix
/// later. This is the ambient spawn pool's *single winner*, and neither of those is one in the
/// game either: a mushroom cave sits on mud, so vanilla's own roster for it hangs off the ground
/// tile (`tileType == 70`, `NPC.cs:3637`, `:3674`) rather than off a biome, and shimmer has no
/// ambient roster at all. Both are independent `SceneMetrics` flags, so they live on [`Zones`]
/// beside `desert` and `meteor`; giving either one a `Biome` seat would make it *displace* a real
/// pool (a mushroom cave in the snow would stop being snow), which is a spawn bug, not a fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Forest,
    Corruption,
    Crimson,
    Jungle,
    Snow,
    Desert,
    Ocean,
    Dungeon,
    /// Only exists after the wall falls, which is why nothing recognised it before hardmode did.
    Hallow,
}

/// What hardmode adds where, on top of whatever the place had before.
///
/// Every entry was read out of `NPC.Spawner` with its real condition rather than from memory: the
/// zone it needs, the depth, and whether it wants day or night. A pool that named the wrong biome
/// would be worse than an empty one, because it would look right.
///
/// These *add* to the ordinary pool rather than replacing it — a hardmode forest still has
/// zombies — except in the hallow, which has no pre-hardmode life of its own.
pub fn hardmode_pool(depth: Depth, biome: Biome, day: bool) -> &'static [u16] {
    use Biome::*;
    use Depth::*;

    match (depth, biome) {
        // The dungeon answers for itself at every depth and adds nothing generic, because vanilla's
        // `else if (ZoneDungeon)` (`NPC.cs:2629-2795`) returns down every path it has and never
        // reaches the underground chain the arm below transcribes. Without this a hardmode dungeon
        // grew Mimics, Rune Wizards and Black Recluses the game never puts in one. What hardmode
        // really adds there is [`hard_dungeon_pick`], which is a chain rather than a pool; the same
        // shape, and the same reason, as [`pool`]'s own `(_, Dungeon)` arm.
        (_, Dungeon) => &[],
        // The two evils, which are the same shape with different names.
        (Surface, Corruption) => &[
            81,  // CorruptSlime
            121, // Slimer
            94,  // Corruptor
        ],
        // The Clinger is a *corruption* enemy rather than a jungle one, whatever its place in a
        // "hardmode underground jungle" roster suggests: the arm it lives in is the two evils'
        // shared one, `tileType == 22 && ZoneCorrupt || 23 || 25 || 112 || 163 || 661`
        // (`NPC.cs:4125`), and 661 is Corrupt Jungle Grass, which is what puts one in a corrupted
        // jungle and nowhere else in a jungle at all. Its own branch is `(Main.hardMode & flag16)
        // && Main.rand.Next(3) == 0 && Main.tile[x, y].type == tileType` (`:4136-4139`), and it is
        // anchored to that tile the same way a Man Eater is (`SpawnNPC(..., 101, 0, spawnTileX,
        // spawnTileY)`); `game::ai::rooted` roots a plant that arrives without an anchor in the
        // solid tile under it, so the plumbing is already here.
        //
        // `flag16` is `spawnTileY >= Main.rockLayer` (`:4127`), which is this server's `Cavern`
        // alone. It is listed on the `Underground | Cavern` arm anyway, exactly as the Cursed
        // Hammer above it already is (`:4132`, the same `flag16`), so the two share one disclosed
        // narrowing rather than each getting its own depth split.
        (Underground | Cavern, Corruption) => &[
            81,  // CorruptSlime
            83,  // CursedHammer
            94,  // Corruptor
            98,  // SeekerHead — a world feeder
            101, // Clinger
            170, // PigronCorruption
            473, // BigMimicCorruption
        ],
        (Surface, Crimson) => &[
            183, // Crimslime
            241, // BloodFeeder
            242, // BloodJelly
        ],
        (Underground | Cavern, Crimson) => &[
            174, // Herpling
            179, // CrimsonAxe
            180, // PigronCrimson
            182, // FloatyGross
            183, // Crimslime
            268, // IchorSticker
            474, // BigMimicCrimson
        ],
        // The hallow, which has nothing but hardmode life.
        (Surface, Hallow) => {
            if day {
                &[
                    75,  // Pixie
                    122, // Gastropod
                ]
            } else {
                &[
                    75,  // Pixie
                    122, // Gastropod
                    137, // IlluminantBat
                    138, // IlluminantSlime
                ]
            }
        }
        (Underground | Cavern, Hallow) => &[
            75,  // Pixie
            84,  // EnchantedSword
            120, // ChaosElemental
            137, // IlluminantBat
            138, // IlluminantSlime
            171, // PigronHallow
            475, // BigMimicHallow
        ],
        // The snow, which gets a great deal.
        (Surface, Snow) => &[
            197, // ArmoredViking
            243, // IceGolem
            250, // AngryNimbus
        ],
        (Underground | Cavern, Snow) => &[
            95,  // DiggerHead
            150, // IceBat
            154, // IceTortoise
            184, // SpikedIceSlime
            197, // ArmoredViking
            206, // IcyMerman
            629, // IceMimic
        ],
        // The jungle. The Angry Trapper is the one member of vanilla's hardmode jungle-grass arm
        // (`tileType == 60 && Main.hardMode && Main.rand.Next(3) != 0`, `NPC.cs:3864-3912`) that
        // was in no pool at all, so every other branch of that arm was here and its rooted one was
        // not: `else if (Main.rand.Next(3) == 0 && Main.tile[x, y].type == tileType)` at
        // `:3905-3908`.
        //
        // It is on both depth arms because vanilla reaches it at both. Its branch sits below the
        // arm's two `surfaceSpawn` ones (152 at `:3866`, 177 at `:3870`) and its three
        // `spawnTileY > Main.worldSurface` ones (205, 236 and the Moss Hornet at `:3874-3904`), and
        // `surfaceSpawn` is `spawnTileY <= Main.worldSurface` (`NPC.cs:1203`), so above the surface
        // line the three middle branches cannot fire and the trapper is what answers when the two
        // surface rolls decline. The arm's own fallthrough, the Giant Tortoise, is already on both
        // arms here for the same reason.
        (Surface, Jungle) => {
            if day {
                &[
                    177, // Derpling
                    153, // GiantTortoise
                    175, // AngryTrapper
                ]
            } else {
                &[
                    152, // GiantFlyingFox
                    153, // GiantTortoise
                    175, // AngryTrapper
                ]
            }
        }
        (Underground | Cavern, Jungle) => &[
            157, // Arapaima
            175, // AngryTrapper
            176, // MossHornet
            205, // Moth
            236, // JungleCreeper
            476, // BigMimicJungle
        ],
        // The desert, whose hardmode life is the underground half of it.
        (Underground | Cavern, Desert) => &[
            78,  // Mummy
            79,  // DarkMummy
            80,  // LightMummy
            510, // DuneSplicerHead
        ],
        // The underworld, which only opens up once a mechanical boss is down; the caller holds
        // that gate because it is progression rather than place.
        (Underworld, _) => &[
            151, // Lavabat
            156, // RedDevil
        ],
        // An ordinary forest surface at night.
        (Surface, _) => {
            if day {
                &[]
            } else {
                &[
                    82,  // Wraith
                    93,  // GiantBat
                    133, // WanderingEye
                    140, // PossessedArmor
                ]
            }
        }
        // ...and everything under it. No Black Recluse (163): `NPC.Spawner` reaches it from exactly
        // one place, the spider nest's own arm (`NPC.cs:1673`, see [`spider_pick`]), so listing it
        // here put recluses in every hardmode cave instead of in the nests they belong to.
        (Underground | Cavern, _) => &[
            77,  // ArmoredSkeleton
            85,  // Mimic
            93,  // GiantBat
            110, // SkeletonArcher
            141, // ToxicSludge
            172, // RuneWizard
        ],
    }
}

/// What a blood moon adds to the surface at night.
///
/// It does not replace the night's pool — a blood moon night still has zombies — it widens it, and
/// the widening is what makes one worth fighting through rather than sleeping past. The Clown only
/// comes in hardmode, which is why he is not simply on the list.
pub fn blood_moon_pool(depth: Depth, hard_mode: bool) -> &'static [u16] {
    const EARLY: [u16; 2] = [
        489, // BloodZombie
        490, // Drippler
    ];
    const LATE: [u16; 3] = [
        489, // BloodZombie
        490, // Drippler
        109, // Clown
    ];
    if depth != Depth::Surface {
        return &[];
    }
    if hard_mode { &LATE } else { &EARLY }
}

/// What the surface night has going on beyond the plain weather and depth of it.
///
/// Its own struct rather than more fields on [`Conditions`], which is documented as the rate and
/// cap inputs: none of these changes how *fast* anything spawns, only what turns up.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seasonal {
    /// `Main.halloween` (`Main.cs:13329-13347`), the real-world calendar rather than world state.
    pub halloween: bool,
    /// `Main.xMas` (`Main.cs:13290-13309`).
    pub xmas: bool,
    /// `Player.ZoneGraveyard` (`NPC.cs:389`), which is the client's own answer (see
    /// [`crate::game::player::Player::in_graveyard`]).
    pub graveyard: bool,
    pub hard_mode: bool,
    pub blood_moon: bool,
    /// `Main.dayTime`. The chain below is the *night* chain, and a graveyard is the one thing that
    /// reaches it in daylight (`NPC.cs:4202`, `if (!ZoneGraveyard && Main.dayTime)`: standing among
    /// tombstones skips the whole daytime block and drops through to this).
    pub day_time: bool,
    /// `Main.moonPhase`. `Terraria.Enums.MoonPhase` names them: 0 is `Full` and 4 is `Empty`, the
    /// new moon, which is the darkest night and the one that doubles the demon eyes.
    pub moon_phase: u8,
    /// `Main.raining` (`NPC.cs:1287`, `raining = Main.raining`), for the surface night's own rain
    /// arm (`NPC.cs:4675`). It is here rather than on [`Conditions`] for the same reason the rest
    /// of this struct is: rain changes what turns up, not how fast.
    pub raining: bool,
}

/// The surface night chain's own arms, ahead of the ordinary pool: `NPC.cs:4539-4740`.
///
/// This is not a pool. Vanilla's surface night is a *sequence* of independent rolls, each of which
/// returns the moment it hits, and the biome pool is only what is left when every one of them
/// misses. Folding these into a weighted list would give the Raven and the Ghost a share of every
/// night instead of one attempt in twelve and one in thirty, so the shape is kept: `None` means
/// nothing here answered and the caller should draw from [`pool`] as before.
///
/// Reached whenever the surface is dark, and *also* in daylight in a graveyard (`NPC.cs:4202`).
/// Not reached in the corruption, the crimson, the jungle or the dungeon, which vanilla answers
/// with their own arms of the outer chain before `else if (surfaceSpawn)` at `NPC.cs:4168` is
/// tested at all.
///
/// Deliberately left to the pools and to other lanes, each with its place in the order below kept
/// so the arms that *are* here land at vanilla's own odds:
///
/// * `NPC.cs:4618` the blood-moon Clown, `:4638` the Possessed Armor and `:4643` the Blood Zombie
///   and Drippler. All three already have a home in [`hardmode_pool`] and [`blood_moon_pool`];
///   taking them here as well would give each of them two sources. The last two now make their
///   rolls and hand the draw back, which is what the promise above is worth: without them a snowy
///   hardmode night would answer with a Wolf where the game had already answered with an armour.
///   (The line numbers here were `:4636` and `:4640` and named the wrong arm; both are corrected.)
/// * `NPC.cs:4665` the Armed Zombie Eskimo, whose gate is `Main.expertMode` and nothing this
///   signature can see. It is the third branch of the snow arm below, so skipping it hands its
///   share to the arm's own fallthrough, which is 161 and already in [`pool`]'s snow night. 431
///   stays disclosed.
/// * `NPC.cs:4691-4711` the Skyblock arm, for a world shape this server does not generate.
/// * `NPC.cs:4712` the Moss Zombie, which is gated on `RollOnlyBadLuckExtreme(30) == 0`. That
///   returns `-1` for any player whose luck is not negative (`Luck.cs:53-60`), and this server
///   models no luck at all, so the arm cannot fire here any more than it fires for a vanilla player
///   with no luck effects. Same reasoning, and the same citation, as the `rate * 0.85` bonus
///   [`rates`] declines to transcribe. NPC 691 therefore stays in `docs/spawn-gaps.tsv` on purpose.
/// * `NPC.cs:4722` the Torch Zombie and `:4743` the armed and styled zombies, which are the
///   ordinary zombie's own variants rather than a season's.
///
/// The negative ids vanilla spawns alongside four of these arms (`-38` to `-43`, `NPC.cs:4569`,
/// `:4581-4610`, and `-54`/`-55` in the rain arm) are not types: `NPCID.FromNetId`
/// (`NPCID.cs:12478`) maps them back onto the very same NPC with a size multiplier
/// (`NPC.cs:8080-8137`), so `SmallRainZombie` and `BigRainZombie` are both 223.
/// `tools/check_spawn_reach.py` drops them from vanilla's roster for that reason, and nothing here
/// models NPC scale, so they are dropped here too.
///
/// `ground_block` is the tile the spawn stands on, the game's own `spawnTileY` (`NPC.cs:329`), which
/// is this server's `y + 1`. Only the snow arm reads it.
pub fn seasonal_night_pick(at: Seasonal, ground_block: u16, rng: &mut SmallRng) -> Option<u16> {
    let one_in = |rng: &mut SmallRng, n: u32| rng.random_ratio(1, n);

    // NPC.cs:4539. The Raven is the season's own bird, and a graveyard has one whether or not it is
    // October.
    if (at.halloween || at.graveyard) && one_in(rng, 12) {
        return Some(301); // Raven
    }
    // NPC.cs:4544. The Ghost is the graveyard's alone: no calendar produces one.
    if at.graveyard && one_in(rng, 30) {
        return Some(316); // Ghost
    }
    // NPC.cs:4549.
    if (at.halloween || at.graveyard) && at.hard_mode && one_in(rng, 10) {
        return Some(304); // HoppinJack
    }
    // NPC.cs:4554: one attempt in six takes the demon-eye branch, and on the new moon a further one
    // in two of the rest takes it as well, so the darkest night of the eight is roughly three times
    // as full of eyes as the others.
    if one_in(rng, 6) || (at.moon_phase == 4 && one_in(rng, 2)) {
        // NPC.cs:4556. Left to `hardmode_pool`, which already carries the Wandering Eye; the roll
        // itself is still made, because skipping it would hand its third of this branch to the arms
        // below and make them a third too common in hardmode.
        if at.hard_mode && one_in(rng, 3) {
            return None;
        }
        // NPC.cs:4561: `Main.rand.Next(317, 319)`, so 317 or 318.
        if at.halloween && one_in(rng, 2) {
            return Some(317 + rng.random_range(0..2)); // DemonEyeOwl, DemonEyeSpaceship
        }
        // NPC.cs:4566. The plain Demon Eye, which the surface night pool also carries; the same
        // reasoning as the Wandering Eye above applies, so the roll stands and the draw is left to
        // the pool.
        if one_in(rng, 2) {
            return None;
        }
        // NPC.cs:4577-4612: the five coloured eyes, one of which is drawn flat.
        const EYES: [u16; 5] = [
            190, // CataractEye
            191, // SleepyEye
            192, // DialatedEye
            193, // GreenEye
            194, // PurpleEye
        ];
        return Some(EYES[rng.random_range(0..EYES.len())]);
    }
    // NPC.cs:4623 and :4628. `RollOnlyBadLuck(300)` is a plain `Main.rand.Next(300)` at luck zero
    // (`Luck.cs:31-37`), unlike its `Extreme` sibling. The pair are the only wedding a blood moon
    // ever throws, and a graveyard has them on an ordinary night.
    if (at.blood_moon || at.graveyard) && one_in(rng, 300) {
        return Some(53); // TheGroom
    }
    if (at.blood_moon || at.graveyard) && one_in(rng, 300) {
        return Some(536); // TheBride
    }
    // NPC.cs:4633. The full moon, and note the third condition: this is *two* attempts in three, not
    // every one, and `!Main.dayTime` is explicit here, so a daylit graveyard gets no werewolves.
    if !at.day_time && at.moon_phase == 0 && at.hard_mode && !one_in(rng, 3) {
        return Some(104); // Werewolf
    }
    // NPC.cs:4638, the Possessed Armor, and `:4643`, the Blood Zombie and the Drippler. Both are
    // already carried by [`hardmode_pool`] and [`blood_moon_pool`], so both rolls are made and the
    // draw is handed straight back: they sit above the snow and rain arms below, and without them a
    // hardmode or blood-moon night would reach those arms every time instead of two thirds and
    // three fifths of the time.
    if !at.day_time && at.hard_mode && one_in(rng, 3) {
        return None;
    }
    // `Main.rand.Next(5) < 2`, which is not a `one_in`.
    if at.blood_moon && rng.random_range(0..5) < 2 {
        return None;
    }
    // NPC.cs:4655, the snow arm. It keys on the tile underfoot rather than on the player's zone, so
    // a patch of ice in a forest answers here too, and it `return`s: a snowy graveyard has no Maggot
    // Zombies and a snowy October no costumed ones, because vanilla never reaches those arms with
    // ice under the spawn.
    //
    // `TileID.Sets.IcesSnow` is `{161, 200, 163, 164, 147}` (`TileID.cs:297`) and the arm adds 162,
    // the thin ice, on its own line. These are tile ids: 163 and 164 here are Purple and Pink Ice,
    // not the two spiders.
    // A `matches!` rather than a `[u16; 6]` and `contains`: every draw that gets past the werewolf
    // reaches this, and on `measure_the_graveyard_and_the_seasonal_chain`'s ordinary night the
    // linear array scan cost +3.8 ns against this form's +0.6 ns.
    if matches!(ground_block, 147 | 161 | 162 | 163 | 164 | 200) {
        // NPC.cs:4657 and `:4661`. The Ice Elemental and the Wolf come from here and from one
        // other place each in the whole spawner (`:5232` is the caverns' own Ice Elemental, the
        // flying chain's snow arm), so without this arm both were unreachable. Note `!ZoneGraveyard`
        // on both: a graveyard in the snow gets neither.
        if !at.graveyard && at.hard_mode && one_in(rng, 4) {
            return Some(169); // IceElemental
        }
        if !at.graveyard && at.hard_mode && one_in(rng, 3) {
            return Some(155); // Wolf
        }
        // NPC.cs:4665's expert Armed Zombie Eskimo and `:4671`'s plain Zombie Eskimo, which is the
        // arm's fallthrough and already in [`pool`]'s snow night. Handed back rather than answered.
        return None;
    }
    // NPC.cs:4675, the rain arm. All three of its outcomes are NPC 223 (`-54` and `-55` are the
    // small and big Rain Zombie, the same type at a different scale), so the inner `Next(3)` and
    // `Next(2)` collapse away.
    if at.raining && one_in(rng, 2) {
        return Some(223); // ZombieRaincoat
    }
    // NPC.cs:4717: the graveyard's own zombie. `maggotZombieChance` is 20 and nothing in
    // `GetZombieSettings` (`NPC.cs:5595-5619`) ever moves it.
    if at.graveyard && one_in(rng, 20) {
        return Some(632); // MaggotZombie
    }
    // NPC.cs:4734: `Main.rand.Next(319, 322)`, so 319, 320 or 321.
    if at.halloween && one_in(rng, 2) {
        return Some(319 + rng.random_range(0..3)); // ZombieDoctor, ZombieSuperman, ZombiePixie
    }
    // NPC.cs:4739: `Main.rand.Next(331, 333)`, so 331 or 332.
    if at.xmas && one_in(rng, 2) {
        return Some(331 + rng.random_range(0..2)); // ZombieXmas, ZombieSweater
    }
    None
}

/// The cavern chain's own seasonal arms, ahead of the ordinary pool: `NPC.cs:5005-5199`.
///
/// The surface's sibling of [`seasonal_night_pick`], and the same shape for the same reason:
/// vanilla's caverns are a *sequence* of independent rolls, each returning the moment it hits, and
/// the pool is only what is left when every one of them misses. `None` means nothing here answered
/// and the caller should draw from [`pool`] as before.
///
/// Named for the cavern rather than "underground" on purpose. Vanilla's own `underGround`
/// (`NPC.cs:1144`) is `spawnTileY <= Main.rockLayer`, the *dirt* layer, whose arm is `NPC.cs:4818`
/// and is not this one. This chain is the fallthrough below the rock layer and above the
/// underworld, which is [`Depth::Cavern`] here.
///
/// `lower_caverns` is `(double)spawnTileY > (Main.rockLayer + (double)Main.maxTilesY) / 2.0`
/// (`NPC.cs:5021`), the bottom half of the stone, which is a different line from
/// [`Conditions::below_dirt_midline`]'s `(worldSurface + rockLayer) / 2`.
///
/// `ground_block` is the tile the spawn stands on, the game's own `spawnTileY` (`NPC.cs:329`:
/// `FindSpawnTile` walks down to the first solid tile, so its `spawnTileY` is this server's
/// `y + 1`). It is here for the two stone arms below and nothing else.
///
/// Deliberately left out, each with its place in the order kept so the arms that *are* here land at
/// vanilla's own odds:
///
/// * `NPC.cs:5007` the Skeleton Merchant, who already has his own branch in [`try_spawn`] with the
///   two conditions this signature cannot see (standing water, and one alive at a time).
/// * `NPC.cs:5017` the Rune Wizard, already carried by [`hardmode_pool`].
/// * `NPC.cs:5088`, `:5100` the ice and snow arms, which belong to the snow pool, and which is also
///   why the caller keeps [`Biome::Snow`] out of this chain: both of them `return` above `:5115`,
///   so a snow cavern never sees a Halloween skeleton.
/// * `NPC.cs:5120` the four Bone Throwing Skeletons. The arm is `int num56 = Main.rand.Next(4)`
///   followed by three `num56 == 0` tests in a row, so 450 and 451 are dead in the game itself and
///   449 is one draw in four with 452 taking the rest. Transcribing the bug faithfully would still
///   leave two of the four in the gap list, and transcribing it *unfaithfully* to clear them is the
///   opposite of the point, so all four stay disclosed.
/// * `NPC.cs:5145-5198` the closing `switch`, whose four cases are the plain Skeleton and its three
///   look-alikes (201-203). [`pool`] already carries 21 for the caverns; the three variants are the
///   same enemy with a different sprite, so they are left rather than given the pool three more
///   entries that would quadruple the skeleton share of every cavern draw.
///
/// `glowshroom_ground` is vanilla's `ZoneGlowshroom && (tileType == 70 || tileType == 190)`, the
/// gate both halves of the chain put in front of their Spore pair (`NPC.cs:5110` and `:5209`). The
/// zone alone is not enough: the spawn also has to be standing on mushroom grass or a mushroom
/// block, so the caller decides it per candidate tile rather than once per player.
pub fn cavern_seasonal_pick(
    at: Seasonal,
    no_worms: bool,
    lower_caverns: bool,
    ground_block: u16,
    glowshroom_ground: bool,
    alive: &dyn Fn(u16) -> bool,
    rng: &mut SmallRng,
) -> Option<u16> {
    let one_in = |rng: &mut SmallRng, n: u32| rng.random_ratio(1, n);

    // NPC.cs:5005, `else if (Main.rand.Next(2) == 0)`: heads is this chain, the walkers, and tails
    // is the flying chain at `:5201` (Illuminant Bat, Jungle Bat, Chaos Elemental, Ice Bat, Giant
    // Bat). Both are [`pool`]'s business apart from the arms below, so the roll is still made and a
    // tails hands the draw straight back: skipping it would make every arm here twice as common.
    if !one_in(rng, 2) {
        // NPC.cs:5209, the flying half's own glowshroom arm and the Spore Bat's only source.
        //
        // Two arms sit above it and neither can answer where this is reached: `:5205`'s Jungle Bat
        // wants `ZoneJungle`, which the caller's own biome exclusion keeps out of this chain
        // entirely, and `:5201`'s Illuminant Bat wants a hardmode hallow. The hallow is *not*
        // excluded by the caller, so in a cavern that reads as hallowed and holds a hundred
        // mushroom tiles at once, vanilla would answer with an Illuminant Bat half the time and
        // this answers Spore Bat every time. That place needs pearlstone and mushroom grass in the
        // same scan box; the narrowing is disclosed rather than paid for with a biome parameter.
        if glowshroom_ground {
            return Some(634); // SporeBat
        }
        return None;
    }
    // NPC.cs:5012-5015, `if (Main.rand.Next(80) == 0)`, one arm below the Skeleton Merchant the
    // caller answers and one above the Rune Wizard [`hardmode_pool`] carries. No progression gate
    // and no depth gate beyond the cavern chain she lives in: she is found at any point in a
    // world's life.
    //
    // She is not a prop. `NPC.aiStyle = 42` (`NPC.cs:11524`) is the disguise, and its routine
    // (`NPC.cs:30360-30389`) drops it: within two hundred pixels of a player it can see, or on
    // being moved or hurt at all, it counts twenty-one ticks and calls `Transform(196)`. All of
    // that was already here and already tested (`game::ai::ambush::lost_girl`, dispatched at ai
    // style 42, whose `Some(NYMPH)` becomes [`crate::game::npc::Npc::become_type`] through
    // `Effects::transform`), so this one missing arm was the only reason either 195 or 196 had a
    // path into a world. An earlier reading of this file recorded the transformation as the
    // missing half and left the arm out on that basis; the transformation is present, and it was
    // the spawn that was missing.
    if one_in(rng, 80) {
        return Some(195); // LostGirl
    }
    // NPC.cs:5021. `offensiveToTim` (a second, likelier one in fifty for a player carrying a magic
    // weapon) is not read here, so Tim is very slightly rarer than the game's, never commoner.
    if lower_caverns && one_in(rng, 200) {
        return Some(45); // Tim
    }
    // NPC.cs:5027 and `:5039`, the marble and granite arms:
    //
    // ```csharp
    // if (nearMarble && Main.rand.Next(4) != 0)
    // {
    //     if (Main.rand.Next(6) != 0 && !AnyNPCs(480) && Main.hardMode) 480; else 481;
    //     return;
    // }
    // if (nearGranite && Main.rand.Next(5) != 0)
    // {
    //     if (Main.rand.Next(6) != 0 && !AnyNPCs(483)) 483; else 482;
    //     return;
    // }
    // ```
    //
    // All four types come from here and nowhere else in `NPC.Spawner`, which is why all four were
    // unreachable. Note what the gates actually say: only the Medusa is hardmode, so a granite
    // pocket has its golems and flyers on day one, and a marble one has its Greek Skeletons.
    //
    // `nearMarble`/`nearGranite` are narrowed to the ground tile itself (`NPC.cs:1059-1065`, the
    // first two of the flag's four branches). The other two are the player's own tile and a pair of
    // random box sweeps up to 61 tiles across (`NPC.cs:1067-1143`), left out for the same reason
    // `underground_desert_spot` and [`spider_pick`] leave out their equivalents: a real marble or
    // granite pocket is floored in its own stone, so the tile test finds the biome from inside it,
    // and the sweeps only widen the arm to spots outside one. The effect is that the four start a
    // little further in than the game's do, never that they turn up in ordinary rock.
    const MARBLE: u16 = 367;
    const GRANITE: u16 = 368;
    if ground_block == MARBLE && !one_in(rng, 4) {
        return Some(if !one_in(rng, 6) && !alive(480) && at.hard_mode {
            480 // Medusa
        } else {
            481 // GreekSkeleton
        });
    }
    if ground_block == GRANITE && !one_in(rng, 5) {
        return Some(if !one_in(rng, 6) && !alive(483) {
            483 // GraniteFlyer
        } else {
            482 // GraniteGolem
        });
    }
    // NPC.cs:5051, `if (Main.hardMode && Main.rand.Next(10) != 0)`: nine hardmode draws in ten
    // never reach the rest of this chain at all. Its own answers (Armored Viking, Armored Skeleton,
    // Icy Merman, Skeleton Archer) are all in [`hardmode_pool`] already, so the roll stands and the
    // draw is handed back. Without it a hardmode Halloween cavern would be ten times as full of
    // costumed skeletons as the game's.
    if at.hard_mode && !one_in(rng, 10) {
        return None;
    }
    // NPC.cs:5078. The graveyard's Ghost underground, and October's: unlike the surface arm at
    // `:4544`, which is the graveyard's alone, the calendar opens this one too. `!noWorms` is
    // vanilla's own condition on it, odd as it looks on something that does not burrow.
    if !no_worms && (at.halloween || at.graveyard) && one_in(rng, 30) {
        return Some(316); // Ghost
    }
    // NPC.cs:5083, the Undead Miner, and `:5105`, this world's own six cavern monsters. Both are
    // already in the caller's draw (44 in [`pool`], the six behind `CAVERN_SENTINEL`), so both
    // rolls are made and handed back. The one in three especially: dropping it would make the
    // Halloween arm below half again as common as the game's.
    if one_in(rng, 20) || one_in(rng, 3) {
        return None;
    }
    // NPC.cs:5110, the walking half's glowshroom arm, and the Spore Skeleton's only source. It sits
    // below the world's own six cavern monsters and above October's costumes, which is why it is
    // here rather than at the head: a mushroom cavern is still mostly an ordinary cavern.
    if glowshroom_ground {
        return Some(635); // SporeSkeleton
    }
    // NPC.cs:5115: `Main.rand.Next(322, 325)`, so 322, 323 or 324. The calendar alone, with no
    // graveyard half: standing among tombstones in June brings no costumes up out of the stone.
    if at.halloween && one_in(rng, 2) {
        // SkeletonTopHat, SkeletonAstonaut, SkeletonAlien
        return Some(322 + rng.random_range(0..3));
    }
    None
}

/// What hallowed ground offers ahead of the hallow pool (`NPC.cs:4039-4061`).
///
/// The whole arm, and its outer gate, is:
///
/// ```csharp
/// else if (((Main.hardMode && underGround) || (Main.remixWorld && Main.rand.Next(2) == 0))
///     && !waterTile && (tileType == 116 || tileType == 117 || tileType == 109 || tileType == 164))
/// {
///     if (downedPlantBoss && (Main.remixWorld || (!Main.dayTime && Main.time < 16200.0))
///         && surfaceSpawn && RollLuck(10) == 0 && !AnyNPCs(661))          661;
///     else if (raining && !AnyNPCs(244) && RollLuck(10) == 0)             244;
///     else if (!Main.dayTime && Main.rand.Next(2) == 0)                   122;
///     else if (Main.rand.Next(10) == 0 || (ZoneWaterCandle && Main.rand.Next(10) == 0)) 86;
///     else                                                                75;
/// }
/// ```
///
/// It is an ordered chain rather than a pool, which is the point: it is the only source in the whole
/// of `NPC.Spawner` for three of its five members. The Prismatic Lacewing (661) is the Empress of
/// Light's only summon, so with this arm missing an entire boss was unreachable; the Rainbow Slime
/// (244) and the Unicorn (86) come from here and nowhere else either, and all three sat in
/// `docs/spawn-gaps.tsv` together for exactly one reason.
///
/// `underGround` is `spawnTileY <= Main.rockLayer` (`NPC.cs:1144`), the *dirt* layer and above, so
/// the caller's gate is [`Depth::Surface`] or [`Depth::Underground`]; `surfaceSpawn` is
/// `spawnTileY <= Main.worldSurface` (`:1203`), which is [`Depth::Surface`] alone. The two are not
/// alternatives: hallowed grass in daylight is both at once, which is why the Lacewing's own arm can
/// carry `surfaceSpawn` inside a branch already gated on `underGround`.
///
/// Two leaves hand the draw back rather than answering, the same way [`cavern_seasonal_pick`] hands
/// back every arm [`pool`] already carries: the Gastropod (122) and the Pixie (75) are both in
/// [`hardmode_pool`]'s hallow entry at every depth this is reached from. Their rolls are still made,
/// in vanilla's own order, so the three arms above them land at the game's odds rather than at
/// inflated ones. The visible difference is that a hallow night here can also answer with an
/// Illuminant Bat or Slime where vanilla's `else` would have said Pixie.
///
/// Three narrowings, each disclosed rather than invented:
///
/// * `Main.remixWorld` is not modelled anywhere in this server, so the outer gate's second half
///   drops and the Lacewing's `(remixWorld || night)` collapses to the night half. Same treatment,
///   and same reason, as the Goblin Scout's arm in [`try_spawn`].
/// * `ZoneWaterCandle` is not modelled either (see [`sky_pick`], where its two arms are dead code in
///   the game itself), so the Unicorn's second chance drops and its gate is the plain one in ten.
/// * `RollLuck(10)` is `Main.rand.Next(10)` at luck zero (`Luck.cs:5-16`).
///
/// `alive` is vanilla's `AnyNPCs`. Both uses of it are real gates rather than politeness: one
/// Lacewing at a time is what stops a hallow night raining Empresses.
pub fn hallow_ground_pick(
    downed_plant_boss: bool,
    day_time: bool,
    time: i32,
    surface_spawn: bool,
    raining: bool,
    alive: &dyn Fn(u16) -> bool,
    rng: &mut SmallRng,
) -> Option<u16> {
    // NPC.cs:4041.
    if downed_plant_boss
        && !day_time
        && time < LACEWING_LATEST
        && surface_spawn
        && rng.random_range(0..LACEWING_ODDS) == 0
        && !alive(PRISMATIC_LACEWING)
    {
        return Some(PRISMATIC_LACEWING);
    }
    // NPC.cs:4045.
    if raining && !alive(244) && rng.random_range(0..RAINBOW_SLIME_ODDS) == 0 {
        return Some(244); // RainbowSlime
    }
    // NPC.cs:4049, the Gastropod, which [`hardmode_pool`] already carries: the roll is made and the
    // draw handed back.
    if !day_time && rng.random_ratio(1, 2) {
        return None;
    }
    // NPC.cs:4053.
    if rng.random_range(0..UNICORN_ODDS) == 0 {
        return Some(86); // Unicorn
    }
    // NPC.cs:4059's `else` is the Pixie, which [`hardmode_pool`] carries too.
    None
}

/// The holiday costume a critter or a plain slime wears, or the type unchanged.
///
/// Vanilla puts these swaps inside the draw rather than after it, as an `else if` above the
/// ordinary answer: `SpawnBunny`'s chain (`NPC.cs:1637-1644`, and again at `:2588-2595` and
/// `:4284-4291`) tries the Halloween bunny, then the Christmas one, then the party one, then the
/// plain one; `GetBasicSlimeToSpawn` (`NPC.cs:5654-5663`) does the same for the surface's blue
/// slime. Applying them to the type already drawn is the same distribution written the other way
/// round, and it does not need the season threaded through every pool table.
///
/// `GetBasicSlimeToSpawn_ChanceToBeHolidaySlime` (`NPC.cs:5678-5685`) is `Main.rand.Next(3) != 0`
/// outside Skyblock, the same two-in-three the bunny uses.
///
/// The slime swap is surface-only because vanilla's is: `GetBasicSlimeToSpawn(surface: false, ...)`
/// has no holiday case at all, and the two tile types with their own answer (jungle grass, and snow
/// or ice) are answered before the default case this replaces.
fn holiday_costume(npc_type: u16, depth: Depth, at: Seasonal, rng: &mut SmallRng) -> u16 {
    const BUNNY: u16 = 46;
    const BLUE_SLIME: u16 = 1;
    let two_in_three = |rng: &mut SmallRng| !rng.random_ratio(1, 3);
    match npc_type {
        BUNNY if at.halloween && two_in_three(rng) => 303, // BunnySlimed
        BUNNY if at.xmas && two_in_three(rng) => 337,      // BunnyXmas
        BLUE_SLIME if depth == Depth::Surface && at.halloween && two_in_three(rng) => 302, // SlimeMasked
        // `Main.rand.Next(333, 337)`, so 333 to 336: the four ribbon colours.
        BLUE_SLIME if depth == Depth::Surface && at.xmas && two_in_three(rng) => {
            333 + rng.random_range(0..4)
        }
        _ => npc_type,
    }
}

/// Classify a spawn point by how far down it is.
pub fn depth_at(world: &World, y: i32) -> Depth {
    if y >= world.height() - UNDERWORLD_DEPTH {
        Depth::Underworld
    } else if y >= i32::from(world.rock_layer) {
        Depth::Cavern
    } else if y >= i32::from(world.surface) {
        Depth::Underground
    } else {
        Depth::Surface
    }
}

/// Half-extents of the biome scan box, in tiles.
///
/// The game scans `SceneMetrics.ZoneScanSize`, a 169-by-124 tile box centred on the tile it is
/// asked about (`SceneMetrics.cs:16`: `1920/16 + 25*2 - 1 = 169` across, `1200/16 + 25*2 - 1 = 124`
/// down). This used a 41-by-41 box (radius 20), which is small enough to miss a biome the player is
/// plainly standing in and large enough only to be fooled by a stray vein: both directions of
/// wrong. 169 is odd, so it is symmetric at plus-or-minus 84; 124 is even, taken as -62..=61.
const BIOME_SCAN_X: i32 = 84;
const BIOME_SCAN_Y_UP: i32 = 62;
const BIOME_SCAN_Y_DOWN: i32 = 61;

/// The per-biome tile counts a scan must reach for the place to read as that biome.
///
/// These are the game's own `SceneMetrics` thresholds, not one flat number: the evils and the
/// hallow are cheap to declare (a small pocket counts), snow and the desert are dear (a genuine
/// biome, not a stray patch). A single flat 60 was used for all of them, which made a handful of
/// sand read as a desert and a real snow field read as forest. (`SceneMetrics.cs:24-58`,`154-175`.)
const EVIL_THRESHOLD: i32 = 300; // CorruptionTileThreshold
const BLOOD_THRESHOLD: i32 = 300; // CrimsonTileThreshold
const HOLY_THRESHOLD: i32 = 125; // HallowTileThreshold
const JUNGLE_THRESHOLD: i32 = 140; // JungleTileThreshold
const SNOW_THRESHOLD: i32 = 1500; // SnowTileNormalThreshold
const DESERT_THRESHOLD: i32 = 1500; // DesertTileNormalThreshold
const DUNGEON_THRESHOLD: i32 = 250; // DungeonTileThreshold
const MUSHROOM_THRESHOLD: i32 = 100; // MushroomTileThreshold (`SceneMetrics.cs:52`)
const METEOR_THRESHOLD: i32 = 75; // MeteorTileThreshold (`SceneMetrics.cs:56`)
const SHIMMER_THRESHOLD: i32 = 300; // ShimmerTileThreshold (`SceneMetrics.cs:24`)

/// Work out the biome from the tiles around a point, the way the game counts zone tiles.
///
/// Faithful to `SceneMetrics.ScanTiles`/`AggregateTileCounts`/`CalculateZones` in its scan box, its
/// per-biome tile lists and thresholds, and its evil-versus-hallow subtraction. Two disclosed
/// narrowings: the ocean is decided by position rather than by `oceanDepths`, as it was before; and
/// the sunflower (`tile 27`) and a couple of hardmode-only additions the game folds into the evil
/// and blood counts are omitted, because this server does not place them. The dungeon is taken on
/// its tile count alone rather than also requiring a dungeon wall at the centre, since a run of
/// dungeon brick is dungeon enough and the wall is not always modelled where this is called.
pub fn biome_at(world: &World, x: i32, y: i32) -> Biome {
    zones_at(world, x, y).biome
}

/// The same scan, also answering the zone flags that are not one of [`Biome`]'s winners.
///
/// The game's zones are independent flags, not one winner: `SceneMetrics.CalculateZones` sets
/// `ZoneDesert = EnoughTilesForDesert` (`SceneMetrics.cs:683`) alongside `ZoneCorrupt`,
/// `ZoneCrimson` and `ZoneHallow`, so a corrupted desert really is both at once. [`Biome`] has to
/// pick one, and it picks the evil, because the evils are what most of the game's own spawn checks
/// read first.
///
/// That collapse costs nothing anywhere else, and one thing here: `ZoneSandstorm` is
/// `ZoneDesert && ...`, so a corrupt, crimson or hallowed desert - which is exactly where the three
/// converted sandsharks live - would never have read as a sandstorm at all, and those three types
/// would have been unreachable in a real world while looking reachable to the roster. The count is
/// already made by the scan above; this only stops throwing it away, so the flag is free.
///
/// `ZoneGlowshroom` and `ZoneMeteor` are here for the same reason and at the same price: neither is
/// ever a `Biome` (a glowing mushroom cave sits on mud and reads as forest; a crater sits in
/// whatever it fell on), both are a plain tile count in the box this already walks, and each is the
/// only gate on a spawn branch of its own. [`Zones::shimmer`] joins them for the same reason,
/// counted off the liquid on the tiles the block counts skip.
pub fn zones_at(world: &World, x: i32, y: i32) -> Zones {
    // The ocean is defined by position rather than tiles.
    if x < 250 || x > world.width() - 250 {
        return Zones {
            biome: Biome::Ocean,
            desert: false,
            glowshroom: false,
            meteor: false,
            shimmer: false,
        };
    }

    // The game's own per-biome tile lists (`SceneMetrics.AggregateTileCounts`). A tile can belong
    // to several at once (corrupt sandstone is both evil and sand, hallowed ice both holy and
    // snow), so each list is checked independently rather than in one match, exactly as the game
    // sums them into separate counts.
    // EvilTileCount (`SceneMetrics.cs:614`): ebonstone, corrupt grass/thorn/ice/sandstone/sand.
    const EVIL_TILES: [u16; 9] = [23, 661, 24, 25, 32, 112, 163, 400, 398];
    // BloodTileCount (`SceneMetrics.cs:615`): crimstone, crimson grass/thorn/ice/sandstone, crimsand, ichor.
    const BLOOD_TILES: [u16; 9] = [199, 662, 201, 203, 200, 401, 399, 234, 352];
    // HolyTileCount (`SceneMetrics.cs:603`): pearlstone, hallow-converted stones, pearlsand, hallowed grass/ice/sandstone.
    const HOLY_TILES: [u16; 9] = [109, 492, 110, 113, 117, 116, 164, 403, 402];
    // JungleTileCount (`SceneMetrics.cs:613`): jungle grass, plants, vines, mud, jungle thorn.
    const JUNGLE_TILES: [u16; 6] = [60, 61, 62, 74, 226, 225];
    // SnowTileCount (`SceneMetrics.cs:604`): snow, snow brick, ice, purple/red ice, slush.
    const SNOW_TILES: [u16; 7] = [147, 148, 161, 162, 164, 163, 200];
    // SandTileCount (`SceneMetrics.cs:620`): sand plus every converted sand/sandstone.
    const SAND_TILES: [u16; 12] = [53, 396, 397, 112, 116, 234, 398, 402, 399, 400, 403, 401];
    // DungeonTileCount (`SceneMetrics.cs:619`): the six dungeon bricks.
    const DUNGEON_TILES: [u16; 6] = [41, 43, 44, 481, 482, 483];
    // MushroomTileCount (`SceneMetrics.cs:617`): mushroom grass, plants, trees and vines.
    const MUSHROOM_TILES: [u16; 4] = [70, 71, 72, 528];
    // MeteorTileCount (`SceneMetrics.cs:618`): meteorite ore, and nothing else.
    const METEORITE: u16 = 37;

    // Raw tile counts, before the game's evil/hallow reconciliation.
    let (mut evil, mut blood, mut holy, mut jungle, mut snow, mut sand, mut dungeon) =
        (0, 0, 0, 0, 0, 0, 0);
    let (mut mushroom, mut meteor, mut shimmer) = (0, 0, 0);
    for dy in -BIOME_SCAN_Y_UP..=BIOME_SCAN_Y_DOWN {
        for dx in -BIOME_SCAN_X..=BIOME_SCAN_X {
            let tile = world.tile(x + dx, y + dy);
            if !tile.is_active() {
                // A tile the block counts skip is exactly where the game counts its liquids:
                // `if (!tile.active()) { if (tile.liquid > 0) _liquidCounts[tile.liquidType()]++;
                // continue; }` (`SceneMetrics.cs:367-372`). Only bucket 3 is ever read as a zone
                // (`ShimmerTileCount = _liquidCounts[3]`, `:601`), so only bucket 3 is counted.
                shimmer += i32::from(
                    tile.liquid > 0
                        && tile.liquid_kind == terrustia_proto::tile::Liquid::Shimmer,
                );
                continue;
            }
            let b = tile.block;
            evil += i32::from(EVIL_TILES.contains(&b));
            blood += i32::from(BLOOD_TILES.contains(&b));
            holy += i32::from(HOLY_TILES.contains(&b));
            jungle += i32::from(JUNGLE_TILES.contains(&b));
            snow += i32::from(SNOW_TILES.contains(&b));
            sand += i32::from(SAND_TILES.contains(&b));
            dungeon += i32::from(DUNGEON_TILES.contains(&b));
            mushroom += i32::from(MUSHROOM_TILES.contains(&b));
            meteor += i32::from(b == METEORITE);
        }
    }

    // The game reconciles the two evils against the hallow before thresholding, so a tile that reads
    // as both does not count for both (`SceneMetrics.cs:648-664`).
    let holy_before = holy;
    holy -= evil;
    holy -= blood;
    evil -= holy_before;
    blood -= holy_before;
    let (holy, evil, blood) = (holy.max(0), evil.max(0), blood.max(0));

    // The dungeon takes precedence, then the first biome to reach its own threshold in a fixed
    // order (the evils first, as the game's own spawn checks read them first). Snow and desert sit
    // last because their thresholds are the dearest and a corrupted snow reads as corruption in the
    // game too.
    // `ZoneDesert = EnoughTilesForDesert` on its own, whatever else the place also is, and the same
    // for the other two flags that are nobody's winner.
    let flags = Zones {
        biome: Biome::Forest,
        desert: sand >= DESERT_THRESHOLD,
        glowshroom: mushroom >= MUSHROOM_THRESHOLD,
        meteor: meteor >= METEOR_THRESHOLD,
        shimmer: shimmer >= SHIMMER_THRESHOLD,
    };
    if dungeon >= DUNGEON_THRESHOLD {
        return Zones {
            biome: Biome::Dungeon,
            ..flags
        };
    }
    for (count, threshold, biome) in [
        (evil, EVIL_THRESHOLD, Biome::Corruption),
        (blood, BLOOD_THRESHOLD, Biome::Crimson),
        (holy, HOLY_THRESHOLD, Biome::Hallow),
        (jungle, JUNGLE_THRESHOLD, Biome::Jungle),
        (snow, SNOW_THRESHOLD, Biome::Snow),
        (sand, DESERT_THRESHOLD, Biome::Desert),
    ] {
        if count >= threshold {
            return Zones { biome, ..flags };
        }
    }
    flags
}

/// The enemies that can appear at a given place and time, pre-hardmode.
///
/// Every id here was resolved from `NPCID` by name and checked against the stats table, because
/// the numbers are not guessable: Undead Miner is 44 rather than 52, Sand Slime is 537, and Blood
/// Crawler is 239. The coloured slimes (Green, Purple, Jungle and the rest) are *negative* net ids
/// — variants of Blue Slime — so they are not in these pools; see `docs/protocol-notes.md`.
pub fn pool(depth: Depth, biome: Biome, day: bool) -> &'static [u16] {
    use Biome::*;
    use Depth::*;

    match (depth, biome) {
        // The hallow does not exist before the wall falls, so nothing pre-hardmode lives there of
        // its own. It borrows the forest's pool so a hallowed forest is not silently empty of the
        // ordinary things; what is *only* there in hardmode is in `hardmode_pool`.
        (depth, Hallow) => pool(depth, Forest, day),
        (Underworld, _) => &[
            24, // FireImp
            59, // LavaSlime
            60, // Hellbat
            62, // Demon
            66, // VoodooDemon
            39, // BoneSerpentHead
        ],
        // The three big Angry Bones are not hardmode and not rare: the dungeon chain's own
        // fallthrough is `int num43 = Main.rand.Next(5)` with cases 0, 1 and 2 taking 294, 295 and
        // 296 (`NPC.cs:2767-2783`), and only cases 3 and 4 reach the plain Angry Bones below it
        // (`:2784-2795`, where `-14` and `-13` are size variants of that same 31 rather than types
        // of their own). Three fifths of what a pre-Skeletron-cleared dungeon actually throws at a
        // player was therefore missing, and the dungeon was correspondingly softer than the game's.
        (_, Dungeon) => &[
            31,  // AngryBones
            294, // AngryBonesBig
            295, // AngryBonesBigMuscle
            296, // AngryBonesBigHelmet
            32,  // DarkCaster
            34,  // CursedSkull
            71,  // DungeonSlime
        ],
        // The ocean's own roster is *aquatic*, and lives in [`water_pool`]: vanilla reaches it only
        // through `waterTile && isOcean` (`NPC.cs:1798`), so a shark cannot appear on dry sand.
        // A dry beach tile falls through to the ordinary surface pool the same way vanilla's does,
        // which is why standing on the shore at night still brings zombies.
        (depth, Ocean) => pool(depth, Forest, day),
        // No Corrupt Bunny (47) here, deliberately: vanilla's `NPC.Spawner` never spawns one at
        // any depth. The only way a Corrupt Bunny exists is `AttemptToConvertNPCToEvil`
        // (`NPC.cs:93050`) turning an ordinary Bunny under a blood moon, which
        // `convert_critters_under_a_blood_moon` now does. Listing it here made them appear out of
        // nothing in daylight.
        (Surface, Corruption) => &[
            6, // EaterofSouls
            7, // DevourerHead
        ],
        (_, Corruption) => &[
            6,  // EaterofSouls
            7,  // DevourerHead
            81, // CorruptSlime
        ],
        (Surface, Crimson) => &[
            173, // Crimera
            181, // FaceMonster
        ],
        (_, Crimson) => &[
            173, // Crimera
            181, // FaceMonster
            239, // BloodCrawler
        ],
        (Surface, Jungle) => &[
            42, // Hornet
            51, // JungleBat
        ],
        // The Spiked Jungle Slime is *not* a hardmode enemy, whatever a summary of the underground
        // jungle roster suggests: its arm is `tileType == 60 && spawnTileY > (worldSurface +
        // rockLayer) / 2.0` (`NPC.cs:3929`) with a one-in-four roll inside it (`:3931-3934`), and
        // `Main.hardMode` appears nowhere in either. It sits in the same arm as the Man Eater
        // (`:3935-3938`) and takes the same depth gate, so it is treated the same way this pool
        // already treats 43: the arm's own "bottom half of the underground layer and everything
        // below" is narrowed to "not the surface", which is the depth granularity [`Depth`] has.
        (_, Jungle) => &[
            42,  // Hornet
            43,  // ManEater
            56,  // Snatcher
            51,  // JungleBat
            204, // SpikedJungleSlime
        ],
        (Surface, Snow) => {
            if day {
                &[
                    147, // IceSlime
                    185, // SnowFlinx
                ]
            } else {
                &[
                    161, // ZombieEskimo
                    167, // UndeadViking
                    147, // IceSlime
                ]
            }
        }
        (_, Snow) => &[
            147, // IceSlime
            185, // SnowFlinx
            167, // UndeadViking
            150, // IceBat
        ],
        (Surface, Desert) => {
            if day {
                &[
                    61,  // Vulture
                    537, // SandSlime
                ]
            } else {
                &[
                    61,  // Vulture
                    537, // SandSlime
                    3,   // Zombie
                ]
            }
        }
        (_, Desert) => &[
            537, // SandSlime
            69,  // Antlion
            580, // WalkingAntlion
        ],
        (Surface, Forest) => {
            if day {
                // Only the slime is hostile here. The bunny, bird, squirrel and frog this used to
                // list are damage-0 critters the game spawns down its own `spawnFriendly` path
                // (`friendly_pool`), never at the player as monsters (`NPC.cs:2452-2624`).
                &[
                    1, // BlueSlime
                ]
            } else {
                &[
                    3, // Zombie
                    2, // DemonEye
                ]
            }
        }
        (Underground, _) => &[
            1,   // BlueSlime
            16,  // MotherSlime
            10,  // GiantWormHead
            44,  // UndeadMiner
            498, // Salamander
        ],
        (Cavern, _) => &[
            21,  // Skeleton
            49,  // CaveBat
            44,  // UndeadMiner
            16,  // MotherSlime
            10,  // GiantWormHead
            93,  // GiantBat
            498, // Salamander
        ],
    }
}

/// The ordinary draw weight, ten so a rarer type can be a single-digit fraction of it.
const ORDINARY_WEIGHT: u32 = 10;

/// Stands in for "this world's own cavern monsters" in a weighted pick, since that draw is a
/// function call rather than a fixed type. Never a real NPC id.
const CAVERN_SENTINEL: u16 = u16::MAX;

/// The Dungeon Guardian (`NPCID.DungeonGuardian`), the near-unkillable scythe the dungeon throws at
/// anyone who enters before Skeletron is down.
const DUNGEON_GUARDIAN: u16 = 68;

/// The Tortured Soul (`NPCID.TorturedSoul`, the table's `DemonTaxCollector`), the one hostile in
/// the game a player turns into a townsperson rather than kills.
pub const TORTURED_SOUL: u16 = 534;

/// The Skeleton Merchant (`NPCID.SkeletonMerchant`), a wandering vendor rather than a resident: he
/// takes no house, joins no town, and leaves on the ordinary despawn timer when nobody is near.
pub const SKELETON_MERCHANT: u16 = 453;

/// The three bound town slimes (`NPCID.BoundTownSlimeOld`, `...Purple`, `...Yellow`), each of which
/// becomes a resident pet once freed.
///
/// They are three finds in three different places rather than one roster: the Old Slime is a
/// caverns find beside the Goblin Tinkerer and the Wizard (`NPC.cs:2095`), the Purple Slime is up
/// in the sky with the Harpies (`NPC.cs:1417`), and the Yellow Slime is a jungle-grass critter
/// draw (`NPC.SpawnFrog`, `NPC.cs:5621-5634`). What they share is the shape of their gate: every
/// one of them is offered only while its own unlock flag is still false, so freeing one is what
/// takes it out of the world's spawn table for good.
pub const BOUND_TOWN_SLIME_OLD: u16 = 685;
pub const BOUND_TOWN_SLIME_PURPLE: u16 = 686;
pub const BOUND_TOWN_SLIME_YELLOW: u16 = 687;

/// The Goblin Scout (`NPCID.GoblinScout`), the game's only source of Tattered Cloth (item 362,
/// `npc_drops.rs`), which is the only ingredient of the Goblin Battle Standard, which is the only
/// way anybody summons a goblin army. Nothing else in the game drops that cloth.
pub const GOBLIN_SCOUT: u16 = 73;

/// The Statue Mimic (`NPCID.StatueMimic`), which stands among the tombstones pretending to be a
/// statue on a plinth until somebody walks close enough (`game::ai::mimic`).
pub const STATUE_MIMIC: u16 = 690;

/// The Rock Golem (`NPCID.RockGolem`), the hardmode caverns' heavy: 1000 life and 85 damage out of
/// the plain stone, which is more than anything else a cave holds.
pub const ROCK_GOLEM: u16 = 631;

/// `Main.rand.Next(50)` in `CheckToSpawnRockGolem` (`NPC.cs:5809`).
const ROCK_GOLEM_ODDS: u32 = 50;

/// Plain Stone (`TileID.Stone`), one of the two grounds a Rock Golem is cut from; the other is
/// every colour of moss ([`terrustia_proto::tile_sets::is_moss`]).
const STONE: u16 = 1;

/// How far above the ground row the Rock Golem's headroom check reads (`NPC.cs:5813`,
/// `spawnTileY - 4`). A golem is 48 pixels tall and stands 1.1x scaled, so it wants a taller hole
/// than the three rows [`open_space`] already asked for.
const ROCK_GOLEM_HEADROOM: i32 = 4;

/// The Lihzahrd Temple's whole ambient roster (`NPC.cs:3914-3924`):
///
/// ```csharp
/// else if (((tileType == 226 || tileType == 232) && ZoneLihzhardTemple) || (Main.remixWorld && ZoneLihzhardTemple))
/// {
///     if (Main.rand.Next(3) == 0) SpawnNPC(..., 226); else SpawnNPC(..., 198);
/// }
/// ```
///
/// Both are unreachable anywhere else in `NPC.Spawner`, so before this the post-Plantera temple -
/// the one place in the game a player farms for Solar Tablet fragments and Lihzahrd Power Cells -
/// spawned nothing at all. The `Main.remixWorld` half of the gate is not modelled here, so the two
/// ground tiles are the whole of it.
const LIHZAHRD: u16 = 198;
const FLYING_SNAKE: u16 = 226;

/// `Main.rand.Next(3)` on the temple's own fork (`NPC.cs:3916`): a Flying Snake one draw in three,
/// a Lihzahrd the other two.
const FLYING_SNAKE_ODDS: u32 = 3;

/// The two ground tiles the temple arm accepts (`TileID.LihzahrdBrick`, `TileID.WoodenSpikes`).
const LIHZAHRD_BRICK: u16 = 226;
const WOODEN_SPIKES: u16 = 232;

/// `WallID.LihzahrdBrickUnsafe`, which is the entire definition of `ZoneLihzhardTemple`
/// (`SceneMetrics.cs:693`). It is an unsafe wall, so `Main.wallHouse[87]` is false
/// (`Main.cs:9880-10745` never sets it) and a temple does not suppress its own spawns.
const TEMPLE_WALL: u16 = 87;

/// `Main.rand.Next(20)` on the Tortured Soul's branch (`NPC.cs:4877`).
const TORTURED_SOUL_ODDS: u32 = 20;

/// The Goblin Scout's two rolls (`NPC.cs:4482`), which are OR'd rather than exclusive: one attempt
/// in fifteen wherever the distance gate is open, and a second, better chance on top of it while a
/// shadow orb has been smashed and the world is still waiting for its first goblin army.
const GOBLIN_SCOUT_ODDS: u32 = 15;
const GOBLIN_SCOUT_ORB_ODDS: u32 = 7;

/// Plain Sand (`TileID.Sand`), the one ground the Goblin Scout is never found on: vanilla's three
/// `tileType == 53` arms (`NPC.cs:4390`, `:4474`, `:4478`) answer every dry sand tile before the
/// chain reaches him.
const SAND: u16 = 53;

/// `RollBadLuckExtreme(25)` on the Statue Mimic's branch (`NPC.cs:1571`), which at luck zero is a
/// plain `Main.rand.Next(25)` (`Luck.cs:40-51`) and so is what this server models, the same
/// narrowing (and the same reasoning) as every other `Roll*Luck` call site in this file.
const STATUE_MIMIC_ODDS: u32 = 25;

/// The dungeon library's own two (`NPCID.WaterBoltMimic`, `NPCID.LibrarianSkeleton`), the only NPCs
/// in the game that spawn on a *bookshelf* rather than on the tile the attempt found
/// (`NPC.cs:2748-2766`).
///
/// Both already had their routines here and neither could ever run one. The mimic is a cursed skull
/// that plays dead (`game::ai::skull`, `NPC.cs:21933-21998`) and the librarian a hardmode caster
/// (`conjuring(693)` in `npc_params`, `NPC.cs:21323-21337`), and with no arm to spawn either,
/// nothing this server could do put one in a world.
pub const WATER_BOLT_MIMIC: u16 = 694;
pub const LIBRARIAN_SKELETON: u16 = 693;

/// Their two rolls, `Main.rand.Next(8)` and `Main.rand.Next(10)` (`NPC.cs:2749`, `:2758`).
///
/// The second is an `else if`, not a second independent chance: the librarian is offered only on the
/// seven attempts in eight the mimic's roll turned down, so it is one in eight against roughly one
/// in eleven rather than one in eight against one in ten.
const WATER_BOLT_MIMIC_ODDS: u32 = 8;
const LIBRARIAN_ODDS: u32 = 10;

/// `TileID.Books` (`TileID.cs:537`), a dungeon shelf's single book tile.
///
/// Vanilla's test is `tile.type != 50` and nothing else (`NPC.cs:62972`), so neither `BooksEcho`
/// (707, `TileID.cs:1851`) nor the ordinary `Bookcases` furniture is a shelf for this purpose.
const BOOK_TILE: u16 = 50;

/// The box the spawner looks for a book in: `new Point(spawnTileX - 16, spawnTileY - 16), 32, 32`
/// (`NPC.cs:2752`, `:2761`). The offset is what makes the box surround the candidate rather than
/// hang off one corner of it.
const BOOK_SEARCH_BACK: i32 = 16;
const BOOK_SEARCH_SPAN: i32 = 32;

/// The Skeleton Merchant's two nested rolls collapsed (`NPC.cs:5004`'s `Next(2)` and `:5007`'s
/// `Next(35)`), because nothing between them can spawn anything.
const SKELETON_MERCHANT_ODDS: u32 = 70;

/// The Prismatic Lacewing, which is not a critter you catch for a collection: killing one is the
/// *only* way the Empress of Light is ever summoned (`NPC.cs:80309-80319`). With no arm spawning
/// it, an entire boss, her fight and her whole drop table were unreachable. The id itself lives in
/// `npc_params` beside the rest of its routine's numbers, since its own AI reads it too.
use terrustia_proto::npc_params::PRISMATIC_LACEWING;

/// The hallow chain's four hallowed grounds, `tileType == 116 || 117 || 109 || 164`
/// (`NPC.cs:4039`): Pearlsand, Pearlstone, HallowedGrass and HallowedIce.
const HALLOW_GROUND: [u16; 4] = [116, 117, 109, 164];

/// The night is 32,400 ticks long, so `Main.time < 16200.0` (`NPC.cs:4041`) is its first half: a
/// Lacewing is found between dusk and midnight and never after it.
const LACEWING_LATEST: i32 = 16_200;

/// `RollLuck(10)` on the Lacewing's arm and again on the Rainbow Slime's (`NPC.cs:4041`, `:4045`),
/// which at luck zero is a plain `Main.rand.Next(10)` (`Luck.cs:5-16`), the same narrowing as every
/// other `Roll*Luck` call site in this file.
const LACEWING_ODDS: u32 = 10;
const RAINBOW_SLIME_ODDS: u32 = 10;

/// `Main.rand.Next(10)` on the Unicorn's arm (`NPC.cs:4053`).
const UNICORN_ODDS: u32 = 10;

/// How often a hostile type is drawn relative to the others sharing its pool, the game's own
/// per-type spawn rate reduced to one number.
///
/// A flat uniform pick ignored this: the underworld draws a Voodoo Demon roughly one time in
/// seventy (`NPC.cs:4893-4897`: a one-in-seven branch, then a one-in-ten inside it), yet a
/// six-way uniform pick handed one out one time in six, about a dozen times too often. Only the
/// underworld's rates are transcribed here, because it is the one pre-hardmode pool whose cascade
/// is a plain sequence of `rand.Next` rolls rather than a thicket of tile and zone flags this
/// server does not model; every other pool keeps the ordinary weight, an even draw among its
/// members, which is what this did before minus the mis-placed critters. The numbers are the
/// cascade's effective shares (Hellbat is the fallthrough, the lava slime a one-in-three before it,
/// and so on down to the Voodoo Demon).
fn draw_weight(npc_type: u16) -> u32 {
    match npc_type {
        60 => 40, // Hellbat, the underworld fallthrough
        59 => 20, // LavaSlime
        62 => 9,  // Demon
        24 => 5,  // FireImp
        39 => 2,  // BoneSerpentHead (also capped, below)
        66 => 1,  // VoodooDemon, the rare one this fix exists for
        _ => ORDINARY_WEIGHT,
    }
}

/// The most of a type the field will hold at once, where the game caps it, else `None`.
///
/// The only verified pre-hardmode cap among the types these pools name is the Bone Serpent, which
/// the game gates on `!AnyNPCs(39)` so a second never begins while one is alive (`NPC.cs:4885`). A
/// world feeder is a screen-long chain of segments; two at once is a wall of them. Other heavies
/// are left uncapped rather than guessing a limit the game does not clearly set.
fn active_cap(npc_type: u16) -> Option<usize> {
    match npc_type {
        39 => Some(1), // BoneSerpentHead
        _ => None,
    }
}

/// Choose one entry from `candidates`, weighted by [`draw_weight`] and skipping any type already at
/// its [`active_cap`] per the live counts `alive` reports.
///
/// Returns the chosen id, which may be [`CAVERN_SENTINEL`] for a world's own cavern monsters, or
/// `None` when everything on offer is capped out. This is the weighted, cap-aware pick that
/// replaced a flat uniform index: the uniform one handed out a rare Voodoo Demon as often as a
/// common Hellbat and let a second Bone Serpent begin while the first was still on screen.
fn choose_weighted(
    candidates: &[u16],
    alive: &dyn Fn(u16) -> usize,
    rng: &mut SmallRng,
) -> Option<u16> {
    let eligible = |ty: u16| match active_cap(ty) {
        Some(cap) => alive(ty) < cap,
        None => true,
    };
    let total: u32 = candidates
        .iter()
        .copied()
        .filter(|&ty| eligible(ty))
        .map(draw_weight)
        .sum();
    if total == 0 {
        return None;
    }
    let mut roll = rng.random_range(0..total);
    for &ty in candidates {
        if !eligible(ty) {
            continue;
        }
        let w = draw_weight(ty);
        if roll < w {
            return Some(ty);
        }
        roll -= w;
    }
    None
}

/// The friendly critters the game draws instead of a monster when `spawnFriendly` is set.
///
/// This is the deferred friendly-critter table `rates` promised: with a populated base quieting the
/// wild, the game does not simply stop spawning, it spawns harmless critters (`NPC.cs:2099-2624`,
/// the whole `else if (spawnFriendly)` branch). Every id here is a real damage-0 critter, keyed by
/// the biome the player stands in rather than by the exact tile the game reads, which is the
/// disclosed narrowing: the game chooses a bird over a bunny by the grass under the spawn and by
/// weather, season and time this server does not all model, so this returns the ordinary set for
/// the place and lets the caller pick evenly among it. The underworld's lava-bait critters and the
/// gold and gem variants are left out on purpose; they are cosmetic rolls on top of these.
/// The whole of a graveyard's friendly draw (`NPC.cs:2101-2115`).
///
/// Not part of [`friendly_pool`], because vanilla's arm is not a variation on the ordinary one: it
/// sits ahead of every other friendly branch and `return`s, so among tombstones there are no birds,
/// no bunnies and no fireflies, only these two.
pub const GRAVEYARD_VERMIN: [u16; 2] = [
    606, // Maggot
    610, // Rat
];

/// The Frog (`NPCID.Frog`), which is not simply drawn: it is the last arm of a chain of its own.
const FROG: u16 = 361;

/// What a frog draw actually answers with (`NPC.SpawnFrog`, `NPC.cs:5621-5634`).
///
/// Vanilla never spawns a frog directly. Both jungle-grass friendly arms (`NPC.cs:2363` and
/// `:3831`) call `SpawnFrog`, which is a three-arm chain with the plain frog last, so the bound
/// Yellow Slime is only ever met by somebody wandering a jungle looking at frogs. It is the one
/// bound town slime with no progression gate and no depth gate whatsoever.
///
/// The middle arm, `RollLuck(goldCritterChance) == 0` -> Gold Frog (445), is left out: the gold
/// and gem critter variants are a class this server does not model at all, as [`friendly_pool`]
/// already declares, and taking only the frog would leave the Gold Bunny and Gold Goldfish out
/// while their sibling was in.
fn spawn_frog(world: &World, alive: &dyn Fn(u16) -> bool, rng: &mut SmallRng) -> u16 {
    // `RollLuck(30)` is `Main.rand.Next(30)` at luck zero (`Luck.cs:5-16`). The `alive` scan is
    // last on purpose: `&&` short-circuits, so an ordinary frog draw never walks the NPC table.
    if !world.progress.unlocked_slime_yellow
        && rng.random_range(0..30) == 0
        && !alive(BOUND_TOWN_SLIME_YELLOW)
    {
        return BOUND_TOWN_SLIME_YELLOW;
    }
    FROG
}

pub fn friendly_pool(depth: Depth, biome: Biome, day: bool) -> &'static [u16] {
    use Biome::*;
    use Depth::*;
    match (depth, biome) {
        // Penguins on snow and ice (`NPC.cs:2328-2337`).
        (_, Snow) => &[
            148, // Penguin
            149, // PenguinBlack
        ],
        // Scorpions on sand (`NPC.cs:2366-2368`).
        (_, Desert) => &[
            366, // ScorpionBlack
            367, // Scorpion
        ],
        // Goldfish and ducks on the water (`NPC.cs:2288-2322`).
        (_, Ocean) => &[
            55,  // Goldfish
            362, // Duck
            364, // DuckWhite
        ],
        // The jungle's tropical birds by day, a frog otherwise (`NPC.cs:2340-2364`).
        (Surface, Jungle) => {
            if day {
                &[
                    361, // Frog
                    671, // ScarletMacaw
                    672, // BlueMacaw
                    673, // Toucan
                    674, // YellowCockatiel
                    675, // GrayCockatiel
                ]
            } else {
                &[
                    361, // Frog
                ]
            }
        }
        (_, Jungle) => &[
            361, // Frog
        ],
        // The ordinary surface: birds, a bunny, a squirrel and butterflies by day; fireflies by
        // night (`NPC.cs:2414`,`2452-2552`). The evils and the hallow borrow it, as their own
        // critter rolls fall back to the same set in the game.
        (Surface, Forest | Corruption | Crimson | Hallow) => {
            if day {
                &[
                    74,  // Bird
                    297, // BirdBlue
                    298, // BirdRed
                    46,  // Bunny
                    299, // Squirrel
                    356, // Butterfly
                ]
            } else {
                &[
                    355, // Firefly
                ]
            }
        }
        // The underworld's friendly spawns are lava-bait critters this server does not model.
        (Underworld, _) => &[],
        // Underground and cavern: the game's own fallback is a bunny, with squirrels near the mouth
        // of a cave (`NPC.cs:2600-2623`).
        (_, _) => &[
            46,  // Bunny
            299, // Squirrel
        ],
    }
}

/// How far down the game looks for ground from a chosen point (`FindGroundTile`).
pub const GROUND_SCAN: i32 = 30;

/// Scan downward for the first solid tile, returning its row.
///
/// The game does this rather than requiring a random point to land exactly on the surface: at any
/// column there is usually one standable row in a 90-tile band, so picking blind would almost
/// never find it.
pub fn find_ground(world: &World, x: i32, from_y: i32) -> Option<i32> {
    find_ground_within(world, x, from_y, from_y + GROUND_SCAN)
}

/// The same descent, stopped at an explicit row rather than a fixed distance.
///
/// `FindSpawnTile` (`NPC.cs:990-993`) walks `for (; j < maxTilesY && j < spawnArea.Bottom && !solid;
/// j++)`, so its budget is "however far it is to the bottom of the spawn box", not a constant. From
/// the top of the box that is a full `2 * SPAWN_RANGE_Y` (94 tiles) rather than [`GROUND_SCAN`]'s
/// 30, which is the difference between reaching an ocean floor and giving up in the water above it.
fn find_ground_within(world: &World, x: i32, from_y: i32, bottom: i32) -> Option<i32> {
    (from_y..bottom.min(world.height())).find(|&y| {
        let tile = world.tile(x, y);
        tile.is_active() && solid(tile.block)
    })
}

/// Whether an NPC can stand at this tile: open space with something solid underneath.
///
/// The open-space half is `NPC.CanSpawnInTile` (`NPC.cs:5431-5442`), which rejects exactly two
/// things: an active solid tile, and lava at *any* depth. Water is explicitly allowed, which is
/// what lets a shark exist.
///
/// This used to be `tile.liquid > 200`, a test blind to which liquid it was looking at, so it had
/// both halves backwards at once: deep water was refused where the game permits it (the whole ocean
/// roster could only appear on a shoreline strip) and shallow lava was accepted where the game
/// refuses it at a single drop.
fn has_room(world: &World, x: i32, y: i32) -> bool {
    let floor = world.tile(x, y + 1);
    open_space(world, x, y) && floor.is_active() && solid(floor.block)
}

/// The open-space half alone, which is all `HasTileSpawnSpace` (`NPC.cs:5406-5413`) ever asks for.
///
/// A flier chosen in the sky has nothing under it and never will, so the floor test in
/// [`has_room`] is the one thing that must not apply to it. The game's own check is over a
/// `(x - 1, y - 3, 2, 3)` rectangle and never mentions the ground: the ground requirement is
/// implicit in the *other* branch, the one that walks down to it.
fn open_space(world: &World, x: i32, y: i32) -> bool {
    for dy in 0..3 {
        let tile = world.tile(x, y - dy);
        if tile.is_active() && solid(tile.block) {
            return false;
        }
        // `Tile.anyLava()`: the kind, not the depth. `liquid_kind` is only meaningful when there is
        // some liquid there, hence the amount test first.
        if tile.liquid > 0 && tile.liquid_kind == terrustia_proto::tile::Liquid::Lava {
            return false;
        }
    }
    true
}

/// Whether a spawn point stands in water deep enough to draw the aquatic roster.
///
/// `NPC.SetSpawnFlagsForChosenTile` (`NPC.cs:1058`): `waterTile = tile[x, y-1].liquid > 0 &&
/// tile[x, y-2].liquid > 0 && tile[x, y-1].liquidType() == 0`, where its `y` is the solid ground
/// row. Ours is one above that (the row the NPC's feet occupy), so the two tiles to test are `y`
/// and `y - 1`. Both must be wet, so a single puddle underfoot is not the sea.
fn water_tile(world: &World, x: i32, y: i32) -> bool {
    let feet = world.tile(x, y);
    feet.liquid > 0
        && world.tile(x, y - 1).liquid > 0
        && feet.liquid_kind == terrustia_proto::tile::Liquid::Water
}

/// `TileID.Sets.Platforms` (`TileID.cs:243`), which is exactly seven tiles and is *not* the same
/// set as `Main.tileSolidTop` ([`terrustia_proto::tile_solid::solid_top`], which has far more in
/// it). Only [`solid_tile2`] needs the distinction.
const PLATFORMS: [u16; 7] = [19, 427, 435, 436, 437, 438, 439];

/// `WorldGen.SolidTile2(i, j)` (`WorldGen.cs:71031-71048`), the looser of the game's two
/// "is this solid" tests:
///
/// ```csharp
/// if (Main.tile[i, j].active() && Main.tileSolid[Main.tile[i, j].type]
///     && ((TileID.Sets.Platforms[Main.tile[i, j].type]
///          && (Main.tile[i, j].halfBrick() || Main.tile[i, j].topSlope()))
///         || Main.tile[i, j].slope() == 0)
///     && !Main.tile[i, j].halfBrick() && !Main.tile[i, j].inActive())
/// ```
///
/// Where `SolidTile` throws every platform out (`!Main.tileSolidTop[...]`, and see
/// `worldgen::traps`'s copy of it), this one keeps a platform that has been hammered into a top
/// slope. The trailing `!halfBrick()` makes the `halfBrick()` inside the platform clause dead, so
/// that clause reduces to "a platform with a top slope"; `topSlope()` is `slope() == 1 || slope()
/// == 2` (`Tile.cs:292-298`), and `inActive()` is the actuated bit.
///
/// The game's own `Main.tile[i, j] == null` arm returns *true*, counting off the map as solid.
/// `World::tile` answers off the map with bare air instead, which is the opposite answer; nothing
/// reaches it, because [`try_spawn`] drops any candidate within ten tiles of an edge long before
/// this is asked.
fn solid_tile2(world: &World, x: i32, y: i32) -> bool {
    let tile = world.tile(x, y);
    tile.is_active()
        && solid(tile.block)
        && (tile.slope == 0 || (PLATFORMS.contains(&tile.block) && matches!(tile.slope, 1 | 2)))
        && !tile.flags.has(terrustia_proto::TileFlags::HALF_BRICK)
        && !tile.flags.has(terrustia_proto::TileFlags::ACTUATED)
}

/// `NPC.IsThisAGoodPlaceForAStatueMimic(x, y)` (`NPC.cs:43891-43898`): two solid tiles side by side
/// with three clear rows above both of them, which is the shape of the plinth a statue stands on.
///
/// ```csharp
/// if (WorldGen.SolidTile2(x, y) && WorldGen.SolidTile2(x + 1, y)
///     && !Main.tile[x, y - 1].active() && !Main.tile[x, y - 2].active()
///     && !Main.tile[x, y - 3].active() && !Main.tile[x + 1, y - 1].active()
///     && !Main.tile[x + 1, y - 2].active() && !Main.tile[x + 1, y - 3].active())
/// ```
///
/// It is the same function the mimic's own AI uses to pick where to reappear
/// (`AI_126_StatueMimic`, `NPC.cs:43994`), which is why it wants two columns rather than one: the
/// thing is 28 pixels wide and has to look like it was always there.
///
/// `ground_y` is the game's own `spawnTileY`, the solid row itself, which this server's spawn row
/// sits one above; the caller passes `y + 1`, the same convention as [`sandstone_check`]. The air
/// test is `active()` rather than `nactive()`, so an actuated tile still counts as being in the
/// way, which is transcribed as-is.
pub fn good_place_for_a_statue_mimic(world: &World, x: i32, ground_y: i32) -> bool {
    solid_tile2(world, x, ground_y)
        && solid_tile2(world, x + 1, ground_y)
        && (1..=3).all(|dy| {
            !world.tile(x, ground_y - dy).is_active()
                && !world.tile(x + 1, ground_y - dy).is_active()
        })
}

/// `WorldGen.SolidTile(i, j, noDoors = false)` (`WorldGen.cs:70650-70671`), the stricter of the
/// game's two "is this solid" tests:
///
/// ```csharp
/// if (Main.tile[i, j].active() && Main.tileSolid[Main.tile[i, j].type]
///     && !Main.tileSolidTop[Main.tile[i, j].type] && !Main.tile[i, j].halfBrick()
///     && Main.tile[i, j].slope() == 0 && !Main.tile[i, j].inActive())
/// ```
///
/// Where [`solid_tile2`] keeps a platform that has been hammered into a top slope, this one throws
/// every platform out outright, and it refuses any slope rather than only a non-top one. `noDoors`
/// is false at the one call site here ([`check_to_spawn_rock_golem`]), so the door carve-out is not
/// transcribed. `worldgen::smooth` has its own copy of the same function for its own pass; this is
/// a second one rather than a shared helper because that one is `pub(super)` inside `worldgen`.
///
/// The game's `Main.tile[i, j] == null` arm returns *true*, counting off the map as solid;
/// `World::tile` answers off the map with bare air, the opposite. Nothing reaches it, for the same
/// reason [`solid_tile2`] does not: [`try_spawn`] drops any candidate within ten tiles of an edge.
fn solid_tile(world: &World, x: i32, y: i32) -> bool {
    let tile = world.tile(x, y);
    tile.is_active()
        && solid(tile.block)
        && !terrustia_proto::tile_solid::solid_top(tile.block)
        && !tile.flags.has(terrustia_proto::TileFlags::HALF_BRICK)
        && tile.slope == 0
        && !tile.flags.has(terrustia_proto::TileFlags::ACTUATED)
}

/// `NPC.CheckToSpawnRockGolem(spawnTileX, spawnTileY, spawnTileType)` (`NPC.cs:5803-5818`):
///
/// ```csharp
/// if (!Main.hardMode || (spawnTileType != 1 && !TileID.Sets.Conversion.Moss[spawnTileType]) || ZoneSnow)
///     return false;
/// if (Main.rand.Next(50) != 0)
///     return false;
/// if (WorldGen.SolidTile(spawnTileX - 1, spawnTileY - 4) || WorldGen.SolidTile(spawnTileX, spawnTileY - 4) || WorldGen.SolidTile(spawnTileX + 1, spawnTileY - 4))
///     return false;
/// return true;
/// ```
///
/// Four clauses and no state: hardmode, plain stone or any moss underfoot, not in the snow, one
/// attempt in fifty, and three tiles of clear ceiling four rows up so there is somewhere for a
/// thing this tall to stand. `TileID.Sets.Conversion.Moss` (`TileID.cs:38`) is
/// [`terrustia_proto::tile_sets::is_moss`], which already holds all eleven of its entries.
///
/// The clause order is the game's own, which is also cheapest-first: three comparisons, then the
/// roll, and only then the three tile reads.
///
/// `ground_y` is the game's `spawnTileY`, the solid row itself, which this server's spawn row sits
/// one above; the caller passes `y + 1`, the same convention as [`good_place_for_a_statue_mimic`]
/// and [`sandstone_check`].
fn check_to_spawn_rock_golem(
    world: &World,
    x: i32,
    ground_y: i32,
    ground_block: u16,
    hard_mode: bool,
    snow: bool,
    rng: &mut SmallRng,
) -> bool {
    if !hard_mode
        || (ground_block != STONE && !terrustia_proto::tile_sets::is_moss(ground_block))
        || snow
    {
        return false;
    }
    if rng.random_range(0..ROCK_GOLEM_ODDS) != 0 {
        return false;
    }
    !(-1..=1).any(|dx| solid_tile(world, x + dx, ground_y - ROCK_GOLEM_HEADROOM))
}

/// What a tile of water draws instead of the land pool.
///
/// Vanilla keeps the water rosters in their own `waterTile` branches ahead of every land branch
/// (`NPC.cs:1766-2000`), which is why a shark is never on the sand and a zombie is never in the sea.
/// Two of those branches are transcribed here, being the two whose types this server already
/// fields:
///
/// * the ocean, `waterTile && isOcean` (`NPC.cs:1798-1920`): Shark, Squid, Crab, Pink Jellyfish;
/// * water below the surface line, `waterTile && spawnTileY > worldSurface` (`NPC.cs:1988-1997`):
///   Blue Jellyfish.
///
/// Deliberately not transcribed, because each would need an NPC this server has no AI for: the
/// hardmode jungle's Arapaima (157), the hardmode crimson's water pair, the Piranha/Angler Fish
/// branch at `NPC.cs:1932`, the corrupt and crimson goldfish at `:1999`, and the ocean's own Sea
/// Snail (220) and Orca (692). An empty slice here means "no water roster for this place", and the
/// caller falls back to the land pool rather than inventing one.
pub fn water_pool(depth: Depth, biome: Biome) -> &'static [u16] {
    match (depth, biome) {
        (_, Biome::Ocean) => &[
            65,  // Shark
            221, // Squid
            67,  // Crab
            64,  // PinkJellyfish
        ],
        // `spawnTileY > Main.worldSurface`: anything below the surface line, which is every band
        // this enum has except the surface itself.
        (Depth::Surface, _) => &[],
        (_, _) => &[
            63, // BlueJellyfish
        ],
    }
}

/// The walls an underground desert is made of, `WallID.Sets.AllowsUndergroundDesertEnemiesToSpawn`
/// (`WallID.cs:42`): plain sandstone and hardened sand, each of their three converted forms, and
/// desert fossil.
///
/// This set, not a biome scan, is what the game's whole underground-desert roster hangs off
/// (`NPC.cs:1682`). That matters twice over. It is exact where a tile-count zone is a guess, and it
/// costs two tile reads where [`biome_at`] costs twenty thousand, so the branch it gates can be
/// tested on every candidate tile rather than once per player per tick.
const DESERT_SPAWN_WALLS: [u16; 9] = [
    187, // Sandstone
    216, // HardenedSand
    217, // CorruptHardenedSand
    218, // CrimsonHardenedSand
    219, // HallowHardenedSand
    220, // CorruptSandstone
    221, // CrimsonSandstone
    222, // HallowSandstone
    223, // DesertFossil
];

/// `WorldGen.checkUnderground` (`WorldGen.cs:10099-10144`), the other half of the underground
/// desert's gate.
///
/// Deep enough down it is simply true, high enough up simply false, and in the band between it asks
/// whether the roof is closed: a 120-by-3 strip 80 tiles above the point has to be at least 80%
/// solid tile, or the point itself has to be walled. Three of those four answers are a handful of
/// reads; only the fourth walks the strip, and see the hoist below for why the caller's own use
/// almost never reaches it.
///
/// The game wraps the whole thing in a bare `catch { return false; }` for its own out-of-bounds
/// tile access. `World::tile` answers for anything off the map already, so there is nothing here to
/// catch and no behaviour dropped by not catching it.
fn check_underground(world: &World, x: i32, y: i32) -> bool {
    /// `num`, `num2` and `num3` in the game's own order: the strip's width, how far above the point
    /// it sits, and how many rows of it are counted.
    const WIDTH: i32 = 120;
    const ABOVE: i32 = 80;
    const ROWS: i32 = 3;

    let surface = f64::from(world.surface);
    if f64::from(y) > surface + f64::from(ABOVE) {
        return true;
    }
    if f64::from(y) < surface / 2.0 {
        return false;
    }
    // The game's own test is `SolidTile(i, j) || Main.tile[x, y].wall > 0`, and its second half
    // reads the *point* rather than the strip: it does not depend on either loop variable, so when
    // it holds every one of the 360 cells counts and the answer is already true. Hoisting it is the
    // same answer, exactly, and it is what keeps this cheap where it is actually reached: the
    // caller only asks after finding a desert wall, and a desert wall is a wall.
    if world.tile(x, y).wall > 0 {
        return true;
    }
    let top = y - ABOVE;
    let left = (x - WIDTH / 2).clamp(0, (world.width() - WIDTH - 1).max(0));
    let mut closed = 0;
    for i in left..left + WIDTH {
        for j in top..top + ROWS {
            let tile = world.tile(i, j);
            if tile.is_active() && solid(tile.block) {
                closed += 1;
            }
        }
    }
    f64::from(closed) >= f64::from(WIDTH * ROWS) * 0.8
}

/// `WallID.SpiderUnsafe`, the wall a spider nest is lined with and the only thing that puts one on
/// the spawn path (`NPC.cs:1662`).
pub const SPIDER_WALL: u16 = 62;

/// The spider nest's roster, `NPC.cs:1662-1680`:
///
/// ```csharp
/// else if ((Main.tile[spawnTileX, spawnTileY].wall == 62 || spawnSpider) && CheckToSpawnSpider(...))
/// {
///     ...
///     else if (Main.hardMode && Main.rand.Next(10) != 0) 163;
///     else                                               164;
/// }
/// ```
///
/// The Wall Creeper and the Black Recluse have no other ambient spawn in the game at all: 163 and
/// 164 are the only two ids `NPC.Spawner` reaches from this arm, so with no arm the Wall Creeper was
/// unreachable outright and the Black Recluse was only reachable because [`hardmode_pool`] carried
/// it in the generic cavern list, which put recluses in every hardmode cave rather than in nests.
///
/// `CheckToSpawnSpider` (`NPC.cs:5790-5801`) is `true` outside the "not the bees" for-the-worthy
/// seed, so it is not transcribed.
///
/// Two things left to the caller, both disclosed:
///
/// * `|| spawnSpider` (`NPC.cs:1145-1176`), which widens the arm to spots merely *near* a spider
///   wall by sweeping a box of radius 5 to 15 one attempt in three. Left out for the same reason
///   `underground_desert_spot` leaves out its twin: it is up to 900 tile reads on the per-candidate
///   path to reach spots the wall test itself misses anyway, and a real nest is walled throughout.
/// * `NPC.cs:1669`, the Stylist at one dry nest tile in eight below the rock layer. She already has
///   a path here, [`bound_gate`], which reaches her through the same rescue table as every other
///   bound resident; giving her a second one would mean two mechanisms racing for the same
///   townsperson. The effect of skipping the branch is that 163 and 164 take that eighth as well,
///   in the window before she is rescued.
fn spider_pick(hard_mode: bool, rng: &mut SmallRng) -> u16 {
    if hard_mode && !rng.random_ratio(1, 10) {
        163 // BlackRecluse
    } else {
        164 // WallCreeper
    }
}

/// Whether a candidate spot is in the underground desert, `NPC.cs:1682`:
///
/// ```csharp
/// else if ((SpawnTileOrAboveHasAnyWallInSet(spawnTileX, spawnTileY,
///               WallID.Sets.AllowsUndergroundDesertEnemiesToSpawn) || spawnUndergroundDesert)
///          && WorldGen.checkUnderground(spawnTileX, spawnTileY))
/// ```
///
/// `y` is this server's spawn row, the one the NPC's feet occupy, so the game's `spawnTileY` (the
/// solid ground tile) is `y + 1` and its "or above" tile is `y` - the same one-row offset
/// [`water_tile`] documents. `SpawnTileOrAboveHasAnyWallInSet` (`NPC.cs:5535-5556`) is exactly those
/// two rows and nothing else; its `InWorld(x, y, 2)` guard needs no counterpart, because
/// `World::tile` already answers off the map with bare air, whose wall is 0 and so is in no set.
///
/// The `|| spawnUndergroundDesert` half is the disclosed narrowing. That flag (`NPC.cs:1178-1201`)
/// widens the branch to spots merely *near* desert walls above the rock layer: one attempt in three
/// sweeps a box of radius 5 to 15 around the chosen tile, and the other two read the wall at the
/// player's own tile. Left out on purpose, because the box is up to 900 tile reads on the
/// per-candidate path (20 candidates per player per tick) to reach spots the wall test misses
/// anyway, and a real underground desert is walled throughout. The effect is that the roster starts
/// a little further inside the biome than the game's does, never that it appears outside it.
pub fn underground_desert_spot(world: &World, x: i32, y: i32) -> bool {
    let walled = |row: i32| DESERT_SPAWN_WALLS.contains(&world.tile(x, row).wall);
    (walled(y + 1) || walled(y)) && check_underground(world, x, y + 1)
}

/// The underground desert's roster, `NPC.cs:1684-1764`.
///
/// The whole 1.4 desert lives here and nowhere else, which is why twelve types were unreachable:
/// the four ghouls, the two lamias, the scorpion, the beast, the djinn, the tomb crawler and both
/// giant antlions have no other ambient spawn in the game at all.
///
/// `spawn_y` is this server's spawn row; the game's `spawnTileY` is one below it, and every depth
/// test here is written against the game's own row. Two things the caller owns rather than this:
/// the Golfer at one attempt in twenty (`NPC.cs:1693-1697`), who arrives through [`bound_gate`]
/// like every other bound resident, and the branch's position in the chain.
///
/// `biome` stands in for the game's three independent `ZoneCorrupt`/`ZoneCrimson`/`ZoneHallow`
/// flags, which `SetSpawnFlags` copies straight off the player (`NPC.cs:381-383`). It is the
/// *player's* zone that picks the ghoul, not the tile under the spawn, so no per-spot scan is
/// needed or wanted. The narrowing is that [`Biome`] names one winner where the game can have two
/// set at once; with one set the two agree exactly, and the game's own "no evil, no hallow" fallback
/// to the plain ghoul is this function's `_` arm.
pub fn underground_desert_pick(
    world: &World,
    spawn_y: i32,
    hard_mode: bool,
    biome: Biome,
    no_worms: bool,
    alive: &dyn Fn(u16) -> usize,
    rng: &mut SmallRng,
) -> u16 {
    // `num10` (`NPC.cs:1684-1692`), which thins the two worms out with depth.
    let ground_y = f64::from(spawn_y + 1);
    let rock = f64::from(world.rock_layer);
    let mut scale = 1.3f32;
    if ground_y > (rock * 2.0 + f64::from(world.height())) / 3.0 {
        scale *= 0.5;
    } else if ground_y > rock {
        scale *= 0.85;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let worm_odds = (50.0 * scale) as u32;
    let worm_roll = |rng: &mut SmallRng| rng.random_range(0..worm_odds.max(1)) == 0;
    let deep = ground_y > f64::from(world.surface) + 100.0;

    // The Dune Splicer, hardmode only (`NPC.cs:1698-1702`).
    if hard_mode && worm_roll(rng) && !no_worms && deep {
        return 510;
    }
    // The Tomb Crawler, at any progression, but only ever one at a time (`NPC.cs:1703-1707`).
    if worm_roll(rng) && !no_worms && deep && alive(513) == 0 {
        return 513;
    }
    // Hardmode's own roster, four attempts in five (`NPC.cs:1708-1746`). The game builds a weighted
    // list and picks uniformly from it, so the duplicated ghoul really is twice as likely as the
    // rest; the lists below are that `List<int>` verbatim.
    if hard_mode && rng.random_range(0..5) != 0 {
        let list: &[u16] = match biome {
            // 525 x2, then the `ZoneCorrupt || ZoneCrimson` pair, then the beast.
            Biome::Corruption => &[525, 525, 533, 529, 532],
            Biome::Crimson => &[526, 526, 533, 529, 532],
            // The hallow takes the *other* pair, since it is neither evil.
            Biome::Hallow => &[527, 527, 530, 528, 532],
            _ => &[524, 524, 530, 528, 532],
        };
        return list[rng.random_range(0..list.len())];
    }
    // ...and the antlions, which are the desert whatever the progression (`NPC.cs:1747-1764`).
    let mut ty = [69, 580, 580, 580, 581][rng.random_range(0..5)];
    if rng.random_range(0..15) == 0 {
        ty = 537; // SandSlime, which replaces whatever was drawn rather than joining the draw.
    } else if rng.random_range(0..10) == 0 {
        // The giants are not a hardmode upgrade: this roll is on the plain path, so a fresh world's
        // underground desert already has them.
        ty = match ty {
            580 => 508, // GiantWalkingAntlion
            581 => 509, // GiantFlyingAntlion
            other => other,
        };
    }
    ty
}

/// `TileID.Sets.Conversion.Sand` (`TileID.cs:30`): sand and its three converted forms.
const SAND_CONVERSION: [u16; 4] = [
    53,  // Sand
    112, // Ebonsand
    116, // Pearlsand
    234, // Crimsand
];

/// `NPC.Spawner.Spawning_SandstoneCheck` (`NPC.cs:5464-5503`): enough sand under the spot to call it
/// a desert rather than a beach.
///
/// Walks up to eight rows down from the ground tile, and in each row up to four tiles right and four
/// left, stopping the moment a row (or a run within it) leaves sand. At least 40 of the possible 72
/// have to be sand. Every loop breaks early, so ordinary rock costs one read. The game's
/// `InWorld(x, y, 10)` guard needs no counterpart for the same reason [`underground_desert_spot`]'s
/// does not: off the map reads as bare air, which is not sand, which breaks the walk.
pub fn sandstone_check(world: &World, x: i32, ground_y: i32) -> bool {
    let sand = |x: i32, y: i32| {
        let tile = world.tile(x, y);
        tile.is_active() && SAND_CONVERSION.contains(&tile.block)
    };
    let mut count = 0;
    for i in 0..8 {
        if !sand(x, ground_y + i) {
            break;
        }
        count += 1;
        for j in 1..=4 {
            if !sand(x + j, ground_y + i) {
                break;
            }
            count += 1;
        }
        for k in 1..=4 {
            if !sand(x - k, ground_y + i) {
                break;
            }
            count += 1;
        }
    }
    count >= 40
}

/// The desert during a sandstorm, `NPC.cs:3952-4022`.
///
/// The gate is `Sandstorm.Happening && ZoneSandstorm && TileID.Sets.Conversion.Sand[tileType] &&
/// Spawning_SandstoneCheck(spawnTileX, spawnTileY)`, and the caller owns the first two: a sandstorm
/// is world weather (`game/weather.rs` already runs one) and `ZoneSandstorm` is
/// `ZoneDesert && SurfaceAtmospherics && Sandstorm.Happening` (`SceneMetrics.cs:706`), which is a
/// property of where the *player* stands. This owns the two per-spot halves and the roster.
///
/// Returns the type and how many tiles below the spawn row to put it: the Dune Splicer alone arrives
/// ten tiles down (`NPC.cs:3975`, `(spawnTileY + 10) * 16`), because it burrows in rather than
/// standing on the sand.
///
/// `ground_block` is the tile the spawn stands on, the game's `tileType`. It decides the sandshark's
/// flavour, and that one *is* per-spot rather than per-player: corrupt sand gives a corrupt
/// sandshark wherever it is, which is the opposite of how the ghouls above resolve their evil.
pub fn sandstorm_pick(
    hard_mode: bool,
    downed_boss1: bool,
    no_worms: bool,
    ground_block: u16,
    alive: &dyn Fn(u16) -> usize,
    rng: &mut SmallRng,
) -> (u16, i32) {
    // Before the Eye of Cthulhu, a sandstorm is tumbleweeds, vultures and antlions
    // (`NPC.cs:3954-3968`).
    if !downed_boss1 && !hard_mode {
        if rng.random_range(0..2) == 0 {
            return (546, 0); // Tumbleweed
        }
        if rng.random_range(0..2) == 0 {
            return (61, 0); // Vulture
        }
        return (69, 0); // Antlion
    }
    // The Sand Elemental, one at a time (`NPC.cs:3969-3972`).
    if hard_mode && rng.random_range(0..20) == 0 && alive(541) == 0 {
        return (541, 0);
    }
    // Dune Splicers, up to four (`NPC.cs:3973-3976`).
    if hard_mode && !no_worms && rng.random_range(0..3) == 0 && alive(510) < 4 {
        return (510, 10);
    }
    // Sandsharks, flavoured by the sand they swim through (`NPC.cs:3977-3992`). The game tests all
    // three sets in sequence rather than as an `else if` chain, so the last match wins; no tile is
    // in two of them, so the order is not observable.
    if hard_mode && !no_worms && rng.random_range(0..2) == 0 {
        // `TileID.Sets.Corrupt` / `Crimson` / `Hallow` (`TileID.cs:333`, `:351`, `:341`), narrowed
        // to the sand members, since `ground_block` has already passed [`SAND_CONVERSION`].
        return (
            match ground_block {
                112 => 543, // Ebonsand: SandsharkCorrupt
                234 => 544, // Crimsand: SandsharkCrimson
                116 => 545, // Pearlsand: SandsharkHallow
                _ => 542,   // SandShark
            },
            0,
        );
    }
    // A mummy for each sand (`NPC.cs:3994-4009`), which the game writes as four `else if` arms with
    // a roll each. Only one tile type can match, so folding them into a lookup keeps both the
    // one-in-three odds and the single roll.
    let mummy = match ground_block {
        53 => Some(78),   // Mummy
        112 => Some(79),  // DarkMummy
        234 => Some(630), // BloodMummy
        116 => Some(80),  // LightMummy
        _ => None,
    };
    if hard_mode
        && let Some(mummy) = mummy
        && rng.random_range(0..3) == 0
    {
        return (mummy, 0);
    }
    // The tail of the chain, which is what an ordinary hardmode sandstorm mostly draws
    // (`NPC.cs:4010-4021`).
    if rng.random_range(0..2) == 0 {
        return (546, 0); // Tumbleweed
    }
    if rng.random_range(0..2) == 0 {
        return (580, 0); // WalkingAntlion
    }
    (581, 0) // FlyingAntlion
}

/// The dungeon's slab walls, `WallID.BlueDungeonSlabUnsafe`, `PinkDungeonSlabUnsafe` and
/// `GreenDungeonSlabUnsafe` (`WallID.cs:257`, `:261`, `:265`).
const DUNGEON_SLAB_WALLS: [u16; 3] = [94, 96, 98];

/// ...and its tile walls, `BlueDungeonTileUnsafe`, `PinkDungeonTileUnsafe` and
/// `GreenDungeonTileUnsafe` (`WallID.cs:259`, `:263`, `:267`).
const DUNGEON_TILE_WALLS: [u16; 3] = [95, 97, 99];

/// `RollLuck(7) == 0` (`NPC.cs:2642-2645`): one attempt in seven ignores the wall it found and takes
/// a style at random, which is what keeps a single-style room from being a single-enemy room.
///
/// `RollLuck` is `Luck.RollLuck(luck, range)` (`NPC.cs:5356-5359`), and with no luck effects that is
/// `Main.rand.Next(range)`. This server models no luck at all, the same narrowing (and the same
/// reasoning) as the `rate * 0.85` bonus [`rates`] declines to transcribe.
const DUNGEON_STYLE_REROLL: u32 = 7;

/// Which of the dungeon's three brick styles a spot is built from: `num40` (`NPC.cs:2631-2645`).
///
/// This is *not* the wall's colour, whatever the ids look like at a glance. 94, 96 and 98 are the
/// blue, pink and green **slab** walls and 95, 97 and 99 the blue, pink and green **tile** walls
/// (`WallID.cs:257-267`), so the game sorts a spot by the shape of its masonry and not by its
/// colour: a blue slab corridor and a green slab corridor offer exactly the same enemies, and the
/// plain brick walls (7, 8 and 9) and everything else fall through to 0.
///
/// Tile is checked after slab and so wins when a spot has one of each within its two rows, which is
/// the game's own order rather than a preference of ours.
///
/// `y` is this server's spawn row, the one the NPC's feet occupy, so the game's `spawnTileY` is
/// `y + 1` and its "or above" tile is `y`: the same one-row offset [`underground_desert_spot`]
/// documents. Two tile reads, on a path that already reads several, and only where the caller has
/// already decided it is standing in a hardmode dungeon.
fn dungeon_brick_style(world: &World, x: i32, y: i32, rng: &mut SmallRng) -> u8 {
    let mut style = 0;
    let has = |walls: &[u16]| {
        walls.contains(&world.tile(x, y + 1).wall) || walls.contains(&world.tile(x, y).wall)
    };
    if has(&DUNGEON_SLAB_WALLS) {
        style = 1;
    }
    if has(&DUNGEON_TILE_WALLS) {
        style = 2;
    }
    if rng.random_range(0..DUNGEON_STYLE_REROLL) == 0 {
        style = rng.random_range(0..3u8);
    }
    style
}

/// The hardmode dungeon's own chain, `NPC.cs:2661-2722`.
///
/// `hardDungeon` is `downedPlantBoss && Main.hardMode` (`NPC.cs:381`), which the caller holds along
/// with the zone. Every arm here returns from the game's spawn attempt the moment it fires, so this
/// is a sequence of independent rolls and not a pool: folding it into [`pool`] would hand the
/// Paladin a share of every dungeon draw instead of one attempt in thirty-five, and would lose the
/// brick style entirely, since three quarters of the roster is chosen by it.
///
/// `None` means no arm answered and the caller should fall through to the ordinary dungeon pool, the
/// way vanilla falls through to `:2723`. `Some(None)` is the one arm that answers with nothing: the
/// caster at `:2691` returns whether or not its `!AnyNPCs` test lets it spawn (`:2703-2707`), so
/// while one is alive a twentieth of the dungeon's attempts produce no NPC at all rather than an
/// Angry Bones.
fn hard_dungeon_pick(
    style: u8,
    alive: &dyn Fn(u16) -> bool,
    rng: &mut SmallRng,
) -> Option<Option<u16>> {
    // `:2661`, the one arm that does not care which brick it is standing on.
    if rng.random_range(0..30) == 0 {
        return Some(Some(287)); // BoneLee
    }
    // `:2666-2680`. Three separate `if`s in the game, each testing its own `num40` before rolling,
    // and `num40` is exactly one of the three: one roll happens, and it is this one.
    let gunner = match style {
        0 => 293, // SkeletonCommando
        1 => 291, // SkeletonSniper
        _ => 292, // TacticalSkeleton
    };
    if rng.random_range(0..15) == 0 {
        return Some(Some(gunner));
    }
    // `:2681`, plain brick only, and never a second one. The game's own operand order: the census
    // test comes before the style and before the roll.
    if !alive(290) && style == 0 && rng.random_range(0..35) == 0 {
        return Some(Some(290)); // Paladin
    }
    // `:2686`, slab and tile only.
    if style != 0 && rng.random_range(0..30) == 0 {
        return Some(Some(289)); // GiantCursedSkull
    }
    // `:2691-2708`, `num41`: 281 for slab, +2 for brick, +4 for tile, then one of the pair. The
    // whole arm returns even when the census turns it away, which is what `Some(None)` carries.
    if rng.random_range(0..20) == 0 {
        let base = match style {
            0 => 283, // Necromancer, NecromancerArmored
            1 => 281, // RaggedCaster, RaggedCasterOpenCoat
            _ => 285, // DiabolistRed, DiabolistWhite
        };
        let caster = base + rng.random_range(0..2u16);
        return Some(if alive(caster) { None } else { Some(caster) });
    }
    // `:2709-2722`, `num42`: 269 for slab, +4 for brick, +8 for tile, then one of the four. Two
    // attempts in three, which makes the armoured bones most of what a post-Plantera dungeon is.
    if rng.random_range(0..3) != 0 {
        let base = match style {
            0 => 273, // BlueArmoredBones and its three variants
            1 => 269, // RustyArmoredBones and its three variants
            _ => 277, // HellArmoredBones and its three variants
        };
        let bones = base + rng.random_range(0..4u16);
        return Some(Some(bones));
    }
    None
}

/// `NPC.AI_FindNearbyBook(searchPosition, 32, 32, out bookPosition, closestBook: true,
/// checkPlayerScreenRanges: true)` (`NPC.cs:62954-63010`), the one shape the spawner ever asks for.
///
/// ```csharp
/// for (int i = num5; i < num6; i++)
///   for (int j = num3; j < num4; j++) {
///     Tile tile = Main.tile[j, i];
///     if (!tile.active() || tile.type != 50) continue;
///     Vector2 vector3 = new Vector2(j, i);
///     if (checkPlayerScreenRanges && !Spawner.CheckNotSpawningOnScreen((int)vector3.X, (int)vector3.Y))
///         continue;
///     float num8 = vector3.Distance(vector2);
///     if (closestBook && num8 < num7) { num7 = num8; vector = vector3; continue; }
///     ...
///   }
/// ```
///
/// Two things about it read as bugs and are the game's, so both are kept:
///
/// * the distance is measured from `vector2`, which is `searchPosition` itself, the box's **top
///   left corner** and not the candidate tile in the middle of it. The shelf that wins is therefore
///   the one nearest that corner, which is up and to the left of where the attempt was made.
/// * "found nothing" and "found a book on the corner tile" are the same answer. `vector` starts as
///   the anchor and the tail (`:62995-63003`) returns false while the winner still equals it, so a
///   book sitting exactly on the corner is reported as no book at all.
///
/// `from` is that corner in tile coordinates and `player` the tile the attempt's own player stands
/// on. Two halves of the game's function are deliberately not transcribed, and neither can change
/// what this returns:
///
/// * the `closestBook: false` path, which fills a twenty-slot buffer and picks from it at random.
///   The spawner never asks for it. Its one caller is the Librarian's own spell placement
///   (`NPC.cs:21323-21337`, a 20-by-30 box), which is AI rather than spawning.
/// * that buffer's `if (num2 >= num) break` (`:62989-62992`), which cuts a row short once twenty
///   books that were *not* closer have gone into it. It cannot lose the winner: inside one row `j`
///   only increases and so does the distance from the corner, so the first book in a row that fails
///   to beat the running best is followed only by books that also fail. (The same monotonicity is
///   why the game gets away with it. On this path the buffer is written and never read, and a
///   twenty-first non-closest book would index one past the end of `_nearbyBooks`, which is
///   `new Point[20]` at `NPC.cs:6608`.)
///
/// `checkPlayerScreenRanges` is [`SAFE_RANGE_X`]/[`SAFE_RANGE_Y`] here, the same box [`try_spawn`]
/// already keeps its candidate tiles out of. Vanilla's `CheckNotSpawningOnScreen`
/// (`NPC.cs:5444-5462`) is a screen plus a safe range around *every* active player, which works out
/// at roughly 64 by 36 tiles against this server's 62 by 35 around the one player the attempt
/// belongs to. That is the narrowing the candidate test already carries, reused rather than widened
/// for one branch.
pub fn find_nearby_book(world: &World, from: (i32, i32), player: (i32, i32)) -> Option<(i32, i32)> {
    // `num3`..`num6`, the box clipped to the map.
    let left = from.0.max(0);
    let right = (from.0 + BOOK_SEARCH_SPAN).min(world.width());
    let top = from.1.max(0);
    let bottom = (from.1 + BOOK_SEARCH_SPAN).min(world.height());
    // `num7`, the running best, and `vector`, which starts as the anchor `vector2` itself.
    let mut best = i32::MAX;
    let mut found = from;
    for y in top..bottom {
        for x in left..right {
            let tile = world.tile(x, y);
            if !tile.is_active() || tile.block != BOOK_TILE {
                continue;
            }
            if (x - player.0).abs() < SAFE_RANGE_X && (y - player.1).abs() < SAFE_RANGE_Y {
                continue;
            }
            // Squared, which orders identically to the game's own `Vector2.Distance` and costs no
            // square root. The box is 32 tiles across, so nothing here can overflow.
            let (dx, dy) = (x - from.0, y - from.1);
            let distance = dx * dx + dy * dy;
            if distance < best {
                best = distance;
                found = (x, y);
            }
        }
    }
    (found != from).then_some(found)
}

/// Mushroom grass, `TileID.MushroomGrass` (`TileID.cs:577`).
///
/// This is the whole gate on the Glowing Mushroom roster, and it is worth being blunt about, because
/// it is the opposite of what the zone flag next to it suggests: `ZoneGlowshroom` has nothing to do
/// with these three branches. Vanilla asks only what the spawn is *standing on*
/// (`NPC.cs:3633`, `:3637`, `:3674`), so a single patch of mushroom grass grown on mud has the
/// Truffle Worm and the mushroom zombies whether or not there is a biome's worth of it around.
pub const MUSHROOM_GRASS: u16 = 70;
/// A glowing mushroom block, `TileID.MushroomBlock` (`TileID.cs:817`).
///
/// Only the Spore pair reads it, and only alongside the zone (`NPC.cs:5110`, `:5209`).
pub const MUSHROOM_BLOCK: u16 = 190;

/// The Glowing Mushroom roster, `NPC.cs:3637-3702`: the two arms that hang off mushroom grass.
///
/// Not a pool, for the same reason [`seasonal_night_pick`] is not: vanilla writes each arm as a
/// sequence of independent rolls that returns the moment one hits, and the arm as a whole is behind
/// a `Main.rand.Next(3) != 0` that lets one attempt in three fall through to the ordinary chain
/// below. `None` is that fallthrough.
///
/// `surface` is the game's `(double)spawnTileY <= Main.worldSurface`, which splits the two arms: the
/// surface one is open at any point in a world's life, the underground one is hardmode only and is
/// where the Truffle Worm lives. Taken from [`Depth`] here, so the boundary sits one tile out from
/// vanilla's (this server's spawn row is one above the ground tile the game measures); the same
/// one-row offset every other depth test in this module carries.
///
/// Three parts of the game's own conditions are deliberately not written out:
///
/// * `Main.tile[spawnTileX, spawnTileY].type == tileType`, on both `:3645` and `:3684`. It asks
///   whether `FindGroundTile` (`NPC.cs:5879-5897`) had to walk further down to find solid ground
///   than the spawn descent already had. Here the ground tile *is* the first solid tile below the
///   candidate by construction, so it is always true.
/// * `!Main.hardMode && Main.rand.Next(4) == 0` at `:3680`, inside an arm whose own gate already
///   requires hardmode. The left half is false, so its roll is never made and the one in eight
///   beside it is the whole of that line.
/// * `!Main.remixWorld || Main.getGoodWorld || spawnTileY < Main.maxTilesY - 360` at `:3674`. This
///   server generates neither seed, so the first disjunct is true and the line cannot fail.
///
/// Deliberately left out with its place in the order kept, so nothing here is commoner than the
/// game's: `NPC.cs:3633`, the Fungo Fish, whose arm needs `waterTile`. The caller answers standing
/// water from [`water_pool`] before it ever reaches this, and that arm is missing vanilla's own
/// `Main.rand.Next(3) == 0` gate (`NPC.cs:1988`), so nothing wet gets this far. 256 stays disclosed
/// until the water chain is transcribed properly.
pub fn mushroom_pick(surface: bool, hard_mode: bool, rng: &mut SmallRng) -> Option<u16> {
    let one_in = |rng: &mut SmallRng, n: u32| rng.random_ratio(1, n);

    if surface {
        // NPC.cs:3637. `Main.rand.Next(3) != 0`, so one attempt in three declines the whole arm.
        if one_in(rng, 3) {
            return None;
        }
        // NPC.cs:3639. Before hardmode the snail has two chances at it, after it only the one.
        if (!hard_mode && one_in(rng, 6)) || one_in(rng, 12) {
            return Some(360); // GlowingSnail
        }
        // NPC.cs:3643-3664, the critter third of the arm.
        if one_in(rng, 3) {
            if one_in(rng, 4) {
                // NPC.cs:3645-3655.
                return Some(if hard_mode && !one_in(rng, 3) {
                    260 // GiantFungiBulb
                } else {
                    259 // FungiBulb
                });
            }
            return Some(if one_in(rng, 2) {
                257 // AnomuraFungus
            } else {
                258 // MushiLadybug
            });
        }
        // NPC.cs:3665-3672, the rest: the mushroom zombies.
        return Some(if one_in(rng, 2) {
            254 // ZombieMushroom
        } else {
            255 // ZombieMushroomHat
        });
    }

    // NPC.cs:3674, the underground arm. Hardmode is part of its own gate, and is checked before the
    // roll, so a pre-hardmode mushroom cave falls straight through to the ordinary underground
    // chain rather than losing an attempt in three to a branch that cannot answer.
    if !hard_mode || one_in(rng, 3) {
        return None;
    }
    // NPC.cs:3676, the Truffle Worm, and the only thing in the game that summons Duke Fishron.
    // `RollLuck(5)` is a plain one in five at luck zero (`Luck.cs:5-16`), which is where every
    // player on this server sits.
    if one_in(rng, 5) {
        return Some(374); // TruffleWorm
    }
    // NPC.cs:3680.
    if one_in(rng, 8) {
        return Some(360); // GlowingSnail
    }
    // NPC.cs:3684-3694.
    if one_in(rng, 4) {
        return Some(if !one_in(rng, 3) {
            260 // GiantFungiBulb
        } else {
            259 // FungiBulb
        });
    }
    // NPC.cs:3695-3702.
    Some(if one_in(rng, 2) {
        257 // AnomuraFungus
    } else {
        258 // MushiLadybug
    })
}

/// The Meteor Head (23), which is every single thing a meteor crater spawns.
///
/// `NPC.cs:2796-2799` is the whole branch: `else if (ZoneMeteor)` with one unconditional
/// `SpawnNPC(..., 23)` inside it, sitting between the dungeon's arm and the fallthrough that holds
/// every biome, every season and every tile type. Standing in a crater there is nothing else.
pub const METEOR_HEAD: u16 = 23;

/// The Harpy (48), which is the whole reason the sky is a place.
pub const HARPY: u16 = 48;
/// The Wyvern's head (87). Its fourteen trailing segments grow from its own first AI tick, the way
/// the Solar Crawltipede's do (`NPC.cs:51700-51730`).
pub const WYVERN_HEAD: u16 = 87;
/// The Martian Probe (399), which is the only thing in the game that starts Martian Madness.
pub const MARTIAN_PROBE: u16 = 399;

/// Whether a candidate tile is high enough to be *sky*, in which case nothing walks down to ground.
///
/// `FindSpawnTile` (`NPC.cs:979-986`), the two branches that set `skyMob`:
///
/// ```csharp
/// if (!invaders && (double)j < Main.worldSurface * 0.3499999940395355 && !spawnFriendly
///     && ((double)num < (double)Main.maxTilesX * 0.45 || (double)num > (double)Main.maxTilesX * 0.55
///         || Main.hardMode))
///     skyMob = true;
/// else if (!invaders && (double)j < Main.worldSurface * 0.44999998807907104 && !spawnFriendly
///     && Main.hardMode && Main.rand.Next(10) == 0)
///     skyMob = true;
/// else { for (; j < Main.maxTilesY && j < spawnArea.Bottom && !solid; j++) {} ... }
/// ```
///
/// So the sky is decided from the *spawn tile*, not from where the player stands: anyone whose
/// spawn box reaches above `worldSurface * 0.35` draws it, which is why harpies find you on a
/// mountain. Pre-hardmode the middle tenth of the map is excluded (`0.45 <= x/width <= 0.55`),
/// which is what keeps them off a fresh spawn point; hardmode drops that exclusion and adds a
/// second, lower band down to `worldSurface * 0.45` at one attempt in ten.
///
/// `!invaders` needs no test here: this server never reaches `try_spawn` while an invasion is
/// running (the invasion path returns first, `systems.rs`), which is the same answer.
/// `!spawnFriendly` is the caller's, since it owns that roll.
fn sky_tile(world: &World, x: i32, y: i32, hard_mode: bool, rng: &mut SmallRng) -> bool {
    let surface = f64::from(world.surface);
    let (fx, fy) = (f64::from(x), f64::from(y));
    let width = f64::from(world.width());
    if fy < surface * 0.3499999940395355 && (fx < width * 0.45 || fx > width * 0.55 || hard_mode) {
        return true;
    }
    fy < surface * 0.44999998807907104 && hard_mode && rng.random_range(0..10) == 0
}

/// `Main.wallLight` (`Main.cs:10717-10732`): the walls daylight comes through, no wall included.
///
/// Only the Martian Probe's own gate reads it, through `skyBehindPlayer`.
const WALL_LIGHT: [u16; 16] = [
    0, 21, 106, 107, 138, 139, 140, 141, 145, 150, 152, 168, 245, 315, 317, 318,
];

/// `skyBehindPlayer` (`NPC.cs:413`): `Main.wallLight[Main.tile[pX, pY].wall] || wall == 73`, read
/// at the player's own tile. A probe scouts people standing under open sky, not people in a house.
fn sky_behind_player(wall: u16) -> bool {
    WALL_LIGHT.contains(&wall) || wall == 73
}

/// What the sky sends, once a tile up there has been chosen (`NPC.cs:1383-1424`).
///
/// An ordered chain, not a weighted draw, and its last arm is unconditional: the Harpy is what the
/// sky is when nothing rarer wins. Transcribed with vanilla's own ordering and rolls.
///
/// Two of the game's arms are deliberately absent, each because the thing it needs does not exist
/// here:
///
/// * `invaders && Main.invasionType == 4` -> Martian Drone (388). This server's invasion spawning
///   is a separate path that never reaches the sky branch, so a drone is a Martian Madness member
///   rather than a sky mob.
/// * the two `ZoneWaterCandle` repeats at `:1409` and `:1418`, which are dead code in the game
///   itself: each repeats the condition of the arm immediately above it, so the earlier arm has
///   already answered whenever the later one could.
///
/// `probe_gate` is vanilla's `flag5`, resolved by the caller because it reads the player's tile.
fn sky_pick(
    hard_mode: bool,
    probe_gate: bool,
    world: &World,
    no_worms: bool,
    alive: &dyn Fn(u16) -> bool,
    rng: &mut SmallRng,
) -> u16 {
    // `NPC.cs:1400-1404`. `maxValue2`/`maxValue3` are 8 and 30 (`:1384-1385`); the water-candle
    // pair that narrows them to 3 and 10 is a player-carried item this server does not model.
    // One probe at a time, and only ever after the Golem: it is the invitation to Martian Madness,
    // so a second one while the first is still scouting would invite it twice.
    if probe_gate
        && hard_mode
        && world.progress.downed_golem
        && ((!world.progress.downed_martians && rng.random_range(0..8) == 0)
            || rng.random_range(0..30) == 0)
        && !alive(MARTIAN_PROBE)
    {
        return MARTIAN_PROBE;
    }
    // `NPC.cs:1412`: one in ten, one at a time, and not while a wall at the player's back keeps
    // burrowers out. A Wyvern is a worm, and `noWorms` is about worms wherever they fly.
    if hard_mode && !alive(WYVERN_HEAD) && !no_worms && rng.random_range(0..10) == 0 {
        return WYVERN_HEAD;
    }
    // `NPC.cs:1417`: one in twenty-five, one at a time, and only while nobody has freed it yet.
    // No progression gate at all, so a Purple Slime is findable on a fresh world by anyone who
    // gets high enough to be in the sky in the first place. `RollLuck(25)` is `Main.rand.Next(25)`
    // at luck zero (`Luck.cs:5-16`), which is what this server models.
    if !world.progress.unlocked_slime_purple
        && rng.random_range(0..25) == 0
        && !alive(BOUND_TOWN_SLIME_PURPLE)
    {
        return BOUND_TOWN_SLIME_PURPLE;
    }
    HARPY
}

/// Pick spawns for this tick.
///
/// Returns the types and pixel positions to create; the caller owns the NPC table, so this stays a
/// pure decision and is straightforward to test.
/// What the events running right now do to the spawn pool.
///
/// A moon or an eclipse does not add to the ordinary pool — it replaces it on the surface, which
/// is why standing outside during one is a different game and standing in a cave is not.
pub struct EventSpawns<'a> {
    /// Which moon is up, and which wave it is on.
    pub moon: Option<(crate::game::moons::Moon, i32)>,
    /// Whether a solar eclipse is happening.
    pub eclipse: bool,
    /// `Sandstorm.Happening` (`Sandstorm.cs:14`), which `game/weather.rs` already simulates.
    ///
    /// Half of `ZoneSandstorm` (`SceneMetrics.cs:706`,
    /// `ZoneDesert && SurfaceAtmospherics && Sandstorm.Happening`); the other two halves are the
    /// player's biome and height, which [`try_spawn`] has to hand.
    pub sandstorm: bool,
    pub downed_plantera: bool,
    pub downed_all_mechs: bool,
    /// Whether the field already holds as many event bosses as it will take.
    pub boss_cap: bool,
    /// `NPC.AnyDanger()` (`NPC.cs:81063-81106`): a moon, an invasion, the Old One's Army, a live
    /// boss, or the Moon Lord's countdown. Only the Martian Probe's gate reads it, and it reads it
    /// as "not while something is already happening".
    ///
    /// Resolved by the caller because half of it is server state `try_spawn` cannot see. The
    /// `DangerThatPreventsOtherDangers` set (the lunar pillars) is folded into the caller's boss
    /// test rather than named separately.
    pub any_danger: bool,
    /// Whether the wall has fallen, which is what opens the hardmode half of every pool.
    pub hard_mode: bool,
    /// ...and whether a mechanical boss is down, which is what opens the underworld's.
    pub downed_mech_any: bool,
    /// How many of a type are alive, for the tables that cap their heavies.
    pub census: &'a dyn Fn(u16) -> usize,
    /// The six cavern enemies this particular world has.
    ///
    /// Not part of the pool tables because they are not the same in every world: each world draws
    /// six of the thirteen from its own id. Two worlds therefore feel different underground, and
    /// a player who knows theirs has Salamanders and no Crawdads is right about that permanently.
    pub cavern_monsters: crate::game::cavern_monsters::CavernMonsters,
    /// Where each lunar pillar that is still standing is, in [`crate::game::lunar::PILLARS`] order.
    ///
    /// Vanilla decides a tower zone on the client, in `SceneMetrics`: `ScanNPCPositions`
    /// (`SceneMetrics.cs:734-751`) keeps the nearest live NPC of every type, and each
    /// `CloseEnoughTo*Tower` (`SceneMetrics.cs:276-282`) is `WithinRangeOfNPC(<pillar>,
    /// NPCEventZoneRadius)` against it. The caller gathers the four positions once a tick, and only
    /// while the event is up, so the test in [`Self::tower_zone`] is four distance comparisons per
    /// player rather than another pass over the NPC store on the spawn path.
    pub towers: [Option<(f32, f32)>; 4],
}

impl EventSpawns<'_> {
    /// Whether anything is running that overrides the surface pool.
    fn running(&self) -> bool {
        self.moon.is_some() || self.eclipse
    }

    /// Which pillar's zone a point is inside, if any.
    ///
    /// The order is vanilla's own `else if` chain (Nebula, Vortex, Stardust, Solar:
    /// `NPC.cs:1297`, `:1321`, `:1347`, `:1356`), so two overlapping zones resolve the way the
    /// game resolves them rather than by whichever pillar happens to be nearer.
    fn tower_zone(&self, at: (f32, f32)) -> Option<u16> {
        use crate::game::lunar;

        /// `SceneMetrics.NPCEventZoneRadius` (`SceneMetrics.cs:130`), in pixels.
        const RADIUS: f32 = 4000.0;
        const ORDER: [u16; 4] = [lunar::NEBULA, lunar::VORTEX, lunar::STARDUST, lunar::SOLAR];

        ORDER.into_iter().find(|&pillar| {
            lunar::PILLARS
                .iter()
                .position(|p| *p == pillar)
                .and_then(|slot| self.towers[slot])
                // `WithinRangeOfNPC` (`SceneMetrics.cs:912-920`) compares squared distances.
                .is_some_and(|(x, y)| {
                    let (dx, dy) = (x - at.0, y - at.1);
                    dx * dx + dy * dy <= RADIUS * RADIUS
                })
        })
    }
}

/// What a lunar pillar's zone spawns, and nothing else spawns there: `NPC.cs:1297-1372`.
///
/// Each arm is a `Utils.SelectRandom` list, which is a uniform draw over the array
/// (`Utils.cs:2628-2631`), so a repeated id is that id's weight. Three of the four then sit inside
/// a `while` that re-rolls whenever the type drawn is already at its own live cap, `CountNPCS`
/// being `alive` here; the Stardust arm has no such loop at all.
///
/// The bound on the loop is ours. Vanilla's `while` is unbounded and cannot hang, because every
/// list has at least one entry with no cap on it (`427` for Vortex, `421` for Nebula, `417` and
/// friends for Solar); the bound is so a future edit to one of these tables cannot spin a tick
/// instead of failing. `None` means the roster was capped out, and the attempt is dropped.
pub fn tower_pool(pillar: u16, alive: &dyn Fn(u16) -> usize, rng: &mut SmallRng) -> Option<u16> {
    use crate::game::lunar;

    /// Nebula Soldier, Nebula Beast, Nebula Headcrab, Nebula Brain (`NPC.cs:1302`).
    const NEBULA: [u16; 11] = [424, 424, 424, 423, 423, 423, 421, 421, 421, 420, 420];
    /// Vortex Soldier, Vortex Hornet, Vortex Rifleman, Vortex Hornet Queen (`NPC.cs:1328`).
    const VORTEX: [u16; 9] = [429, 429, 429, 429, 427, 427, 425, 425, 426];
    /// Stardust Soldier, Spider, Jellyfish, Worm, Cell (`NPC.cs:1349`).
    const STARDUST: [u16; 8] = [411, 411, 411, 409, 409, 407, 402, 405];
    /// Solar Spearman, Solenian, Corite, Crawltipede, Sroller, Drakomire Rider, Drakomire
    /// (`NPC.cs:1364`).
    const SOLAR: [u16; 7] = [518, 419, 418, 412, 417, 416, 415];
    /// What half of the Corite draws become instead (`NPC.cs:1368`).
    const SOLAR_INSTEAD_OF_CORITE: [u16; 4] = [415, 416, 419, 417];
    /// How many re-rolls before the roster is called capped out. See this function's own doc.
    const ATTEMPTS: usize = 64;

    let pick = |list: &[u16], rng: &mut SmallRng| list[rng.random_range(0..list.len())];

    for _ in 0..ATTEMPTS {
        let drawn = match pillar {
            // `NPC.cs:1297-1319`.
            lunar::NEBULA => {
                let num = pick(&NEBULA, rng);
                if (num == 424 && alive(424) >= 3)
                    || (num == 423 && alive(423) >= 3)
                    || (num == 420 && alive(420) >= 3)
                {
                    continue;
                }
                num
            }
            // `NPC.cs:1321-1345`.
            lunar::VORTEX => {
                let num = pick(&VORTEX, rng);
                if (num == 425 && alive(425) >= 3)
                    || (num == 426 && alive(426) >= 3)
                    || (num == 429 && alive(429) >= 4)
                {
                    continue;
                }
                num
            }
            // `NPC.cs:1347-1354`: one draw, no cap, no loop.
            lunar::STARDUST => pick(&STARDUST, rng),
            // `NPC.cs:1356-1372`. The Corite is re-drawn on a coin flip *inside* the loop, so a
            // re-draw that lands on a capped type re-rolls the whole thing, as the game's does.
            lunar::SOLAR => {
                let mut num = pick(&SOLAR, rng);
                if num == 418 && rng.random_range(0..2) == 0 {
                    num = pick(&SOLAR_INSTEAD_OF_CORITE, rng);
                }
                if (num == 518 && alive(518) >= 2) || (num == 412 && alive(412) >= 1) {
                    continue;
                }
                num
            }
            _ => return None,
        };
        return Some(drawn);
    }
    None
}

/// How many townsfolk are close enough to a point to quiet it down.
///
/// The game counts them through `SceneMetrics`, which is roughly what is on screen. This uses a
/// radius in the same neighbourhood: far enough that a house nearby counts, close enough that a
/// town on the other side of the world does not make the whole map safe.
fn town_npcs_near(npcs: &NpcStore, at: (f32, f32)) -> u32 {
    /// Tiles, converted to pixels below.
    const REACH: f32 = 100.0 * 16.0;

    npcs.iter()
        .filter(|(_, npc)| npc.stats.town_npc && npc.is_alive())
        .filter(|(_, npc)| {
            (npc.position.0 - at.0).abs() < REACH && (npc.position.1 - at.1).abs() < REACH
        })
        .count() as u32
}

/// Half-extents, in pixels, of the box the cap looks in for the NPCs that already fill it.
///
/// The game counts against a player's `maxSpawns` only the NPCs whose active box overlaps that
/// player (`NPC.cs:78706`: `activeRangeX = sWidth*2.1`, `activeRangeY = sHeight*2.1`, so 1920*2.1
/// and 1200*2.1 pixels, about 252 tiles across and 157 down). The old cap counted every NPC in the
/// whole world, so a monster on the far side of the map held a lone player's own spawns down.
const ACTIVE_RANGE_X: f32 = 1920.0 * 2.1;
const ACTIVE_RANGE_Y: f32 = 1200.0 * 2.1;

/// The spawn weight already near a point: the sum of `npcSlots` over the live, non-town NPCs whose
/// active box overlaps it, which is exactly what the game checks a player's `maxSpawns` against
/// (`NPC.cs:313`, `player.nearbyActiveNPCs`, accumulated in `CheckActive` weighted by each NPC's
/// own `npcSlots`). A statue-spawned monster does not count, as it does not in the game (it carries
/// no spawn slots), which is what lets a statue farm keep working.
pub fn nearby_active_npcs(npcs: &NpcStore, at: (f32, f32)) -> f32 {
    npcs.iter()
        .filter(|(_, npc)| npc.is_alive() && !npc.stats.town_npc && !npc.from_statue)
        .filter(|(_, npc)| {
            (npc.position.0 - at.0).abs() < ACTIVE_RANGE_X
                && (npc.position.1 - at.1).abs() < ACTIVE_RANGE_Y
        })
        .map(|(_, npc)| npc.stats.npc_slots)
        .sum()
}

/// Each player's last biome scan, so the rate can read a zone without paying for one every tick.
///
/// `biome_at` walks a 169-by-124 tile box, measured at 78 us on a full-size world. The rate needs
/// it on every attempt, not only the roughly one in six hundred that places something
/// (`NPC.cs:591-660`), and 78 us per player per tick is 19.8 ms at this server's 255-player bar,
/// which is the whole 16.67 ms tick budget and then some. Vanilla has no equivalent cost: a real
/// dedicated server never scans at all, because the *client* runs `SceneMetrics` and sends its
/// zones up in packet 36.
///
/// So the scan is reused until it is either a second old or the player has walked far enough for it
/// to be stale. Both bounds are conservative against the box being scanned: 16 tiles is a fifth of
/// its half-width, so a cached answer is still one taken from well inside the same neighbourhood.
/// Caching alone was not enough, because it left nothing bounding how many scans land on one tick.
/// Every client in a join burst becomes `is_playing()` on the same tick with an empty entry, so the
/// first fill bought 255 scans at once, and those entries then carried the same `at` and expired
/// together sixty ticks later. A real 255-player soak measured `phase=spawning phase_us=20763`,
/// which is 266 scans: every player, in one tick, over the whole frame budget.
///
/// The average was never the problem, and reporting it is what hid this: 255 players over a
/// 60-tick refresh is 4.25 scans a tick, and the example that measured 345 us drove *one* slot for
/// 60,000 ticks. A per-tick mean over one player cannot see a per-tick maximum over 255.
///
/// So [`Self::BUDGET`] bounds the scans a single tick may buy, and stale answers are served past
/// it. **That budget is also the stagger.** Serving 8 of 255 expiries spreads their next `at` over
/// 32 ticks by construction, and re-spreads them after anything that re-synchronises the group (a
/// mass rejoin, a restart, everyone teleporting to a boss). A phase-based stagger would bound the
/// tick too, but it makes a drifted player wait up to 59 ticks for a fresh answer, which silently
/// guts the `DRIFT` guarantee; and an age-based one does not bound anything, because drift resets
/// the phase and the slots reconverge.
#[derive(Debug)]
pub struct BiomeCache {
    /// What tick it is, set by [`Self::advance`] before each spawn pass.
    now: u64,
    /// Scans left to spend this tick, reset by [`Self::advance`].
    left: u32,
    /// Indexed by player slot.
    entries: Vec<Option<Scan>>,
}

/// One cached scan: the tick it was taken, where, and what it said (see [`Zones`]).
type Scan = (u64, i32, i32, Zones);

/// Hand-written rather than derived so a fresh cache starts with a full budget. A derived `Default`
/// gives `left: 0`, which would make a cache that has not been advanced yet refuse every scan and
/// answer `None` forever.
impl Default for BiomeCache {
    fn default() -> Self {
        Self {
            now: 0,
            left: Self::BUDGET,
            entries: Vec::new(),
        }
    }
}

impl BiomeCache {
    /// Ticks a scan stays good for when the player has not moved far.
    const REFRESH: u64 = 60;
    /// ...and how far they may move before it is taken again anyway, in tiles.
    const DRIFT: i32 = 16;
    /// Scans a single tick may buy, however many players want one.
    ///
    /// Demand at the 255-player bar is `255 / REFRESH` = 4.25 a tick, so 8 is about 1.9x demand and
    /// never builds a backlog, while `8 * 78 us` = 624 us is 3.7% of the frame. The bound does not
    /// grow with the player count, because this is a constant and the scanned box is a fixed
    /// 169x124 tiles.
    ///
    /// What it costs: when every slot expires at once, the oldest entry reaches
    /// `REFRESH + ceil(255 / BUDGET)` = 92 ticks, or 1.53 s, before it is refreshed. For a biome
    /// that moves at Clentaminator speed under a standing player, that is nothing.
    ///
    /// ponytail: the budget goes to the lowest slots that want it, since `try_spawn` walks players
    /// in slot order. Under ordinary play it rotates on its own, because a slot that just scanned is
    /// fresh next tick. Eight clients crossing `DRIFT` every single tick in slots 0-7 could hold it
    /// and leave the rest on stale answers, but that needs 960 tiles a second, so it is a
    /// hacked-client vector rather than a normal one, and the 624 us bound holds either way. If it
    /// is ever seen, the fix is a rotating start cursor over `entries`.
    const BUDGET: u32 = 8;

    /// Tell the cache what tick it is, and refill the scan budget. Called once, before the spawn
    /// pass, so the budget is per pass and resets even on a tick where nothing reads.
    pub fn advance(&mut self, ticks: u64) {
        self.now = ticks;
        self.left = Self::BUDGET;
    }

    /// The last answer taken for this player, however old, and never a fresh scan.
    ///
    /// For callers on a packet path rather than on the tick. A scan is 78 us and a client decides
    /// how often it sends, so a handler must not be able to ask for one: interleaving a move with
    /// whatever packet it is handling would invalidate the entry as fast as the handler refilled
    /// it, and a hundred such pairs in a tick is 7.8 ms of a 16.67 ms budget bought with two
    /// packets. The spawn pass already refreshes this every tick for every active player, so this
    /// is the same answer [`Self::read`] would give in all but the first tick of a session.
    /// Hands back the whole [`Zones`], not just its winning [`Biome`]: the entry already holds it,
    /// and the caller's question (a shopping zone set) is the game's own independent-flag one.
    pub fn last(&self, slot: usize) -> Option<Zones> {
        self.entries.get(slot).copied().flatten().map(|e| e.3)
    }

    /// This player's zone, scanning only when the last answer has gone stale and this tick still
    /// has budget for it.
    ///
    /// `None` means "no answer available this tick", not "forest": a slot with no entry at all has
    /// no stale answer to fall back on, and handing back a default would put every joining player in
    /// the wrong spawn pool for the half second before its turn comes round. The caller skips that
    /// player for the tick instead, which costs nothing observable at roughly one attempt in 600
    /// placing anything.
    pub fn read(&mut self, world: &World, slot: usize, x: i32, y: i32) -> Option<Zones> {
        if self.entries.len() <= slot {
            self.entries.resize(slot + 1, None);
        }
        if let Some((at, sx, sy, zones)) = self.entries[slot]
            && self.now.saturating_sub(at) < Self::REFRESH
            && (x - sx).abs() <= Self::DRIFT
            && (y - sy).abs() <= Self::DRIFT
        {
            return Some(zones);
        }
        if self.left == 0 {
            // Out of scans this tick. A stale answer is still an answer; nothing is not.
            return self.entries[slot].map(|(_, _, _, zones)| zones);
        }
        self.left -= 1;
        let zones = zones_at(world, x, y);
        self.entries[slot] = Some((self.now, x, y, zones));
        Some(zones)
    }
}

/// One spawn attempt in this many considers a bound townsperson instead of an enemy.
///
/// Deliberately steep. A handful of them exist in a world's whole lifetime and each is a resident
/// you cannot otherwise have, so they want to be a find rather than a fixture.
const BOUND_RARITY: u32 = 120;

/// Whether a bound townsperson may be found at this depth, biome and spot.
///
/// Each gate is the real `NPC.Spawner.SpawnNPC` condition for that bound NPC rather than "anywhere
/// underground": without them the Wizard, Mechanic and Goblin Tinkerer were all findable on day
/// one, skipping the hardmode / Skeletron / goblin-army progression the game puts in front of
/// them. The gates key on world progress the server already tracks, plus the depth and biome of
/// the candidate spot. `spawn_y` is the tile row, used for the Mechanic's exact depth threshold.
///
/// Two deliberate narrowings, both because the server does not model the tile the real gate reads:
/// the Stylist's real gate is a spider-nest wall (wall 62, `NPC.cs:1662-1671`), approximated here
/// as "the caverns"; and the Angler's is the ocean surface/water (`NPC.cs:1778-1928`), so he is
/// gated to the ocean biome and is therefore never *mis*-found in a cave, even though this
/// underground-only bound path never reaches the ocean to place him (a disclosed gap, not a fake).
pub fn bound_gate(bound: u16, world: &World, depth: Depth, biome: Biome, spawn_y: i32) -> bool {
    let p = &world.progress;
    match bound {
        // Goblin Tinkerer: the goblin army beaten, deeper than the rock layer but above the
        // underworld (`NPC.cs:2087`: downedGoblins && deeperThanRockLayer && spawnTileY < maxTilesY-210).
        105 => p.downed_goblins && depth == Depth::Cavern,
        // Wizard: hardmode, same caverns band (`NPC.cs:2091`).
        106 => p.hard_mode && depth == Depth::Cavern,
        // Bound Old Slime: Skeletron beaten, and the same caverns band as the two above
        // (`NPC.cs:2095`: `downedBoss3 && deeperThanRockLayer && spawnTileY < maxTilesY - 210`).
        // Its arm sits immediately after the Wizard's in the same `else if` chain, on the same
        // three geometry tests, which is why it belongs in this gate rather than in a chain of its
        // own. The `!unlockedSlimeOldSpawn` half is `rescues::still_bound`, as with every other
        // find here; the `!AnyNPCs(685)` half is `pick_bound`'s own "not already standing about".
        BOUND_TOWN_SLIME_OLD => p.downed_boss3 && depth == Depth::Cavern,
        // Mechanic: Skeletron beaten, below (worldSurface*4 + rockLayer)/5 (`NPC.cs:2656`).
        123 => {
            let threshold = (f64::from(world.surface) * 4.0 + f64::from(world.rock_layer)) / 5.0;
            p.downed_boss3 && f64::from(spawn_y) > threshold
        }
        // Stylist: the spider nest, approximated as the caverns (`NPC.cs:1662-1671`; see above).
        354 => depth == Depth::Cavern,
        // Angler: the ocean (`NPC.cs:1778-1928`; see above).
        376 => biome == Biome::Ocean,
        // Bartender: the Old One's Army becomes available once the Eater of Worlds / Brain of
        // Cthulhu is down (`NPC.cs:1658`, `DD2Event.ReadyToFindBartender => NPC.downedBoss2`).
        579 => p.downed_boss2,
        // Golfer: the underground desert (`NPC.cs:1682-1697`).
        589 => biome == Biome::Desert && matches!(depth, Depth::Underground | Depth::Cavern),
        _ => false,
    }
}

/// Somebody still tied up somewhere in this world, if any are left to find here.
///
/// Refuses anyone already rescued, anyone already standing about waiting to be talked to (so a
/// world cannot end up with two Mechanics or a corridor full of bound wizards), and anyone whose
/// real progression / biome / depth gate this spot does not satisfy.
fn pick_bound(
    world: &World,
    npcs: &NpcStore,
    depth: Depth,
    biome: Biome,
    spawn_y: i32,
    rng: &mut SmallRng,
) -> Option<u16> {
    let waiting: Vec<u16> = crate::game::rescues::RESCUES
        .iter()
        .map(|r| r.bound)
        // The bound Old Slime is found down here too but is not freed by talking, so it is not in
        // the rescue table (see `rescues::RESCUES`). Its own gate is in `bound_gate`, and the two
        // other bound slimes are absent from that gate, so chaining them in would find nothing.
        .chain(std::iter::once(BOUND_TOWN_SLIME_OLD))
        .filter(|bound| crate::game::rescues::still_bound(&world.progress, *bound))
        .filter(|bound| bound_gate(*bound, world, depth, biome, spawn_y))
        .filter(|bound| {
            !npcs
                .iter()
                .any(|(_, n)| n.npc_type == *bound && n.is_alive())
        })
        .collect();
    if waiting.is_empty() {
        return None;
    }
    Some(waiting[rng.random_range(0..waiting.len())])
}

pub fn try_spawn(
    world: &World,
    npcs: &NpcStore,
    players: &[Option<Player>],
    events: &EventSpawns<'_>,
    journey: &JourneyPowers,
    biomes: &mut BiomeCache,
    rng: &mut SmallRng,
) -> Vec<(u16, (f32, f32))> {
    let active: Vec<&Player> = players
        .iter()
        .flatten()
        .filter(|p| p.is_playing() && p.life > 0)
        .collect();
    if active.is_empty() {
        return Vec::new();
    }

    // The cap is per-player and near-player, as the game's is: each player is gated on their own
    // `maxSpawns` against the spawn weight already close to them (`NPC.cs:312-313`), inside the loop
    // below. There is no world-global slot total and no flat +30%-per-player multiplier here; a
    // second player raises the world's monster count only because they carry their own near-player
    // budget where they stand, which is what the game does and what a single global cap could not.
    let mut out = Vec::new();
    // `NPC.cs:266`, `numberOfActivePlayers`: read once, before the loop consumes the list.
    let active_players = active.len() as u32;
    for player in active {
        let (px, py) = (
            (player.position.0 / 16.0) as i32,
            (player.position.1 / 16.0) as i32,
        );

        // `CanSpawnEnemiesNear` (`NPC.cs:358-362`): nothing spawns anywhere near a live Moon Lord,
        // `player.isNearNPC(398, MoonLordFightingDistance)` with that distance being 4500 px
        // (`NPC.cs:6036`). The fight is meant to be the Moon Lord and its parts, not the Moon Lord
        // plus whatever the surface would ordinarily have sent.
        const MOON_LORD: u16 = 398;
        const MOON_LORD_FIGHTING_DISTANCE: f32 = 4500.0;
        let player_centre = (
            player.position.0 + crate::game::ai::PLAYER_WIDTH as f32 / 2.0,
            player.position.1 + crate::game::ai::PLAYER_HEIGHT as f32 / 2.0,
        );
        if npcs.iter().any(|(_, n)| {
            n.npc_type == MOON_LORD && n.is_alive() && {
                let (dx, dy) = (
                    n.center().0 - player_centre.0,
                    n.center().1 - player_centre.1,
                );
                dx.hypot(dy) < MOON_LORD_FIGHTING_DISTANCE
            }
        }) {
            continue;
        }

        // Which pillar's zone this player is standing in, if any. Four distance comparisons
        // against positions the caller already gathered, so nothing here walks the NPC store.
        let tower = events.tower_zone(player_centre);

        // Journey mode's `SpawnRate`, gated on the world's own difficulty being literally
        // Journey (`Main.IsJourneyMode` — every one of its five real vanilla call sites checks
        // this before reading the power at all; the power itself has no effect outside a Journey
        // world, even for a player who somehow has it set). Both real effects — a hard "spawns
        // off" at the slider's exact floor, and the ordinary rate scaling otherwise — are checked
        // here; only the *rate*, not the shared cap above, is adjusted per player — this
        // function's own cap is already one number shared across every active player rather than
        // vanilla's fully independent per-player `maxSpawns`, an existing simplification predating
        // this power, not something worth restructuring just to extend one Journey slider into.
        let journey_world = world.game_mode == 3;
        if journey_world && journey.spawns_disabled(player.slot) {
            continue;
        }

        // The biome is the *player's* zone, worked out once from where they stand, not re-read at
        // each candidate tile. The game classifies the zone on the player (`SceneMetrics` scans
        // around the player's centre and `SetSpawnFlags` copies `player.Zone*` straight across,
        // `NPC.cs:382-397`); reading it at the far edge of the spawn box instead let a player in
        // the middle of a biome draw the wrong pool whenever a candidate happened to land just
        // outside it.
        //
        // It is read here, before the rate roll rather than after, because `GetSpawnRate` itself
        // reads it: a whole block of rate and cap modifiers keys on the zone (`NPC.cs:591-660`).
        // That is why it now goes through [`BiomeCache`] rather than scanning outright: paying for
        // the scan on every attempt rather than only a successful one is 78 us per player per tick,
        // which does not fit in a tick at this server's player bar.
        // `None` means this tick had no scan budget left and this player has no earlier answer to
        // fall back on, which happens only in the first ticks after a join burst. Skip them rather
        // than guess a zone: every rate and cap modifier below keys on it, so a wrong guess puts
        // them in the wrong spawn pool, and at roughly one attempt in 600 placing anything, a
        // player missing a few attempts is not observable.
        let Some(zones) = biomes.read(world, usize::from(player.slot), px, py) else {
            continue;
        };
        let player_biome = zones.biome;
        // `ZoneSandstorm` (`SceneMetrics.cs:706`): `ZoneDesert && SurfaceAtmospherics &&
        // Sandstorm.Happening`, where `SurfaceAtmospherics` is `IsSurfaceForAtmospherics`
        // (`WorldGen.cs:11003-11014`) and outside a remix world is simply `y <= worldSurface`. It is
        // a *player* zone like every other, so it is answered once here rather than per candidate
        // tile; only the sand under the chosen spot is decided down there.
        //
        // `zones.desert` rather than `player_biome == Desert`: a corrupt, crimson or hallowed desert
        // is both zones at once in the game, and it is where three of the four sandsharks live.
        let sandstorm_zone = events.sandstorm && zones.desert && py <= i32::from(world.surface);
        // The wall at the player's own tile, read once: three player zones hang off it here, the
        // same three the game reads off its one `Framing.GetTileSafely(TileCenter)`
        // (`SceneMetrics.cs:670-693`). `noWorms` wants `Main.wallHouse[]` of it (`NPC.cs:411`),
        // `ZoneLihzhardTemple` wants it to be exactly 87, and the sky's own probe gate wants to
        // know whether there is open air behind the player.
        let player_wall = world.tile(px, py).wall;
        // `Player.ZoneLihzhardTemple` (`SceneMetrics.cs:693`), a player zone like the rest and so
        // answered here rather than per candidate tile: only the brick underfoot is decided down
        // in the loop.
        let temple_zone = player_wall == TEMPLE_WALL;
        let near = nearby_active_npcs(npcs, player.position);

        // The rate and cap are the player's own, not one number for the world: two people in the
        // same world can be standing in a quiet forest and a busy cavern at the same moment.
        let conditions = Conditions {
            depth: rate_depth_at(world, py),
            biome: player_biome,
            hard_mode: world.progress.hard_mode,
            day_time: world.day_time,
            blood_moon: world.blood_moon,
            eclipse: world.eclipse,
            // `NPC.cs:543` and `:772` both carry the height half of this condition.
            event_moon: world.pumpkin_moon || world.snow_moon,
            // `NPC.cs:543` and `:772`, `player.position.Y < Main.worldSurface * 16.0`.
            above_surface_line: py < i32::from(world.surface),
            town_npcs: town_npcs_near(npcs, player.position),
            nearby_active_npcs: near,
            // `NPC.cs:686`, `player.position.Y / 16 > (worldSurface + rockLayer) / 2`.
            below_dirt_midline: py > (i32::from(world.surface) + i32::from(world.rock_layer)) / 2,
            downed_boss3: world.progress.downed_boss3,
            // `NPC.cs:411`, read at the player's own tile.
            behind_a_house_wall: terrustia_proto::housing::wall_encloses(player_wall),
            active_players,
            in_tower_zone: tower.is_some(),
            graveyard: player.in_graveyard(),
            meteor: zones.meteor,
            lihzahrd_temple: temple_zone,
        };
        // The season and the graveyard, read once per player per attempt: two bools off the world
        // and one bit off the zone packet this player last sent, so nothing here walks a tile.
        let seasonal = Seasonal {
            halloween: world.halloween,
            xmas: world.xmas,
            graveyard: conditions.graveyard,
            hard_mode: events.hard_mode,
            blood_moon: world.blood_moon,
            day_time: world.day_time,
            moon_phase: world.moon_phase,
            // `NPC.cs:372`, `raining = Main.raining`.
            raining: world.raining,
        };
        let (mut rate, band, spawn_friendly) = rates(conditions, rng);
        let no_worms = no_worms(conditions);
        // This player's own near-player cap, checked before the rate roll, exactly as the game does
        // (`NPC.cs:312-317`: `nearbyActiveNPCs >= maxSpawns` first, then `rand.Next(spawnRate)`).
        if near >= band {
            continue;
        }
        if journey_world {
            let multiplier = journey.spawn_rate_multiplier(player.slot);
            rate = ((rate as f32) / multiplier).max(1.0) as u32;
        }
        if rng.random_range(0..rate.max(1)) != 0 {
            continue;
        }
        // `spawnFriendly` (`NPC.cs:795-924`, see `rates`'s own doc): when a populated base has
        // quieted the wild, this attempt draws a harmless critter instead of a monster rather than
        // being thrown away. It is carried down into the candidate loop below, where the same
        // ground and safe-zone checks apply, and resolved against `friendly_pool`'s critter table.

        // Try a handful of candidate tiles rather than scanning the whole area.
        for _ in 0..20 {
            let x = px + rng.random_range(-SPAWN_RANGE_X..=SPAWN_RANGE_X);
            let from_y = py + rng.random_range(-SPAWN_RANGE_Y..=SPAWN_RANGE_Y);
            if x < 10 || from_y < 10 || x >= world.width() - 10 || from_y >= world.height() - 40 {
                continue;
            }

            // A house wall is the reason a walled base is safe, and it is tested on the *chosen*
            // tile before the descent to ground, not on where the descent lands (`NPC.cs:977`):
            //
            // ```csharp
            // if ((Main.tile[num, j].nactive() && Main.tileSolid[Main.tile[num, j].type])
            //     || (!ignoreSafeWalls && Main.wallHouse[Main.tile[num, j].wall])) continue;
            // ```
            //
            // `wall_encloses` is `Main.wallHouse` exactly (all 279 ids, `Main.cs:9880-10745`), which
            // is why housing and spawn suppression agree about what a wall is: the same set decides
            // both, in the game and here. `ignoreSafeWalls` is exactly one thing: standing inside a
            // lunar pillar's zone (`NPC.cs:404-409`), where a walled arena does not keep the
            // escort out. It has to be honoured, or a player could simply wall the pillar in and
            // stand there while a shield that only falls to kills never moved.
            //
            // Without this test, a fully walled and fully lit base spawned zombies inside itself.
            let chosen = world.tile(x, from_y);
            if (chosen.is_active() && solid(chosen.block))
                || (tower.is_none() && terrustia_proto::housing::wall_encloses(chosen.wall))
            {
                continue;
            }

            // High enough up, the tile is taken where it is and nothing walks down to ground
            // (`NPC.cs:979-986`, and see [`sky_tile`]): the sky is a place, not a shortfall of
            // ground, and a Harpy has no floor. Without this branch the descent below threw every
            // sky candidate away, so nothing that lives up there could ever be chosen: the Harpy
            // and the Wyvern were unreachable in this server outright.
            //
            // A friendly attempt is excluded by the game at the same point it decides this, so it
            // is excluded here too: `!spawnFriendly` is part of both `skyMob` branches. So is
            // `!invaders` (`NPC.cs:981`, `:983`), which is why a tower zone never reaches the sky
            // and its escort always walks down to ground.
            let sky = tower.is_none()
                && !spawn_friendly
                && sky_tile(world, x, from_y, events.hard_mode, rng);

            // Drop to whatever ground is under the chosen point, then stand on top of it. The
            // descent stops at the bottom of the spawn box, as `NPC.cs:990-993` does, rather than a
            // fixed 30 tiles: from the top of the box that is up to 94 tiles, which is the reach an
            // ocean needs.
            let y = if sky {
                from_y
            } else {
                let Some(ground) = find_ground_within(world, x, from_y, py + SPAWN_RANGE_Y) else {
                    continue;
                };
                ground - 1
            };

            // Never spawn on top of somebody.
            if (x - px).abs() < SAFE_RANGE_X && (y - py).abs() < SAFE_RANGE_Y {
                continue;
            }
            // `HasTileSpawnSpace` asks only for open space; the floor half of [`has_room`] belongs
            // to the descent, which a sky tile skipped.
            if !(if sky {
                open_space(world, x, y)
            } else {
                has_room(world, x, y)
            }) {
                continue;
            }

            // A pillar's zone owns the spawn chain outright. Vanilla's four `ZoneTower*` arms sit
            // at the head of `SpawnAnNPC`'s `else if` chain (`NPC.cs:1297-1372`), ahead of the sky,
            // the invasions, the water, every biome and every moon, so inside a zone the only
            // things that appear are that pillar's own escort. This is the whole lunar event: the
            // shield is a count of escort killed (`game/lunar.rs`), so with nothing spawning the
            // four pillars could not be damaged at all and the Moon Lord was unreachable.
            if let Some(pillar) = tower {
                let alive_count = |ty: u16| {
                    npcs.iter()
                        .filter(|(_, n)| n.npc_type == ty && n.is_alive())
                        .count()
                };
                let Some(npc_type) = tower_pool(pillar, &alive_count, rng) else {
                    continue;
                };
                out.push((npc_type, (x as f32 * 16.0, y as f32 * 16.0)));
                break;
            }

            let depth = depth_at(world, y);
            // The two desert branches are decided at the *tile*, not from the player's zone, which
            // is what lets them be tested on every candidate: the underground desert is two wall
            // reads (`underground_desert_spot`) and the sandstorm is one tile read plus a sand
            // check that breaks out of its own first row on anything else. Neither goes near
            // `biome_at`.
            let desert_spot = !sky && underground_desert_spot(world, x, y);
            // The game's `tileType`: the ground tile the spawn stands on, which this server's row
            // `y` sits one above. Read once here rather than per branch, since three of them want
            // it. A sky spawn has no ground under it and so has no `tileType` at all.
            let ground_block = (!sky).then(|| world.tile(x, y + 1).block);
            let sandstorm_spot = sandstorm_zone
                && ground_block.is_some_and(|g| {
                    SAND_CONVERSION.contains(&g) && sandstone_check(world, x, y + 1)
                });
            // `tileType == 70`, and nothing else: the whole Glowing Mushroom roster hangs off the
            // ground tile rather than off `ZoneGlowshroom` (`NPC.cs:3637`, `:3674`, and see
            // [`mushroom_pick`]).
            let mushroom_ground = ground_block == Some(MUSHROOM_GRASS);
            // The Spore pair's own gate, which wants the zone *and* the tile
            // (`NPC.cs:5110`, `:5209`).
            let glowshroom_ground =
                zones.glowshroom && matches!(ground_block, Some(MUSHROOM_GRASS | MUSHROOM_BLOCK));
            // An event owns the surface while it runs, and nothing below it. Except the sky:
            // vanilla's `skyMob` arm sits above every event arm in the same `else if` chain
            // (`NPC.cs:1383`), so a pumpkin moon does not reach up there.
            let event_type = if !sky && events.running() && depth == Depth::Surface {
                match (events.moon, events.eclipse) {
                    (Some((moon, wave)), _) if !world.day_time => crate::game::moons::moon_spawn(
                        moon,
                        wave,
                        events.census,
                        events.boss_cap,
                        rng,
                    ),
                    (_, true) if world.day_time => Some(crate::game::moons::eclipse_spawn(
                        events.downed_plantera,
                        events.downed_all_mechs,
                        events.census,
                        rng,
                    )),
                    _ => None,
                }
            } else {
                None
            };
            // Somebody tied up, once in a long while, deep enough down to be worth finding.
            //
            // Rare and unique on purpose: these are the *only* way their residents ever arrive, so
            // one of them failing to appear is a whole townsperson missing — the Mechanic, and with
            // her every piece of wire in the game. Each one is gated on its real vanilla condition
            // (`bound_gate`), so the Wizard, Mechanic and Goblin Tinkerer are no longer findable
            // day one and the Golfer wants the underground desert.
            // A bound resident is a monster-path find, never a friendly attempt's.
            if !spawn_friendly
                && matches!(depth, Depth::Underground | Depth::Cavern)
                && rng.random_range(0..BOUND_RARITY) == 0
                && let Some(bound) = pick_bound(world, npcs, depth, player_biome, y, rng)
            {
                out.push((bound, (x as f32 * 16.0, y as f32 * 16.0)));
                break;
            }

            let npc_type = match event_type {
                Some(npc_type) => npc_type,
                // The sky answers for itself, ahead of every other branch, because vanilla's own
                // `else if (skyMob)` sits ahead of them (`NPC.cs:1383`): above the sky line there
                // is no biome to read, no water to stand in and no event to hold the surface.
                None if sky => {
                    // `flag5` (`NPC.cs:1387-1391`): a probe scouts the outer two thirds of the
                    // map, at somebody standing under open sky, and not while anything else is
                    // already going on.
                    let half = world.width() / 2;
                    let probe_gate = (x - half).abs() as f32 / half as f32 > 0.33
                        && sky_behind_player(player_wall)
                        && !events.any_danger;
                    let alive =
                        |ty: u16| npcs.iter().any(|(_, n)| n.npc_type == ty && n.is_alive());
                    sky_pick(events.hard_mode, probe_gate, world, no_worms, &alive, rng)
                }
                // A graveyard's Statue Mimic (`NPC.cs:1571-1574`):
                //
                // ```csharp
                // else if (downedBoss3 && ZoneGraveyard && !noWorms && RollBadLuckExtreme(25) == 0
                //     && !AnyNPCs(690) && IsThisAGoodPlaceForAStatueMimic(spawnTileX, spawnTileY))
                // ```
                //
                // The arm sits this high in vanilla's chain too: one below the invasions, and above
                // the spider nest, both deserts, all the water, the dungeon, the meteor and every
                // biome pool. So it carries no depth and no biome gate of its own, and none is
                // invented here: a graveyard is a graveyard wherever the tombstones are.
                //
                // Its AI and its immortal-until-provoked pose were already built and tested
                // (`game::ai::mimic`, and `IMMORTAL_TYPE` in `game::server`); the spawn was the one
                // missing half, so nothing this server could do put one in a world.
                //
                // The conditions are in vanilla's own order, which is also cheapest-first: the two
                // free bools, then `noWorms`, then a one-in-twenty-five roll, and only then the
                // store scan and the eight tile reads of the plinth check.
                None if world.progress.downed_boss3
                    && seasonal.graveyard
                    && !no_worms
                    && rng.random_range(0..STATUE_MIMIC_ODDS) == 0
                    && !npcs
                        .iter()
                        .any(|(_, n)| n.npc_type == STATUE_MIMIC && n.is_alive())
                    && good_place_for_a_statue_mimic(world, x, y + 1) =>
                {
                    STATUE_MIMIC
                }
                // A spider nest, which owns its own arm the way the desert below owns its
                // (`NPC.cs:1662`), one row above it in vanilla's chain and therefore one arm above
                // it here. The wall is read on the game's own `spawnTileY`, the solid ground tile,
                // which is this server's `y + 1`; it carries no depth or biome gate of its own,
                // because vanilla's does not either.
                None if world.tile(x, y + 1).wall == SPIDER_WALL => {
                    spider_pick(events.hard_mode, rng)
                }
                // The underground desert, which is its own chain and answers for everything in it
                // (`NPC.cs:1682-1765`). It sits here because that is where vanilla puts it: ahead of
                // every water branch, ahead of `spawnFriendly`, ahead of `ZoneDungeon` and ahead of
                // every biome pool. A friendly attempt reaching a sandstone wall therefore draws a
                // ghoul rather than a scorpion, which is the game's own ordering rather than an
                // oversight here.
                None if desert_spot => {
                    let alive_count = |ty: u16| {
                        npcs.iter()
                            .filter(|(_, n)| n.npc_type == ty && n.is_alive())
                            .count()
                    };
                    underground_desert_pick(
                        world,
                        y,
                        events.hard_mode,
                        player_biome,
                        no_worms,
                        &alive_count,
                        rng,
                    )
                }
                // A friendly attempt draws a harmless critter for this place; if there is no
                // critter for it (the underworld), the attempt is dropped rather than turned into a
                // monster the game would not have spawned here.
                None if spawn_friendly => {
                    // A graveyard's friendly draw is its own and it is exactly two things
                    // (`NPC.cs:2101-2115`): a Maggot or a Rat, on dry land, and nothing at all
                    // standing in water. The arm sits ahead of every other friendly one, so no
                    // bird, bunny or firefly is drawn among the tombstones.
                    let critters = if seasonal.graveyard {
                        if water_tile(world, x, y) {
                            continue;
                        }
                        &GRAVEYARD_VERMIN[..]
                    } else {
                        friendly_pool(depth, player_biome, world.day_time)
                    };
                    if critters.is_empty() {
                        continue;
                    }
                    let critter = critters[rng.random_range(0..critters.len())];
                    // A frog is a chain rather than a draw, and vanilla runs that chain exactly
                    // where this pool answers 361 (`NPC.cs:2363`, `:3831` -> `SpawnFrog`). No
                    // holiday costume applies: `holiday_costume` only ever dresses a bunny or a
                    // plain slime, and the jungle-grass arm is answered ahead of the case it reads.
                    if critter == FROG {
                        let alive =
                            |ty: u16| npcs.iter().any(|(_, n)| n.npc_type == ty && n.is_alive());
                        spawn_frog(world, &alive, rng)
                    } else {
                        holiday_costume(critter, depth, seasonal, rng)
                    }
                }
                // Below the dungeon before Skeletron falls, the dungeon answers with the Dungeon
                // Guardian instead of its ordinary residents (`NPC.cs:2646-2654`: `!downedBoss3` in
                // the `ZoneDungeon` branch spawns 68 and returns). This is the wall the game puts up
                // so a fresh character cannot walk in and farm dungeon loot early; without it, Angry
                // Bones and Dark Casters spawned pre-Skeletron.
                None if player_biome == Biome::Dungeon && !world.progress.downed_boss3 => {
                    DUNGEON_GUARDIAN
                }
                // Standing in water draws the aquatic roster instead of the land one, which is how
                // vanilla orders it: every `waterTile` branch (`NPC.cs:1766-2000`) sits ahead of
                // every land branch, so the sea gets sharks and the beach gets zombies.
                None if water_tile(world, x, y) && !water_pool(depth, player_biome).is_empty() => {
                    let wet = water_pool(depth, player_biome);
                    wet[rng.random_range(0..wet.len())]
                }
                // A meteor crater, which answers with one thing and nothing else
                // (`NPC.cs:2796-2799`). The arm sits where vanilla's does: below the water and the
                // dungeon, above the whole fallthrough that holds every biome, season and tile
                // type. Standing in a crater there are Meteor Heads and there is nothing else, and
                // that is the entire point of the place.
                None if zones.meteor => METEOR_HEAD,
                // The Glowing Mushroom roster (`NPC.cs:3637-3702`), which asks only what the spawn
                // is standing on. It declines one attempt in three, and every attempt at all
                // underground before hardmode, and the ordinary chain below answers those.
                None if mushroom_ground
                    && let Some(npc_type) =
                        mushroom_pick(depth == Depth::Surface, events.hard_mode, rng) =>
                {
                    npc_type
                }
                // The Lihzahrd Temple, which answers with its own two and nothing else
                // (`NPC.cs:3914-3924`, see [`LIHZAHRD`]). The arm sits exactly where vanilla puts
                // it: below the Glowing Mushroom arm at `:3637` and above the sandstorm at `:3952`,
                // inside the same fallthrough both of those live in.
                //
                // No progression gate of any kind, and none is invented here: vanilla's arm never
                // reads `downedPlantBoss`. The Lihzahrd Power Cell door is what actually keeps a
                // player out of the temple before Plantera, and anybody who gets in early meets the
                // residents. The zone is the player's (a Lihzahrd brick wall at their own tile) and
                // the ground is the candidate's, which is exactly how the game splits it.
                //
                // One disclosed narrowing, the same one the Goblin Scout and Skeleton Merchant arms
                // carry: two arms above this one in vanilla's chain are not modelled here at all and
                // so never decline in this server's favour. `:3802`'s Grasshopper is the only critter
                // arm up there a temple can reach (it excludes snow, both evils and the hallow, but
                // not the jungle a temple reads as), and `:3737`'s one-in-seventy-five souls arm has
                // no biome gate at all in hardmode. The effect is that these two are very slightly
                // commoner here than in the game, never rarer.
                None if temple_zone
                    && matches!(ground_block, Some(LIHZAHRD_BRICK | WOODEN_SPIKES)) =>
                {
                    if rng.random_range(0..FLYING_SNAKE_ODDS) == 0 {
                        FLYING_SNAKE
                    } else {
                        LIHZAHRD
                    }
                }
                // A sandstorm over the desert, which owns the surface while it blows
                // (`NPC.cs:3952-4022`). It sits after the water and the dungeon, as vanilla's does,
                // and pushes its own result because one member of it does not stand where it was
                // chosen: the Dune Splicer burrows in ten tiles down.
                None if sandstorm_spot => {
                    let alive_count = |ty: u16| {
                        npcs.iter()
                            .filter(|(_, n)| n.npc_type == ty && n.is_alive())
                            .count()
                    };
                    let (npc_type, drop) = sandstorm_pick(
                        events.hard_mode,
                        world.progress.downed_boss1,
                        no_worms,
                        ground_block.unwrap_or_default(),
                        &alive_count,
                        rng,
                    );
                    out.push((npc_type, (x as f32 * 16.0, (y + drop) as f32 * 16.0)));
                    break;
                }
                // Hallowed ground, which has a chain of its own ahead of the hallow pool
                // (`NPC.cs:4039-4061`, and see [`hallow_ground_pick`] for the whole of it). It sits
                // here because that is where vanilla puts it: below the sandstorm and the four sand
                // conversions, above the two evils' own tile arms and above `else if (surfaceSpawn)`.
                //
                // Keyed on the tile underfoot rather than on [`Biome::Hallow`], because vanilla's is:
                // the gate is `tileType == 116 || 117 || 109 || 164` with no `ZoneHallow` anywhere in
                // it, so a single pearlstone ledge in an otherwise ordinary cavern roof is hallowed
                // ground for this purpose and a hallowed biome floored in plain stone is not.
                // `underGround` (`NPC.cs:1144`) is the dirt layer and above, so the caverns are out.
                //
                // One disclosed reorder: vanilla tests `!waterTile` before the four tile types, and
                // this tests the tiles first. Both are pure, so the branch taken is the same; the
                // order is chosen so the array scan shuts the gate before `water_tile`'s own tile
                // read is made. Measured at 0.89 ns a candidate on ordinary stone and 16.85 ns for
                // the whole chain on the one night it can answer with everything
                // ([`tests::measure_the_hallowed_ground_arm`]).
                None if events.hard_mode
                    && matches!(depth, Depth::Surface | Depth::Underground)
                    && ground_block.is_some_and(|g| HALLOW_GROUND.contains(&g))
                    && !water_tile(world, x, y)
                    && let Some(npc_type) = hallow_ground_pick(
                        world.progress.downed_plantera,
                        world.day_time,
                        world.time,
                        depth == Depth::Surface,
                        world.raining,
                        &|ty| npcs.iter().any(|(_, n)| n.npc_type == ty && n.is_alive()),
                        rng,
                    ) =>
                {
                    npc_type
                }
                // The underworld arm's own first branch (`NPC.cs:4877`):
                //
                // ```csharp
                // else if (Main.hardMode && !savedTaxCollector && Main.rand.Next(20) == 0 && !AnyNPCs(534))
                // ```
                //
                // It sits ahead of everything else the underworld spawns, which is why it is a
                // branch here rather than an entry in the pool: at one in twenty it is a fifth of
                // the whole underworld draw while it is open, and it closes for good once the world
                // has its Tax Collector. Without it the Tortured Soul never spawned, and since he
                // is the only way a Tax Collector ever exists, an entire townsperson (his shop,
                // his happiness, his arrival) was unreachable.
                None if depth == Depth::Underworld
                    && events.hard_mode
                    && !world.progress.saved_tax_collector
                    && rng.random_range(0..TORTURED_SOUL_ODDS) == 0
                    && !npcs
                        .iter()
                        .any(|(_, n)| n.npc_type == TORTURED_SOUL && n.is_alive()) =>
                {
                    TORTURED_SOUL
                }
                // The caverns' Rock Golem (`NPC.cs:4921-4924`):
                //
                // ```csharp
                // else if (CheckToSpawnRockGolem(spawnTileX, spawnTileY, tileType))
                // {
                //     SpawnNPC(spawnTileX * 16 + 8, spawnTileY * 16, 631);
                // }
                // ```
                //
                // A branch rather than a pool entry, because its gate is a *tile* gate with a
                // ceiling check on it ([`check_to_spawn_rock_golem`]) rather than anything a
                // depth-and-biome pool can express: a golem comes out of the plain stone or out of
                // moss, and only where there is a hole tall enough to hold one.
                //
                // It sits exactly where vanilla puts it. The arm is reached only once `underGround`
                // (`spawnTileY <= Main.rockLayer`, `NPC.cs:1144`) and the underworld arm at
                // `:4871` have both declined, which is this server's `Cavern` and nothing else, and
                // it is one row above the whole cavern chain the two arms below transcribe
                // (`:4925` onward, and `:5004`'s Skeleton Merchant). No biome exclusion beyond
                // vanilla's own `ZoneSnow`: the two evils and the jungle divert on their *tiles*
                // (`:4066`, `:4125`, `:3929-3948`) rather than their zones, and plain stone in a
                // corrupted cavern really does grow golems in the game. The dungeon is the one
                // exception, because `else if (ZoneDungeon)` (`:2629`) returns long before this.
                //
                // `ZoneSnow` is read here as `player_biome == Biome::Snow`, a narrowing this file
                // makes everywhere: the game's zones are independent flags and [`Biome`] picks one
                // winner, so a snowy place that reads as something else still grows golems here.
                //
                // 631's AI is already wired: it is `ai_style` 3, so `game::ai::fighter` walks it,
                // chases and hits. What is not transcribed is its own stand-and-throw block inside
                // `AI_003_Fighters` (`NPC.cs:56866-56931`, projectile 909), which is an AI-lane gap
                // rather than a spawn one and is left named rather than half-built.
                None if depth == Depth::Cavern
                    && player_biome != Biome::Dungeon
                    && let Some(ground) = ground_block
                    && check_to_spawn_rock_golem(
                        world,
                        x,
                        y + 1,
                        ground,
                        events.hard_mode,
                        player_biome == Biome::Snow,
                        rng,
                    ) =>
                {
                    ROCK_GOLEM
                }
                // The cavern chain's rare wanderer (`NPC.cs:5004-5010`):
                //
                // ```csharp
                // else if (Main.rand.Next(2) == 0)
                // {
                //     if (Main.rand.Next(35) == 0 && !ZoneShadowCandle && !waterTile && CountNPCS(453) == 0)
                // ```
                //
                // One in seventy, at any point in a world's progression: he is not a hardmode or a
                // dungeon spawn, whatever the wikis say. Two deliberate narrowings, both of which
                // only make him very slightly more common than the game does. `ZoneShadowCandle`
                // is not modelled here at all, and vanilla reaches this arm only after the cavern
                // chain's earlier branches decline, whose per-tile conditions this server does not
                // read. The biome exclusion is that chain's own upstream tile diversions
                // (`NPC.cs:4066`, `:4125` for the two evils, `:3929-3948` for jungle mud): those
                // tiles are answered before the fallthrough he lives in, so he is not found there.
                None if depth == Depth::Cavern
                    && !matches!(
                        player_biome,
                        Biome::Corruption | Biome::Crimson | Biome::Jungle | Biome::Dungeon
                    )
                    && !water_tile(world, x, y)
                    && rng.random_range(0..SKELETON_MERCHANT_ODDS) == 0
                    && !npcs
                        .iter()
                        .any(|(_, n)| n.npc_type == SKELETON_MERCHANT && n.is_alive()) =>
                {
                    SKELETON_MERCHANT
                }
                None => {
                    let biome = player_biome;
                    // The surface *day* has a chain of its own too, and out at the edges of the map
                    // it holds the one thing in it that is not a critter (`NPC.cs:4482-4485`,
                    // inside `if (!ZoneGraveyard && Main.dayTime)` at `:4202`):
                    //
                    // ```csharp
                    // else if (!waterTile && (num45 > Main.maxTilesX / 3 || Main.remixWorld)
                    //     && (Main.rand.Next(15) == 0
                    //         || (!downedGoblins && WorldGen.shadowOrbSmashed
                    //             && Main.rand.Next(7) == 0)))
                    // ```
                    //
                    // `num45` is `Math.Abs(spawnTileX - Main.spawnTileX)` (`NPC.cs:4204`), the
                    // distance from the *world's* spawn point rather than from the player: a scout
                    // is something met out at the edges, never in the back garden. Both rolls sit
                    // under that one distance gate and are OR'd rather than exclusive, so a smashed
                    // shadow orb adds a second chance on top of the first rather than replacing it.
                    //
                    // Without this the Goblin Scout was in no pool and no branch, so Tattered Cloth
                    // never dropped, so the Goblin Battle Standard could not be crafted, so the
                    // goblin army could not be summoned: the recipe (`recipes.rs`), the drop
                    // (`npc_drops.rs`) and the invasion (`on_summon`'s `-1`) were all already here
                    // and all three were unreachable behind this one missing arm.
                    //
                    // Three narrowings, each disclosed rather than invented. `Main.remixWorld` is
                    // not modelled anywhere in this server, so the distance gate is the whole gate.
                    // The sand exclusion stands in for vanilla's three `tileType == 53` arms above
                    // this one, which answer every dry sand tile first, so a beach and a desert keep
                    // their own daytime residents. And the critter arms above it are not modelled at
                    // all, which only makes him slightly more common here than in the game, exactly
                    // as with the Skeleton Merchant's arm below.
                    let surface_day = depth == Depth::Surface
                        && world.day_time
                        && !seasonal.graveyard
                        && !matches!(
                            biome,
                            Biome::Corruption | Biome::Crimson | Biome::Jungle | Biome::Dungeon
                        );
                    if surface_day
                        && ground_block != Some(SAND)
                        && !water_tile(world, x, y)
                        && (x - i32::from(world.spawn_x)).abs() > world.width() / 3
                        && (rng.random_range(0..GOBLIN_SCOUT_ODDS) == 0
                            || (!world.progress.downed_goblins
                                && world.progress.shadow_orb_smashed
                                && rng.random_range(0..GOBLIN_SCOUT_ORB_ODDS) == 0))
                    {
                        out.push((GOBLIN_SCOUT, (x as f32 * 16.0, y as f32 * 16.0)));
                        break;
                    }
                    // The surface night has a chain of its own ahead of the pool, and a graveyard
                    // reaches it in daylight too (`NPC.cs:4202`). It answers only where vanilla's
                    // `else if (surfaceSpawn)` (`NPC.cs:4168`) is reached: the corruption, the
                    // crimson, the jungle and the dungeon are all answered by earlier arms of the
                    // outer chain and never get here.
                    let seasonal_ground = depth == Depth::Surface
                        && (!world.day_time || seasonal.graveyard)
                        && !matches!(
                            biome,
                            Biome::Corruption | Biome::Crimson | Biome::Jungle | Biome::Dungeon
                        );
                    if seasonal_ground
                        && let Some(npc_type) =
                            seasonal_night_pick(seasonal, world.tile(x, y + 1).block, rng)
                    {
                        out.push((npc_type, (x as f32 * 16.0, y as f32 * 16.0)));
                        break;
                    }
                    // The caverns have a chain of their own ahead of their pool, and it is where
                    // October and a graveyard reach underground (`NPC.cs:5005-5199`). The biome
                    // exclusions are vanilla's own diversions above it: the two evils and the
                    // jungle answer at `NPC.cs:4066`, `:4125` and `:3929-3948` on their tiles, the
                    // dungeon has its own arm, and the snow's `:5088`/`:5100` both return before
                    // the season is ever asked. One tile read (the stone underfoot, for the marble
                    // and granite arms) plus three comparisons and at most six `rng` draws, on a
                    // path that already made several.
                    let cavern_chain = depth == Depth::Cavern
                        && !matches!(
                            biome,
                            Biome::Corruption
                                | Biome::Crimson
                                | Biome::Jungle
                                | Biome::Dungeon
                                | Biome::Snow
                        );
                    // `NPC.cs:5021`, the bottom half of the stone.
                    let lower_caverns = y > (i32::from(world.rock_layer) + world.height()) / 2;
                    let alive =
                        |ty: u16| npcs.iter().any(|(_, n)| n.npc_type == ty && n.is_alive());
                    if cavern_chain
                        && let Some(npc_type) = cavern_seasonal_pick(
                            seasonal,
                            no_worms,
                            lower_caverns,
                            world.tile(x, y + 1).block,
                            glowshroom_ground,
                            &alive,
                            rng,
                        )
                    {
                        out.push((npc_type, (x as f32 * 16.0, y as f32 * 16.0)));
                        break;
                    }
                    // The dungeon's hardmode chain, which sits ahead of the dungeon's ordinary
                    // fallthrough exactly as vanilla puts `NPC.cs:2661-2722` ahead of `:2723`. It
                    // opens only once Plantera is down in a hardmode world, which is `hardDungeon`
                    // (`NPC.cs:381`, `downedPlantBoss && Main.hardMode`), so it costs nothing at all
                    // before then and two tile reads after it.
                    if biome == Biome::Dungeon && events.hard_mode && events.downed_plantera {
                        let alive =
                            |ty: u16| npcs.iter().any(|(_, n)| n.npc_type == ty && n.is_alive());
                        let style = dungeon_brick_style(world, x, y, rng);
                        match hard_dungeon_pick(style, &alive, rng) {
                            Some(Some(npc_type)) => {
                                out.push((npc_type, (x as f32 * 16.0, y as f32 * 16.0)));
                                break;
                            }
                            // The caster arm fired and its one-at-a-time gate turned it away, which
                            // vanilla answers by returning with nothing spawned.
                            Some(None) => continue,
                            None => {}
                        }
                    }
                    // The dungeon library, `NPC.cs:2748-2771`:
                    //
                    // ```csharp
                    // bool flag13 = false;
                    // if (Main.rand.Next(8) == 0) {
                    //     Point bookPosition = Point.Zero;
                    //     if (AI_FindNearbyBook(new Point(spawnTileX - 16, spawnTileY - 16), 32, 32,
                    //             out bookPosition, closestBook: true, checkPlayerScreenRanges: true)) {
                    //         SpawnNPC(bookPosition.X * 16 + 8, bookPosition.Y * 16, 694, 0, 0f, 0f, 0f, 3f);
                    //         flag13 = true;
                    //     }
                    // } else if (Main.rand.Next(10) == 0) {
                    //     ...same, 693, and no ai...
                    // }
                    // int num43 = Main.rand.Next(5);
                    // if (flag13) return;
                    // ```
                    //
                    // Neither of the two is placed where the attempt found ground: both stand on the
                    // shelf, wherever in the box it turned out to be, which is why this pushes its
                    // own position rather than falling through to the draw below. With no shelf in
                    // the box `flag13` stays false and the attempt goes on to the ordinary
                    // fallthrough, so a corridor with no library in it is unaffected.
                    //
                    // The mimic's `3f` is its dormant state and is seeded by `NpcStore::spawn`,
                    // which is this server's `NewNPC` and the one door every NPC comes through.
                    //
                    // Two things about the placement are disclosed rather than matched. Vanilla
                    // draws `num43` before it reads `flag13`, which costs a roll off its own stream
                    // and nothing else; this server's `rng` is not that stream, so the dead draw is
                    // not reproduced. And the arm sits ahead of five rolls that vanilla makes first
                    // (`:2723-2747`: the Dungeon Slime, the two traps, the Cursed Skull and the Dark
                    // Caster) which this server folded into [`pool`]'s dungeon arm rather than
                    // keeping as a chain. So the library pair is met a little sooner here than in
                    // the game, where roughly three attempts in ten are taken before the roll is
                    // ever made. Never anywhere the game would not put them: the shelf test is what
                    // decides that, and it is exact.
                    if biome == Biome::Dungeon {
                        let library = if rng.random_ratio(1, WATER_BOLT_MIMIC_ODDS) {
                            Some(WATER_BOLT_MIMIC)
                        } else if rng.random_ratio(1, LIBRARIAN_ODDS) {
                            Some(LIBRARIAN_SKELETON)
                        } else {
                            None
                        };
                        // `spawnTileY` is the ground tile, which is this server's `y + 1`: the same
                        // one-row offset every other tile test in this module carries.
                        if let Some(npc_type) = library
                            && let Some((bx, by)) = find_nearby_book(
                                world,
                                (x - BOOK_SEARCH_BACK, y + 1 - BOOK_SEARCH_BACK),
                                (px, py),
                            )
                        {
                            out.push((npc_type, (bx as f32 * 16.0, by as f32 * 16.0)));
                            break;
                        }
                    }
                    // A graveyard's daylight draws the *night* pool, for the same reason: with the
                    // daytime block skipped, the chain below `NPC.cs:4202` is the one that answers,
                    // and its fallthrough is the zombie (`NPC.cs:4770-4816`), not the day's slime.
                    let day = world.day_time && !seasonal.graveyard;
                    let ordinary = pool(depth, biome, day);
                    // Hardmode adds to what a place had rather than replacing it, so a hardmode
                    // forest still has zombies in it. The underworld's additions wait for a
                    // mechanical boss, which is progression rather than place and so is held here.
                    let extra = if events.hard_mode
                        && (depth != Depth::Underworld || events.downed_mech_any)
                    {
                        hardmode_pool(depth, biome, day)
                    } else {
                        &[]
                    };
                    // ...and a blood moon widens the night on top of both.
                    let bloody = if world.blood_moon && !world.day_time {
                        blood_moon_pool(depth, events.hard_mode)
                    } else {
                        &[]
                    };
                    // The caverns also draw from the six this world happens to have, which is a
                    // world-specific list rather than a table. It counts as one entry in the
                    // draw, as the game counts it — not six — so a world's own monsters are a
                    // seasoning on the cavern pool rather than most of it. A sentinel stands in for
                    // it in the weighted pick below.
                    let world_specific = depth == Depth::Cavern && biome == Biome::Forest;
                    let mut candidates: Vec<u16> = Vec::with_capacity(
                        ordinary.len() + extra.len() + bloody.len() + usize::from(world_specific),
                    );
                    candidates.extend_from_slice(ordinary);
                    candidates.extend_from_slice(extra);
                    candidates.extend_from_slice(bloody);
                    if world_specific {
                        candidates.push(CAVERN_SENTINEL);
                    }
                    // `noWorms` (`NPC.cs:3704`): a town, or a wall at the player's back, keeps
                    // burrowers out. Dropping them from the draw rather than throwing the whole
                    // attempt away is what the game's `else if` chain amounts to: the branch is
                    // skipped and a later one answers instead.
                    if no_worms {
                        candidates.retain(|ty| !NO_WORMS_GATES.contains(ty));
                    }
                    let alive_count = |ty: u16| {
                        npcs.iter()
                            .filter(|(_, n)| n.npc_type == ty && n.is_alive())
                            .count()
                    };
                    let Some(ty) = choose_weighted(&candidates, &alive_count, rng) else {
                        continue;
                    };
                    if ty == CAVERN_SENTINEL {
                        events.cavern_monsters.pick(rng)
                    } else {
                        holiday_costume(ty, depth, seasonal, rng)
                    }
                }
            };

            // Position is the NPC's top-left, so it stands on the tile below.
            out.push((npc_type, (x as f32 * 16.0, y as f32 * 16.0)));
            break;
        }
        // `SpawnNPC` (`NPC.cs:291-306`) walks the player list and `break`s the moment
        // `TrySpawnAnNPC` returns true, so at most one NPC is spawned server-wide per tick however
        // many people are playing. Without this each player got their own draw, so a busy server
        // spawned monsters N times as fast as the game does.
        if !out.is_empty() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every NPC type this server can put into a world by itself, from every ambient producer it
    /// has, with no console command and nobody summoning anything.
    ///
    /// Built by *calling* the producers rather than by reading them, so it cannot drift from what
    /// they actually answer. It is the half of the reachability check that has to run here;
    /// vanilla's half needs the decompiled tree and lives in `tools/check_spawn_reach.py`, which
    /// runs this test to get this set. A type in vanilla's list and not in this one is an NPC
    /// nobody playing here can ever meet, which is exactly how the Harpy and the Wyvern went
    /// missing.
    ///
    /// Deliberately not counted as reachable, because none of them is ambient spawning: statues,
    /// the admin `/spawn` command, boss summon items, the transformations one NPC undergoes on
    /// another's death, and the segments a worm head grows behind itself.
    fn ambient_roster() -> std::collections::BTreeSet<u16> {
        use crate::game::{army, cavern_monsters, event::Invasion, lunar, moons, rescues};

        let mut set = std::collections::BTreeSet::new();
        const DEPTHS: [Depth; 4] = [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ];
        const BIOMES: [Biome; 9] = [
            Biome::Forest,
            Biome::Corruption,
            Biome::Crimson,
            Biome::Jungle,
            Biome::Snow,
            Biome::Desert,
            Biome::Ocean,
            Biome::Dungeon,
            Biome::Hallow,
        ];
        for depth in DEPTHS {
            for biome in BIOMES {
                for day in [true, false] {
                    set.extend(pool(depth, biome, day));
                    set.extend(hardmode_pool(depth, biome, day));
                    set.extend(friendly_pool(depth, biome, day));
                }
                set.extend(water_pool(depth, biome));
            }
            for hard_mode in [true, false] {
                set.extend(blood_moon_pool(depth, hard_mode));
            }
        }

        // The sky, asked through its own chain rather than listed, so deleting an arm of it shows
        // up here as a type that stopped being reachable.
        let mut sky = World::empty(800, 600, "roster");
        sky.progress.downed_golem = true;
        for seed in 0..200u64 {
            for hard_mode in [false, true] {
                for probe_gate in [false, true] {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    set.insert(sky_pick(
                        hard_mode,
                        probe_gate,
                        &sky,
                        false,
                        &|_| false,
                        &mut rng,
                    ));
                }
            }
        }
        // The two desert chains, asked through their own producers for the same reason the sky and
        // the pillars are: deleting an arm of one shows up here as a type that stopped being
        // reachable, rather than as nothing at all. Both are sampled across every input that
        // changes what they can answer - progression, the player's zone, whether burrowers are
        // allowed, and (for the sandstorm) which sand is underfoot.
        let mut desert = World::empty(800, 600, "roster");
        desert.surface = 200;
        desert.rock_layer = 300;
        for seed in 0..400u64 {
            for hard_mode in [false, true] {
                for biome in [
                    Biome::Forest,
                    Biome::Corruption,
                    Biome::Crimson,
                    Biome::Hallow,
                ] {
                    for no_worms in [false, true] {
                        // Two rows: one below `worldSurface + 100` where the worms are allowed, and
                        // one above it where they are not.
                        for spawn_y in [250, 350] {
                            let mut rng = SmallRng::seed_from_u64(seed);
                            set.insert(underground_desert_pick(
                                &desert,
                                spawn_y,
                                hard_mode,
                                biome,
                                no_worms,
                                &|_| 0,
                                &mut rng,
                            ));
                        }
                    }
                }
            }
            for hard_mode in [false, true] {
                for downed_boss1 in [false, true] {
                    for no_worms in [false, true] {
                        for ground in SAND_CONVERSION {
                            let mut rng = SmallRng::seed_from_u64(seed);
                            set.insert(
                                sandstorm_pick(
                                    hard_mode,
                                    downed_boss1,
                                    no_worms,
                                    ground,
                                    &|_| 0,
                                    &mut rng,
                                )
                                .0,
                            );
                        }
                    }
                }
            }
        }
        // The spider nest's two, asked through their own producer for the same reason: it is the
        // only place in the game either of them comes from.
        for hard_mode in [false, true] {
            for seed in 0..200u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                set.insert(spider_pick(hard_mode, &mut rng));
            }
        }
        // The surface night's own chain, asked through itself for the same reason the sky is:
        // deleting an arm shows up here as a type that stopped being reachable. Every combination
        // of the flags it reads, since each one opens arms the others do not, both moon phases it
        // names, and two floors, because the snow arm keys on the tile underfoot.
        for halloween in [false, true] {
            for xmas in [false, true] {
                for graveyard in [false, true] {
                    for hard_mode in [false, true] {
                        for blood_moon in [false, true] {
                            for raining in [false, true] {
                                for moon_phase in [0u8, 4] {
                                    let at = Seasonal {
                                        halloween,
                                        xmas,
                                        graveyard,
                                        hard_mode,
                                        blood_moon,
                                        day_time: false,
                                        moon_phase,
                                        raining,
                                    };
                                    for ground_block in [2u16, 147] {
                                        for seed in 0..4_000u64 {
                                            let mut rng = SmallRng::seed_from_u64(seed);
                                            set.extend(seasonal_night_pick(
                                                at,
                                                ground_block,
                                                &mut rng,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // ...and the caverns' own chain, asked the same way. Both `no_worms` states, because the
        // Ghost's arm is gated on one of them, and both halves of the stone, because Tim's is.
        // Three floors, because two of the arms key on the stone underfoot: plain rock, marble
        // (367) and granite (368).
        for halloween in [false, true] {
            for graveyard in [false, true] {
                for hard_mode in [false, true] {
                    let at = Seasonal {
                        halloween,
                        graveyard,
                        hard_mode,
                        ..Seasonal::default()
                    };
                    for no_worms in [false, true] {
                        for lower_caverns in [false, true] {
                            // Both halves of the chain have a glowshroom arm, and each is the only
                            // source of its own Spore, so the flag is sampled like the rest.
                            for glowshroom in [false, true] {
                                for ground_block in [1u16, 367, 368] {
                                    for seed in 0..4_000u64 {
                                        let mut rng = SmallRng::seed_from_u64(seed);
                                        set.extend(cavern_seasonal_pick(
                                            at,
                                            no_worms,
                                            lower_caverns,
                                            ground_block,
                                            glowshroom,
                                            &|_| false,
                                            &mut rng,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ...and hallowed ground's own chain, asked the same way. Every flag it reads opens an arm
        // the others do not: Plantera down and the first half of the night for the Lacewing, rain
        // for the Rainbow Slime, and the surface for the Lacewing again. With nothing alive, since
        // two of its arms are gated on a census of themselves.
        for downed_plantera in [false, true] {
            for day_time in [false, true] {
                for raining in [false, true] {
                    for surface_spawn in [false, true] {
                        for seed in 0..4_000u64 {
                            let mut rng = SmallRng::seed_from_u64(seed);
                            set.extend(hallow_ground_pick(
                                downed_plantera,
                                day_time,
                                0,
                                surface_spawn,
                                raining,
                                &|_| false,
                                &mut rng,
                            ));
                        }
                    }
                }
            }
        }

        // ...and the hardmode dungeon's, asked the same way. All three brick styles, because three
        // quarters of that roster is chosen by which one a spot is built from, and with nothing
        // alive, because two of its arms are gated on a census of themselves.
        for style in 0..3u8 {
            for seed in 0..4_000u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                set.extend(hard_dungeon_pick(style, &|_| false, &mut rng).flatten());
            }
        }

        // The Glowing Mushroom roster, asked through its own chain for the same reason the sky and
        // the deserts are. Both arms, and both progression states, since the underground one is
        // hardmode-only and the surface one gives its snail a second chance before hardmode.
        for surface in [false, true] {
            for hard_mode in [false, true] {
                for seed in 0..4_000u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    set.extend(mushroom_pick(surface, hard_mode, &mut rng));
                }
            }
        }

        // The holiday costumes, which swap a critter or a plain slime for a dressed-up one after it
        // is drawn rather than sitting in a pool of their own.
        for at in [
            Seasonal {
                halloween: true,
                ..Seasonal::default()
            },
            Seasonal {
                xmas: true,
                ..Seasonal::default()
            },
        ] {
            for base in [46, 1] {
                for seed in 0..200u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    set.insert(holiday_costume(base, Depth::Surface, at, &mut rng));
                }
            }
        }

        // ...and what a friendly attempt draws in a graveyard, which is its own two-entry list
        // rather than an arm of `friendly_pool`.
        set.extend(GRAVEYARD_VERMIN);

        // The frog's own chain, asked through itself for the same reason the sky is: it is where
        // the bound Yellow Slime lives, and deleting its arm should show up here as a type that
        // stopped being reachable rather than as nothing at all.
        let jungle = World::empty(800, 600, "roster");
        for seed in 0..200u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            set.insert(spawn_frog(&jungle, &|_| false, &mut rng));
        }

        // The dungeon's doorman, and the residents found tied up underground.
        set.insert(DUNGEON_GUARDIAN);
        set.extend(rescues::RESCUES.iter().map(|r| r.bound));
        // The bound Old Slime is found by the same underground path but is not a talk-rescue, so
        // it is not in that table (`pick_bound` chains it in by hand, and `bound_gate` is what
        // limits it to the caverns). The bound Purple Slime is not listed here at all: it comes
        // out of `sky_pick` above, which is where it actually lives.
        set.insert(BOUND_TOWN_SLIME_OLD);
        // The two `try_spawn` answers with a branch of their own rather than a pool entry: the
        // Tortured Soul the underworld offers once per world, and the Skeleton Merchant the caverns
        // offer one attempt in seventy. Both are asserted to be reachable by `try_spawn` itself in
        // this module's own tests, so listing them here cannot drift into a claim their branches
        // have stopped making good.
        set.insert(TORTURED_SOUL);
        set.insert(SKELETON_MERCHANT);
        // ...and the two more with a branch of their own: the Goblin Scout the surface day offers
        // out past a third of the map from spawn, and the Statue Mimic a graveyard offers on a
        // plinth once Skeletron is down. Both are asserted reachable through `try_spawn` itself by
        // this module's own tests, so listing them here cannot drift into a claim that their
        // branches still make good.
        set.insert(GOBLIN_SCOUT);
        set.insert(STATUE_MIMIC);
        // ...and the Rock Golem the hardmode caverns cut out of stone or moss, whose gate is a tile
        // and a ceiling rather than a pool. Asserted reachable through `try_spawn` itself by this
        // module's own tests, so listing it here cannot drift into a claim its branch still makes
        // good.
        set.insert(ROCK_GOLEM);
        // ...and the temple's two, whose branch is a one-in-three fork with no roster behind it
        // (`NPC.cs:3914-3924`). `try_spawn` reaching both is asserted by this module's own tests,
        // so listing them here cannot drift into a claim the branch still makes good.
        set.insert(LIHZAHRD);
        set.insert(FLYING_SNAKE);
        // ...and the dungeon library's pair, which are a branch rather than a pool entry for a
        // reason no other arm has: neither stands where the attempt found ground, they stand on the
        // bookshelf `find_nearby_book` picked (`NPC.cs:2748-2766`). Both are asserted reachable
        // through `try_spawn` itself by this module's own tests.
        set.insert(WATER_BOLT_MIMIC);
        set.insert(LIBRARIAN_SKELETON);
        // ...and the Meteor Head, whose branch has no roll and no roster at all: standing in a
        // crater, it is the one thing that comes (`NPC.cs:2796-2799`). `try_spawn` reaching it is
        // asserted by this module's own tests, so listing it here cannot drift into a claim the
        // branch still makes good.
        set.insert(METEOR_HEAD);
        // The six a world happens to have are drawn from thirteen, so every world's own set counts.
        for world_id in 0..200 {
            set.extend(cavern_monsters::CavernMonsters::for_world(world_id).flat());
        }
        // King Slime arrives on his own during a slime rain, which nothing else summons.
        set.insert(crate::game::slime_rain::KING_SLIME);

        // The four lunar pillar zones, asked through their own producer for the same reason the
        // sky is asked through its own chain: deleting an arm of one shows up here as a type that
        // stopped being reachable.
        for pillar in lunar::PILLARS {
            for seed in 0..200u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                set.extend(tower_pool(pillar, &|_| 0, &mut rng));
            }
        }

        // The rosters that already carry a membership test are read through it, which is exact
        // where sampling their spawn functions would only be likely.
        //
        // A membership test is only allowed to stand in for a roster when something actually
        // spawns from that roster. Each of the three below has a real path: the moons through
        // `moons::moon_spawn` in `spawn_at`, invasions through the invasion spawner, and the Old
        // One's Army through `apply_army`.
        //
        // `lunar::belongs_to` was here too and stays out, however reachable the pillar escort has
        // since become. It exists to classify a kill (`Lunar::note_kill`, which drops a pillar's
        // shield) and to decide what despawns when the event ends, and counting it as reachability
        // made the emptiest fight in the game invisible to the one tool built to find exactly that.
        // The escort is above, drawn from [`tower_pool`], which is what actually spawns it.
        for npc_type in 0..terrustia_proto::npc_data::NPC_COUNT {
            if moons::moon_points(npc_type) > 0
                || army::belongs(npc_type)
                || [
                    Invasion::Goblin,
                    Invasion::FrostLegion,
                    Invasion::Pirate,
                    Invasion::Martian,
                ]
                .into_iter()
                .any(|kind| belongs_to(kind, npc_type))
            {
                set.insert(npc_type);
            }
        }

        // The eclipse has no membership table, so its own function is asked directly, enough times
        // and under both progression states for every arm of it to have answered.
        for seed in 0..4000u64 {
            for (plantera, mechs) in [(false, false), (true, true)] {
                let mut rng = SmallRng::seed_from_u64(seed);
                set.insert(crate::game::moons::eclipse_spawn(
                    plantera,
                    mechs,
                    &|_| 0,
                    &mut rng,
                ));
            }
        }
        set
    }

    /// The reachable set, printed for `tools/check_spawn_reach.py` and checked for the one thing
    /// that needs no decompiled tree: that every producer names a type that exists and can be
    /// spawned at somebody.
    #[test]
    fn every_ambient_producer_names_a_real_type() {
        use terrustia_proto::npc_data::npc_stats;

        let roster = ambient_roster();
        assert!(roster.len() > 150, "only {} types reachable", roster.len());
        for npc_type in &roster {
            assert!(
                npc_stats(*npc_type).is_some(),
                "{npc_type} is reachable from a spawn producer but is not an NPC type",
            );
        }
        println!(
            "SPAWN-REACH {}",
            roster
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    /// Nothing running: what the ordinary world looks like to `try_spawn`.
    fn quiet() -> EventSpawns<'static> {
        EventSpawns {
            moon: None,
            eclipse: false,
            sandstorm: false,
            downed_plantera: false,
            downed_all_mechs: false,
            boss_cap: false,
            any_danger: false,
            hard_mode: false,
            downed_mech_any: false,
            census: &|_| 0,
            cavern_monsters: crate::game::cavern_monsters::CavernMonsters::for_world(7),
            towers: [None; 4],
        }
    }
    use crate::world::worldgen;
    use rand::SeedableRng;

    fn test_world() -> World {
        worldgen::generate(800, 600, "spawn", 7)
    }

    #[test]
    fn depth_bands_follow_the_world_layers() {
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;
        assert_eq!(depth_at(&world, 100), Depth::Surface);
        assert_eq!(depth_at(&world, 250), Depth::Underground);
        assert_eq!(depth_at(&world, 350), Depth::Cavern);
        assert_eq!(depth_at(&world, 599 - 1), Depth::Underworld);
    }

    /// The rate bands sit a screen height (75 tiles) below the layers the pool bands use
    /// (`NPC.cs:487`, `:508`, `sHeight => 1200` at `:6793`).
    ///
    /// Fails before the fix, when both questions were answered by `depth_at`: every rate band was
    /// 75 tiles too shallow, roughly doubling the spawn rate through the dirt-layer band.
    #[test]
    fn the_rate_bands_sit_a_screen_height_below_the_pool_bands() {
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;

        // Just below the surface line is still the surface for rate purposes, and already
        // underground for pool purposes.
        assert_eq!(depth_at(&world, 210), Depth::Underground);
        assert_eq!(rate_depth_at(&world, 210), Depth::Surface);
        assert_eq!(rate_depth_at(&world, 275), Depth::Surface);
        assert_eq!(rate_depth_at(&world, 276), Depth::Underground);

        // The same 75 tiles again at the rock layer.
        assert_eq!(depth_at(&world, 310), Depth::Cavern);
        assert_eq!(rate_depth_at(&world, 310), Depth::Underground);
        assert_eq!(rate_depth_at(&world, 375), Depth::Underground);
        assert_eq!(rate_depth_at(&world, 376), Depth::Cavern);

        // The underworld boundary carries no offset in the game either, so the two agree there.
        let underworld = world.height() - UNDERWORLD_DEPTH;
        assert_eq!(rate_depth_at(&world, underworld), Depth::Underworld);
        assert_eq!(depth_at(&world, underworld), Depth::Underworld);
    }

    /// Every hardmode pool names real, hostile types, and each biome's are its own.
    #[test]
    fn the_hardmode_pools_are_real_and_placed_right() {
        use terrustia_proto::npc_data::npc_stats;
        let mut anywhere = std::collections::HashSet::new();
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            for biome in [
                Biome::Forest,
                Biome::Corruption,
                Biome::Crimson,
                Biome::Jungle,
                Biome::Snow,
                Biome::Desert,
                Biome::Ocean,
                Biome::Dungeon,
                Biome::Hallow,
            ] {
                for day in [true, false] {
                    for npc_type in hardmode_pool(depth, biome, day) {
                        let stats = npc_stats(*npc_type)
                            .unwrap_or_else(|| panic!("{npc_type} in {biome:?} is not a type"));
                        assert!(
                            !stats.friendly && !stats.town_npc,
                            "{} is friendly and should not be spawned at anyone",
                            stats.name
                        );
                        anywhere.insert(*npc_type);
                    }
                }
            }
        }
        assert!(
            anywhere.len() > 40,
            "only {} hardmode types",
            anywhere.len()
        );
    }

    /// The hallow is empty before hardmode and full after it.
    #[test]
    fn the_hallow_only_lives_in_hardmode() {
        // Before: it borrows the forest's ordinary life rather than being barren.
        assert_eq!(
            pool(Depth::Surface, Biome::Hallow, true),
            pool(Depth::Surface, Biome::Forest, true),
        );
        // After: it has its own, and nothing it has is the forest's.
        let hallow = hardmode_pool(Depth::Surface, Biome::Hallow, true);
        assert!(!hallow.is_empty());
        let forest = hardmode_pool(Depth::Surface, Biome::Forest, true);
        assert!(
            hallow.iter().all(|t| !forest.contains(t)),
            "the hallow is sharing the forest's hardmode life"
        );
    }

    /// A blood moon widens the night rather than replacing it, and only on the surface.
    #[test]
    fn a_blood_moon_widens_the_night() {
        use terrustia_proto::npc_data::npc_stats;
        let early = blood_moon_pool(Depth::Surface, false);
        let late = blood_moon_pool(Depth::Surface, true);
        assert!(!early.is_empty());
        assert!(late.len() > early.len(), "hardmode adds the Clown");
        assert!(late.contains(&109), "the Clown");
        assert!(!early.contains(&109), "but not before hardmode");
        for npc_type in late {
            assert!(npc_stats(*npc_type).is_some(), "{npc_type} is not a type");
        }
        // Underground is untouched: a blood moon is a thing that happens to the sky.
        for depth in [Depth::Underground, Depth::Cavern, Depth::Underworld] {
            assert!(blood_moon_pool(depth, true).is_empty(), "{depth:?}");
        }
    }

    /// The two evils get different creatures, not the same list twice.
    #[test]
    fn the_evils_are_not_the_same_list() {
        for depth in [Depth::Surface, Depth::Cavern] {
            let corrupt = hardmode_pool(depth, Biome::Corruption, false);
            let crimson = hardmode_pool(depth, Biome::Crimson, false);
            assert!(!corrupt.is_empty() && !crimson.is_empty());
            assert!(
                corrupt.iter().all(|t| !crimson.contains(t)),
                "the two evils share {depth:?} spawns"
            );
        }
    }

    #[test]
    fn every_pool_is_non_empty_and_known() {
        // A pool that names an NPC this build does not define would spawn nothing at all.
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            for biome in [
                Biome::Forest,
                Biome::Corruption,
                Biome::Crimson,
                Biome::Jungle,
                Biome::Snow,
                Biome::Desert,
                Biome::Ocean,
                Biome::Dungeon,
            ] {
                for day in [true, false] {
                    let types = pool(depth, biome, day);
                    assert!(!types.is_empty(), "{depth:?}/{biome:?} day={day} is empty");
                    for t in types {
                        assert!(
                            terrustia_proto::npc_data::npc_stats(*t).is_some(),
                            "{depth:?}/{biome:?} names unknown NPC {t}"
                        );
                    }
                }
            }
        }
    }

    /// Every id in a hostile pool is a monster, never a damage-0 critter (`NPC.cs`: critters spawn
    /// down the `spawnFriendly` path, never at the player). Fails before the fix, when the day
    /// forest listed the bunny, bird, squirrel and frog and the ocean listed the goldfish, all
    /// damage 0, so a base under attack could be "attacked" by a bunny.
    #[test]
    fn no_hostile_pool_names_a_critter() {
        use terrustia_proto::npc_data::npc_stats;
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            for biome in [
                Biome::Forest,
                Biome::Corruption,
                Biome::Crimson,
                Biome::Jungle,
                Biome::Snow,
                Biome::Desert,
                Biome::Ocean,
                Biome::Dungeon,
                Biome::Hallow,
            ] {
                for day in [true, false] {
                    for t in pool(depth, biome, day) {
                        let stats = npc_stats(*t).expect("a real type");
                        assert!(
                            stats.damage > 0,
                            "{depth:?}/{biome:?} lists the critter {} in its hostile pool",
                            stats.name
                        );
                    }
                }
            }
        }
    }

    /// The friendly-critter table `rates` deferred names only real, harmless critters. Every entry
    /// is a defined type and every one has zero contact damage, so the friendly fork can never hand
    /// back a monster.
    #[test]
    fn spawn_friendly_lists_only_real_critters() {
        use terrustia_proto::npc_data::npc_stats;
        let mut saw_some = false;
        for depth in [
            Depth::Surface,
            Depth::Underground,
            Depth::Cavern,
            Depth::Underworld,
        ] {
            for biome in [
                Biome::Forest,
                Biome::Corruption,
                Biome::Crimson,
                Biome::Jungle,
                Biome::Snow,
                Biome::Desert,
                Biome::Ocean,
                Biome::Dungeon,
                Biome::Hallow,
            ] {
                for day in [true, false] {
                    for t in friendly_pool(depth, biome, day) {
                        saw_some = true;
                        let stats = npc_stats(*t).expect("a real critter type");
                        assert_eq!(
                            stats.damage, 0,
                            "{depth:?}/{biome:?} friendly table names the monster {}",
                            stats.name
                        );
                    }
                }
            }
        }
        assert!(saw_some, "the friendly table is empty everywhere");
    }

    /// The underworld draws its heavies at the game's rates, not a flat uniform share. The Voodoo
    /// Demon is a roughly-one-in-seventy roll in the game (`NPC.cs:4893-4897`); a six-way uniform
    /// pick handed it out one time in six. Sampling the weighted pick, the Voodoo Demon stays under
    /// a twentieth and the Hellbat (the cascade's fallthrough) is the plurality. Fails before the
    /// fix: a uniform pick puts every underworld type at one in six.
    #[test]
    fn the_underworld_draws_a_voodoo_demon_rarely() {
        // The underworld pool, minus the capped Bone Serpent (uncapped here, count always 0).
        let underworld = pool(Depth::Underworld, Biome::Forest, true);
        let none_alive = |_: u16| 0usize;
        let mut rng = SmallRng::seed_from_u64(99);
        let mut voodoo = 0u32;
        let mut hellbat = 0u32;
        const N: u32 = 60_000;
        for _ in 0..N {
            match choose_weighted(underworld, &none_alive, &mut rng) {
                Some(66) => voodoo += 1,
                Some(60) => hellbat += 1,
                _ => {}
            }
        }
        assert!(
            voodoo < N / 20,
            "voodoo demons drawn {voodoo}/{N}, far more than the game's ~1/70",
        );
        assert!(
            hellbat > voodoo * 10,
            "the hellbat should dominate the underworld: {hellbat} vs {voodoo} voodoo",
        );
    }

    /// A type already at its active cap is never drawn (`active_cap`; the game's `!AnyNPCs(39)` on
    /// the Bone Serpent, `NPC.cs:4885`). Fails before the fix, when a uniform pick had no notion of
    /// a cap at all and would start a second serpent while the first was alive.
    #[test]
    fn a_capped_type_is_never_drawn_while_at_its_cap() {
        let underworld = pool(Depth::Underworld, Biome::Forest, true);
        assert!(
            underworld.contains(&39),
            "the underworld has the bone serpent"
        );
        // One bone serpent already alive: it is at its cap of one.
        let serpent_alive = |t: u16| usize::from(t == 39);
        let mut rng = SmallRng::seed_from_u64(7);
        for _ in 0..20_000 {
            let drawn = choose_weighted(underworld, &serpent_alive, &mut rng);
            assert_ne!(drawn, Some(39), "drew a second bone serpent past its cap");
        }
    }

    #[test]
    fn surface_forest_swaps_between_day_and_night() {
        let day = pool(Depth::Surface, Biome::Forest, true);
        let night = pool(Depth::Surface, Biome::Forest, false);
        assert!(day.contains(&1), "slimes by day");
        assert!(night.contains(&3), "zombies by night");
        assert!(!night.contains(&46), "no bunnies at night");
    }

    #[test]
    fn the_ocean_is_decided_by_position_not_tiles() {
        let world = test_world();
        assert_eq!(biome_at(&world, 10, 100), Biome::Ocean);
        assert_eq!(biome_at(&world, world.width() - 10, 100), Biome::Ocean);
    }

    /// Fill a `w`-by-`h` block of one tile type with its top-left `dx`,`dy` from a centre.
    #[allow(clippy::too_many_arguments)]
    fn fill_block(
        world: &mut World,
        cx: i32,
        cy: i32,
        dx: i32,
        dy: i32,
        w: i32,
        h: i32,
        block: u16,
    ) {
        use terrustia_proto::tile::Tile;
        for yy in 0..h {
            for xx in 0..w {
                world.set_tile(cx + dx + xx, cy + dy + yy, Tile::block(block));
            }
        }
    }

    /// Blank the whole 169x124 scan box (with a margin) to plain dirt, so what the scan reads is
    /// only what a test then paints on. The generated test world is a cramped 800 wide, close
    /// enough that its evil band sometimes sits inside the box around the middle; that is a fact
    /// about a tiny world, not the scan, so a test that wants a clean forest makes one.
    fn plain_box(world: &mut World, cx: i32, cy: i32) {
        use terrustia_proto::tile::Tile;
        for dy in -70..=70 {
            for dx in -90..=90 {
                world.set_tile(cx + dx, cy + dy, Tile::block(0)); // dirt
            }
        }
    }

    #[test]
    fn plain_terrain_reads_as_forest() {
        let mut world = test_world();
        let (cx, cy) = (world.width() / 2, i32::from(world.surface) + 30);
        plain_box(&mut world, cx, cy);
        assert_eq!(biome_at(&world, cx, cy), Biome::Forest);
    }

    /// The biome scan reads the game's 169x124 box against the game's per-biome thresholds, not a
    /// 41x41 box against a flat 60 (`SceneMetrics.cs:16`,`24-58`). Fails before the fix on two
    /// counts, each its own assertion below: a 100-tile pocket of corruption used to *be*
    /// corruption (the flat 60 threshold, now 300), and corruption sitting thirty tiles out was
    /// invisible (past the old radius-20 box, inside the new one).
    #[test]
    fn the_biome_scan_uses_the_games_box_and_thresholds() {
        const EBONSTONE: u16 = 23;
        let base = test_world();
        let cx = base.width() / 2;
        let cy = i32::from(base.surface) + 40;

        // A clean forest, then a pocket of 100 corrupt tiles (10x10): over the old flat 60, under
        // the real 300, so this must now read as forest where it used to read as corruption.
        let mut world = test_world();
        plain_box(&mut world, cx, cy);
        assert_eq!(
            biome_at(&world, cx, cy),
            Biome::Forest,
            "baseline is forest"
        );
        fill_block(&mut world, cx, cy, -5, -5, 10, 10, EBONSTONE);
        assert_eq!(
            biome_at(&world, cx, cy),
            Biome::Forest,
            "a 100-tile pocket is under the 300 corruption threshold",
        );

        // A genuine 400-tile corruption (20x20) does read as corruption.
        let mut world = test_world();
        plain_box(&mut world, cx, cy);
        fill_block(&mut world, cx, cy, -10, -10, 20, 20, EBONSTONE);
        assert_eq!(biome_at(&world, cx, cy), Biome::Corruption);

        // 400 corrupt tiles placed entirely thirty-plus tiles to the right are inside the game's
        // box but were outside the old radius-20 one: they must be counted, so this reads as
        // corruption where the old scan saw an empty forest.
        let mut world = test_world();
        plain_box(&mut world, cx, cy);
        fill_block(&mut world, cx, cy, 25, -10, 20, 20, EBONSTONE);
        assert_eq!(
            biome_at(&world, cx, cy),
            Biome::Corruption,
            "the scan must reach past the old radius-20 box",
        );
    }

    /// Run a stretch of spawn ticks for one player standing at a tile, and report what was drawn.
    ///
    /// The NPC store is deliberately left empty between ticks, as the other `try_spawn` tests
    /// leave it: nothing accumulates, so the near-player cap never closes and a run of ticks is a
    /// run of independent attempts.
    fn spawns_at(world: &World, hard_mode: bool, px: i32, py: i32, ticks: u32) -> Vec<u16> {
        spawns_at_in(world, hard_mode, false, px, py, ticks)
    }

    /// The same, for a player who is (or is not) standing in a graveyard.
    ///
    /// The graveyard is a bit in the zone packet the client last sent, exactly as it reaches a real
    /// server: `[id][zone1][zone2][zone3][zone4][zone5][townNPCs]` with `ZoneGraveyard` at
    /// `zone4[6]` (`NetMessage.cs:936-946`, `Player.cs:3771`). Setting the bit here is what a client
    /// standing among twenty-eight tombstones would send.
    fn spawns_at_in(
        world: &World,
        hard_mode: bool,
        graveyard: bool,
        px: i32,
        py: i32,
        ticks: u32,
    ) -> Vec<u16> {
        spawns_with_zone(
            world,
            &EventSpawns {
                hard_mode,
                ..quiet()
            },
            graveyard,
            px,
            py,
            ticks,
        )
        .into_iter()
        .map(|(npc_type, _)| npc_type)
        .collect()
    }

    /// The same with the events under the test's control, and the position kept: a sandstorm has to
    /// be switched on from outside, and one of its members does not stand where it was chosen.
    fn spawns_with(
        world: &World,
        events: &EventSpawns<'_>,
        px: i32,
        py: i32,
        ticks: u32,
    ) -> Vec<(u16, (f32, f32))> {
        spawns_with_zone(world, events, false, px, py, ticks)
    }

    /// The one body the three helpers above are views onto.
    fn spawns_with_zone(
        world: &World,
        events: &EventSpawns<'_>,
        graveyard: bool,
        px: i32,
        py: i32,
        ticks: u32,
    ) -> Vec<(u16, (f32, f32))> {
        let npcs = NpcStore::new();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (px as f32 * 16.0, py as f32 * 16.0);
        if graveyard {
            player.zone = Some(bytes::Bytes::from_static(&[0, 0, 0, 0, 1 << 6, 0, 0]));
        }
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(20260830);
        let mut seen = Vec::new();
        let mut biomes = BiomeCache::default();
        for _ in 0..ticks {
            seen.extend(try_spawn(
                world,
                &npcs,
                &players,
                events,
                &JourneyPowers::default(),
                &mut biomes,
                &mut rng,
            ));
        }
        seen
    }

    /// A plain forest surface and a place to stand on it.
    ///
    /// Deliberately not `test_world`: the generated world's own spawn point for seed 7 sits in the
    /// corruption, whose surface is answered by an earlier arm of vanilla's chain (`NPC.cs:4125`)
    /// and never reaches the seasonal one. A flat stone floor above the surface line is the
    /// simplest thing `biome_at` calls a forest, which is what these tests are about.
    fn forest_surface() -> (World, (i32, i32)) {
        let world = flat_world(90);
        let at = (400, 88);
        assert_eq!(biome_at(&world, at.0, at.1), Biome::Forest);
        assert_eq!(depth_at(&world, at.1), Depth::Surface);
        (world, at)
    }

    /// The same, after dark.
    fn night_world() -> (World, (i32, i32)) {
        let (mut world, at) = forest_surface();
        world.day_time = false;
        (world, at)
    }

    /// A graveyard owns the surface night: the Ghost, the Groom, the Bride and the Maggot Zombie
    /// appear nowhere else on an ordinary night (`NPC.cs:4544`, `:4623`, `:4628`, `:4717`), and
    /// before this the server had no notion of a graveyard at all, so none of the four was in any
    /// producer and nobody could ever meet one.
    ///
    /// Neutralised by making `seasonal_night_pick` return `None` on its first line: every one of
    /// the four assertions below fails, and the same run with `graveyard: false` is unaffected,
    /// which is the other half of the claim.
    #[test]
    fn a_graveyard_brings_out_the_ghosts_and_the_wedding() {
        let (world, (px, py)) = night_world();
        let seen = spawns_at_in(&world, false, true, px, py, 200_000);
        let found: std::collections::BTreeSet<u16> = seen.iter().copied().collect();
        for (npc_type, name) in [
            (316u16, "Ghost"),
            (53, "TheGroom"),
            (536, "TheBride"),
            (632, "MaggotZombie"),
            (301, "Raven"),
        ] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) in a graveyard: {found:?}"
            );
        }

        // And none of them anywhere else, which is what makes the graveyard the reason they came.
        let ordinary = spawns_at_in(&world, false, false, px, py, 200_000);
        let ordinary: std::collections::BTreeSet<u16> = ordinary.iter().copied().collect();
        for npc_type in [316u16, 53, 536, 632, 301] {
            assert!(
                !ordinary.contains(&npc_type),
                "{npc_type} turned up on an ordinary night: {ordinary:?}"
            );
        }
    }

    /// A graveyard runs the *night* roster in broad daylight: `NPC.cs:4202`'s daytime block is
    /// gated on `!ZoneGraveyard`, so standing among tombstones skips it entirely and drops through
    /// to the chain below, whose fallthrough is the zombie rather than the day's blue slime.
    ///
    /// Neutralised by dropping the `&& !seasonal.graveyard` from the `day` binding in `try_spawn`:
    /// the daylit graveyard then draws the day pool and no zombie appears, failing the assertion.
    #[test]
    fn a_graveyard_is_dark_at_noon() {
        let (world, (px, py)) = forest_surface();
        assert!(world.day_time, "an untouched world starts in daylight");
        let seen = spawns_at_in(&world, false, true, px, py, 20_000);
        assert!(
            seen.contains(&3),
            "no zombies in a daylit graveyard: {:?}",
            seen.iter().collect::<std::collections::BTreeSet<_>>()
        );
    }

    /// A town in a graveyard has vermin instead of songbirds: the friendly branch's own graveyard
    /// arm (`NPC.cs:2101-2115`) sits ahead of every other, so a Maggot or a Rat is the *only* thing
    /// a friendly attempt can draw there.
    ///
    /// Neutralised by dropping the `if seasonal.graveyard` fork in `try_spawn`'s friendly arm, so
    /// both cases read `friendly_pool`: no Maggot or Rat is drawn in an hour of ticks and the first
    /// assertion fails. The second assertion is what makes it a *replacement* rather than an
    /// addition, and it fails the same way in reverse if the arm is made additive.
    #[test]
    fn a_town_among_the_tombstones_draws_vermin_and_nothing_else() {
        const GUIDE: u16 = 22;
        let (world, (px, py)) = forest_surface();
        let mut npcs = NpcStore::new();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (px as f32 * 16.0, py as f32 * 16.0);
        player.zone = Some(bytes::Bytes::from_static(&[0, 0, 0, 0, 1 << 6, 0, 0]));
        // Three townsfolk right where the player stands, which is what turns `spawnFriendly` on.
        for _ in 0..3 {
            npcs.spawn(GUIDE, player.position);
        }
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(21);
        let mut biomes = BiomeCache::default();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..20_000 {
            for (npc_type, _) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut biomes,
                &mut rng,
            ) {
                seen.insert(npc_type);
            }
        }
        for vermin in GRAVEYARD_VERMIN {
            assert!(
                seen.contains(&vermin),
                "no {vermin} in a graveyard: {seen:?}"
            );
        }
        for songbird in friendly_pool(Depth::Surface, Biome::Forest, true) {
            assert!(
                !seen.contains(songbird),
                "the ordinary critter {songbird} should not reach a graveyard: {seen:?}"
            );
        }
    }

    /// Halloween is a real-world date, and it dresses the night up: the Raven, the two costumed
    /// demon eyes and the three costumed zombies (`NPC.cs:4539`, `:4561`, `:4734`).
    ///
    /// Neutralised by forcing `Seasonal::halloween` to `false` where `try_spawn` builds it: none of
    /// the six types below is drawn in two hundred thousand ticks.
    #[test]
    fn halloween_dresses_the_night_up() {
        let (mut world, (px, py)) = night_world();
        world.halloween = true;
        let seen = spawns_at_in(&world, false, false, px, py, 200_000);
        let found: std::collections::BTreeSet<u16> = seen.iter().copied().collect();
        for (npc_type, name) in [
            (301u16, "Raven"),
            (317, "DemonEyeOwl"),
            (318, "DemonEyeSpaceship"),
            (319, "ZombieDoctor"),
            (320, "ZombieSuperman"),
            (321, "ZombiePixie"),
        ] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) at Halloween: {found:?}"
            );
        }
    }

    /// The caverns have a season too, and it is a different chain from the surface's: October
    /// dresses the skeletons up (`NPC.cs:5115`, `Main.rand.Next(322, 325)`) and October *or* a
    /// graveyard puts a Ghost in the stone (`NPC.cs:5078`, unlike the surface's `:4544`, which is
    /// the graveyard's alone).
    ///
    /// Neutralised arm by arm, each run and each failing its own assertion:
    ///
    /// * dropping the `NPC.cs:5115` branch: "no SkeletonTopHat (322) in an October cavern: {316}".
    /// * cutting `:5078`'s condition down to `at.graveyard` alone: "no Ghost (316) in an October
    ///   cavern: {322, 323, 324}".
    /// * cutting it down to `at.halloween` alone instead: "no Ghost in an underground graveyard".
    /// * dropping `!no_worms` from `:5078`: "a Ghost got past `noWorms`".
    /// * dropping `lower_caverns` from `:5021`: "an ordinary cavern answered the seasonal chain",
    ///   Tim having reached the upper stone.
    /// * dropping the `:5051` hardmode gate: "hardmode should reach this chain far less often:
    ///   6879 vs 6879".
    #[test]
    fn the_caverns_have_a_season_of_their_own() {
        // Plain stone underfoot: the two arms that key on the floor are the stone test below.
        let sample = |at: Seasonal, no_worms: bool, lower_caverns: bool| {
            (0..40_000u64)
                .filter_map(|seed| {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    cavern_seasonal_pick(
                        at,
                        no_worms,
                        lower_caverns,
                        1,
                        false,
                        &|_| false,
                        &mut rng,
                    )
                })
                .collect::<Vec<u16>>()
        };
        let set = |v: Vec<u16>| v.into_iter().collect::<std::collections::BTreeSet<u16>>();
        let october = Seasonal {
            halloween: true,
            ..Seasonal::default()
        };
        let graveyard = Seasonal {
            graveyard: true,
            ..Seasonal::default()
        };
        let plain = Seasonal::default();

        let found = set(sample(october, false, false));
        for (npc_type, name) in [
            (316u16, "Ghost"),
            (322, "SkeletonTopHat"),
            (323, "SkeletonAstonaut"),
            (324, "SkeletonAlien"),
        ] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) in an October cavern: {found:?}"
            );
        }

        // A graveyard underground is the Ghost and only the Ghost: the costumes are the calendar's.
        let found = set(sample(graveyard, false, false));
        assert!(
            found.contains(&316),
            "no Ghost in an underground graveyard: {found:?}"
        );
        for npc_type in [322u16, 323, 324] {
            assert!(
                !found.contains(&npc_type),
                "{npc_type} wore a costume in a June graveyard: {found:?}"
            );
        }

        // An ordinary cavern in the upper stone answers one thing and one only, the Lost Girl
        // (`NPC.cs:5012`), who carries no season and no progression gate. Everything else above is
        // therefore the season's doing rather than the chain's.
        assert_eq!(
            set(sample(plain, false, false)),
            std::collections::BTreeSet::from([195u16]),
            "an ordinary cavern answered more than the Lost Girl"
        );
        // ...and the bottom half of the stone adds exactly one more: Tim (`NPC.cs:5021`).
        assert_eq!(
            set(sample(plain, false, true)),
            std::collections::BTreeSet::from([45u16, 195]),
            "the lower caverns should add Tim and nothing else"
        );

        // `!noWorms` is vanilla's own gate on the Ghost, odd as it looks on something that floats.
        assert!(
            !set(sample(graveyard, true, false)).contains(&316),
            "a Ghost got past `noWorms`"
        );

        // `NPC.cs:5051` swallows nine hardmode draws in ten before the season is ever asked.
        let classic = sample(october, false, false).len();
        let hard = sample(
            Seasonal {
                hard_mode: true,
                ..october
            },
            false,
            false,
        )
        .len();
        assert!(
            hard * 4 < classic,
            "hardmode should reach this chain far less often: {hard} vs {classic}"
        );
    }

    /// The same chain through `try_spawn`, which is what proves it is wired to the caverns rather
    /// than merely written: an October cavern brings up costumed skeletons and an ordinary one does
    /// not.
    ///
    /// Neutralised by pointing `try_spawn`'s `cavern_chain` gate at `Depth::Underworld` instead of
    /// `Depth::Cavern`, so the chain is still written and still called but no longer reaches the
    /// caverns: "no SkeletonTopHat (322) in an October cavern: {10, 16, 21, 44, 49, 93, 354, 453,
    /// 496, 497, 498, 504, 506}", the plain cavern pool and nothing else.
    #[test]
    fn halloween_reaches_down_into_the_caverns() {
        let mut world = flat_world(250);
        let (px, py) = (400, 248);
        assert_eq!(depth_at(&world, py), Depth::Cavern);
        assert_eq!(biome_at(&world, px, py), Biome::Forest);
        world.halloween = true;

        let seen = spawns_at_in(&world, false, false, px, py, 200_000);
        let found: std::collections::BTreeSet<u16> = seen.iter().copied().collect();
        for (npc_type, name) in [
            (322u16, "SkeletonTopHat"),
            (323, "SkeletonAstonaut"),
            (324, "SkeletonAlien"),
        ] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) in an October cavern: {found:?}"
            );
        }

        world.halloween = false;
        let ordinary = spawns_at_in(&world, false, false, px, py, 200_000);
        let ordinary: std::collections::BTreeSet<u16> = ordinary.iter().copied().collect();
        for npc_type in [322u16, 323, 324] {
            assert!(
                !ordinary.contains(&npc_type),
                "{npc_type} turned up in an ordinary cavern: {ordinary:?}"
            );
        }
    }

    /// The caverns hand out a Lost Girl one attempt in eighty (`NPC.cs:5012-5015`), and she is the
    /// only path into the Nymph there has ever been.
    ///
    /// Both halves are asserted, because either one alone would be a half-wired lane: `try_spawn`
    /// really puts a 195 in a cavern, and the routine she arrives with really turns her into a 196
    /// (`game::ai::ambush::lost_girl`, `NPC.cs:30360-30389`). Vanilla's own ambient spawning never
    /// produces a 196 by any other route either, which is why 196 is not in `docs/spawn-gaps.tsv`
    /// while 195 was.
    ///
    /// The transformation half is `game::ai::ambush`'s own three tests, which already drive the
    /// routine through all three of vanilla's tells and assert it hands back a 196; this is the
    /// spawn half, which was the missing one.
    ///
    /// Neutralised by deleting the `one_in(rng, 80)` arm from [`cavern_seasonal_pick`]: "no Lost
    /// Girl in the caverns", 195 never appearing among the 200,000 attempts.
    #[test]
    fn the_caverns_hand_out_a_lost_girl() {
        let world = flat_world(250);
        let (px, py) = (400, 248);
        assert_eq!(depth_at(&world, py), Depth::Cavern);

        let found: std::collections::BTreeSet<u16> = spawns_at(&world, false, px, py, 200_000)
            .into_iter()
            .collect();
        assert!(
            found.contains(&195),
            "no Lost Girl in the caverns: {found:?}"
        );
    }

    /// Hallowed ground's own chain (`NPC.cs:4039-4061`), arm by arm.
    ///
    /// Three of its five are found nowhere else in `NPC.Spawner` at all, and the first of them is
    /// the Empress of Light's only summon, so the whole of that boss hung off this one branch.
    ///
    /// Neutralised arm by arm, each run and each failing its own assertion:
    ///
    /// * dropping `downed_plant_boss` from `:4041`: "a Lacewing before Plantera".
    /// * dropping `time < LACEWING_LATEST`: "a Lacewing after midnight".
    /// * dropping `surface_spawn`: "a Lacewing underground".
    /// * dropping `!day_time`: "a Lacewing in daylight".
    /// * dropping `!alive(661)`: "a second Lacewing while one is already up".
    /// * dropping `raining` from `:4045`: "a Rainbow Slime in the dry".
    /// * dropping `!alive(244)`: "a second Rainbow Slime while one is already up".
    /// * deleting the `:4053` arm: "no Unicorn on hallowed ground: {}".
    #[test]
    fn hallowed_ground_has_a_chain_of_its_own() {
        let sample = |downed: bool, day: bool, time: i32, surface: bool, raining: bool| {
            (0..40_000u64)
                .filter_map(|seed| {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    hallow_ground_pick(downed, day, time, surface, raining, &|_| false, &mut rng)
                })
                .collect::<std::collections::BTreeSet<u16>>()
        };

        // Plantera down, the first half of a dry night, on the surface: the Lacewing's own window.
        let window = sample(true, false, 0, true, false);
        assert!(
            window.contains(&PRISMATIC_LACEWING),
            "no Lacewing in its own window: {window:?}"
        );
        // ...and the Unicorn, which waits on nothing but the ground it stands on.
        assert!(
            window.contains(&86),
            "no Unicorn on hallowed ground: {window:?}"
        );

        // Each of the Lacewing's four world conditions, taken away one at a time.
        assert!(
            !sample(false, false, 0, true, false).contains(&PRISMATIC_LACEWING),
            "a Lacewing before Plantera"
        );
        assert!(
            !sample(true, false, LACEWING_LATEST, true, false).contains(&PRISMATIC_LACEWING),
            "a Lacewing after midnight"
        );
        assert!(
            !sample(true, false, 0, false, false).contains(&PRISMATIC_LACEWING),
            "a Lacewing underground"
        );
        assert!(
            !sample(true, true, 0, true, false).contains(&PRISMATIC_LACEWING),
            "a Lacewing in daylight"
        );

        // `!AnyNPCs(661)`: one at a time, which is what stops a hallow night raining Empresses.
        let crowded = (0..40_000u64)
            .filter_map(|seed| {
                let mut rng = SmallRng::seed_from_u64(seed);
                hallow_ground_pick(
                    true,
                    false,
                    0,
                    true,
                    false,
                    &|ty| ty == PRISMATIC_LACEWING,
                    &mut rng,
                )
            })
            .collect::<std::collections::BTreeSet<u16>>();
        assert!(
            !crowded.contains(&PRISMATIC_LACEWING),
            "a second Lacewing while one is already up: {crowded:?}"
        );

        // The Rainbow Slime wants rain and one of itself, and nothing else.
        assert!(
            sample(false, true, 0, false, true).contains(&244),
            "no Rainbow Slime in the rain"
        );
        assert!(
            !sample(false, true, 0, false, false).contains(&244),
            "a Rainbow Slime in the dry"
        );
        let wet = (0..40_000u64)
            .filter_map(|seed| {
                let mut rng = SmallRng::seed_from_u64(seed);
                hallow_ground_pick(false, true, 0, false, true, &|ty| ty == 244, &mut rng)
            })
            .collect::<std::collections::BTreeSet<u16>>();
        assert!(
            !wet.contains(&244),
            "a second Rainbow Slime while one is already up: {wet:?}"
        );
    }

    /// The same chain through `try_spawn`, which is what proves it is wired to hallowed ground
    /// rather than merely written, and that it reads the tile underfoot rather than the biome.
    ///
    /// Neutralised by changing the arm's `HALLOW_GROUND.contains(&g)` to `g == SAND`, so the chain
    /// is still written and still called but no longer reaches pearlstone: "no Prismatic Lacewing
    /// (661) on hallowed ground", and no Unicorn either.
    #[test]
    fn hallowed_ground_reaches_the_lacewing_through_try_spawn() {
        const HALLOWED_GRASS: u16 = 109;
        let mut world = flat_world_of(80, HALLOWED_GRASS);
        let (px, py) = (400, 78);
        assert_eq!(depth_at(&world, py), Depth::Surface);
        world.progress.downed_plantera = true;
        world.day_time = false;
        world.time = 0;
        world.raining = true;

        let found: std::collections::BTreeSet<u16> = spawns_at(&world, true, px, py, 200_000)
            .into_iter()
            .collect();
        for (npc_type, name) in [
            (PRISMATIC_LACEWING, "Prismatic Lacewing"),
            (244u16, "Rainbow Slime"),
            (86, "Unicorn"),
        ] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) on hallowed ground: {found:?}"
            );
        }

        // The same night on plain stone, which no amount of hallow in the air makes hallowed
        // ground: vanilla's gate is the tile, and there is no `ZoneHallow` anywhere in it.
        let mut stone = flat_world_of(80, 1);
        stone.progress.downed_plantera = true;
        stone.day_time = false;
        stone.time = 0;
        stone.raining = true;
        let ordinary: std::collections::BTreeSet<u16> = spawns_at(&stone, true, px, py, 200_000)
            .into_iter()
            .collect();
        assert!(
            !ordinary.contains(&PRISMATIC_LACEWING),
            "a Lacewing on plain stone: {ordinary:?}"
        );
    }

    /// A meteor crater answers with Meteor Heads and with nothing else at all (`NPC.cs:2796-2799`).
    ///
    /// The branch has no roll and no roster inside it: one `SpawnNPC(..., 23)` and a closing brace.
    /// Standing in a crater, that is the whole of it, which is what makes a meteorite field a place
    /// you clear rather than a place you walk through. The same ground without the meteorite draws
    /// the ordinary cavern pool, which is what proves the zone rather than the terrain is doing it.
    ///
    /// Neutralised by deleting the `None if zones.meteor` arm: "a crater spawned 21, not a Meteor
    /// Head", the plain cavern skeleton, and 23 never appears at all.
    #[test]
    fn a_crater_spawns_meteor_heads_and_nothing_else() {
        const METEORITE: u16 = 37;
        let world = flat_world_of(250, METEORITE);
        let (px, py) = (400, 248);
        assert_eq!(depth_at(&world, py), Depth::Cavern);
        assert!(
            zones_at(&world, px, py).meteor,
            "a floor of meteorite is a crater",
        );

        // A bound resident is not a monster and is not the crater's doing: vanilla's own bound arms
        // (`NPC.cs:1662`, `:2087-2098`) all sit *above* `else if (ZoneMeteor)`, so a crater is where
        // one may still be found. Rescues are not what this test is about.
        let monsters = |seen: Vec<u16>| {
            seen.into_iter()
                .filter(|ty| {
                    !terrustia_proto::npc_data::npc_stats(*ty)
                        .expect("a real type")
                        .friendly
                })
                .collect::<Vec<u16>>()
        };
        let seen = monsters(spawns_at(&world, false, px, py, 40_000));
        assert!(!seen.is_empty(), "the crater never spawned anything");
        for npc_type in &seen {
            assert_eq!(
                *npc_type, METEOR_HEAD,
                "a crater spawned {npc_type}, not a Meteor Head",
            );
        }

        // The same cavern floored with stone instead: an ordinary pool, and no Meteor Head in it.
        let stone = flat_world(250);
        assert!(!zones_at(&stone, px, py).meteor);
        let ordinary: std::collections::BTreeSet<u16> =
            monsters(spawns_at(&stone, false, px, py, 40_000))
                .into_iter()
                .collect();
        assert!(
            ordinary.len() > 1 && !ordinary.contains(&METEOR_HEAD),
            "an ordinary cavern drew {ordinary:?}",
        );
    }

    /// The Glowing Mushroom roster hangs off the ground tile, not off `ZoneGlowshroom`.
    ///
    /// This is the thing worth pinning, because it is the opposite of what the zone flag beside it
    /// suggests. Vanilla's two arms (`NPC.cs:3637`, `:3674`) test `tileType == 70` and nothing else
    /// about the biome, so the surface arm is open from the first day of a world and the
    /// underground one waits only on hardmode. Only the Spore pair (`:5110`, `:5209`) wants the
    /// zone as well, and that is the next test.
    ///
    /// Neutralised twice: dropping the `!hard_mode ||` from the underground arm's gate gives "no
    /// TruffleWorm (374) in a hardmode mushroom cave" turned inside out - the worm appears before
    /// hardmode, failing the classic-mode assertion; and returning `None` unconditionally from the
    /// surface arm gives "no ZombieMushroom (254) in a surface mushroom patch: {}".
    #[test]
    fn the_mushroom_roster_hangs_off_the_ground_tile() {
        let sample = |surface: bool, hard_mode: bool| {
            (0..40_000u64)
                .filter_map(|seed| {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    mushroom_pick(surface, hard_mode, &mut rng)
                })
                .collect::<std::collections::BTreeSet<u16>>()
        };

        // The surface arm, which needs no progression at all.
        let classic = sample(true, false);
        for (npc_type, name) in [
            (254u16, "ZombieMushroom"),
            (255, "ZombieMushroomHat"),
            (257, "AnomuraFungus"),
            (258, "MushiLadybug"),
            (259, "FungiBulb"),
            (360, "GlowingSnail"),
        ] {
            assert!(
                classic.contains(&npc_type),
                "no {name} ({npc_type}) in a surface mushroom patch: {classic:?}",
            );
        }
        // `Main.hardMode && Main.rand.Next(3) != 0` (`NPC.cs:3647`) is the only source of the big
        // bulb, and the Truffle Worm is the underground arm's alone.
        assert!(
            !classic.contains(&260) && !classic.contains(&374),
            "a classic surface patch drew {classic:?}",
        );
        assert!(sample(true, true).contains(&260), "no GiantFungiBulb");

        // The underground arm is hardmode's outright: `Main.hardMode` is part of its own gate.
        assert!(
            sample(false, false).is_empty(),
            "a pre-hardmode mushroom cave answers nothing here",
        );
        let hard = sample(false, true);
        for (npc_type, name) in [
            (374u16, "TruffleWorm"),
            (360, "GlowingSnail"),
            (259, "FungiBulb"),
            (260, "GiantFungiBulb"),
            (257, "AnomuraFungus"),
            (258, "MushiLadybug"),
        ] {
            assert!(
                hard.contains(&npc_type),
                "no {name} ({npc_type}) in a hardmode mushroom cave: {hard:?}",
            );
        }
        // The mushroom zombies are the surface arm's alone: nothing walks in the caves.
        assert!(
            !hard.contains(&254) && !hard.contains(&255),
            "a mushroom zombie underground: {hard:?}",
        );

        // One attempt in three declines outright (`Main.rand.Next(3) != 0` on both arms), which is
        // what lets the ordinary chain still answer inside a mushroom biome.
        let declined = (0..40_000u64)
            .filter(|seed| {
                let mut rng = SmallRng::seed_from_u64(*seed);
                mushroom_pick(true, false, &mut rng).is_none()
            })
            .count();
        assert!(
            (12_000..15_000).contains(&declined),
            "{declined} of 40000 attempts declined, expected about a third",
        );
    }

    /// The Spore pair is the one thing here that really does want `ZoneGlowshroom`, and it wants the
    /// ground tile too: `ZoneGlowshroom && (tileType == 70 || tileType == 190)` (`NPC.cs:5110` for
    /// the Skeleton, `:5209` for the Bat).
    ///
    /// The two sit in opposite halves of the cavern chain's own coin flip, so a mushroom cavern gets
    /// both and everywhere else gets neither.
    ///
    /// Neutralised by dropping `glowshroom_ground` from both arms (making each unconditional): the
    /// last assertion fails with "an ordinary cavern grew spores: {634}", the bat having reached a
    /// stone cave.
    #[test]
    fn the_spore_pair_wants_the_zone_and_the_ground() {
        let sample = |glowshroom: bool| {
            (0..40_000u64)
                .filter_map(|seed| {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    cavern_seasonal_pick(
                        Seasonal::default(),
                        false,
                        false,
                        1,
                        glowshroom,
                        &|_| false,
                        &mut rng,
                    )
                })
                .collect::<std::collections::BTreeSet<u16>>()
        };
        let mushroom = sample(true);
        assert!(
            mushroom.contains(&634) && mushroom.contains(&635),
            "a mushroom cavern should grow both spores: {mushroom:?}",
        );
        assert!(
            !sample(false).contains(&634) && !sample(false).contains(&635),
            "an ordinary cavern grew spores: {:?}",
            sample(false),
        );
    }

    /// The same three arms through `try_spawn`, which is what proves they are wired to the ground
    /// rather than merely written.
    ///
    /// Neutralised by pinning `mushroom_ground` and `glowshroom_ground` to `false` in `try_spawn`:
    /// "no TruffleWorm (374) standing on mushroom grass" and "no SporeSkeleton (635) in a mushroom
    /// cavern", with the plain cavern pool answering instead.
    #[test]
    fn a_mushroom_cave_is_wired_to_the_spawner() {
        // Deep enough to be caverns, so both the underground mushroom arm and the cavern chain that
        // holds the Spore pair are in play.
        let world = flat_world_of(250, MUSHROOM_GRASS);
        let (px, py) = (400, 248);
        assert_eq!(depth_at(&world, py), Depth::Cavern);
        let zones = zones_at(&world, px, py);
        assert!(zones.glowshroom, "a floor of mushroom grass is the zone");
        assert_eq!(zones.biome, Biome::Forest, "and it is nobody else's biome");

        let hard: std::collections::BTreeSet<u16> = spawns_at(&world, true, px, py, 200_000)
            .into_iter()
            .collect();
        for (npc_type, name) in [
            (374u16, "TruffleWorm"),
            (360, "GlowingSnail"),
            (259, "FungiBulb"),
            (635, "SporeSkeleton"),
            (634, "SporeBat"),
        ] {
            assert!(
                hard.contains(&npc_type),
                "no {name} ({npc_type}) in a mushroom cavern: {hard:?}",
            );
        }

        // Before hardmode the underground arm is shut, so the worm and the snail are gone and the
        // Spore pair - which has no progression gate at all - is not.
        let classic: std::collections::BTreeSet<u16> = spawns_at(&world, false, px, py, 200_000)
            .into_iter()
            .collect();
        assert!(
            !classic.contains(&374),
            "a Truffle Worm before hardmode: {classic:?}",
        );
        assert!(
            classic.contains(&634) && classic.contains(&635),
            "the Spore pair waits on no boss: {classic:?}",
        );

        // ...and the surface arm, which waits on nothing at all.
        let surface = flat_world_of(80, MUSHROOM_GRASS);
        assert_eq!(depth_at(&surface, 78), Depth::Surface);
        let day: std::collections::BTreeSet<u16> = spawns_at(&surface, false, px, 78, 200_000)
            .into_iter()
            .collect();
        for (npc_type, name) in [(254u16, "ZombieMushroom"), (255, "ZombieMushroomHat")] {
            assert!(
                day.contains(&npc_type),
                "no {name} ({npc_type}) on a surface mushroom patch: {day:?}",
            );
        }
    }

    /// Marble and granite are the only home the four of them have (`NPC.cs:5027-5050`), and only
    /// the Medusa waits for hardmode: a granite pocket has its golems and flyers on day one.
    ///
    /// Neutralised arm by arm, each failing its own assertion:
    ///
    /// * deleting the `NPC.cs:5027` marble arm: "no GreekSkeleton (481) on a marble floor: {}".
    /// * deleting `:5039`'s granite arm: "no GraniteFlyer (483) on a granite floor: {}".
    /// * dropping `&& at.hard_mode` from `:5029`: "a Medusa turned up before hardmode".
    /// * dropping `!alive(483)` from `:5041`: "a second Granite Flyer while one is already up".
    /// * dropping the `world.tile(x, y + 1).block` argument in `try_spawn` for a literal `1`: "no
    ///   GraniteGolem (482) in a granite cavern", the arm written but wired to nothing.
    #[test]
    fn marble_and_granite_are_where_those_four_live() {
        let stone = |ground_block: u16, hard_mode: bool, alive: &dyn Fn(u16) -> bool| {
            (0..40_000u64)
                .filter_map(|seed| {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    cavern_seasonal_pick(
                        Seasonal {
                            hard_mode,
                            ..Seasonal::default()
                        },
                        false,
                        false,
                        ground_block,
                        false,
                        alive,
                        &mut rng,
                    )
                })
                .collect::<std::collections::BTreeSet<u16>>()
        };

        // Pre-hardmode marble is the Greek Skeleton alone: `NPC.cs:5029` ends `&& Main.hardMode`.
        let marble = stone(367, false, &|_| false);
        assert!(
            marble.contains(&481),
            "no GreekSkeleton (481) on a marble floor: {marble:?}"
        );
        assert!(!marble.contains(&480), "a Medusa turned up before hardmode");
        assert!(
            stone(367, true, &|_| false).contains(&480),
            "no Medusa on a hardmode marble floor"
        );

        // Granite has no such gate, and its two split on `!AnyNPCs(483)` (`NPC.cs:5041`).
        let granite = stone(368, false, &|_| false);
        for (npc_type, name) in [(483u16, "GraniteFlyer"), (482, "GraniteGolem")] {
            assert!(
                granite.contains(&npc_type),
                "no {name} ({npc_type}) on a granite floor: {granite:?}"
            );
        }
        assert!(
            !stone(368, false, &|ty| ty == 483).contains(&483),
            "a second Granite Flyer while one is already up"
        );

        // Ordinary stone answers none of the four, which is what makes the pocket the reason.
        let plain = stone(1, true, &|_| false);
        for npc_type in [480u16, 481, 482, 483] {
            assert!(
                !plain.contains(&npc_type),
                "{npc_type} turned up on plain rock: {plain:?}"
            );
        }

        // ...and it is wired to the caverns rather than merely written: a granite-floored hall.
        let floor = 300;
        let mut world = hall_world(floor);
        for x in 0..world.width() {
            world.set_tile(x, floor, terrustia_proto::Tile::block(368));
        }
        assert_eq!(depth_at(&world, floor - 1), Depth::Cavern);
        assert_eq!(
            biome_at(&world, world.width() / 2, floor - 1),
            Biome::Forest
        );
        assert!(
            spawns_of(&world, floor, &quiet(), 482, 4) > 0,
            "no GraniteGolem (482) in a granite cavern"
        );
    }

    /// Christmas puts two zombies in seasonal knitwear (`NPC.cs:4739`,
    /// `Main.rand.Next(331, 333)`), and dresses the surface slime in ribbons
    /// (`NPC.cs:5660-5662`).
    ///
    /// Neutralised by forcing `Seasonal::xmas` to `false` in `try_spawn`: none of the four appears.
    #[test]
    fn christmas_puts_the_zombies_in_sweaters() {
        let (mut world, (px, py)) = night_world();
        world.xmas = true;
        let seen = spawns_at_in(&world, false, false, px, py, 200_000);
        let found: std::collections::BTreeSet<u16> = seen.iter().copied().collect();
        for npc_type in [331u16, 332] {
            assert!(
                found.contains(&npc_type),
                "no {npc_type} at Christmas: {found:?}"
            );
        }

        // The daytime slime wears ribbons too, and that swap is a different site
        // (`GetBasicSlimeToSpawn`, not the night chain). Two draws in three take a ribbon
        // (`GetBasicSlimeToSpawn_ChanceToBeHolidaySlime`, `NPC.cs:5678-5685`), so the plain blue
        // slime is still there and should be outnumbered rather than gone.
        let (mut day, _) = forest_surface();
        day.xmas = true;
        let seen = spawns_at_in(&day, false, false, px, py, 50_000);
        let ribboned = seen.iter().filter(|&&ty| (333..=336).contains(&ty)).count();
        let plain_slime = seen.iter().filter(|&&ty| ty == 1).count();
        assert!(ribboned > 0, "no ribboned slime at Christmas");
        assert!(
            ribboned > plain_slime,
            "{ribboned} ribboned slimes against {plain_slime} plain ones: the two-in-three roll is \
             not being made"
        );
        // All four colours, since `Main.rand.Next(333, 337)` is a flat draw over them.
        for colour in 333u16..=336 {
            assert!(seen.contains(&colour), "no slime in ribbon colour {colour}");
        }
    }

    /// The full moon in hardmode is the only thing that brings a Werewolf, and only two attempts in
    /// three at that (`NPC.cs:4633`: `!Main.dayTime && Main.moonPhase == 0 && Main.hardMode &&
    /// Main.rand.Next(3) != 0`).
    ///
    /// Neutralised by deleting the Werewolf arm from `seasonal_night_pick`: the first assertion
    /// fails. The second half of the test is the gate itself, which is what stops a Werewolf on
    /// every other night of the eight.
    #[test]
    fn a_full_moon_in_hardmode_brings_out_the_werewolves() {
        let (mut world, (px, py)) = night_world();
        world.moon_phase = 0; // MoonPhase.Full
        let seen = spawns_at_in(&world, true, false, px, py, 50_000);
        assert!(
            seen.contains(&104),
            "no Werewolf under a hardmode full moon: {:?}",
            seen.iter().collect::<std::collections::BTreeSet<_>>()
        );

        // Not on any other phase, and not before the wall falls.
        world.moon_phase = 1;
        assert!(
            !spawns_at_in(&world, true, false, px, py, 50_000).contains(&104),
            "a Werewolf on a waning moon"
        );
        world.moon_phase = 0;
        assert!(
            !spawns_at_in(&world, false, false, px, py, 50_000).contains(&104),
            "a Werewolf before hardmode"
        );
    }

    /// The five coloured eyes are the tail of the demon-eye branch, and the new moon
    /// (`MoonPhase.Empty`, phase 4) doubles how often that branch is entered at all
    /// (`NPC.cs:4554`: `Main.rand.Next(6) == 0 || (Main.moonPhase == 4 && Main.rand.Next(2) == 0)`).
    ///
    /// Neutralised by dropping the `|| (at.moon_phase == 4 && one_in(rng, 2))` half of that
    /// condition: the two counts below come out equal and the assertion fails. Neutralised the
    /// other way, by deleting the whole eye arm, the first assertion fails instead.
    #[test]
    fn the_new_moon_is_the_night_of_the_eyes() {
        let count = |phase: u8| {
            let at = Seasonal {
                moon_phase: phase,
                ..Seasonal::default()
            };
            let mut rng = SmallRng::seed_from_u64(99);
            (0..200_000)
                .filter(|_| {
                    matches!(seasonal_night_pick(at, 2, &mut rng), Some(ty) if (190..=194).contains(&ty))
                })
                .count()
        };
        let full = count(0);
        let new_moon = count(4);
        assert!(full > 0, "no coloured eyes at all on an ordinary night");
        // Entering the branch goes from one attempt in six to one in six plus one in two of the
        // remaining five sixths, which is 7/12: a factor of 3.5, so 3x is a wide floor.
        assert!(
            new_moon > full * 3,
            "the new moon should be thick with eyes: {new_moon} against {full}"
        );
    }

    /// Ice underfoot on a surface night is the Ice Elemental's and the Wolf's only home outside the
    /// caverns' flying chain (`NPC.cs:4655-4673`), and the arm keys on the *tile* rather than on the
    /// player's zone: 169 was in no producer at all and 155 in none either.
    ///
    /// The floor is snow in a world the zone scan still calls a forest (169 snow tiles in a
    /// 169-by-124 box, against a `SnowTileNormalThreshold` of 1500), which is what makes this a
    /// test of the tile and not of the biome.
    ///
    /// Neutralised arm by arm:
    ///
    /// * deleting the ice-and-snow `matches!` block: "no IceElemental (169) on ice".
    /// * dropping `!at.graveyard` from both hardmode arms: "a Wolf in a snowy graveyard".
    /// * dropping `at.hard_mode` from both: "an Ice Elemental before hardmode".
    /// * passing `2` instead of `world.tile(x, y + 1).block` in `try_spawn`: "no IceElemental (169)
    ///   on ice", the arm written but wired to nothing.
    #[test]
    fn ice_underfoot_brings_the_wolves_out() {
        let (mut world, (px, py)) = night_world();
        for x in 0..world.width() {
            world.set_tile(x, 90, terrustia_proto::Tile::block(147)); // SnowBlock
        }
        assert_eq!(
            biome_at(&world, px, py),
            Biome::Forest,
            "one row of snow is not a snow biome, which is the point of this test"
        );

        let found: std::collections::BTreeSet<u16> =
            spawns_at_in(&world, true, false, px, py, 60_000)
                .into_iter()
                .collect();
        for (npc_type, name) in [(169u16, "IceElemental"), (155, "Wolf")] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) on ice: {found:?}"
            );
        }

        // `!ZoneGraveyard` on both arms (`NPC.cs:4657`, `:4661`): tombstones in the snow get neither.
        let haunted: std::collections::BTreeSet<u16> =
            spawns_at_in(&world, true, true, px, py, 60_000)
                .into_iter()
                .collect();
        assert!(!haunted.contains(&155), "a Wolf in a snowy graveyard");
        assert!(
            !haunted.contains(&169),
            "an Ice Elemental in a snowy graveyard"
        );

        // ...and `Main.hardMode` on both, so a fresh world's snow is just cold.
        let early: std::collections::BTreeSet<u16> =
            spawns_at_in(&world, false, false, px, py, 60_000)
                .into_iter()
                .collect();
        assert!(!early.contains(&169), "an Ice Elemental before hardmode");
        assert!(!early.contains(&155), "a Wolf before hardmode");
    }

    /// Rain on a surface night is the Zombie Raincoat's only home (`NPC.cs:4675-4689`), and all
    /// three of vanilla's outcomes there are the same 223 at different scales.
    ///
    /// Neutralised by deleting the `at.raining` arm: "no ZombieRaincoat (223) in the rain".
    /// Neutralised the other way, by dropping `at.raining` from the condition so it fires dry: the
    /// second assertion fails instead.
    #[test]
    fn rain_puts_a_coat_on_the_zombies() {
        let (mut world, (px, py)) = night_world();
        world.raining = true;
        let wet: std::collections::BTreeSet<u16> =
            spawns_at_in(&world, false, false, px, py, 60_000)
                .into_iter()
                .collect();
        assert!(
            wet.contains(&223),
            "no ZombieRaincoat (223) in the rain: {wet:?}"
        );

        world.raining = false;
        let dry: std::collections::BTreeSet<u16> =
            spawns_at_in(&world, false, false, px, py, 60_000)
                .into_iter()
                .collect();
        assert!(!dry.contains(&223), "a raincoat on a dry night: {dry:?}");
    }

    /// A world whose surface line is at 200, so the sky line (`worldSurface * 0.35`) is row 70 and
    /// everything above it is open air the generator never touched.
    fn sky_world() -> World {
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;
        world
    }

    /// A cavern of open air behind sandstone walls, floored every eight rows, which is what an
    /// underground desert is to the spawner: not a biome scan but a wall
    /// (`WallID.Sets.AllowsUndergroundDesertEnemiesToSpawn`).
    ///
    /// Rows 300 to 400 so every candidate is deeper than `worldSurface + 100` and shallower than
    /// the underworld, and wide enough to cover the whole `SPAWN_RANGE_X` box. `floor` is the block
    /// the ledges are made of, which is also what the zone scan around the player reads: sand makes
    /// it a desert, ebonsand makes it a corrupt one, and the walls stay sandstone either way.
    fn desert_cavern(wall: u16, floor: u16) -> World {
        use terrustia_proto::tile::Tile;
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;
        for y in 295i32..400 {
            for x in 250..550 {
                let mut tile = if y % 8 == 0 {
                    Tile::block(floor)
                } else {
                    Tile::AIR
                };
                tile.wall = wall;
                world.set_tile(x, y, tile);
            }
        }
        world
    }

    /// The underground desert has a roster of its own, and before this it had no arm at all: eleven
    /// types (both ghoul families, the lamias, the scorpion, the beast, the djinn and both giant
    /// antlions) have no other ambient spawn anywhere in the game, so a player who dug into one met
    /// sand slimes and antlions and nothing else.
    ///
    /// Neutralised by forcing `desert_spot` to `false` in `try_spawn`: nothing but the ordinary
    /// pool comes out over forty thousand ticks and every assertion below fails.
    #[test]
    fn the_underground_desert_draws_its_own_roster() {
        let world = desert_cavern(187, 53); // Sandstone wall, sand ledges.
        let seen = spawns_at(&world, false, 400, 350, 40_000);
        let set: std::collections::BTreeSet<u16> = seen.iter().copied().collect();
        for (npc_type, name) in [
            (69u16, "Antlion"),
            (580, "WalkingAntlion"),
            (581, "FlyingAntlion"),
            (508, "GiantWalkingAntlion"),
            (509, "GiantFlyingAntlion"),
            (513, "TombCrawlerHead"),
            (537, "SandSlime"),
        ] {
            assert!(
                set.contains(&npc_type),
                "no {name} in the underground desert: {set:?}",
            );
        }
        // Nothing from the ordinary cavern pool: vanilla's branch answers for the whole spot.
        assert!(
            !set.contains(&21),
            "a Skeleton reached a desert spot: {set:?}"
        );
    }

    /// `SpawnTileOrAboveHasAnyWallInSet` reads two rows, not one (`NPC.cs:5535-5556`), and it is
    /// the *ground* tile plus the one above it, which for this server's spawn row `y` is `y + 1`
    /// and `y`. Half a sandstone wall is still an underground desert; no sandstone wall is not.
    ///
    /// Neutralised by dropping either `walled(y + 1)` or `walled(y)` from
    /// `underground_desert_spot`: one of the two middle assertions fails each time. The world the
    /// spawn test above uses has the wall on both rows, so it cannot see this on its own.
    #[test]
    fn the_desert_wall_is_read_on_the_ground_tile_and_the_one_above_it() {
        use terrustia_proto::tile::Tile;
        let mut world = desert_cavern(0, 53); // sand ledges, no wall anywhere.
        // Row 351 is open, row 352 is a ledge, so 351 is a spawn row.
        assert!(
            !underground_desert_spot(&world, 400, 351),
            "no wall, no desert"
        );

        let mut floor = Tile::block(53);
        floor.wall = 187;
        world.set_tile(400, 352, floor);
        assert!(
            underground_desert_spot(&world, 400, 351),
            "the ground tile alone"
        );

        let mut world = desert_cavern(0, 53);
        let mut above = Tile::AIR;
        above.wall = 187;
        world.set_tile(400, 351, above);
        assert!(
            underground_desert_spot(&world, 400, 351),
            "the tile above alone"
        );

        // ...and a wall that is not one of the nine does not count, however sandy the floor is.
        let mut world = desert_cavern(0, 53);
        let mut floor = Tile::block(53);
        floor.wall = 1; // StoneWall
        world.set_tile(400, 352, floor);
        assert!(
            !underground_desert_spot(&world, 400, 351),
            "a stone wall is not a desert"
        );
    }

    /// The giant antlions are *not* a hardmode upgrade. The one-in-ten roll that turns a Walking or
    /// Flying Antlion into its giant (`NPC.cs:1752-1763`) sits on the plain fallthrough path, which
    /// is the only path a pre-hardmode world ever takes, so a fresh world's underground desert
    /// already has them.
    ///
    /// Neutralised by moving the giant swap inside the `hard_mode` branch of
    /// `underground_desert_pick`: this fails, and the hardmode half below still passes, which is
    /// what makes the assertion about progression rather than about reachability.
    #[test]
    fn the_giant_antlions_do_not_wait_for_hardmode() {
        let world = desert_cavern(187, 53);
        let early: std::collections::BTreeSet<u16> = spawns_at(&world, false, 400, 350, 10_000)
            .into_iter()
            .collect();
        assert!(
            early.contains(&508),
            "no giant walking antlion pre-hardmode"
        );
        assert!(early.contains(&509), "no giant flying antlion pre-hardmode");
        // ...and none of hardmode's own roster is out early.
        for locked in [524u16, 528, 530, 532, 510] {
            assert!(
                !early.contains(&locked),
                "{locked} appeared before hardmode: {early:?}",
            );
        }
    }

    /// Hardmode opens the ghouls, and which ghoul is a function of the *player's* zone rather than
    /// of the tile under the spawn: `SetSpawnFlags` copies `ZoneCorrupt`/`ZoneCrimson`/`ZoneHallow`
    /// straight off the player (`NPC.cs:381-383`) and the branch reads those copies
    /// (`NPC.cs:1710-1741`). A clean desert gives the plain ghoul with the scorpion and the light
    /// lamia; a corrupt one gives the corrupt ghoul with the djinn and the dark lamia.
    ///
    /// Neutralised by collapsing `underground_desert_pick`'s `match biome` to its `_` arm: the
    /// corrupt half fails outright, since 525, 533 and 529 stop being reachable at all.
    #[test]
    fn the_ghoul_follows_the_players_zone_not_the_tile() {
        let clean = desert_cavern(187, 53);
        let seen: std::collections::BTreeSet<u16> = spawns_at(&clean, true, 400, 350, 10_000)
            .into_iter()
            .collect();
        for (npc_type, name) in [
            (524u16, "DesertGhoul"),
            (530, "DesertScorpionWalk"),
            (528, "DesertLamiaLight"),
            (532, "DesertBeast"),
            (510, "DuneSplicerHead"),
        ] {
            assert!(
                seen.contains(&npc_type),
                "no {name} in a clean desert: {seen:?}"
            );
        }
        assert!(!seen.contains(&525), "a corrupt ghoul in a clean desert");
        assert!(!seen.contains(&533), "a djinn in a clean desert");

        // The same cavern with ebonsand ledges: the walls are still sandstone, so it is still the
        // underground desert, but the zone around the player now reads as corruption.
        let corrupt = desert_cavern(187, 112);
        assert_eq!(biome_at(&corrupt, 400, 350), Biome::Corruption);
        let seen: std::collections::BTreeSet<u16> = spawns_at(&corrupt, true, 400, 350, 10_000)
            .into_iter()
            .collect();
        for (npc_type, name) in [
            (525u16, "DesertGhoulCorruption"),
            (533, "DesertDjinn"),
            (529, "DesertLamiaDark"),
        ] {
            assert!(
                seen.contains(&npc_type),
                "no {name} in a corrupt desert: {seen:?}"
            );
        }
        assert!(!seen.contains(&524), "the plain ghoul in a corrupt desert");
        assert!(!seen.contains(&530), "the scorpion in a corrupt desert");
    }

    /// A room of the Lihzahrd Temple: open air behind Lihzahrd brick walls, floored in Lihzahrd
    /// brick every eight rows.
    ///
    /// Both halves of vanilla's gate are here on purpose and are separable: `wall` is what the
    /// *player* stands in front of (`ZoneLihzhardTemple`, `SceneMetrics.cs:693`) and `floor` is the
    /// `tileType` under the candidate (`NPC.cs:3914`). Rows 295 to 400 put every candidate in the
    /// caverns, below `rock_layer`, which is where a real temple sits.
    fn temple(wall: u16, floor: u16) -> World {
        use terrustia_proto::tile::Tile;
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;
        for y in 295i32..400 {
            for x in 250..550 {
                let mut tile = if y % 8 == 0 {
                    Tile::block(floor)
                } else {
                    Tile::AIR
                };
                tile.wall = wall;
                world.set_tile(x, y, tile);
            }
        }
        world
    }

    /// The temple's whole ambient roster, which had no arm at all before this: the Lihzahrd and the
    /// Flying Snake come from `NPC.cs:3914-3924` and from nowhere else in `NPC.Spawner`, so the
    /// post-Plantera temple - the room players farm for Solar Tablet fragments and Power Cells -
    /// was silent on this server.
    ///
    /// Both halves of the gate get their own assertion, because either one alone would have looked
    /// like a pass: a temple wall over ordinary stone is not the temple, and Lihzahrd brick with no
    /// temple wall behind the player is a Lihzahrd brick floor somebody built.
    ///
    /// Neutralised three ways, each rerun (an assertion failure stops the test, so each line names
    /// the one that fires first):
    /// * turning the whole `None if temple_zone` arm off: "no Lihzahrd (198) in the temple:
    ///   {42, 43, 51, 56, 354}", the ordinary cavern pool answering in its place.
    /// * dropping `temple_zone` from the arm's guard: "the temple roster reached a brick floor
    ///   outside a temple: {198, 226, 354}".
    /// * dropping the `ground_block` half: "the temple roster reached a stone floor:
    ///   {198, 226, 354}".
    #[test]
    fn the_lihzahrd_temple_has_a_roster_of_its_own() {
        let world = temple(TEMPLE_WALL, LIHZAHRD_BRICK);
        assert_eq!(depth_at(&world, 350), Depth::Cavern);
        let seen: std::collections::BTreeSet<u16> = spawns_at(&world, false, 400, 350, 20_000)
            .into_iter()
            .collect();
        for (npc_type, name) in [(LIHZAHRD, "Lihzahrd"), (FLYING_SNAKE, "FlyingSnake")] {
            assert!(
                seen.contains(&npc_type),
                "no {name} ({npc_type}) in the temple: {seen:?}"
            );
        }
        // The arm answers for the whole spot, as vanilla's does: nothing from the cavern pool.
        // A bound resident is the one other thing that can turn up, because that branch sits above
        // every biome arm in the game too (`NPC.cs:1658-1697`, `:2087-2098`, all of them above the
        // fallthrough the temple arm lives in), so it is subtracted rather than asserted away.
        let bound: std::collections::BTreeSet<u16> = crate::game::rescues::RESCUES
            .iter()
            .map(|r| r.bound)
            .chain([BOUND_TOWN_SLIME_OLD])
            .collect();
        let wild: std::collections::BTreeSet<u16> = seen.difference(&bound).copied().collect();
        assert_eq!(
            wild,
            [LIHZAHRD, FLYING_SNAKE].into_iter().collect(),
            "something other than the temple's own two got in: {seen:?}"
        );

        // Wooden Spikes are the second ground tile the arm accepts (`NPC.cs:3914`).
        let spiked: std::collections::BTreeSet<u16> =
            spawns_at(&temple(TEMPLE_WALL, WOODEN_SPIKES), false, 400, 350, 20_000)
                .into_iter()
                .collect();
        assert!(
            spiked.contains(&LIHZAHRD) && spiked.contains(&FLYING_SNAKE),
            "the spike floor is a temple floor too: {spiked:?}"
        );

        // The zone is the player's wall: Lihzahrd brick with no temple wall behind them is just a
        // floor, and the caverns answer for it.
        let unwalled: std::collections::BTreeSet<u16> =
            spawns_at(&temple(0, LIHZAHRD_BRICK), false, 400, 350, 20_000)
                .into_iter()
                .collect();
        assert!(
            !unwalled.contains(&LIHZAHRD) && !unwalled.contains(&FLYING_SNAKE),
            "the temple roster reached a brick floor outside a temple: {unwalled:?}"
        );
        assert!(!unwalled.is_empty(), "the caverns answered for it instead");

        // ...and the ground is the candidate's: a temple wall over ordinary stone is not a temple.
        let stone_floor: std::collections::BTreeSet<u16> =
            spawns_at(&temple(TEMPLE_WALL, 1), false, 400, 350, 20_000)
                .into_iter()
                .collect();
        assert!(
            !stone_floor.contains(&LIHZAHRD) && !stone_floor.contains(&FLYING_SNAKE),
            "the temple roster reached a stone floor: {stone_floor:?}"
        );
    }

    /// A surface desert with a sandstorm blowing over it, deep enough sand for
    /// `Spawning_SandstoneCheck` to pass.
    ///
    /// The scan box has to read as desert for `ZoneSandstorm`, which wants 1500 sand tiles, so the
    /// whole box is sand with a ledge every eight rows carved out of it. Rows 120 to 200 keep every
    /// candidate at or above `worldSurface`, which is `SurfaceAtmospherics`.
    fn sandstorm_desert(sand: u16) -> World {
        use terrustia_proto::tile::Tile;
        let mut world = test_world();
        world.surface = 200;
        world.rock_layer = 300;
        for y in 100i32..260 {
            for x in 250..550 {
                // Nine solid rows of sand under every three open ones: the sandstone check reads
                // eight rows down from the ground tile, and a spawn needs three clear rows above
                // it, so both are satisfied everywhere.
                let tile = if y % 12 == 0 || (y % 12) > 3 {
                    Tile::block(sand)
                } else {
                    Tile::AIR
                };
                world.set_tile(x, y, tile);
            }
        }
        world
    }

    /// A sandstorm over a desert has a roster of its own (`NPC.cs:3952-4022`), and this server ran
    /// sandstorms without ever spawning one of its members: the Tumbleweed, the Sand Elemental, all
    /// four Sandsharks and the Blood Mummy had no ambient spawn at all.
    ///
    /// Neutralised by forcing `sandstorm_spot` to `false`: the pre-hardmode half loses its
    /// Tumbleweed and the hardmode half loses everything asserted below, so both halves fail.
    /// Neutralised a second way by setting `events.sandstorm` to `false` in this test, which fails
    /// it the same way and proves the arm really is gated on the weather rather than on the sand.
    #[test]
    fn a_sandstorm_draws_the_sandstorm_roster() {
        let world = sandstorm_desert(53); // plain Sand
        let storm = EventSpawns {
            sandstorm: true,
            ..quiet()
        };
        let early: std::collections::BTreeSet<u16> = spawns_with(&world, &storm, 400, 160, 30_000)
            .into_iter()
            .map(|(ty, _)| ty)
            .collect();
        assert!(
            early.contains(&546),
            "no tumbleweed in an early sandstorm: {early:?}"
        );
        assert!(
            early.contains(&61),
            "no vulture in an early sandstorm: {early:?}"
        );
        assert!(
            early.contains(&69),
            "no antlion in an early sandstorm: {early:?}"
        );
        assert!(
            !early.contains(&542),
            "a sandshark before hardmode: {early:?}",
        );

        let storm = EventSpawns {
            sandstorm: true,
            hard_mode: true,
            ..quiet()
        };
        let late = spawns_with(&world, &storm, 400, 160, 30_000);
        let set: std::collections::BTreeSet<u16> = late.iter().map(|(ty, _)| *ty).collect();
        for (npc_type, name) in [
            (541u16, "SandElemental"),
            (542, "SandShark"),
            (546, "Tumbleweed"),
            (78, "Mummy"),
            (580, "WalkingAntlion"),
            (581, "FlyingAntlion"),
        ] {
            assert!(
                set.contains(&npc_type),
                "no {name} in a hardmode sandstorm: {set:?}"
            );
        }
        // The Dune Splicer burrows in ten tiles below the spot it was chosen at
        // (`NPC.cs:3975`, `SpawnNPC(x, (spawnTileY + 10) * 16, 510)`), which is the one member that
        // does not stand where it was drawn. Every ledge in this world has its open rows at
        // `y % 12` of 1, 2 and 3, and a spawn needs three clear rows, so every other type lands on
        // a row `== 3 (mod 12)` and every splicer ten below that, `== 1 (mod 12)`.
        let rows = |want: u16| -> Vec<i32> {
            late.iter()
                .filter(|(ty, _)| *ty == want)
                .map(|(_, (_, y))| (*y / 16.0) as i32 % 12)
                .collect()
        };
        let splicers = rows(510);
        assert!(
            !splicers.is_empty(),
            "no dune splicer in a hardmode sandstorm"
        );
        assert!(
            splicers.iter().all(|row| *row == 1),
            "a dune splicer did not burrow ten tiles in: {splicers:?}",
        );
        let tumbleweeds = rows(546);
        assert!(
            tumbleweeds.iter().all(|row| *row == 3),
            "a tumbleweed was moved off its ledge: {tumbleweeds:?}",
        );
    }

    /// The sandshark's flavour is decided by the sand under the spawn, not by the player's zone -
    /// the opposite of how the ghouls above resolve their evil (`NPC.cs:3979-3991`, three
    /// `TileID.Sets` tests against `tileType`).
    ///
    /// Neutralised by returning a bare `542` from `sandstorm_pick`'s sandshark arm: all three
    /// converted sandsharks stop being reachable and this fails on the first of them.
    #[test]
    fn the_sandshark_takes_the_flavour_of_the_sand_it_swims_in() {
        let storm = EventSpawns {
            sandstorm: true,
            hard_mode: true,
            ..quiet()
        };
        for (sand, shark, name) in [
            (53u16, 542u16, "Sand/SandShark"),
            (112, 543, "Ebonsand/SandsharkCorrupt"),
            (234, 544, "Crimsand/SandsharkCrimson"),
            (116, 545, "Pearlsand/SandsharkHallow"),
        ] {
            let world = sandstorm_desert(sand);
            let set: std::collections::BTreeSet<u16> =
                spawns_with(&world, &storm, 400, 160, 30_000)
                    .into_iter()
                    .map(|(ty, _)| ty)
                    .collect();
            assert!(set.contains(&shark), "{name}: no {shark} drawn: {set:?}");
            for other in [542u16, 543, 544, 545] {
                assert!(
                    other == shark || !set.contains(&other),
                    "{name}: {other} drawn over the wrong sand: {set:?}",
                );
            }
        }
    }

    /// A Harpy can be drawn at sky height, which is the whole bug: `Depth` had no sky band and
    /// nothing walked the sky, so every candidate tile up there was thrown away by the descent to
    /// ground and NPC 48 appeared in no pool at all.
    ///
    /// Neutralised by forcing `sky` to `false` in `try_spawn`'s candidate loop: no Harpy is drawn
    /// in ten thousand ticks, and the assertion below fails.
    #[test]
    fn a_harpy_can_be_drawn_in_the_sky() {
        let world = sky_world();
        // Row 40 is well above the sky line; column 300 is outside the middle tenth of the map
        // that `NPC.cs:981` excludes before hardmode (800 * 0.45 = 360).
        let seen = spawns_at(&world, false, 300, 40, 10_000);
        assert!(
            seen.contains(&HARPY),
            "nothing found a harpy in the sky: {} spawns, {:?}",
            seen.len(),
            seen.iter().collect::<std::collections::BTreeSet<_>>(),
        );
    }

    /// ...and a Wyvern once the wall is down, which is the hardmode half of the same branch
    /// (`NPC.cs:1412`, one attempt in ten).
    ///
    /// Neutralised two ways, each failing this test: forcing `sky` to `false` as above, and
    /// dropping the `hard_mode` term from `sky_pick`'s Wyvern arm so it can never be reached.
    #[test]
    fn a_wyvern_can_be_drawn_in_the_sky_in_hardmode() {
        let world = sky_world();
        let seen = spawns_at(&world, true, 300, 40, 10_000);
        assert!(
            seen.contains(&WYVERN_HEAD),
            "hardmode sky produced no wyvern: {} spawns, {:?}",
            seen.len(),
            seen.iter().collect::<std::collections::BTreeSet<_>>(),
        );
        // The Harpy is still the sky's default, not something the Wyvern replaced.
        assert!(seen.contains(&HARPY), "hardmode sky produced no harpy");
    }

    /// The two height bands and the middle-of-the-map exclusion, read off the tile the way
    /// `FindSpawnTile` reads it (`NPC.cs:979-986`).
    ///
    /// Before hardmode the middle tenth of the map is not sky at all, which is what keeps harpies
    /// off a fresh world's spawn point; hardmode drops that exclusion and adds the second band,
    /// down to `worldSurface * 0.45`, at one attempt in ten.
    #[test]
    fn the_sky_starts_where_the_game_says_it_does() {
        let world = sky_world(); // surface 200, width 800: sky line 70, deep line 90.
        let mut rng = SmallRng::seed_from_u64(4);
        let outside = 300; // < 800 * 0.45
        let middle = 400; // inside 360..=440

        assert!(sky_tile(&world, outside, 69, false, &mut rng));
        assert!(!sky_tile(&world, outside, 70, false, &mut rng));
        assert!(
            !sky_tile(&world, middle, 40, false, &mut rng),
            "the middle tenth is not sky before hardmode",
        );
        assert!(
            sky_tile(&world, middle, 40, true, &mut rng),
            "hardmode drops the middle-of-the-map exclusion",
        );
        // The second band only exists in hardmode, and only one attempt in ten.
        assert!(!sky_tile(&world, outside, 80, false, &mut rng));
        assert!(
            (0..200).any(|_| sky_tile(&world, outside, 80, true, &mut rng)),
            "hardmode's second band never fired",
        );
        assert!(
            (0..200).any(|_| !sky_tile(&world, outside, 80, true, &mut rng)),
            "the second band is a roll, not a certainty",
        );
        assert!(
            !sky_tile(&world, outside, 90, true, &mut rng),
            "and it stops at worldSurface * 0.45",
        );
    }

    #[test]
    fn nothing_spawns_without_players() {
        let world = test_world();
        let npcs = NpcStore::new();
        let mut rng = SmallRng::seed_from_u64(1);
        assert!(
            try_spawn(
                &world,
                &npcs,
                &[],
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            )
            .is_empty()
        );
    }

    /// A base with three or more residents stops producing monsters entirely (C1-b item 8):
    /// `NPC.cs:917-921`'s own classic-mode `spawnFriendly = true` on every attempt. What that
    /// attempt draws instead is now a real thing, not a dropped spawn: the game spawns harmless
    /// critters near a populated base, not nothing (`NPC.cs:2099-2624`). So a minute of ticks
    /// produces spawns, and every one of them is a damage-0 critter, never a monster. Fails before
    /// the critter table was wired: `try_spawn` used to skip the friendly attempt outright, so the
    /// count was flatly zero and the "some critters appear" half of this could never hold.
    #[test]
    fn a_populated_base_produces_critters_and_no_monsters() {
        use terrustia_proto::npc_data::npc_stats;
        const GUIDE: u16 = 22;
        let world = test_world();
        let mut npcs = NpcStore::new();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        // A town cannot quiet an evil (`NPC.cs:800`'s `!flag` clause), and this world's own spawn
        // point happens to sit in one, so find a plain forest column to build the town in instead.
        let py = i32::from(world.spawn_y);
        let px = (260..540)
            .step_by(4)
            .find(|&x| {
                !matches!(
                    biome_at(&world, x, py),
                    Biome::Corruption | Biome::Crimson | Biome::Ocean
                )
            })
            .expect("the test world has somewhere that is not an evil");
        player.position = (px as f32 * 16.0, py as f32 * 16.0);
        // Three townsfolk standing right where the player is, well inside town_npcs_near's reach.
        for _ in 0..3 {
            npcs.spawn(GUIDE, player.position);
        }
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(13);
        let mut spawned = 0;
        for _ in 0..3600 {
            for (npc_type, _) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            ) {
                spawned += 1;
                let stats = npc_stats(npc_type).expect("a real type");
                assert_eq!(
                    stats.damage, 0,
                    "a populated base spawned a monster ({}), not a critter",
                    stats.name
                );
            }
        }
        assert!(
            spawned > 0,
            "a populated base should still produce friendly critters, not nothing"
        );
    }

    /// Build a world whose middle is a dungeon: an open pocket around `(cx, cy)` with a dungeon-
    /// brick floor and a deep dungeon-brick fill below it, enough brick in the scan box for
    /// `biome_at` to read Dungeon. Returns the world and the tile centre.
    fn dungeon_world() -> (World, (i32, i32)) {
        use terrustia_proto::tile::Tile;
        const DUNGEON_BRICK: u16 = 41;
        let mut world = test_world();
        let cx = world.width() / 2;
        let cy = i32::from(world.surface) + 70; // underground, clear of the surface
        for yy in (cy - 55)..=(cy + 55) {
            for xx in (cx - 110)..=(cx + 110) {
                // Air at and above the walk level, solid dungeon brick below it.
                let tile = if yy <= cy {
                    Tile::AIR
                } else {
                    Tile::block(DUNGEON_BRICK)
                };
                world.set_tile(xx, yy, tile);
            }
        }
        assert_eq!(
            biome_at(&world, cx, cy),
            Biome::Dungeon,
            "the middle is a dungeon"
        );
        (world, (cx, cy))
    }

    /// One live player standing on a tile, which is all `try_spawn` needs of a player list.
    fn player_standing_at(cx: i32, cy: i32) -> Vec<Option<Player>> {
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (cx as f32 * 16.0, cy as f32 * 16.0);
        vec![Some(player)]
    }

    /// The dungeon before Skeletron is beaten spawns the Dungeon Guardian (68), not its ordinary
    /// residents (`NPC.cs:2646-2654`). Once Skeletron is down the same dungeon spawns Angry Bones
    /// and the rest and never the Guardian. Fails before the gate: a fresh dungeon spawned ordinary
    /// enemies a new character could farm.
    #[test]
    fn the_dungeon_gates_on_the_guardian_before_skeletron() {
        let (mut world, (cx, cy)) = dungeon_world();
        let npcs = NpcStore::new();
        let players = player_standing_at(cx, cy);

        // Skeletron not yet beaten: every spawn the dungeon offers is the Guardian.
        world.progress.downed_boss3 = false;
        let mut rng = SmallRng::seed_from_u64(3);
        let mut before = 0;
        for _ in 0..40_000 {
            for (npc_type, _) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            ) {
                assert_eq!(
                    npc_type, DUNGEON_GUARDIAN,
                    "pre-Skeletron dungeon spawned an ordinary enemy"
                );
                before += 1;
            }
        }
        assert!(
            before > 0,
            "the pre-Skeletron dungeon never spawned anything"
        );

        // Skeletron down: the ordinary dungeon pool returns and the Guardian never does.
        world.progress.downed_boss3 = true;
        let mut rng = SmallRng::seed_from_u64(3);
        let mut after = 0;
        for _ in 0..40_000 {
            for (npc_type, _) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            ) {
                assert_ne!(
                    npc_type, DUNGEON_GUARDIAN,
                    "the Guardian should be gone once Skeletron is down"
                );
                // The bound Mechanic's own gate is exactly "the dungeon, after Skeletron"
                // (`NPC.cs:2656`), so she is a correct find here rather than a stray draw from the
                // pool. Rescues are not what this test is about.
                let stats = terrustia_proto::npc_data::npc_stats(npc_type).expect("a real type");
                if stats.friendly {
                    continue;
                }
                assert!(
                    pool(depth_at(&world, cy), Biome::Dungeon, world.day_time).contains(&npc_type),
                    "post-Skeletron dungeon spawned {npc_type}, not a dungeon regular",
                );
                after += 1;
            }
        }
        assert!(
            after > 0,
            "the post-Skeletron dungeon never spawned anything"
        );
    }

    /// The dungeon's fallthrough is mostly the *big* Angry Bones, not the plain one: `NPC.cs:2767`
    /// rolls `Main.rand.Next(5)` and three of its five cases (`:2774-2782`) are 294, 295 and 296,
    /// with only the other two reaching 31 at `:2794`. None of the three is gated on `hardDungeon`,
    /// so they are what a player meets the first time they walk into a cleared dungeon.
    ///
    /// Neutralised by removing the three from `pool`'s dungeon arm: nothing but the four originals
    /// is drawn in forty thousand ticks and the first assertion fails.
    #[test]
    fn the_dungeon_is_mostly_big_angry_bones() {
        let (mut world, (cx, cy)) = dungeon_world();
        world.progress.downed_boss3 = true;
        let npcs = NpcStore::new();
        let players = player_standing_at(cx, cy);

        let mut rng = SmallRng::seed_from_u64(3);
        let mut found = std::collections::BTreeSet::new();
        for _ in 0..40_000 {
            for (npc_type, _) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            ) {
                found.insert(npc_type);
            }
        }
        for (npc_type, name) in [
            (294u16, "AngryBonesBig"),
            (295, "AngryBonesBigMuscle"),
            (296, "AngryBonesBigHelmet"),
        ] {
            assert!(
                found.contains(&npc_type),
                "no {name} ({npc_type}) in a cleared dungeon: {found:?}"
            );
        }
    }

    /// Lay a shelf of `TileID.Books` (50) across one row of a dungeon pocket.
    ///
    /// Books are frame-important (`tile_sets::frame_important`) and not solid
    /// (`tile_solid::solid(50)` is false), so a row of them neither blocks [`open_space`] three
    /// rows above the floor nor gives anything a place to stand: the only thing it changes is what
    /// [`find_nearby_book`] can see.
    fn shelve_books(world: &mut World, cx: i32, row: i32) {
        for xx in (cx - 110)..=(cx + 110) {
            world.set_tile(xx, row, terrustia_proto::Tile::framed(BOOK_TILE, 0, 0));
        }
    }

    /// The dungeon library, `NPC.cs:2748-2766`: one attempt in eight is a Water Bolt Mimic and one
    /// of the rest in ten a Librarian Skeleton, and both of them only where the box around the
    /// candidate holds a bookshelf, and both of them standing *on the shelf* rather than on the
    /// floor the attempt found.
    ///
    /// Fails before the fix on every count: 693 and 694 were in no pool and no branch, so neither
    /// the mimic's play-dead routine (`game::ai::skull`) nor the librarian's caster entry
    /// (`conjuring(693)`) could ever run in a real world.
    ///
    /// Neutralised three ways. Deleting the `if biome == Biome::Dungeon` library block from
    /// `try_spawn` fails the first assertion, "a dungeon library produced no Water Bolt Mimic".
    /// Dropping the `tile.block != BOOK_TILE` half of `find_nearby_book`'s tile test makes every
    /// solid tile a shelf and fails the bookless-dungeon assertion instead. Dropping the
    /// `SAFE_RANGE_X` clause from its screen test fails the shelf loop, "694 spawned on a shelf
    /// the player is looking at"; note that
    /// `spawns_appear_outside_the_safe_zone_and_on_solid_ground` does *not* catch that one, since
    /// its world has no bookshelf in it, so the assertion here is the only guard on it.
    #[test]
    fn the_dungeon_library_stands_its_pair_on_a_bookshelf() {
        let (mut world, (cx, cy)) = dungeon_world();
        world.progress.downed_boss3 = true;
        // Three rows above the walk level, which is inside the 32-tall box (`spawnTileY - 16`) and
        // clear of `open_space`'s own three rows.
        let shelf = cy - 3;
        let bookless = world.clone();
        shelve_books(&mut world, cx, shelf);
        assert_eq!(
            biome_at(&world, cx, cy),
            Biome::Dungeon,
            "a row of shelves must not stop the pocket reading as a dungeon"
        );

        let library = |world: &World, ticks: u32| {
            spawns_with(world, &quiet(), cx, cy, ticks)
                .into_iter()
                .filter(|(ty, _)| *ty == WATER_BOLT_MIMIC || *ty == LIBRARIAN_SKELETON)
                .collect::<Vec<_>>()
        };

        // One in eight and one in eleven of a dungeon's attempts, so this wants a long run: the
        // pre-Skeletron flat rate of 10 (`NPC.cs:787-790`) is gone once `downedBoss3` is set and
        // the dungeon is back on the ordinary rate. This one measures 657 mimics and 474
        // librarians, which is the 8-to-11 split the two `else if` rolls predict.
        const TICKS: u32 = 400_000;
        let found = library(&world, TICKS);
        assert!(
            found.iter().any(|(ty, _)| *ty == WATER_BOLT_MIMIC),
            "a dungeon library produced no Water Bolt Mimic"
        );
        assert!(
            found.iter().any(|(ty, _)| *ty == LIBRARIAN_SKELETON),
            "a dungeon library produced no Librarian Skeleton"
        );

        // Every one of them stands on the shelf row, not on the floor the candidate was found on
        // (`SpawnNPC(bookPosition.X * 16 + 8, bookPosition.Y * 16, ...)`), and outside the box the
        // player can see (`checkPlayerScreenRanges: true`).
        for (ty, (bx, by)) in &found {
            assert_eq!(
                (by / 16.0) as i32,
                shelf,
                "{ty} spawned off the shelf at ({bx}, {by})"
            );
            assert!(
                ((bx / 16.0) as i32 - cx).abs() >= SAFE_RANGE_X,
                "{ty} spawned on a shelf the player is looking at"
            );
        }

        // The same dungeon with nothing to read has neither of them: the shelf is the whole gate,
        // and `AI_FindNearbyBook` returning false is what makes the attempt fall through.
        assert!(
            library(&bookless, TICKS).is_empty(),
            "a dungeon with no bookshelf produced the library pair anyway"
        );

        // ...and so does a cavern lined with shelves, because the arm is inside `else if
        // (ZoneDungeon)` (`NPC.cs:2629`) and a bookcase in a cave is not a library.
        let floor = 300;
        let mut cave = hall_world(floor);
        assert_eq!(biome_at(&cave, 400, floor - 1), Biome::Forest);
        shelve_books(&mut cave, 400, floor - 4);
        assert!(
            spawns_with(&cave, &quiet(), 400, floor - 1, TICKS)
                .into_iter()
                .all(|(ty, _)| ty != WATER_BOLT_MIMIC && ty != LIBRARIAN_SKELETON),
            "a bookshelf outside the dungeon produced the library pair"
        );
    }

    /// `AI_FindNearbyBook` on its own (`NPC.cs:62954-63010`), asked the way the spawner asks it.
    #[test]
    fn the_nearest_book_is_nearest_the_corner_of_the_box() {
        let mut world = flat_world(300);
        // Far enough from the "player" that nothing here is inside the screen box.
        let player = (10_000, 10_000);
        let corner = (400, 200);
        let book = |world: &mut World, x: i32, y: i32| {
            world.set_tile(x, y, terrustia_proto::Tile::framed(BOOK_TILE, 0, 0));
        };

        // Nothing to find, and `tile.type != 50` means exactly that: a box full of ordinary stone
        // is a box with no shelf in it.
        assert_eq!(find_nearby_book(&world, corner, player), None);
        let mut stone = flat_world(300);
        stone.set_tile(425, 225, terrustia_proto::Tile::block(1));
        assert_eq!(find_nearby_book(&stone, corner, player), None);

        // One book anywhere in the 32-by-32 box is found; one outside it is not.
        book(&mut world, 425, 225);
        assert_eq!(find_nearby_book(&world, corner, player), Some((425, 225)));
        let mut outside = flat_world(300);
        book(&mut outside, 432, 225);
        assert_eq!(find_nearby_book(&outside, corner, player), None);

        // `vector2` is `searchPosition`, the box's top-left corner, so the winner is the book
        // nearest *that* and not the one nearest the middle of the box where the candidate stood.
        // (416, 216) is the middle; (405, 205) is far from it and much closer to the corner.
        book(&mut world, 405, 205);
        assert_eq!(find_nearby_book(&world, corner, player), Some((405, 205)));

        // A book sitting exactly on the corner reads as no book at all: `vector` never moves off
        // `vector2`, and the tail returns false (`:62998-63002`).
        let mut on_corner = flat_world(300);
        book(&mut on_corner, corner.0, corner.1);
        assert_eq!(find_nearby_book(&on_corner, corner, player), None);

        // `checkPlayerScreenRanges: true`: a shelf the player can see is skipped and the next one
        // out wins instead. The test is `|dx| < 62 && |dy| < 35`, so a player at (363, 205) has
        // (405, 205) inside the box (42 across) and (425, 225) exactly on its edge (62), and only
        // the first is hidden.
        let near = (425 - SAFE_RANGE_X, 205);
        assert_eq!(
            find_nearby_book(&world, corner, near),
            Some((425, 225)),
            "the close book should have been hidden by the player's own screen"
        );
    }

    /// Paint one wall id across the whole dungeon pocket, so every candidate spot in it reads the
    /// same brick style. The unsafe dungeon walls are deliberately not in `Main.wallHouse`
    /// (`Main.cs`, which never sets 7, 8, 9 or 94 to 99), so painting them does not make the pocket
    /// a house and does not disqualify it as a spawn spot.
    fn paint_dungeon_wall(world: &mut World, cx: i32, cy: i32, wall: u16) {
        for yy in (cy - 55)..=(cy + 55) {
            for xx in (cx - 110)..=(cx + 110) {
                let mut tile = world.tile(xx, yy);
                tile.wall = wall;
                world.set_tile(xx, yy, tile);
            }
        }
    }

    /// The hardmode dungeon is sorted by the *shape* of its masonry, not by its colour.
    ///
    /// `num40` (`NPC.cs:2631-2645`) reads 94, 96 and 98 (the blue, pink and green *slab* walls) as
    /// one group and 95, 97 and 99 (their *tile* walls) as another, with plain brick and everything
    /// else as a third (`WallID.cs:257-267`). Each group owns three quarters of the post-Plantera
    /// roster (`NPC.cs:2661-2722`): slab gives the Rusty armoured bones, the Ragged Casters and the
    /// Skeleton Sniper, tile the Hell armoured bones, the Diabolists and the Tactical Skeleton, and
    /// brick the Blue armoured bones, the Necromancers, the Skeleton Commando and the Paladin. Bone
    /// Lee is the one arm that ignores the wall.
    ///
    /// Neutralised twice. Removing the `hard_dungeon_pick` block from `try_spawn` leaves every one
    /// of the twenty-five types missing and the first assertion fails on 269. Making
    /// `dungeon_brick_style` answer a constant 0 makes the wall unreadable: the slab and tile phases
    /// find none of their own roster and fail the same way, while the brick phase still passes,
    /// which is exactly the shape of the bug this catches.
    #[test]
    fn the_hardmode_dungeon_is_sorted_by_its_masonry() {
        let (mut world, (cx, cy)) = dungeon_world();
        world.progress.downed_boss3 = true;
        let npcs = NpcStore::new();
        let players = player_standing_at(cx, cy);
        let events = EventSpawns {
            hard_mode: true,
            downed_plantera: true,
            ..quiet()
        };

        // The wall to paint, that style's own roster (its four armoured bones first), and one
        // armoured bones id belonging to a different style.
        for (wall, own, foreign) in [
            (
                94u16,
                [269u16, 270, 271, 272, 281, 282, 289, 291].as_slice(),
                273u16,
            ),
            (95, [277, 278, 279, 280, 285, 286, 289, 292].as_slice(), 273),
            (7, [273, 274, 275, 276, 283, 284, 290, 293].as_slice(), 269),
        ] {
            paint_dungeon_wall(&mut world, cx, cy, wall);
            let mut rng = SmallRng::seed_from_u64(11);
            let mut counts = std::collections::BTreeMap::<u16, u32>::new();
            // One cache for the phase rather than a fresh one per tick, which is what the server
            // itself holds: the player never moves, so the scan it takes on the first tick stays
            // good for all of them, and fifty thousand full biome scans become one.
            let mut cache = BiomeCache::default();
            for _ in 0..50_000 {
                for (npc_type, _) in try_spawn(
                    &world,
                    &npcs,
                    &players,
                    &events,
                    &JourneyPowers::default(),
                    &mut cache,
                    &mut rng,
                ) {
                    *counts.entry(npc_type).or_default() += 1;
                }
            }
            for want in own {
                assert!(
                    counts.contains_key(want),
                    "wall {wall}: no {want} among {counts:?}"
                );
            }
            assert!(
                counts.contains_key(&287),
                "wall {wall}: no Bone Lee, who belongs to every style"
            );
            // The one-in-seven reroll (`NPC.cs:2642-2645`) still lets the other two styles through,
            // so this is a skew and not an absence: a spot's own armoured bones should outnumber a
            // foreign style's by roughly nineteen to one.
            let mine: u32 = own[..4].iter().filter_map(|t| counts.get(t)).sum();
            let theirs: u32 = (foreign..foreign + 4).filter_map(|t| counts.get(&t)).sum();
            assert!(
                mine > theirs * 3,
                "wall {wall}: {mine} of its own armoured bones against {theirs} foreign"
            );
        }
    }

    #[test]
    fn spawns_appear_outside_the_safe_zone_and_on_solid_ground() {
        let world = test_world();
        let npcs = NpcStore::new();
        let mut rng = SmallRng::seed_from_u64(9);

        let (tx, ty) = (world.spawn_x as i32, world.spawn_y as i32);
        let (tx_px, ty_px) = (tx as f32 * 16.0, ty as f32 * 16.0);
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (tx_px, ty_px);
        let players = vec![Some(player)];

        // Run many ticks so the one-in-600 roll fires repeatedly.
        let mut seen = 0;
        for _ in 0..20_000 {
            for (npc_type, (px, py)) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            ) {
                seen += 1;
                assert!(
                    terrustia_proto::npc_data::npc_stats(npc_type).is_some(),
                    "spawned an unknown type {npc_type}"
                );
                let (x, y) = ((px / 16.0) as i32, (py / 16.0) as i32);
                assert!(
                    (x - tx).abs() >= SAFE_RANGE_X || (y - ty).abs() >= SAFE_RANGE_Y,
                    "spawned inside the safe zone at ({x}, {y}) vs player ({tx}, {ty})"
                );
                assert!(has_room(&world, x, y), "spawned somewhere with no room");
            }
            if seen > 20 {
                break;
            }
        }
        assert!(seen > 0, "nothing ever spawned");
    }

    #[test]
    fn ground_is_found_by_scanning_down() {
        let world = test_world();
        let x = world.width() / 2;
        // Start well above the terrain; the scan should land on the first solid row.
        let surface = (0..world.height())
            .find(|y| world.tile(x, *y).is_active())
            .expect("the column has ground");
        assert_eq!(find_ground(&world, x, surface - 10), Some(surface));
        // And starting on the ground finds it immediately.
        assert_eq!(find_ground(&world, x, surface), Some(surface));
    }

    #[test]
    fn scanning_gives_up_rather_than_falling_through_the_world() {
        let world = test_world();
        // Deep sky above the terrain, more than the scan depth.
        assert_eq!(find_ground(&world, world.width() / 2, 0), None);
    }

    /// One playing player standing where the test world's spawn point is, for the whole-loop tests
    /// below.
    fn player_at(position: (f32, f32)) -> Vec<Option<Player>> {
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = position;
        vec![Some(player)]
    }

    /// A walled base suppresses spawns inside itself (`NPC.cs:977`).
    ///
    /// Fails before the fix: the candidate loop had no wall test at all, so a fully walled, fully
    /// lit base spawned zombies in its own living room. Built as a flat hall with stone walls behind
    /// every open tile and a stone floor, wide enough that the whole spawn box is inside it.
    #[test]
    fn a_walled_base_suppresses_spawns_inside_itself() {
        let mut world = World::empty(800, 600, "walled");
        let floor = 300;
        // A wide, tall hall: solid floor, and every open tile above it backed by a stone wall.
        for x in 0..world.width() {
            world.set_tile(x, floor, terrustia_proto::Tile::block(1));
            for y in (floor - 120)..floor {
                let mut walled = terrustia_proto::Tile::AIR;
                walled.wall = 4; // stone wall, one of `Main.wallHouse`
                world.set_tile(x, y, walled);
            }
        }
        world.surface = 100;
        world.rock_layer = 200;

        let npcs = NpcStore::new();
        let players = player_at(((world.width() / 2) as f32 * 16.0, (floor - 1) as f32 * 16.0));
        let mut rng = SmallRng::seed_from_u64(11);
        let mut seen = 0;
        for _ in 0..60_000 {
            seen += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        assert_eq!(seen, 0, "{seen} spawns inside a fully walled base");

        // The same hall with the walls stripped out is not safe, so the test is measuring the wall
        // and not some other reason nothing could spawn there.
        for x in 0..world.width() {
            for y in (floor - 120)..floor {
                world.set_tile(x, y, terrustia_proto::Tile::AIR);
            }
        }
        let mut open = 0;
        for _ in 0..60_000 {
            open += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        assert!(open > 0, "an unwalled hall of the same shape should spawn");
    }

    /// `SpawnNPC` (`NPC.cs:291-306`) stops at the first player who spawns something, so a tick
    /// produces at most one NPC however many people are playing.
    ///
    /// Fails before the fix, which gave every player their own draw: a busy server spawned
    /// monsters N times as fast as the game does. Driven hard with journey mode's slider at the top
    /// so the rate is fast enough for two players to collide on the same tick often.
    #[test]
    fn a_tick_spawns_at_most_one_npc_however_many_players_there_are() {
        let mut world = test_world();
        world.game_mode = 3;
        let npcs = NpcStore::new();
        let (tx, ty) = (world.spawn_x as i32, world.spawn_y as i32);

        let mut players = Vec::new();
        for (slot, offset) in [(0u8, 0), (1, 400)] {
            let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
            drop(out_rx);
            let mut player = Player::new(slot, "127.0.0.1:1".parse().unwrap(), out_tx);
            player.state = crate::game::ConnState::Playing;
            player.position = ((tx + offset) as f32 * 16.0, ty as f32 * 16.0);
            players.push(Some(player));
        }

        let mut boosted = JourneyPowers::default();
        boosted.set_spawn_rate_slider(0, 1.0);
        boosted.set_spawn_rate_slider(1, 1.0);

        let mut rng = SmallRng::seed_from_u64(31);
        let mut cache = BiomeCache::default();
        let mut total = 0;
        for tick in 0..40_000u64 {
            cache.advance(tick);
            let batch = try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &boosted,
                &mut cache,
                &mut rng,
            );
            assert!(
                batch.len() <= 1,
                "a tick spawned {} NPCs: {batch:?}",
                batch.len()
            );
            total += batch.len();
        }
        assert!(total > 100, "the run has to actually spawn things: {total}");
    }

    /// The biome cache answers with the scan it would have run, and takes a fresh one once it is a
    /// second old or the player has walked out of its neighbourhood.
    ///
    /// It exists because `biome_at` is 78 us on a full-size world and the rate needs it on every
    /// attempt: uncached, that is 19.8 ms per tick at 255 players, over the whole tick budget.
    #[test]
    fn the_biome_cache_agrees_with_a_fresh_scan_and_refreshes_on_time() {
        let mut world = test_world();
        let (x, y) = (world.width() / 2, i32::from(world.surface) + 10);
        let mut cache = BiomeCache::default();

        let fresh = zones_at(&world, x, y);
        assert_eq!(
            cache.read(&world, 0, x, y),
            Some(fresh),
            "a first read scans"
        );

        // Walking beyond the drift bound takes a new scan: the outer columns read as ocean by
        // position, which is a different answer from the forest just cached.
        assert_ne!(fresh.biome, Biome::Ocean);
        cache.advance(30);
        assert_eq!(cache.read(&world, 0, x + 8, y), Some(fresh), "still fresh");
        assert_eq!(
            cache.read(&world, 0, 10, y).map(|z| z.biome),
            Some(Biome::Ocean),
            "and drifted"
        );
        // A slot of its own is not the same slot.
        assert_eq!(cache.read(&world, 1, x, y), Some(fresh));

        // Age alone is only observable when the world underneath changes, so paint enough
        // ebonstone into the scan box to cross `EVIL_THRESHOLD` and watch the answer follow.
        let mut aged = BiomeCache::default();
        aged.advance(100);
        assert_eq!(aged.read(&world, 0, x, y), Some(fresh));
        for dx in -20..20 {
            for dy in -20..20 {
                world.set_tile(x + dx, y + dy, terrustia_proto::Tile::block(23));
            }
        }
        assert_eq!(biome_at(&world, x, y), Biome::Corruption, "a real evil now");
        aged.advance(100 + BiomeCache::REFRESH - 1);
        assert_eq!(
            aged.read(&world, 0, x, y),
            Some(fresh),
            "a scan under a second old is still used",
        );
        aged.advance(100 + BiomeCache::REFRESH);
        assert_eq!(
            aged.read(&world, 0, x, y).map(|z| z.biome),
            Some(Biome::Corruption),
            "and a second later it is taken again",
        );
    }

    /// A join burst cannot buy a scan for every player on one tick.
    ///
    /// No vanilla line to cite: a real dedicated server never scans at all, because the client runs
    /// `SceneMetrics` and sends its zones up in packet 36. The citation is the measurement. A scan
    /// is 78 us (`examples/biome_scan_cost.rs`), so 255 of them is 19,890 us against a 16,667 us
    /// tick, and a real 255-player soak measured `phase=spawning phase_us=20763` and failed the
    /// release gate on it.
    ///
    /// Fails before the budget: every one of the 255 reads scanned, because nothing bounded them.
    #[test]
    fn a_join_burst_cannot_buy_a_scan_for_every_player_in_one_tick() {
        let world = test_world();
        let (x, y) = (world.width() / 2, i32::from(world.surface) + 10);
        let mut cache = BiomeCache::default();

        // Every slot arrives with no entry at all, which is the first fill: 255 clients becoming
        // playable on the same tick.
        cache.advance(1);
        for slot in 0..255 {
            let _ = cache.read(&world, slot, x, y);
        }
        let scanned = (0..255).filter(|s| cache.last(*s).is_some()).count();
        assert!(
            scanned <= BiomeCache::BUDGET as usize,
            "{scanned} scans on one tick, budget is {}",
            BiomeCache::BUDGET,
        );

        // Everyone is served inside the refresh window rather than starved.
        for tick in 2..=64 {
            cache.advance(tick);
            for slot in 0..255 {
                let _ = cache.read(&world, slot, x, y);
            }
        }
        assert!(
            (0..255).all(|s| cache.last(s).is_some()),
            "every slot should have an answer within the refresh window",
        );

        // And the stale path is budgeted too, not only the empty one. Moving every player past
        // DRIFT on the same tick is synchronised expiry in its instant form, and the outer columns
        // read as ocean, so a rescan is visible as a changed answer.
        cache.advance(65);
        for slot in 0..255 {
            let _ = cache.read(&world, slot, 10, y);
        }
        let moved = (0..255)
            .filter(|s| cache.last(*s).map(|z| z.biome) == Some(Biome::Ocean))
            .count();
        assert!(
            moved <= BiomeCache::BUDGET as usize,
            "{moved} rescans on one tick, budget is {}",
            BiomeCache::BUDGET,
        );
    }

    /// Nothing spawns while a Moon Lord is on the field near you (`NPC.cs:358-362`,
    /// `MoonLordFightingDistance = 4500` at `NPC.cs:6036`).
    ///
    /// Fails before the fix, which had no suppression at all: the fight came with whatever the
    /// surface would ordinarily have sent, on top of the Moon Lord and its parts.
    #[test]
    fn a_moon_lord_on_the_field_stops_everything_else_spawning() {
        const MOON_LORD: u16 = 398;
        let world = test_world();
        let (tx, ty) = (world.spawn_x as i32, world.spawn_y as i32);
        let players = player_at((tx as f32 * 16.0, ty as f32 * 16.0));

        let count = |npcs: &NpcStore, seed: u64| {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut seen = 0;
            for _ in 0..20_000 {
                seen += try_spawn(
                    &world,
                    npcs,
                    &players,
                    &quiet(),
                    &JourneyPowers::default(),
                    &mut BiomeCache::default(),
                    &mut rng,
                )
                .len();
            }
            seen
        };

        assert!(count(&NpcStore::new(), 4) > 0, "the world spawns normally");

        // A Moon Lord standing on the player.
        let mut npcs = NpcStore::new();
        npcs.spawn(MOON_LORD, (tx as f32 * 16.0, ty as f32 * 16.0))
            .expect("a slot");
        assert_eq!(count(&npcs, 4), 0, "not while the Moon Lord is here");

        // ...and one 500 tiles away is well past the 4500 px reach, so it suppresses nothing.
        let mut far = NpcStore::new();
        far.spawn(MOON_LORD, ((tx + 500) as f32 * 16.0, ty as f32 * 16.0))
            .expect("a slot");
        assert!(count(&far, 4) > 0, "a distant Moon Lord is not this fight");
    }

    /// End to end: a wall at the player's back keeps Devourers out of the draw while the rest of
    /// the corruption still spawns (`NPC.cs:411`, `:3704`).
    ///
    /// Fails before the fix, when `noWorms` was not modelled: a walled base in the corruption still
    /// had Devourers coming through the floor.
    #[test]
    fn a_wall_at_your_back_keeps_devourers_out_of_a_corrupt_pool() {
        const DEVOURER: u16 = 7;
        let mut world = World::empty(800, 600, "corrupt");
        world.surface = 100;
        world.rock_layer = 200;
        let floor = 90;
        // A wide band of ebonstone, well past `EVIL_THRESHOLD`, with a floor to stand on.
        for x in 250..550 {
            for y in floor..floor + 30 {
                world.set_tile(x, y, terrustia_proto::Tile::block(23));
            }
        }
        let (px, py) = (400, floor - 1);
        let npcs = NpcStore::new();
        let players = player_at((px as f32 * 16.0, py as f32 * 16.0));

        let run = |world: &World, seed: u64| {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut seen = std::collections::HashSet::new();
            for _ in 0..40_000 {
                for (npc_type, _) in try_spawn(
                    world,
                    &npcs,
                    &players,
                    &quiet(),
                    &JourneyPowers::default(),
                    &mut BiomeCache::default(),
                    &mut rng,
                ) {
                    seen.insert(npc_type);
                }
            }
            seen
        };

        assert_eq!(biome_at(&world, px, py), Biome::Corruption);
        let wild = run(&world, 12);
        assert!(
            wild.contains(&DEVOURER),
            "an unwalled corruption should send Devourers: {wild:?}",
        );

        // Now put a house wall behind the player, and nothing else.
        let mut walled = world.tile(px, py);
        walled.wall = 4;
        world.set_tile(px, py, walled);

        let sheltered = run(&world, 12);
        assert!(
            !sheltered.contains(&DEVOURER),
            "a wall at your back stops burrowers: {sheltered:?}",
        );
        assert!(
            !sheltered.is_empty(),
            "but not everything else, or this proves nothing",
        );
    }

    /// Lava is refused at any depth and water is not refused at all (`NPC.cs:5431-5442`).
    ///
    /// Fails before the fix, which tested `liquid > 200` without looking at the kind: it rejected
    /// deep water the game permits and accepted shallow lava the game forbids.
    #[test]
    fn lava_is_refused_at_any_depth_and_water_is_not_refused_at_all() {
        use terrustia_proto::tile::Liquid;
        let mut world = World::empty(200, 200, "liquids");
        let floor = 100;
        for x in 90..110 {
            world.set_tile(x, floor, terrustia_proto::Tile::block(1));
        }

        // Dry: room to stand.
        assert!(has_room(&world, 100, floor - 1));

        // Filled to the brim with water: still room, because a shark lives there.
        for dy in 1..=3 {
            world.set_tile(
                100,
                floor - dy,
                terrustia_proto::Tile::AIR.with_liquid(Liquid::Water, 255),
            );
        }
        assert!(
            has_room(&world, 100, floor - 1),
            "deep water is where the ocean roster lives",
        );

        // A single drop of lava, far short of the old 200 threshold, is refused.
        world.set_tile(
            100,
            floor - 1,
            terrustia_proto::Tile::AIR.with_liquid(Liquid::Lava, 1),
        );
        assert!(
            !has_room(&world, 100, floor - 1),
            "`anyLava()` is about the kind, not the depth",
        );
    }

    /// Water draws the aquatic roster and dry land does not (`NPC.cs:1798`, `:1988`).
    ///
    /// Fails before the fix twice over: the ocean roster was in the *land* pool, so sharks appeared
    /// on dry sand, and `has_room` refused the water they should actually have come from.
    #[test]
    fn the_ocean_roster_comes_out_of_water_and_not_off_the_sand() {
        let ocean_water = water_pool(Depth::Surface, Biome::Ocean);
        assert!(
            ocean_water.contains(&65) && ocean_water.contains(&221),
            "the shark and the squid are the ocean's water roster: {ocean_water:?}",
        );
        for &wet in ocean_water {
            assert!(
                !pool(Depth::Surface, Biome::Ocean, true).contains(&wet)
                    && !pool(Depth::Surface, Biome::Ocean, false).contains(&wet),
                "{wet} is aquatic and must not be drawable from dry ocean sand",
            );
        }
        // Below the surface, still water, a different roster.
        assert_eq!(water_pool(Depth::Cavern, Biome::Forest), &[63]);
        assert!(water_pool(Depth::Surface, Biome::Forest).is_empty());

        // And end to end: a player floating in a walled-off sea gets the water roster.
        let mut world = World::empty(800, 600, "sea");
        world.surface = 100;
        world.rock_layer = 200;
        let floor = 150;
        for x in 0..world.width() {
            world.set_tile(x, floor, terrustia_proto::Tile::block(1));
            for y in (floor - 60)..floor {
                world.set_tile(
                    x,
                    y,
                    terrustia_proto::Tile::AIR
                        .with_liquid(terrustia_proto::tile::Liquid::Water, 255),
                );
            }
        }
        // `biome_at` calls the outer 250 columns ocean, so stand there.
        let npcs = NpcStore::new();
        let players = player_at((100.0 * 16.0, (floor - 30) as f32 * 16.0));
        let mut rng = SmallRng::seed_from_u64(5);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..60_000 {
            for (npc_type, _) in try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            ) {
                seen.insert(npc_type);
            }
        }
        assert!(!seen.is_empty(), "nothing spawned in the sea at all");
        for npc_type in &seen {
            // A bound resident is a rescue rather than a spawn from any roster, and vanilla's own
            // ocean branch is where the Angler is found (`NPC.cs:1800`), so they are not the
            // subject here.
            let stats = terrustia_proto::npc_data::npc_stats(*npc_type).expect("a real type");
            if stats.friendly {
                continue;
            }
            assert!(
                ocean_water.contains(npc_type),
                "the sea drew {npc_type} ({}), which is not in its water roster",
                stats.name,
            );
        }
    }

    /// A flat world with a floor to stand on, for the tower-zone tests below.
    fn flat_world(floor: i32) -> World {
        flat_world_of(floor, 1)
    }

    /// The same, with the floor made of something in particular: the two zone flags this module
    /// classifies and every mushroom arm are decided by what the ground is made of.
    fn flat_world_of(floor: i32, block: u16) -> World {
        let mut world = World::empty(800, 600, "pillar arena");
        world.surface = 100;
        world.rock_layer = 200;
        for x in 0..world.width() {
            for y in floor..(floor + 4) {
                world.set_tile(x, y, terrustia_proto::Tile::block(block));
            }
        }
        world
    }

    /// Everything a pillar standing `offset` pixels from the player draws over `ticks` attempts.
    fn tower_spawns(
        world: &World,
        pillar: u16,
        at: (i32, i32),
        offset: f32,
        ticks: u32,
    ) -> Vec<u16> {
        let mut towers = [None; 4];
        let slot = crate::game::lunar::PILLARS
            .iter()
            .position(|p| *p == pillar)
            .expect("a real pillar");
        towers[slot] = Some((at.0 as f32 * 16.0 + offset, at.1 as f32 * 16.0));

        let events = EventSpawns {
            hard_mode: true,
            towers,
            ..quiet()
        };
        let npcs = NpcStore::new();
        let players = player_at((at.0 as f32 * 16.0, at.1 as f32 * 16.0));
        let mut rng = SmallRng::seed_from_u64(0x10_0000 + u64::from(pillar));
        let mut biomes = BiomeCache::default();
        let mut seen = Vec::new();
        for _ in 0..ticks {
            seen.extend(
                try_spawn(
                    world,
                    &npcs,
                    &players,
                    &events,
                    &JourneyPowers::default(),
                    &mut biomes,
                    &mut rng,
                )
                .into_iter()
                .map(|(npc_type, _)| npc_type),
            );
        }
        seen
    }

    /// Inside a pillar's zone, that pillar's escort is the only thing the world produces, and
    /// outside it none of the escort appears at all.
    ///
    /// This is the whole Lunar Apocalypse. A pillar's shield is a count of its own escort killed
    /// (`game/lunar.rs`) and a pillar takes no damage while the shield holds, so with nothing
    /// spawning the four towers were indestructible and the Moon Lord unreachable.
    ///
    /// Neutralised by deleting the `if let Some(pillar) = tower` arm from `try_spawn`'s candidate
    /// loop: every zone then draws the surface forest roster instead and the first assertion in
    /// each arm fails on a Zombie.
    #[test]
    fn a_pillar_zone_spawns_its_own_escort_and_nothing_else() {
        use crate::game::lunar;

        let floor = 150;
        let world = flat_world(floor);
        let at = (400, floor - 2);

        // The four rosters, `NPC.cs:1302`, `:1328`, `:1349`, `:1364`, as sets.
        let rosters: [(u16, &[u16]); 4] = [
            (lunar::NEBULA, &[420, 421, 423, 424]),
            (lunar::VORTEX, &[425, 426, 427, 429]),
            (lunar::STARDUST, &[402, 405, 407, 409, 411]),
            (lunar::SOLAR, &[412, 415, 416, 417, 418, 419, 518]),
        ];

        for (pillar, roster) in rosters {
            let seen = tower_spawns(&world, pillar, at, 0.0, 4_000);
            assert!(
                !seen.is_empty(),
                "pillar {pillar} spawned nothing at all in four thousand ticks",
            );
            for npc_type in &seen {
                assert!(
                    roster.contains(npc_type),
                    "pillar {pillar}'s zone drew {npc_type}, which is not on its list",
                );
            }
            // ...and every uncapped entry really is reachable, so a typo in one weight cannot
            // quietly drop a type out of the fight.
            let drawn: std::collections::BTreeSet<u16> = seen.into_iter().collect();
            for npc_type in roster {
                assert!(
                    drawn.contains(npc_type),
                    "pillar {pillar}'s zone never drew {npc_type} in four thousand ticks",
                );
            }
        }

        // `SceneMetrics.NPCEventZoneRadius` is 4000 px (`SceneMetrics.cs:130`), so a pillar half a
        // world away is not a zone: the ordinary surface roster answers instead.
        let far = tower_spawns(&world, lunar::SOLAR, at, 5_000.0, 4_000);
        assert!(!far.is_empty(), "the ordinary world stopped spawning too");
        for npc_type in &far {
            assert!(
                lunar::belongs_to(*npc_type).is_none(),
                "{npc_type} spawned five thousand pixels from its pillar",
            );
        }
    }

    /// What the tower-zone test costs on the spawn path, which runs once per player per tick.
    ///
    /// It is deliberately not a scan: a real client works its zone out in `SceneMetrics` by walking
    /// every NPC (`SceneMetrics.cs:734-751`), and doing that here would be another pass over the
    /// store per player per tick on top of the biome scan that already had to be cached to fit.
    /// Instead the caller gathers the four positions once, only while the event is up, and this is
    /// four distance comparisons against a stack array. On an M-series laptop, with the arguments
    /// behind a `black_box` so the loop cannot be hoisted (which costs more than the test itself
    /// does): 1.0 ns with no apocalypse running, which is almost always, 1.4 ns with all four
    /// standing and no zone matched, 1.0 ns standing inside one. At the 255-player bar the worst
    /// case is 0.4 us of a 16.67 ms tick, which is 0.002% of it.
    /// What the graveyard and the seasonal chain cost the tick, which is the thing to be careful
    /// about here: vanilla works `ZoneGraveyard` out by counting tombstones in a 169-by-124 tile box
    /// (`SceneMetrics.cs:16`, `:356`, `:622`), and doing *that* per player per tick would be about
    /// twenty-one thousand tile reads a head, which does not fit. It is not done. The client already
    /// counts them and sends the answer up in packet 36, exactly as it does for every other zone, so
    /// the server's whole graveyard test is one byte and one mask.
    ///
    /// Measured on an M-series laptop, arguments behind a `black_box` so nothing is hoisted:
    ///
    /// * `Player::in_graveyard`: 0.4 ns with no zone packet yet, 0.6 ns with one. That is one
    ///   `Option` test, one bounds-checked byte and a mask. It runs once per player per attempt, so
    ///   at the 255-player bar it is 0.15 us of a 16.67 ms tick: 0.001% of it. A tile scan for the
    ///   same answer would be twenty-one thousand tile reads a head and could not be afforded.
    /// * `seasonal_night_pick` on an ordinary night, the case that runs almost always: 4.2 ns. It
    ///   is reached at most once per tick server-wide, well after the rate roll has let an attempt
    ///   through and `try_spawn` has settled on a tile, so it is nanoseconds per *tick*, not per
    ///   player.
    /// * The same in a graveyard at Halloween with every arm live: 15.3 ns, which is the chain
    ///   actually rolling its dice rather than falling out of the first condition.
    ///
    /// The snow and rain arms (`NPC.cs:4655`, `:4675`) were added to that chain later and were
    /// measured against the same build on the same machine rather than against the numbers above:
    /// 4.89 ns before and 5.50 ns after on the ordinary night, and 17.95 against 16.68 in the
    /// graveyard, so the two arms and the two rolls above them cost about 0.6 ns of a run that
    /// happens at most once per *tick*. Written as an array of the six ice and snow tiles and
    /// `contains` instead, the same measurement was 8.68 ns, which is why it is a `matches!`.
    #[test]
    #[ignore]
    fn measure_the_graveyard_and_the_seasonal_chain() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let mut bare = Player::new(0, "127.0.0.1:1".parse().unwrap(), tx.clone());
        let mut haunted = Player::new(0, "127.0.0.1:1".parse().unwrap(), tx);
        haunted.zone = Some(bytes::Bytes::from_static(&[0, 0, 0, 0, 1 << 6, 0, 0]));

        for (name, player) in [
            ("no zone packet yet", &mut bare),
            ("graveyard", &mut haunted),
        ] {
            let n = 10_000_000;
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for _ in 0..n {
                sink += u32::from(std::hint::black_box(&*player).in_graveyard());
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("in_graveyard, {name}: {each:.2} ns/call (sink {sink})");
        }

        for (name, at) in [
            ("ordinary night", Seasonal::default()),
            (
                "graveyard at Halloween",
                Seasonal {
                    halloween: true,
                    graveyard: true,
                    hard_mode: true,
                    blood_moon: true,
                    moon_phase: 4,
                    ..Seasonal::default()
                },
            ),
        ] {
            let n = 10_000_000;
            let mut rng = SmallRng::seed_from_u64(1);
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for _ in 0..n {
                sink += u32::from(
                    seasonal_night_pick(
                        std::hint::black_box(at),
                        std::hint::black_box(2),
                        &mut rng,
                    )
                    .unwrap_or(0),
                );
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("seasonal_night_pick, {name}: {each:.2} ns/call (sink {sink})");
        }
    }

    /// What the bound town slimes cost the two chains they were added to.
    ///
    /// Neither adds a scan to the common path. `sky_pick` gains one bool read and, only when that
    /// bool is false, one `rand` call; `spawn_frog` is a whole new function but sits behind a `u16`
    /// compare on a draw that only ever happens in a jungle, and its `alive` closure (the one thing
    /// here that walks the NPC table) is the last term of a `&&` chain, so it runs on roughly one
    /// frog draw in thirty and never at all once the slime is freed.
    ///
    /// Measured on an M-series laptop, arguments behind a `black_box`:
    ///
    /// * `sky_pick`, fresh world: 2.49 ns. Freed (the arm short-circuits on the flag): 0.71 ns.
    /// * `spawn_frog`, fresh world: 2.44 ns. Freed: 0.72 ns, the flag read and nothing else.
    ///
    /// The freed numbers are the steady state of any world past its rescues, and they are the flag
    /// read alone: the arm costs nothing at all once the slime has moved in.
    ///
    /// Both are reached at most once per tick server-wide, after the rate roll has let an attempt
    /// through and a tile has been settled on, so this is nanoseconds per *tick*, not per player.
    #[test]
    #[ignore]
    fn measure_the_town_slime_arms() {
        let mut fresh = World::empty(800, 600, "bench");
        fresh.progress.downed_golem = true;
        let mut freed = fresh.clone();
        freed.progress.unlocked_slime_purple = true;
        freed.progress.unlocked_slime_yellow = true;
        let never = |_: u16| false;

        for (name, world) in [("fresh", &fresh), ("freed", &freed)] {
            let n = 10_000_000;
            let mut rng = SmallRng::seed_from_u64(1);
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for _ in 0..n {
                sink += u32::from(sky_pick(
                    false,
                    false,
                    std::hint::black_box(world),
                    false,
                    &never,
                    &mut rng,
                ));
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("sky_pick, {name}: {each:.2} ns/call (sink {sink})");

            let mut rng = SmallRng::seed_from_u64(1);
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for _ in 0..n {
                sink += u32::from(spawn_frog(std::hint::black_box(world), &never, &mut rng));
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("spawn_frog, {name}: {each:.2} ns/call (sink {sink})");
        }
    }

    /// What the Statue Mimic's plinth check costs, since it is the one new per-candidate thing in
    /// [`try_spawn`] that reads tiles rather than bools.
    ///
    /// It is behind `downedBoss3 && ZoneGraveyard && !noWorms` and then a one-in-twenty-five roll,
    /// so an ordinary world never reaches it at all and a graveyard reaches it once in twenty-five
    /// candidates. Measured on this machine, release, with the column walked so no read is served
    /// from the last call's line: 23.14 ns on a plinth (all eight reads taken) and 4.52 ns on a
    /// floor of half bricks, where the first `solid_tile2` shuts it.
    ///
    /// The number that matters is not that one but how often anything reaches it. Both this lane's
    /// arms sit *inside* the candidate loop, which `try_spawn` enters only after the rate roll lets
    /// an attempt through: over 2,000,000 calls on an ordinary forest day the loop resolved 5,223
    /// times, one call in 383. So the whole addition is amortised over that, and it is below what
    /// wall-clock timing can resolve here: `try_spawn` measured 242-527 ns a call with these arms
    /// and 213-290 ns with both cut out, four runs each, the two ranges overlapping and neither
    /// reproducible to better than a third of itself while other lanes share the machine. The
    /// arithmetic is the answer where the clock is not.
    #[test]
    #[ignore]
    fn measure_the_statue_mimic_plinth_check() {
        let floor = 90;
        let plinth = flat_world(floor);
        let half_bricks = {
            let mut world = plinth.clone();
            for x in 0..world.width() {
                let mut tile = terrustia_proto::Tile::block(1);
                tile.flags.set(terrustia_proto::TileFlags::HALF_BRICK, true);
                world.set_tile(x, floor, tile);
            }
            world
        };

        for (name, world) in [("plinth", &plinth), ("half bricks", &half_bricks)] {
            let n = 10_000_000;
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for i in 0..n {
                sink += u32::from(good_place_for_a_statue_mimic(
                    std::hint::black_box(world),
                    200 + (i % 128),
                    floor,
                ));
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("good_place_for_a_statue_mimic, {name}: {each:.2} ns/call (sink {sink})");
        }
    }

    /// What the Rock Golem's gate costs, since it is this lane's one new per-candidate thing in
    /// [`try_spawn`] that does more than look at a pool.
    ///
    /// Its ground tile is [`try_spawn`]'s own `ground_block`, already read for the sandstorm, the
    /// mushroom arms and the Goblin Scout, so the branch adds no tile read of its own before the
    /// roll. The shape is cheapest-first, and each rung of it is measured here: a non-stone floor
    /// (dirt) leaves after three comparisons, plain stone spends a `SmallRng` draw and leaves
    /// forty-nine times in fifty, and only the fiftieth pays for three [`solid_tile`] reads.
    ///
    /// It is measured against [`good_place_for_a_statue_mimic`] in the same run, that being the
    /// nearest thing already in this loop and already accepted as cheap enough for it. Both
    /// numbers below are the `dev` profile (which this workspace has optimised, with debuginfo)
    /// rather than `--release`: the machine had no disk left for a release build with three other
    /// lanes sharing it, and the ratio is the point either way.
    ///
    /// The ground column is walked so no read is served from the last call's cache line. Measured:
    /// 5.32 ns on dirt, 6.56 ns on stone under open sky, 6.54 ns on stone under a ceiling, against
    /// 99.41 ns for the plinth check in the same run. The two stone figures agree because the
    /// ceiling reads only happen on the one attempt in fifty that gets past the roll (the sink
    /// counts 200,037 of 10,000,000, which is that fiftieth).
    ///
    /// So the branch costs about a fifteenth of a check this loop already carries, and only when
    /// `depth == Depth::Cavern`, on a `ground_block` [`try_spawn`] had already read.
    #[test]
    #[ignore]
    fn measure_the_rock_golem_gate() {
        let floor = 300;
        let stone = flat_world_of(floor, STONE);
        let dirt = flat_world_of(floor, 0);
        let roofed = {
            let mut world = stone.clone();
            for x in 0..world.width() {
                world.set_tile(
                    x,
                    floor - ROCK_GOLEM_HEADROOM,
                    terrustia_proto::Tile::block(1),
                );
            }
            world
        };

        for (name, world, block) in [
            ("dirt", &dirt, 0u16),
            ("stone, open", &stone, STONE),
            ("stone, roofed", &roofed, STONE),
        ] {
            let n = 10_000_000;
            let mut rng = SmallRng::seed_from_u64(631);
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for i in 0..n {
                sink += u32::from(check_to_spawn_rock_golem(
                    std::hint::black_box(world),
                    200 + (i % 128),
                    floor,
                    block,
                    true,
                    false,
                    &mut rng,
                ));
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("check_to_spawn_rock_golem, {name}: {each:.2} ns/call (sink {sink})");
        }

        // The reference, in the same run and the same build: an eight-tile-read check that is
        // already in this loop and already accepted as cheap enough for it.
        let plinth = flat_world(90);
        let n = 10_000_000;
        let start = std::time::Instant::now();
        let mut sink = 0u32;
        for i in 0..n {
            sink += u32::from(good_place_for_a_statue_mimic(
                std::hint::black_box(&plinth),
                200 + (i % 128),
                90,
            ));
        }
        let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
        println!("good_place_for_a_statue_mimic, plinth: {each:.2} ns/call (sink {sink})");
    }

    /// What the dungeon library's bookshelf scan costs, since it is by far the widest per-candidate
    /// tile read this module has: 32 by 32 is 1024 tiles against the plinth check's eight.
    ///
    /// It is behind `ZoneDungeon` and then a one-in-eight roll (or one in eleven), so nowhere but a
    /// dungeon reaches it at all and a dungeon reaches it on about one candidate in five. The scan
    /// has no early exit, by design: vanilla's has none either, and a shelf found early cannot end
    /// the search because a later row can still hold one closer to the corner.
    ///
    /// Measured on this machine, release, walking the anchor so no read is served from the previous
    /// call's cache line. The honest reading of the four numbers below is a ratio and a count, not a
    /// wall clock: other lanes share this machine and the same build measured 2.97 us and 62 us for
    /// the same 1024 reads twenty minutes apart. What does hold in every run is that
    /// `find_nearby_book` costs what its 1024 `World::tile` reads cost and no more (the two lines
    /// track each other within a third across five runs, in both directions), and that the empty box
    /// and the shelved box cost the same, because the tile fetch dominates and it is taken on all
    /// 1024 either way. On the quietest run the scan was 2.97 us, which is 2.9 ns a tile read: the
    /// same per-read figure `measure_the_statue_mimic_plinth_check` arrived at independently.
    ///
    /// The number that decides whether that matters is the last one, and it is exact rather than
    /// timed: over 2,000,000 `try_spawn` calls on a dungeon with a shelf running through it, the
    /// scan ran 5,405 times, one call in 370. At 2.97 us that is 8 ns amortised onto a `try_spawn`
    /// that measured 499 ns on the same quiet run, so about 1.6% of it, and nothing at all outside a
    /// dungeon.
    #[test]
    #[ignore]
    fn measure_the_bookshelf_scan() {
        let empty = flat_world(300);
        let shelved = {
            let mut world = empty.clone();
            for x in 0..world.width() {
                world.set_tile(x, 210, terrustia_proto::Tile::framed(BOOK_TILE, 0, 0));
            }
            world
        };

        for (name, world) in [("empty", &empty), ("shelved", &shelved)] {
            let n = 200_000;
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for i in 0..n {
                sink += u32::from(
                    find_nearby_book(
                        std::hint::black_box(world),
                        (200 + (i % 128), 200),
                        (10_000, 10_000),
                    )
                    .is_some(),
                );
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("find_nearby_book, {name}: {each:.2} ns/call (sink {sink})");
        }

        // The same 1024 reads with nothing but the tile fetch, so the scan's own arithmetic can be
        // told apart from what `World::tile` costs.
        let n = 200_000;
        let start = std::time::Instant::now();
        let mut sink = 0u32;
        for i in 0..n {
            let left = 200 + (i % 128);
            for y in 200..232 {
                for x in left..left + 32 {
                    sink += u32::from(std::hint::black_box(&shelved).tile(x, y).is_active());
                }
            }
        }
        let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
        println!("bare 1024 World::tile reads: {each:.2} ns/call (sink {sink})");

        // ...and what a whole `try_spawn` costs on a dungeon that has a library in it, which is the
        // number that actually matters: the scan is behind a rate roll and then a one-in-five.
        let (mut dungeon, (cx, cy)) = dungeon_world();
        dungeon.progress.downed_boss3 = true;
        shelve_books(&mut dungeon, cx, cy - 3);
        let npcs = NpcStore::new();
        let players = player_standing_at(cx, cy);
        let mut rng = SmallRng::seed_from_u64(7);
        let mut biomes = BiomeCache::default();
        let n = 2_000_000;
        let start = std::time::Instant::now();
        let (mut sink, mut scans) = (0usize, 0usize);
        for _ in 0..n {
            for (npc_type, _) in try_spawn(
                std::hint::black_box(&dungeon),
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut biomes,
                &mut rng,
            ) {
                sink += 1;
                // In this world a scan never comes back empty: the shelf spans the whole pocket, so
                // every candidate's box holds one outside the player's screen box. So a library
                // spawn is a scan, and the count of them is the count of scans.
                scans +=
                    usize::from(npc_type == WATER_BOLT_MIMIC || npc_type == LIBRARIAN_SKELETON);
            }
        }
        let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
        println!(
            "try_spawn in a shelved dungeon: {each:.2} ns/call ({sink} spawned, {scans} scans, \
             one scan per {:.0} calls)",
            f64::from(n) / scans as f64
        );
    }

    /// What hallowed ground's own arm costs a candidate that is not standing on it, which is every
    /// candidate in most worlds.
    ///
    /// The gate reads no tiles of its own: `events.hard_mode` is a bool, the depth was resolved a
    /// dozen lines above, and `HALLOW_GROUND.contains` scans four `u16` against the ground tile the
    /// candidate loop already read for the sandstorm and mushroom arms. Only once all three pass
    /// does anything else happen, and the store scan behind `alive` is behind four more conditions
    /// and a one-in-ten roll on top of that.
    ///
    /// One deliberate reorder against vanilla, disclosed because it is a reorder: `NPC.cs:4039`
    /// tests `!waterTile` before the four tile types, and this tests the tile types first. Both are
    /// pure, so the branch it takes is identical; the order is chosen so the array scan shuts the
    /// gate before `water_tile`'s own tile read is ever made.
    ///
    /// Measured on this machine, release build, the numbers printed by this test.
    #[test]
    #[ignore]
    fn measure_the_hallowed_ground_arm() {
        let n = 10_000_000;

        // The gate, on ordinary stone: what a non-hallowed world pays per candidate.
        for (name, ground) in [("stone", 1u16), ("pearlstone", 117u16)] {
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for _ in 0..n {
                sink += u32::from(
                    HALLOW_GROUND.contains(std::hint::black_box(&ground))
                        && matches!(std::hint::black_box(Depth::Surface), Depth::Surface),
                );
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("hallow gate, {name}: {each:.2} ns/call (sink {sink})");
        }

        // ...and the chain itself, on the one night it can answer with everything it has.
        let mut rng = SmallRng::seed_from_u64(7);
        let start = std::time::Instant::now();
        let mut sink = 0u32;
        for _ in 0..n {
            sink += u32::from(
                hallow_ground_pick(
                    std::hint::black_box(true),
                    std::hint::black_box(false),
                    std::hint::black_box(0),
                    std::hint::black_box(true),
                    std::hint::black_box(true),
                    &|_| false,
                    &mut rng,
                )
                .is_some(),
            );
        }
        let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
        println!("hallow_ground_pick, full night: {each:.2} ns/call (sink {sink})");
    }

    #[test]
    #[ignore]
    fn measure_the_tower_zone_test() {
        use crate::game::lunar;

        let none = quiet();
        let mut far = quiet();
        far.towers = [Some((900_000.0, 900_000.0)); 4];
        let mut inside = quiet();
        let slot = lunar::PILLARS
            .iter()
            .position(|p| *p == lunar::SOLAR)
            .expect("a real pillar");
        inside.towers[slot] = Some((1_000.0, 1_000.0));

        for (name, events) in [
            ("no apocalypse", &none),
            ("four standing, out of range", &far),
            ("inside the solar zone", &inside),
        ] {
            let n = 10_000_000;
            let start = std::time::Instant::now();
            let mut sink = 0u32;
            for i in 0..n {
                let at = std::hint::black_box((1_000.0 + (i % 4) as f32, 1_000.0));
                sink += u32::from(std::hint::black_box(events).tower_zone(at).unwrap_or(0));
            }
            let each = start.elapsed().as_secs_f64() / f64::from(n) * 1e9;
            println!("{name}: {each:.2} ns/test (sink {sink})");
        }
    }

    #[test]
    fn spawning_is_frequent_enough_to_matter() {
        // Picking a blind point and demanding it be the surface almost never works; scanning down
        // is what makes the spawn rate real. A minute of ticks should produce several spawns.
        let world = test_world();
        let npcs = NpcStore::new();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (
            f32::from(world.spawn_x) * 16.0,
            f32::from(world.spawn_y) * 16.0,
        );
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(11);
        let mut spawned = 0;
        for _ in 0..3600 {
            spawned += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        assert!(
            spawned >= 3,
            "only {spawned} spawns in a minute of ticks; the spawn point search is too fussy"
        );
    }

    #[test]
    fn the_cap_stops_further_spawns() {
        let world = test_world();
        let mut npcs = NpcStore::new();
        // Fill well past the single-player cap.
        for _ in 0..40 {
            npcs.spawn(3, (0.0, 0.0));
        }

        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (100.0, 100.0);
        let players = vec![Some(player)];

        let mut rng = SmallRng::seed_from_u64(3);
        for _ in 0..5_000 {
            assert!(
                try_spawn(
                    &world,
                    &npcs,
                    &players,
                    &quiet(),
                    &JourneyPowers::default(),
                    &mut BiomeCache::default(),
                    &mut rng,
                )
                .is_empty(),
                "spawned past the cap"
            );
        }
    }

    /// The cap is near-player, not world-global: a crowd of monsters on the far side of the map
    /// does not hold a lone player's own spawns down (`NPC.cs:313`, `player.nearbyActiveNPCs`).
    /// Fails before the fix, when the cap counted every NPC in the world, so the same far-off crowd
    /// silenced spawns everywhere at once.
    #[test]
    fn far_off_monsters_do_not_cap_a_lone_player() {
        let world = test_world();
        let mut npcs = NpcStore::new();

        // A whole screen of players' worth of monsters, parked at the far edge of the world.
        let (sx, sy) = (world.spawn_x as i32, world.spawn_y as i32);
        let far_x = if sx > world.width() / 2 {
            10
        } else {
            world.width() - 10
        };
        assert!(
            ((far_x - sx).abs() as f32 * 16.0) > ACTIVE_RANGE_X,
            "the crowd must be outside the active range to make the point",
        );
        for _ in 0..60 {
            npcs.spawn(3, (far_x as f32 * 16.0, sy as f32 * 16.0));
        }

        let players = {
            let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
            drop(out_rx);
            let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
            player.state = crate::game::ConnState::Playing;
            player.position = (sx as f32 * 16.0, sy as f32 * 16.0);
            vec![Some(player)]
        };

        let mut rng = SmallRng::seed_from_u64(4);
        let mut seen = 0;
        for _ in 0..20_000 {
            seen += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
            if seen > 0 {
                break;
            }
        }
        assert!(
            seen > 0,
            "a lone player should still spawn with the only other monsters a map away",
        );
    }

    /// A single test player, `is_playing` and clear of the safe zone, with a real channel behind
    /// it — the same construction `spawns_appear_outside_the_safe_zone_and_on_solid_ground` above
    /// already uses.
    fn one_player(world: &World) -> Vec<Option<Player>> {
        let (tx, ty) = (world.spawn_x as i32, world.spawn_y as i32);
        let (out_tx, out_rx) = tokio::sync::mpsc::channel(1);
        drop(out_rx);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = crate::game::ConnState::Playing;
        player.position = (tx as f32 * 16.0, ty as f32 * 16.0);
        vec![Some(player)]
    }

    /// Journey mode's `SpawnRate` at its exact floor (`0.0`) disables spawns outright for that
    /// player — `GetShouldDisableSpawnsFor`'s own hard condition, not merely the 0.1× remap floor
    /// `spawn_rate_multiplier` alone would give.
    #[test]
    fn spawn_rate_at_the_floor_disables_spawns_in_a_journey_world() {
        let mut world = test_world();
        world.game_mode = 3; // Journey
        let npcs = NpcStore::new();
        let players = one_player(&world);
        let mut journey = JourneyPowers::default();
        journey.set_spawn_rate_slider(0, 0.0);

        let mut rng = SmallRng::seed_from_u64(11);
        let mut seen = 0;
        for _ in 0..20_000 {
            seen += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &journey,
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        assert_eq!(seen, 0, "spawns should be disabled outright at the floor");
    }

    /// The same slider at its floor has no effect at all outside a Journey world — every one of
    /// `SpawnRateSliderPerPlayerPower`'s five real vanilla call sites gates on `Main.IsJourneyMode`
    /// before reading the power, and this is the one behaviour among the three per-player powers
    /// where getting that gate wrong is easy to miss testing, since an ungated implementation would
    /// otherwise look identical to a correct one on an ordinary difficulty.
    #[test]
    fn spawn_rate_has_no_effect_outside_a_journey_world() {
        let world = test_world(); // game_mode 0: ordinary
        let npcs = NpcStore::new();
        let players = one_player(&world);
        let mut journey = JourneyPowers::default();
        journey.set_spawn_rate_slider(0, 0.0); // would disable spawns entirely, in a Journey world

        let mut rng = SmallRng::seed_from_u64(11);
        let mut seen = 0;
        for _ in 0..20_000 {
            seen += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &journey,
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        assert!(
            seen > 0,
            "an ordinary-difficulty world should spawn normally regardless of the slider"
        );
    }

    /// Above the floor, the slider scales how often spawns roll — pinned as a real measured ratio
    /// across many ticks, not just "some vs none": a 10× player should see roughly ten times as
    /// many spawn events as a 1× player over the same window.
    #[test]
    fn spawn_rate_at_its_top_spawns_far_more_often_than_the_default_in_a_journey_world() {
        let mut world = test_world();
        world.game_mode = 3;
        let npcs = NpcStore::new();
        let players = one_player(&world);

        let ordinary = JourneyPowers::default(); // 0.5 -> 1x, the default
        let mut boosted = JourneyPowers::default();
        boosted.set_spawn_rate_slider(0, 1.0); // the top of the slider -> 10x

        const TICKS: usize = 200_000;
        let mut ordinary_seen = 0;
        let mut rng = SmallRng::seed_from_u64(21);
        for _ in 0..TICKS {
            ordinary_seen += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &ordinary,
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        let mut boosted_seen = 0;
        let mut rng = SmallRng::seed_from_u64(21);
        for _ in 0..TICKS {
            boosted_seen += try_spawn(
                &world,
                &npcs,
                &players,
                &quiet(),
                &boosted,
                &mut BiomeCache::default(),
                &mut rng,
            )
            .len();
        }
        assert!(
            boosted_seen > ordinary_seen * 5,
            "10x should spawn noticeably more often than 1x over {TICKS} ticks: \
             {boosted_seen} boosted vs {ordinary_seen} ordinary"
        );
    }

    /// The bound townsfolk are gated on their real progression, biome and depth, so the Wizard,
    /// Mechanic and Goblin Tinkerer are not findable on day one (`NPC.cs:2087-2091,2656`). Fails
    /// before the fix, when every still-bound resident was offered the moment a player reached a
    /// cavern, with no hardmode / Skeletron / goblin-army gate at all.
    #[test]
    fn bound_townsfolk_are_gated_on_real_progression() {
        let mut world = test_world();
        world.progress.hard_mode = false;
        world.progress.downed_boss3 = false;
        world.progress.downed_goblins = false;
        let y = i32::from(world.rock_layer) + 5; // squarely in the caverns
        assert_eq!(depth_at(&world, y), Depth::Cavern);

        // Day one, deep in a plain cavern: none of the three progression-gated finds are eligible.
        assert!(
            !bound_gate(106, &world, Depth::Cavern, Biome::Forest, y),
            "the Wizard needs hardmode"
        );
        assert!(
            !bound_gate(123, &world, Depth::Cavern, Biome::Forest, y),
            "the Mechanic needs Skeletron down"
        );
        assert!(
            !bound_gate(105, &world, Depth::Cavern, Biome::Forest, y),
            "the Goblin Tinkerer needs the army beaten"
        );

        // The Stylist has no progression gate (a spider nest is a day-one find), so she is eligible.
        assert!(bound_gate(354, &world, Depth::Cavern, Biome::Forest, y));

        // Hardmode opens the Wizard; Skeletron opens the Mechanic; the beaten army the Goblin.
        world.progress.hard_mode = true;
        world.progress.downed_boss3 = true;
        world.progress.downed_goblins = true;
        assert!(bound_gate(106, &world, Depth::Cavern, Biome::Forest, y));
        assert!(bound_gate(123, &world, Depth::Cavern, Biome::Forest, y));
        assert!(bound_gate(105, &world, Depth::Cavern, Biome::Forest, y));

        // The Golfer wants the underground desert, not a forest cavern.
        assert!(!bound_gate(589, &world, Depth::Cavern, Biome::Forest, y));
        assert!(bound_gate(
            589,
            &world,
            Depth::Underground,
            Biome::Desert,
            y
        ));

        // And a fresh cavern only ever offers the Stylist, never a progression-gated resident.
        let mut rng = SmallRng::seed_from_u64(5);
        world.progress.hard_mode = false;
        world.progress.downed_boss3 = false;
        world.progress.downed_goblins = false;
        for _ in 0..500 {
            if let Some(bound) = pick_bound(
                &world,
                &NpcStore::new(),
                Depth::Cavern,
                Biome::Forest,
                y,
                &mut rng,
            ) {
                assert_eq!(bound, 354, "only the Stylist is a day-one cavern find");
            }
        }
    }

    /// The bound Old Slime is a caverns find beside the Goblin Tinkerer and the Wizard, gated on
    /// Skeletron and closed for good once it is freed (`NPC.cs:2095`).
    ///
    /// Fails before the fix, when 685 was in no producer at all: `pick_bound` read only the talk
    /// rescue table and `bound_gate` had no arm for it, so the Old Slime was unreachable however
    /// far a world progressed.
    #[test]
    fn the_caverns_offer_a_bound_old_slime_once_skeletron_is_down() {
        let mut world = test_world();
        world.progress = Default::default();
        let y = i32::from(world.rock_layer) + 5;
        assert_eq!(depth_at(&world, y), Depth::Cavern);

        assert!(
            !bound_gate(
                BOUND_TOWN_SLIME_OLD,
                &world,
                Depth::Cavern,
                Biome::Forest,
                y
            ),
            "the Old Slime needs Skeletron down"
        );
        world.progress.downed_boss3 = true;
        assert!(bound_gate(
            BOUND_TOWN_SLIME_OLD,
            &world,
            Depth::Cavern,
            Biome::Forest,
            y
        ));
        assert!(
            !bound_gate(
                BOUND_TOWN_SLIME_OLD,
                &world,
                Depth::Underground,
                Biome::Forest,
                y
            ),
            "it is a caverns find, not a dirt-layer one"
        );

        // And it is actually offered: `pick_bound` has to reach it, which is the half a gate
        // alone cannot prove.
        let mut rng = SmallRng::seed_from_u64(11);
        let offered = (0..2_000).any(|_| {
            pick_bound(
                &world,
                &NpcStore::new(),
                Depth::Cavern,
                Biome::Forest,
                y,
                &mut rng,
            ) == Some(BOUND_TOWN_SLIME_OLD)
        });
        assert!(offered, "a post-Skeletron cavern must be able to offer it");

        // Freeing it shuts the arm: `!unlockedSlimeOldSpawn` is half of vanilla's condition, so a
        // world with the resident in a house must never offer a second bound one.
        world.progress.unlocked_slime_old = true;
        let mut rng = SmallRng::seed_from_u64(11);
        for _ in 0..2_000 {
            assert_ne!(
                pick_bound(
                    &world,
                    &NpcStore::new(),
                    Depth::Cavern,
                    Biome::Forest,
                    y,
                    &mut rng,
                ),
                Some(BOUND_TOWN_SLIME_OLD),
                "a freed Old Slime must not be found again"
            );
        }
    }

    /// The sky offers a bound Purple Slime one attempt in twenty-five until somebody frees it
    /// (`NPC.cs:1417`), and never once it is freed.
    ///
    /// Fails before the fix, when `sky_pick` had no arm for it at all and the sky was only ever a
    /// probe, a Wyvern or a Harpy.
    #[test]
    fn the_sky_offers_a_bound_purple_slime_until_somebody_frees_it() {
        let mut world = World::empty(800, 600, "sky slime");
        let never = |_: u16| false;

        let mut rng = SmallRng::seed_from_u64(3);
        let seen = (0..2_000)
            .filter(|_| {
                sky_pick(false, false, &world, false, &never, &mut rng) == BOUND_TOWN_SLIME_PURPLE
            })
            .count();
        assert!(
            seen > 0,
            "a fresh sky must be able to offer a bound Purple Slime"
        );

        // One at a time: `!AnyNPCs(686)` means a slime already up there stops a second.
        let already = |ty: u16| ty == BOUND_TOWN_SLIME_PURPLE;
        let mut rng = SmallRng::seed_from_u64(3);
        for _ in 0..2_000 {
            assert_ne!(
                sky_pick(false, false, &world, false, &already, &mut rng),
                BOUND_TOWN_SLIME_PURPLE,
                "only one bound Purple Slime is ever in the sky at once"
            );
        }

        world.progress.unlocked_slime_purple = true;
        let mut rng = SmallRng::seed_from_u64(3);
        for _ in 0..2_000 {
            assert_ne!(
                sky_pick(false, false, &world, false, &never, &mut rng),
                BOUND_TOWN_SLIME_PURPLE,
                "a freed Purple Slime must not be found again"
            );
        }
    }

    /// A frog draw is `NPC.SpawnFrog`'s chain, not a frog (`NPC.cs:5621-5634`): one in thirty is a
    /// bound Yellow Slime until somebody purifies it.
    ///
    /// Fails before the fix, when the jungle's friendly pool answered 361 flat and 687 was in no
    /// producer at all.
    #[test]
    fn a_frog_draw_can_be_a_bound_yellow_slime_until_it_is_freed() {
        let mut world = World::empty(800, 600, "frog");
        let never = |_: u16| false;

        let mut rng = SmallRng::seed_from_u64(7);
        let seen = (0..3_000)
            .filter(|_| spawn_frog(&world, &never, &mut rng) == BOUND_TOWN_SLIME_YELLOW)
            .count();
        assert!(seen > 0, "a fresh jungle must be able to offer one");
        assert!(
            seen < 3_000 / 4,
            "the plain frog is still the common answer, not the slime: {seen} of 3000"
        );

        let already = |ty: u16| ty == BOUND_TOWN_SLIME_YELLOW;
        let mut rng = SmallRng::seed_from_u64(7);
        for _ in 0..3_000 {
            assert_eq!(
                spawn_frog(&world, &already, &mut rng),
                FROG,
                "only one bound Yellow Slime is ever about at once"
            );
        }

        world.progress.unlocked_slime_yellow = true;
        let mut rng = SmallRng::seed_from_u64(7);
        for _ in 0..3_000 {
            assert_eq!(
                spawn_frog(&world, &never, &mut rng),
                FROG,
                "a freed Yellow Slime must not be found again"
            );
        }
    }

    /// A flat, open, empty hall with a single floor row at `floor`, in a world whose surface and
    /// rock layer are fixed so `depth_at` is predictable. `World::empty` is all air, so the floor
    /// is the only tile in it and the biome scan reads a plain forest.
    fn hall_world(floor: i32) -> World {
        let mut world = World::empty(800, 600, "hall");
        world.surface = 100;
        world.rock_layer = 200;
        for x in 0..world.width() {
            world.set_tile(x, floor, terrustia_proto::Tile::block(1));
        }
        world
    }

    /// How many of `wanted` a long run of spawn attempts produces at `floor` in `world`.
    fn spawns_of(
        world: &World,
        floor: i32,
        events: &EventSpawns<'_>,
        wanted: u16,
        seed: u64,
    ) -> usize {
        let npcs = NpcStore::new();
        let players = player_at(((world.width() / 2) as f32 * 16.0, (floor - 1) as f32 * 16.0));
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut seen = 0;
        for _ in 0..60_000 {
            seen += try_spawn(
                world,
                &npcs,
                &players,
                events,
                &JourneyPowers::default(),
                &mut BiomeCache::default(),
                &mut rng,
            )
            .iter()
            .filter(|(npc_type, _)| *npc_type == wanted)
            .count();
        }
        seen
    }

    /// The underworld offers a Tortured Soul (`NPC.cs:4877`), and only under vanilla's own three
    /// conditions: hardmode, and a world that has not got its Tax Collector yet.
    ///
    /// Fails before the fix, when 534 was in no pool and no branch: nothing this server could do
    /// would ever put one in a world, so the Tax Collector - his shop, his happiness, his arrival
    /// message, all of it already written - was unreachable for good.
    #[test]
    fn the_underworld_offers_a_tortured_soul_once_a_world_is_in_hardmode() {
        let floor = 600 - UNDERWORLD_DEPTH + 50;
        let mut world = hall_world(floor);
        assert_eq!(
            depth_at(&world, floor - 1),
            Depth::Underworld,
            "the hall has to be in the underworld for this to test anything"
        );

        // Pre-hardmode: `Main.hardMode` is the first clause of the branch, so never.
        assert_eq!(
            spawns_of(&world, floor, &quiet(), TORTURED_SOUL, 5),
            0,
            "a pre-hardmode underworld spawned a Tortured Soul"
        );

        let mut hard = quiet();
        hard.hard_mode = true;
        world.progress.hard_mode = true;
        assert!(
            spawns_of(&world, floor, &hard, TORTURED_SOUL, 5) > 0,
            "a hardmode underworld never offered a Tortured Soul"
        );

        // `!savedTaxCollector`: once the world has him, the branch is shut for good.
        world.progress.saved_tax_collector = true;
        assert_eq!(
            spawns_of(&world, floor, &hard, TORTURED_SOUL, 5),
            0,
            "a world that already has its Tax Collector kept spawning Tortured Souls"
        );
    }

    /// The caverns offer a Skeleton Merchant (`NPC.cs:5004-5010`), at any point in a world's
    /// progression, and never in the underworld above or the underground below him.
    ///
    /// Fails before the fix, when 453 was in no pool and no branch, so the one wandering vendor in
    /// the game could not be met however long a world was played.
    #[test]
    fn the_caverns_offer_a_skeleton_merchant_at_any_progression() {
        let cavern_floor = 300;
        let world = hall_world(cavern_floor);
        assert_eq!(depth_at(&world, cavern_floor - 1), Depth::Cavern);
        assert!(
            spawns_of(&world, cavern_floor, &quiet(), SKELETON_MERCHANT, 9) > 0,
            "a plain cavern never offered a Skeleton Merchant"
        );

        // He is a cavern spawn, not an underworld one: vanilla's underworld arm is a whole branch
        // earlier in the same chain and never reaches him.
        let hell_floor = 600 - UNDERWORLD_DEPTH + 50;
        let hell = hall_world(hell_floor);
        assert_eq!(depth_at(&hell, hell_floor - 1), Depth::Underworld);
        assert_eq!(
            spawns_of(&hell, hell_floor, &quiet(), SKELETON_MERCHANT, 9),
            0,
            "the underworld spawned a Skeleton Merchant"
        );
    }

    /// The surface offers a Goblin Scout by day, and only out past a third of the map from the
    /// world's spawn point (`NPC.cs:4482-4485`).
    ///
    /// Fails before the fix, when 73 was in no pool and no branch. That silence reached further
    /// than one missing enemy: he is the game's only source of Tattered Cloth, the cloth is the
    /// only ingredient of the Goblin Battle Standard, and the standard is the only way anybody
    /// summons a goblin army. All three of those were already written here and all three were
    /// unreachable.
    ///
    /// Neutralised by deleting the `out.push((GOBLIN_SCOUT, ...))` block from `try_spawn`: the
    /// first and last assertions fail ("a surface day far from spawn offered no Goblin Scout").
    /// Neutralised again by dropping the `(x - world.spawn_x).abs() > world.width() / 3` clause:
    /// the near-spawn assertion fails instead, scouts appearing in the back garden.
    #[test]
    fn the_surface_day_offers_a_goblin_scout_out_past_a_third_of_the_map() {
        // Long enough that the two rolls are told apart by their counts rather than by luck. The
        // run measures 69 scouts on the plain roll and 226 with a shadow orb smashed, against the
        // 3.0x the two rates predict (1/15 against 1/15 + 14/15 * 1/7); the assertions below leave
        // that a wide berth. The whole test is about a second.
        const TICKS: u32 = 400_000;
        let scouts = |world: &World, (px, py): (i32, i32)| {
            spawns_at(world, false, px, py, TICKS)
                .into_iter()
                .filter(|ty| *ty == GOBLIN_SCOUT)
                .count()
        };

        // `World::empty` puts the spawn point at the middle of the map, which is where these tests
        // stand their player, so every candidate tile is inside the spawn box and the gate is shut.
        let (near_spawn, at) = forest_surface();
        assert_eq!(
            scouts(&near_spawn, at),
            0,
            "a Goblin Scout turned up within a third of the map of the world spawn"
        );

        // The same world with the spawn point moved to the far edge: now every candidate is more
        // than `width / 3` away from it, which is the whole of vanilla's `num45` gate.
        let mut world = near_spawn.clone();
        world.spawn_x = 0;
        assert!(
            (at.0 - i32::from(world.spawn_x)).abs() - SPAWN_RANGE_X > world.width() / 3,
            "the whole spawn box has to be outside the gate for this to test the gate"
        );
        let baseline = scouts(&world, at);
        assert!(
            baseline > 0,
            "a surface day far from spawn offered no Goblin Scout"
        );

        // `!ZoneGraveyard && Main.dayTime` (`NPC.cs:4202`): the whole block he lives in is daytime
        // only, and a graveyard skips it even in daylight.
        let mut night = world.clone();
        night.day_time = false;
        assert_eq!(
            scouts(&night, at),
            0,
            "the surface night offered a Goblin Scout"
        );
        assert_eq!(
            spawns_at_in(&world, false, true, at.0, at.1, TICKS)
                .into_iter()
                .filter(|ty| *ty == GOBLIN_SCOUT)
                .count(),
            0,
            "a graveyard offered a Goblin Scout in daylight"
        );

        // `!waterTile`: standing in the sea is not standing on the ground. `water_tile` reads the
        // spawn row and the one above it, which for a floor at `at.1 + 2` are `at.1 + 1` and `at.1`.
        let mut flooded = world.clone();
        for x in 0..flooded.width() {
            for y in at.1..=(at.1 + 1) {
                flooded.set_tile(
                    x,
                    y,
                    terrustia_proto::Tile::AIR
                        .with_liquid(terrustia_proto::tile::Liquid::Water, 255),
                );
            }
        }
        assert_eq!(
            scouts(&flooded, at),
            0,
            "a flooded surface offered a Goblin Scout"
        );

        // The second roll (`!downedGoblins && WorldGen.shadowOrbSmashed && rand.Next(7) == 0`) is
        // OR'd onto the first rather than replacing it, so a smashed shadow orb takes him from one
        // attempt in fifteen to roughly one in five. Well over double either way, which is what
        // makes this an assertion rather than a coin toss.
        let mut hunted = world.clone();
        hunted.progress.shadow_orb_smashed = true;
        let raised = scouts(&hunted, at);
        assert!(
            raised > baseline * 2,
            "a smashed shadow orb barely changed the Goblin Scout rate: {baseline} -> {raised}"
        );

        // ...and it closes for good once the world has had its goblin army.
        hunted.progress.downed_goblins = true;
        let after = scouts(&hunted, at);
        assert!(
            after > 0 && after < raised / 2,
            "a downed goblin army left the shadow-orb rate up: {raised} -> {after}"
        );
    }

    /// A graveyard stands a Statue Mimic on a plinth once Skeletron is down
    /// (`NPC.cs:1571-1574`), and only where there is a plinth to stand it on
    /// (`IsThisAGoodPlaceForAStatueMimic`, `NPC.cs:43891-43898`).
    ///
    /// Fails before the fix: the AI (`game::ai::mimic`) and the immortal-until-provoked pose
    /// (`IMMORTAL_TYPE`) were both already built and tested, and 690 was in no pool and no branch,
    /// so none of that work could ever run in a real world.
    ///
    /// Neutralised by deleting the `None if world.progress.downed_boss3 && seasonal.graveyard ...`
    /// arm from `try_spawn`: the first assertion fails, "a graveyard after Skeletron offered no
    /// Statue Mimic". Neutralised again by making `good_place_for_a_statue_mimic` return `true` on
    /// its first line: the half-brick assertion fails instead, a mimic standing on a floor no
    /// statue could stand on.
    #[test]
    fn a_graveyard_stands_a_statue_mimic_on_a_plinth() {
        // He is a one-in-twenty-five draw off a surface spawn rate of 600, so a short run sees none
        // of him by luck alone rather than by any rule. This one measures 37.
        const TICKS: u32 = 400_000;
        let mimics = |world: &World, graveyard: bool, (px, py): (i32, i32)| {
            spawns_at_in(world, false, graveyard, px, py, TICKS)
                .into_iter()
                .filter(|ty| *ty == STATUE_MIMIC)
                .count()
        };

        let (mut world, at) = forest_surface();
        world.progress.downed_boss3 = true;
        assert!(
            mimics(&world, true, at) > 0,
            "a graveyard after Skeletron offered no Statue Mimic"
        );

        // `downedBoss3`: the arm's first clause, so a world that has not beaten Skeletron never
        // reaches it.
        let mut fresh = world.clone();
        fresh.progress.downed_boss3 = false;
        assert_eq!(
            mimics(&fresh, true, at),
            0,
            "a graveyard before Skeletron offered a Statue Mimic"
        );

        // `ZoneGraveyard`: he is a graveyard spawn and nothing else. Ordinary ground has no arm for
        // him at any depth, at any time of day, at any progression.
        assert_eq!(
            mimics(&world, false, at),
            0,
            "an ordinary forest offered a Statue Mimic"
        );

        // `IsThisAGoodPlaceForAStatueMimic`: the floor has to read as `SolidTile2`, which a half
        // brick does not. Everything else about this world is unchanged, and `has_room` still
        // accepts the floor (it asks only whether the block type is solid), so this isolates the
        // plinth check on its own.
        let mut half_bricks = world.clone();
        for x in 0..half_bricks.width() {
            let mut tile = terrustia_proto::Tile::block(1);
            tile.flags.set(terrustia_proto::TileFlags::HALF_BRICK, true);
            half_bricks.set_tile(x, at.1 + 1, tile);
        }
        assert_eq!(
            mimics(&half_bricks, true, at),
            0,
            "a Statue Mimic stood on a floor of half bricks"
        );
    }

    /// `IsThisAGoodPlaceForAStatueMimic` (`NPC.cs:43891-43898`) on its own: two solid columns and
    /// three clear rows above both of them.
    #[test]
    fn a_statue_mimic_wants_two_solid_tiles_and_three_clear_rows() {
        let floor = 90;
        let plinth = flat_world(floor);
        assert!(good_place_for_a_statue_mimic(&plinth, 400, floor));

        // `SolidTile2(x + 1, y)`: one column of plinth is not enough, and the check is asked at
        // `x` and `x + 1` rather than either side of `x`.
        let mut gapped = plinth.clone();
        gapped.set_tile(401, floor, terrustia_proto::Tile::AIR);
        assert!(!good_place_for_a_statue_mimic(&gapped, 400, floor));
        assert!(good_place_for_a_statue_mimic(&gapped, 398, floor));

        // The three clear rows, over both columns: six tiles, each of which alone shuts it.
        for dx in 0..2 {
            for dy in 1..=3 {
                let mut blocked = plinth.clone();
                blocked.set_tile(400 + dx, floor - dy, terrustia_proto::Tile::block(1));
                assert!(
                    !good_place_for_a_statue_mimic(&blocked, 400, floor),
                    "a block at ({dx}, -{dy}) left the spot good"
                );
            }
        }

        // `SolidTile2`'s own three exclusions: a half brick, an actuated tile, and a slope. Vanilla
        // keeps exactly one kind of sloped tile, a platform hammered to a *top* slope, and this is
        // the only spawn check in the game that cares.
        let sloped = |slope: u8, block: u16, half: bool, actuated: bool| {
            let mut world = plinth.clone();
            for x in 400..402 {
                // Platforms are frame-important, so they cannot go through `Tile::block`.
                let mut tile = terrustia_proto::Tile::framed(block, 0, 0);
                tile.slope = slope;
                tile.flags.set(terrustia_proto::TileFlags::HALF_BRICK, half);
                tile.flags
                    .set(terrustia_proto::TileFlags::ACTUATED, actuated);
                world.set_tile(x, floor, tile);
            }
            good_place_for_a_statue_mimic(&world, 400, floor)
        };
        assert!(sloped(0, 1, false, false), "plain stone is a plinth");
        assert!(!sloped(0, 1, true, false), "a half brick is not a plinth");
        assert!(
            !sloped(0, 1, false, true),
            "an actuated tile is not a plinth"
        );
        assert!(!sloped(1, 1, false, false), "sloped stone is not a plinth");
        // A platform (19) is in `Main.tileSolid`, so it passes on a top slope (1 and 2) and on no
        // slope at all, and fails on a bottom slope (3 and 4).
        assert!(sloped(0, PLATFORMS[0], false, false));
        assert!(sloped(1, PLATFORMS[0], false, false));
        assert!(sloped(2, PLATFORMS[0], false, false));
        assert!(!sloped(3, PLATFORMS[0], false, false));
        assert!(!sloped(4, PLATFORMS[0], false, false));
    }

    /// Everything drawn over `ticks` attempts standing at `at`, as a set.
    fn roster_at(
        world: &World,
        hard_mode: bool,
        at: (i32, i32),
        ticks: u32,
    ) -> std::collections::BTreeSet<u16> {
        spawns_at(world, hard_mode, at.0, at.1, ticks)
            .into_iter()
            .collect()
    }

    /// The underground jungle's own two, both of which were in no pool at all.
    ///
    /// The Angry Trapper (175) is the hardmode jungle-grass arm's rooted branch
    /// (`NPC.cs:3905-3908`, inside `tileType == 60 && Main.hardMode && Main.rand.Next(3) != 0` at
    /// `:3864`), and it is reached above the surface line too: the three branches between it and
    /// the two `surfaceSpawn` ones all want `spawnTileY > Main.worldSurface`, which is the exact
    /// negation of `surfaceSpawn` (`NPC.cs:1203`).
    ///
    /// The Spiked Jungle Slime (204) is *not* a hardmode enemy. Its arm is `tileType == 60 &&
    /// spawnTileY > (Main.worldSurface + Main.rockLayer) / 2.0` (`NPC.cs:3929`) with a one-in-four
    /// roll inside it (`:3931`), and `Main.hardMode` appears in neither, so a fresh world's
    /// underground jungle has them from the first day.
    ///
    /// `game::ai::rooted` already drives the trapper, down to its own reach, pull and speed cap
    /// (`npc_params::rooted`'s `175` arm), and `game::ai::slime` already drives the slime: the
    /// spawn was the only missing half of both.
    ///
    /// Neutralised by deleting `175` from both `hardmode_pool` jungle arms: the two trapper
    /// assertions fail ("a hardmode underground jungle offered no Angry Trapper", then the surface
    /// one). Neutralised again by deleting `204` from `pool`'s `(_, Jungle)` arm: the
    /// pre-hardmode assertion fails instead.
    #[test]
    fn the_underground_jungle_has_a_roster_of_its_own() {
        const TICKS: u32 = 200_000;

        // A cavern floor of jungle grass, which is what the biome scan counts (`JUNGLE_TILES`).
        let floor = 300;
        let deep = flat_world_of(floor, 60);
        let at = (400, floor - 1);
        assert_eq!(biome_at(&deep, at.0, at.1), Biome::Jungle);
        assert_eq!(depth_at(&deep, at.1), Depth::Cavern);

        let early = roster_at(&deep, false, at, TICKS);
        assert!(
            early.contains(&204),
            "a pre-hardmode underground jungle offered no Spiked Jungle Slime"
        );
        assert!(
            !early.contains(&175),
            "a pre-hardmode jungle offered an Angry Trapper"
        );

        let late = roster_at(&deep, true, at, TICKS);
        assert!(
            late.contains(&175),
            "a hardmode underground jungle offered no Angry Trapper"
        );

        // ...and the trapper again up in the canopy, where the arm's three
        // `spawnTileY > Main.worldSurface` branches cannot fire at all.
        let surface_floor = 90;
        let top = flat_world_of(surface_floor, 60);
        let up = (400, surface_floor - 1);
        assert_eq!(biome_at(&top, up.0, up.1), Biome::Jungle);
        assert_eq!(depth_at(&top, up.1), Depth::Surface);
        let sun = roster_at(&top, true, up, TICKS);
        assert!(
            sun.contains(&175),
            "a hardmode jungle surface offered no Angry Trapper"
        );
        // 204's own arm carries a depth gate the surface never satisfies.
        assert!(
            !sun.contains(&204),
            "the jungle surface offered a Spiked Jungle Slime"
        );
    }

    /// A corrupt cavern grows a Clinger once the wall is down (`NPC.cs:4136-4139`), and never
    /// before it.
    ///
    /// The Clinger belongs to the two evils' shared arm (`NPC.cs:4125`) rather than to the jungle:
    /// what puts one in a *jungle* is tile 661, Corrupt Jungle Grass, in that same list. Its AI is
    /// already built and parameterised, ichor shot and all (`game::ai::rooted`, and
    /// `npc_params::rooted`'s `101` arm); 101 was simply in no pool.
    ///
    /// Neutralised by deleting `101` from `hardmode_pool`'s `(Underground | Cavern, Corruption)`
    /// arm: the second assertion fails, "a hardmode corrupt cavern offered no Clinger".
    #[test]
    fn a_corrupt_cavern_grows_a_clinger_in_hardmode() {
        const TICKS: u32 = 200_000;
        let floor = 300;
        // Ebonstone, which is in the scan's `EVIL_TILES`.
        let world = flat_world_of(floor, 25);
        let at = (400, floor - 1);
        assert_eq!(biome_at(&world, at.0, at.1), Biome::Corruption);
        assert_eq!(depth_at(&world, at.1), Depth::Cavern);

        assert!(
            !roster_at(&world, false, at, TICKS).contains(&101),
            "a pre-hardmode corruption offered a Clinger"
        );
        assert!(
            roster_at(&world, true, at, TICKS).contains(&101),
            "a hardmode corrupt cavern offered no Clinger"
        );
    }

    /// The hardmode caverns cut a Rock Golem out of the stone (`NPC.cs:4921-4924`, gated by
    /// `CheckToSpawnRockGolem` at `:5803-5818`).
    ///
    /// It is a *cavern* enemy, not a jungle one: its arm is reached only once `underGround`
    /// (`spawnTileY <= Main.rockLayer`, `NPC.cs:1144`) and the underworld arm at `:4871` have both
    /// declined, and its ground is plain stone or moss rather than mud.
    ///
    /// Fails before the fix, when 631 was in no pool and no branch, so the single hardest thing an
    /// ordinary cave holds (1000 life, 85 damage) could not be met however long a world was played.
    ///
    /// Neutralised by deleting the `None if depth == Depth::Cavern && ... check_to_spawn_rock_golem`
    /// arm from `try_spawn`: the stone and moss assertions both fail ("a hardmode stone cavern cut
    /// no Rock Golem"). Neutralised again by dropping the `|| snow` clause from
    /// `check_to_spawn_rock_golem`: the snow assertion fails instead, golems coming out of a
    /// snow-bound cavern.
    #[test]
    fn the_hardmode_caverns_cut_a_rock_golem_out_of_stone_and_moss() {
        const TICKS: u32 = 200_000;
        let floor = 300;
        let golems = |world: &World, hard_mode: bool| {
            spawns_at(world, hard_mode, 400, floor - 1, TICKS)
                .into_iter()
                .filter(|ty| *ty == ROCK_GOLEM)
                .count()
        };

        let stone = flat_world_of(floor, STONE);
        assert_eq!(depth_at(&stone, floor - 1), Depth::Cavern);
        assert_eq!(biome_at(&stone, 400, floor - 1), Biome::Forest);
        assert_eq!(
            golems(&stone, false),
            0,
            "a pre-hardmode cavern cut a Rock Golem"
        );
        assert!(
            golems(&stone, true) > 0,
            "a hardmode stone cavern cut no Rock Golem"
        );

        // `TileID.Sets.Conversion.Moss` counts as stone here, and dirt does not.
        let moss = flat_world_of(floor, 179);
        assert!(
            golems(&moss, true) > 0,
            "a hardmode moss cavern cut no Rock Golem"
        );
        let dirt = flat_world_of(floor, 0);
        assert_eq!(golems(&dirt, true), 0, "a dirt floor cut a Rock Golem");

        // `ZoneSnow`. The floor stays stone, so only the zone moves: the snow goes *under* it,
        // which is the one way to change the scan's answer without changing the tile the check
        // itself reads.
        let mut snowy = stone.clone();
        for x in 0..snowy.width() {
            for y in (floor + 1)..(floor + 13) {
                snowy.set_tile(x, y, terrustia_proto::Tile::block(147));
            }
        }
        assert_eq!(biome_at(&snowy, 400, floor - 1), Biome::Snow);
        assert_eq!(golems(&snowy, true), 0, "a snowy cavern cut a Rock Golem");

        // The ceiling, which is four rows above the ground row and so is a row `open_space`'s three
        // never reach. It is made of dirt so that a candidate landing on the ceiling itself is
        // turned away by the tile gate rather than passing this one.
        let mut low = stone.clone();
        for x in 0..low.width() {
            low.set_tile(
                x,
                floor - ROCK_GOLEM_HEADROOM,
                terrustia_proto::Tile::block(0),
            );
        }
        assert_eq!(
            golems(&low, true),
            0,
            "a Rock Golem stood up under a four-row ceiling"
        );
    }

    /// `CheckToSpawnRockGolem` (`NPC.cs:5803-5818`) on its own, clause by clause.
    #[test]
    fn check_to_spawn_rock_golem_wants_stone_or_moss_under_a_clear_ceiling() {
        let floor = 300;
        let world = flat_world_of(floor, STONE);
        // One attempt in fifty, so each case is asked enough times to tell "never" from "rarely":
        // an open gate passes about eight of these four hundred draws, a shut one none.
        let passes = |world: &World, block: u16, hard_mode: bool, snow: bool| {
            let mut rng = SmallRng::seed_from_u64(631);
            (0..400)
                .filter(|_| {
                    check_to_spawn_rock_golem(world, 400, floor, block, hard_mode, snow, &mut rng)
                })
                .count()
        };

        assert!(passes(&world, STONE, true, false) > 0, "plain stone");
        assert_eq!(passes(&world, STONE, false, false), 0, "!Main.hardMode");
        assert_eq!(passes(&world, STONE, true, true), 0, "ZoneSnow");
        assert_eq!(passes(&world, 0, true, false), 0, "dirt is not stone");
        // All eleven of `TileID.Sets.Conversion.Moss` (`TileID.cs:38`).
        for moss in [179u16, 180, 181, 182, 183, 381, 534, 536, 539, 625, 627] {
            assert!(
                passes(&world, moss, true, false) > 0,
                "moss {moss} was refused"
            );
        }

        // `SolidTile(x - 1, y - 4) || SolidTile(x, y - 4) || SolidTile(x + 1, y - 4)`: any one of
        // the three shuts it, and the row is four above the ground row rather than three.
        for dx in -1..=1 {
            let mut low = world.clone();
            low.set_tile(
                400 + dx,
                floor - ROCK_GOLEM_HEADROOM,
                terrustia_proto::Tile::block(1),
            );
            assert_eq!(
                passes(&low, STONE, true, false),
                0,
                "a block at dx={dx} left the spot good"
            );
            // One row lower is not the row it reads.
            let mut lower = world.clone();
            lower.set_tile(
                400 + dx,
                floor - ROCK_GOLEM_HEADROOM - 1,
                terrustia_proto::Tile::block(1),
            );
            assert!(
                passes(&lower, STONE, true, false) > 0,
                "a block at dx={dx} five rows up shut the spot"
            );
        }
    }

    /// A spider nest is the only place either spider comes from (`NPC.cs:1662-1680`), and the arm
    /// keys on the nest's own wall rather than on a depth or a biome.
    ///
    /// Neutralised by deleting the `None if world.tile(x, y + 1).wall == SPIDER_WALL` arm from
    /// `try_spawn`: the first assertion fails with "a spider nest produced no Wall Creeper", and
    /// the hardmode one with "a hardmode nest produced no Black Recluse". Neutralised again by
    /// putting `163` back in `hardmode_pool`'s generic `(Underground | Cavern, _)` list: the last
    /// assertion fails, an ordinary walled-off hardmode cavern producing recluses.
    #[test]
    fn a_spider_nest_is_where_the_spiders_are() {
        let floor = 300;
        let nest = {
            let mut world = hall_world(floor);
            assert!(
                !terrustia_proto::housing::wall_encloses(SPIDER_WALL),
                "a nest wall must not read as a house wall, or nothing would spawn in one"
            );
            for x in 0..world.width() {
                let mut tile = terrustia_proto::Tile::block(1);
                tile.wall = SPIDER_WALL;
                world.set_tile(x, floor, tile);
            }
            world
        };
        assert_eq!(depth_at(&nest, floor - 1), Depth::Cavern);

        // Pre-hardmode the arm answers with the Wall Creeper alone (`NPC.cs:1677-1680`).
        assert!(
            spawns_of(&nest, floor, &quiet(), 164, 3) > 0,
            "a spider nest produced no Wall Creeper"
        );
        let mut hard = quiet();
        hard.hard_mode = true;
        assert!(
            spawns_of(&nest, floor, &hard, 163, 3) > 0,
            "a hardmode nest produced no Black Recluse"
        );

        // ...and the same cavern without the wall has neither, which is what makes the nest the
        // reason they came rather than the depth (`NPC.cs:1673`, the only `163` in the spawner).
        let plain = hall_world(floor);
        assert_eq!(
            spawns_of(&plain, floor, &hard, 164, 3),
            0,
            "a Wall Creeper turned up in a cavern with no nest"
        );
        assert_eq!(
            spawns_of(&plain, floor, &hard, 163, 3),
            0,
            "a Black Recluse turned up in a cavern with no nest"
        );
    }
}

/// Whether an NPC type counts against an invasion's remaining size.
///
/// Only the invasion's own members count. A goblin army is not shortened by killing the bats that
/// happened to be in the way, and the game keeps these rosters as flat lists for exactly that
/// reason.
pub fn belongs_to(kind: crate::game::event::Invasion, npc_type: u16) -> bool {
    use crate::game::event::Invasion;
    match kind {
        Invasion::Goblin => matches!(npc_type, 26 | 27 | 28 | 29 | 111 | 471),
        Invasion::FrostLegion => matches!(npc_type, 143..=145),
        Invasion::Pirate => matches!(npc_type, 212 | 213 | 214 | 215 | 216 | 252 | 491),
        Invasion::Martian => matches!(
            npc_type,
            381 | 382 | 383 | 385 | 386 | 388 | 389 | 390 | 395 | 520
        ),
    }
}

#[cfg(test)]
mod invasion_tests {
    use super::belongs_to;
    use crate::game::event::Invasion;

    /// Only an invasion's own members shorten it.
    #[test]
    fn bystanders_do_not_count_against_an_invasion() {
        assert!(belongs_to(Invasion::Goblin, 28), "a goblin peon does");
        assert!(!belongs_to(Invasion::Goblin, 1), "a blue slime does not");
        assert!(
            !belongs_to(Invasion::Goblin, 143),
            "nor does a member of a different invasion"
        );
        assert!(belongs_to(Invasion::Pirate, 491), "the Dutchman counts");
        assert!(belongs_to(Invasion::Martian, 395), "so does the saucer");
    }
}
