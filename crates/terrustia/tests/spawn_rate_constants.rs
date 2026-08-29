//! Golden pins for `game::spawn::rates`'s modifiers against `NPC.GetSpawnRate` in the decompiled
//! 1.4.5.8 source (`NPC.cs`), so a drift in any one multiplier fails here naming exactly which one
//! moved, rather than only showing up as a vaguely wrong spawn feel.
//!
//! `spawn.rs`'s own `#[cfg(test)] mod rate_tests` already checks *direction* ("caverns are busier
//! than the surface"). This file adds the *exact* number for each modifier, each isolated by holding
//! every other axis at its neutral value, with the citation that makes the number checkable against
//! source rather than merely plausible.
//!
//! Every citation is to the non-`remixWorld` branch of `GetSpawnRate`: this server does not model
//! the "Don't Starve" seed, and `spawn.rs`'s own module doc already restricts scope to
//! depth/hardmode/events/town suppression, so nothing here claims coverage of the rest of the real
//! method (journey mode's slider, the jungle/desert/sandstorm/dungeon/getGoodWorld branches, the
//! `nearbyActiveNPCs` self-correction, and more all exist in source and are simply out of scope for
//! what this module implements).
//!
//! `rates()` keeps the running rate and cap as `f32` throughout and casts once at the very end,
//! where the game's own `GetSpawnRate` reassigns an `int` at every step and so truncates after each
//! multiplication. Every combination this file exercises happens to land on an exact multiple at
//! every intermediate step (600, 540, 300, 360, 243, 240, 108, 120, 72 — never a fraction), so the
//! two truncation orders cannot disagree here. That is a property of these specific inputs, not a
//! general proof the two orders always agree.

use rand::SeedableRng;
use rand::rngs::SmallRng;
use terrustia::game::spawn::{Conditions, Depth, MAX_SPAWNS, SPAWN_RATE, rates};

fn plain() -> Conditions {
    Conditions {
        depth: Depth::Surface,
        hard_mode: false,
        day_time: true,
        blood_moon: false,
        eclipse: false,
        event_moon: false,
        town_npcs: 0,
    }
}

/// A fresh RNG for a call with `town_npcs: 0` (or an event overruling the town), which never
/// touches `rates`'s own town-suppression roll and so cannot care which one it gets.
fn any_rng() -> SmallRng {
    SmallRng::seed_from_u64(0)
}

/// `NPC.cs:6190`: `private static int defaultSpawnRate = 600;`
/// `NPC.cs:6192`: `private static int defaultMaxSpawns = 5;`
#[test]
fn the_baseline_matches_defaultspawnrate_and_defaultmaxspawns() {
    assert_eq!(SPAWN_RATE, 600, "NPC.cs:6190, defaultSpawnRate");
    assert_eq!(MAX_SPAWNS, 5.0, "NPC.cs:6192, defaultMaxSpawns");
    assert_eq!(
        rates(plain(), &mut any_rng()),
        (600, 5.0, false),
        "a plain surface daytime world"
    );
}

/// `NPC.cs:478-482`:
/// ```csharp
/// if (Main.hardMode) {
///     spawnRate = (int)((double)defaultSpawnRate * 0.9);
///     maxSpawns = defaultMaxSpawns + 1;
/// }
/// ```
/// The cap bump is additive (`+1`), not a multiplier — worth pinning separately since every other
/// modifier in this file is multiplicative and it would be easy to fold this one in wrongly.
#[test]
fn hardmode_is_09_times_the_rate_and_one_more_slot() {
    let (rate, cap, _) = rates(
        Conditions {
            hard_mode: true,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(rate, (600.0 * 0.9) as u32, "NPC.cs:480");
    assert_eq!(cap, 6.0, "NPC.cs:481, defaultMaxSpawns + 1");
}

/// `NPC.cs:483-486`:
/// ```csharp
/// if (player.position.Y > (float)(Main.UnderworldLayer * 16)) {
///     maxSpawns = (int)((float)maxSpawns * 2f);
/// }
/// ```
/// This is the first depth check in the method and the only one that never touches `spawnRate` at
/// all — every other depth branch changes both numbers.
#[test]
fn the_underworld_only_doubles_the_cap() {
    let (rate, cap, _) = rates(
        Conditions {
            depth: Depth::Underworld,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(rate, 600, "NPC.cs:483-486 never assigns spawnRate");
    assert_eq!(cap, 10.0, "NPC.cs:485, maxSpawns * 2f");
}

/// `NPC.cs:502-506`, the non-`remixWorld` branch below `rockLayer`:
/// ```csharp
/// spawnRate = (int)((double)spawnRate * 0.4);
/// maxSpawns = (int)((float)maxSpawns * 1.9f);
/// ```
#[test]
fn caverns_are_04_times_the_rate_and_19_times_the_cap() {
    let (rate, cap, _) = rates(
        Conditions {
            depth: Depth::Cavern,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(rate, (600.0 * 0.4) as u32, "NPC.cs:504");
    assert_eq!(cap, 5.0 * 1.9, "NPC.cs:505");
}

/// `NPC.cs:515-524`, the non-`remixWorld` branch below `worldSurface`:
/// ```csharp
/// else if (Main.hardMode) {
///     spawnRate = (int)((double)spawnRate * 0.45);
///     maxSpawns = (int)((float)maxSpawns * 1.8f);
/// } else {
///     spawnRate = (int)((double)spawnRate * 0.5);
///     maxSpawns = (int)((float)maxSpawns * 1.7f);
/// }
/// ```
/// The hardmode branch multiplies the `spawnRate` the earlier hardmode check (478-482) already
/// discounted, so the two stack: `600 * 0.9 * 0.45 = 243`, not `600 * 0.45 = 270`.
#[test]
fn underground_is_05_17_pre_hardmode_and_045_18_stacked_with_hardmode() {
    let (rate, cap, _) = rates(
        Conditions {
            depth: Depth::Underground,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(rate, (600.0 * 0.5) as u32, "NPC.cs:522, pre-hardmode");
    assert_eq!(cap, 5.0 * 1.7, "NPC.cs:523, pre-hardmode");

    let (rate, cap, _) = rates(
        Conditions {
            depth: Depth::Underground,
            hard_mode: true,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(
        rate, 243,
        "NPC.cs:517 stacked on NPC.cs:480: 600 * 0.9 * 0.45"
    );
    assert_eq!(
        cap,
        6.0 * 1.8,
        "NPC.cs:518, on the hardmode-bumped cap of 6"
    );
}

/// `NPC.cs:534-537`, the non-`remixWorld`, non-daytime surface branch:
/// ```csharp
/// spawnRate = (int)((double)spawnRate * 0.6);
/// maxSpawns = (int)((float)maxSpawns * 1.3f);
/// ```
#[test]
fn surface_night_is_06_times_the_rate_and_13_times_the_cap() {
    let (rate, cap, _) = rates(
        Conditions {
            day_time: false,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(rate, (600.0 * 0.6) as u32, "NPC.cs:536");
    assert_eq!(cap, 5.0 * 1.3, "NPC.cs:537");
}

/// `NPC.cs:538-542`, nested inside the non-daytime branch:
/// ```csharp
/// if (Main.bloodMoon) {
///     spawnRate = (int)((double)spawnRate * 0.3);
///     maxSpawns = (int)((float)maxSpawns * 1.8f);
/// }
/// ```
/// Stacks on the night discount just above: `600 * 0.6 * 0.3 = 108`.
#[test]
fn a_blood_moon_is_03_times_the_already_nighttime_rate() {
    let (rate, cap, _) = rates(
        Conditions {
            day_time: false,
            blood_moon: true,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(
        rate, 108,
        "NPC.cs:540 stacked on NPC.cs:536: 600 * 0.6 * 0.3"
    );
    assert_eq!(cap, 5.0 * 1.3 * 1.8, "NPC.cs:541, stacked on the night cap");
}

/// `NPC.cs:543-547`, also nested inside the non-daytime branch:
/// ```csharp
/// if ((Main.pumpkinMoon || Main.snowMoon) && ...) {
///     spawnRate = (int)((double)spawnRate * 0.2);
///     maxSpawns *= 2;
/// }
/// ```
/// A pumpkin/frost moon only ever runs at night, so this is exercised together with `day_time:
/// false` here, the same as the game's own nesting.
#[test]
fn an_event_moon_is_02_times_the_already_nighttime_rate() {
    let (rate, cap, _) = rates(
        Conditions {
            day_time: false,
            event_moon: true,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(
        rate, 72,
        "NPC.cs:545 stacked on NPC.cs:536: 600 * 0.6 * 0.2"
    );
    assert_eq!(cap, 5.0 * 1.3 * 2.0, "NPC.cs:546, maxSpawns *= 2");
}

/// `NPC.cs:549-553`, the daytime counterpart:
/// ```csharp
/// else if (Main.dayTime && Main.eclipse) {
///     spawnRate = (int)((double)spawnRate * 0.2);
///     maxSpawns = (int)((float)maxSpawns * 1.9f);
/// }
/// ```
#[test]
fn a_daytime_eclipse_is_02_times_the_rate_and_19_times_the_cap() {
    let (rate, cap, _) = rates(
        Conditions {
            eclipse: true,
            ..plain()
        },
        &mut any_rng(),
    );
    assert_eq!(rate, (600.0 * 0.2) as u32, "NPC.cs:551");
    assert_eq!(cap, 5.0 * 1.9, "NPC.cs:552");
}

/// `NPC.cs:750-757`, the floor and ceiling every other modifier in the method is clamped inside:
/// ```csharp
/// if ((double)spawnRate < (double)defaultSpawnRate * 0.1) {
///     spawnRate = (int)((double)defaultSpawnRate * 0.1);
/// }
/// if (maxSpawns > defaultMaxSpawns * 3) {
///     maxSpawns = defaultMaxSpawns * 3;
/// }
/// ```
/// Stacking every *surface* event at once is what actually pushes both bounds: the underworld
/// branch (`NPC.cs:483-486`) never touches `spawnRate` at all, so `spawn.rs`'s own
/// `the_rate_is_bounded` test — which stacks `Depth::Underworld` with every event flag — proves the
/// values never fall below the floor without ever actually driving them there (`worst.0` lands at
/// `540`, from the hardmode discount alone, comfortably above the `60` floor its own assertion only
/// checks `>=` against). This test instead stacks night, a blood moon *and* an event moon on the
/// surface, which the code applies as three independent multiplications
/// (`spawn.rs`'s `Depth::Surface` arm, three separate `if`s, not `else if`s) and so is the
/// combination that actually reaches both clamps.
#[test]
fn the_bounds_are_exactly_60_and_15() {
    assert_eq!(
        (SPAWN_RATE as f32 * 0.1) as u32,
        60,
        "NPC.cs:752, defaultSpawnRate * 0.1"
    );
    assert_eq!(MAX_SPAWNS * 3.0, 15.0, "NPC.cs:756, defaultMaxSpawns * 3");

    let worst = rates(
        Conditions {
            depth: Depth::Surface,
            hard_mode: true,
            day_time: false,
            blood_moon: true,
            eclipse: false,
            event_moon: true,
            town_npcs: 0,
        },
        &mut any_rng(),
    );
    assert_eq!(worst.0, 60, "clamped at the floor");
    assert_eq!(worst.1, 15.0, "clamped at the ceiling");
}

/// Town suppression's real shape (C1-b item 8, fixed): `spawn.rs` used to model it as one
/// deterministic `(slower, fewer)` pair per headcount, applied the same way regardless of what the
/// game itself actually rolls. Real vanilla (`NPC.cs:795-924`) is gated on the same event
/// exclusions `spawn.rs` already models (no invasion, no active blood/pumpkin/snow moon at night,
/// no daytime eclipse, not in a corrupt/crimson/meteor/Old One's Army zone — `NPC.cs:800`), which
/// `spawn.rs`'s own `event` guard correctly mirrors in spirit. Past that gate, in the ordinary
/// surface/underground branch that covers every base a player actually builds (`NPC.cs:856-923`,
/// the underworld's own separate, simpler fork at `NPC.cs:802-855` is not modelled — a base built
/// in the underworld is the rare exception, not the case this quiets), it is a per-attempt coin
/// flip between two outcomes (`Main.rand.Next(...)`) rather than a fixed multiplier: some attempts
/// scale `spawnRate`, others leave it untouched and instead shrink `maxSpawns` and force the spawn
/// to be a friendly critter (`spawnFriendly = true`) instead of a monster. `rates` now returns that
/// third element directly (`(rate, cap, spawn_friendly)`) and takes an `&mut SmallRng` to roll it,
/// matching the shape of a genuinely probabilistic function rather than a table lookup.
///
/// `townNPCs >= 3` is the one headcount where classic (non-expert) mode is fully deterministic:
/// ```csharp
/// // NPC.cs:903-923, the townNPCs >= 3 branch, ordinary (non-graveyard) case:
/// else if (townNPCs >= 3) {
///     noWorms = true;
///     if (ZoneGraveyard && ...) { /* not modelled here: a graveyard-specific sub-case */ }
///     else {
///         if (!Main.expertMode || Main.rand.Next(30) != 0) {
///             spawnFriendly = true;
///         }
///         maxSpawns = (int)((double)(float)maxSpawns * 0.6);
///     }
/// }
/// ```
/// `!Main.expertMode` is unconditionally `true` in classic mode, so `spawnFriendly` is set on
/// *every* attempt — `spawnRate` is never assigned in this branch at all. `spawn.rs::rates` mirrors
/// this exactly: `town_npcs >= 3` always returns `spawn_friendly: true` and the unchanged base rate.
#[test]
fn town_suppression_at_three_residents_leaves_the_rate_unchanged_in_classic_mode() {
    let (base, base_cap, base_friendly) = rates(plain(), &mut any_rng());
    assert!(
        !base_friendly,
        "no town at all never forces a friendly spawn"
    );

    let mut rng = SmallRng::seed_from_u64(7);
    let (three, three_cap, three_friendly) = rates(
        Conditions {
            town_npcs: 3,
            ..plain()
        },
        &mut rng,
    );
    assert_eq!(
        three, base,
        "NPC.cs:917-921: classic mode always sets spawnFriendly instead of touching spawnRate"
    );
    assert!(three_friendly, "and always forces a friendly spawn");
    assert_eq!(three_cap, base_cap * 0.6, "NPC.cs:921, maxSpawns * 0.6");
}
