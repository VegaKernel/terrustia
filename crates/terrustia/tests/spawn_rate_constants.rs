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

/// `NPC.cs:6190`: `private static int defaultSpawnRate = 600;`
/// `NPC.cs:6192`: `private static int defaultMaxSpawns = 5;`
#[test]
fn the_baseline_matches_defaultspawnrate_and_defaultmaxspawns() {
    assert_eq!(SPAWN_RATE, 600, "NPC.cs:6190, defaultSpawnRate");
    assert_eq!(MAX_SPAWNS, 5.0, "NPC.cs:6192, defaultMaxSpawns");
    assert_eq!(rates(plain()), (600, 5.0), "a plain surface daytime world");
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
    let (rate, cap) = rates(Conditions {
        hard_mode: true,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        depth: Depth::Underworld,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        depth: Depth::Cavern,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        depth: Depth::Underground,
        ..plain()
    });
    assert_eq!(rate, (600.0 * 0.5) as u32, "NPC.cs:522, pre-hardmode");
    assert_eq!(cap, 5.0 * 1.7, "NPC.cs:523, pre-hardmode");

    let (rate, cap) = rates(Conditions {
        depth: Depth::Underground,
        hard_mode: true,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        day_time: false,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        day_time: false,
        blood_moon: true,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        day_time: false,
        event_moon: true,
        ..plain()
    });
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
    let (rate, cap) = rates(Conditions {
        eclipse: true,
        ..plain()
    });
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

    let worst = rates(Conditions {
        depth: Depth::Surface,
        hard_mode: true,
        day_time: false,
        blood_moon: true,
        eclipse: false,
        event_moon: true,
        town_npcs: 0,
    });
    assert_eq!(worst.0, 60, "clamped at the floor");
    assert_eq!(worst.1, 15.0, "clamped at the ceiling");
}

/// Town suppression's real shape does not survive being pinned as a golden.
///
/// `spawn.rs` models it as one deterministic `(slower, fewer)` pair per headcount
/// (`0 => (1.0, 1.0)`, `1 => (2.0, 0.6)`, `2 | _ => (3.0, 0.6)`, at `spawn.rs:95-100`), applied the
/// same way regardless of where the player is standing.
///
/// Real vanilla's town suppression (`NPC.cs:795-924`) is neither of those things. It is gated on the
/// same event exclusions `spawn.rs` already models (no invasion, no active blood/pumpkin/snow moon
/// at night, no daytime eclipse, not in a corrupt/crimson/meteor/Old One's Army zone —
/// `NPC.cs:800`), which `spawn.rs`'s own `event` guard at `spawn.rs:93-94` correctly mirrors in
/// spirit. But past that gate it forks on a *second* axis `Conditions` has no field for at all —
/// whether the player is in the underworld region (`NPC.cs:802`, `Center.Y / 16f >
/// Main.UnderworldLayer`) — and, in the ordinary surface/underground branch that covers every base
/// a player actually builds (`NPC.cs:856-923`), it is a per-tick coin flip between two outcomes
/// (`Main.rand.Next(...)`) rather than a fixed multiplier: some ticks scale `spawnRate`, others
/// leave it untouched and instead shrink `maxSpawns` and force the spawn to be a friendly critter
/// (`spawnFriendly = true`) instead of a monster. There is no single number source assigns for "the"
/// rate multiplier at a given headcount, so nothing here pins one for `townNPCs == 1` or `== 2` —
/// per this lane's own rule, a value that cannot be derived from source is left out rather than
/// guessed at.
///
/// `townNPCs >= 3` is the one headcount where classic (non-expert) mode *is* deterministic, and it
/// is a real, clean divergence:
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
/// *every* tick — `spawnRate` is never assigned in this branch at all. The real classic-mode rate
/// multiplier for three or more townsfolk is `1.0` (unchanged): every attempt still happens on
/// schedule, it just always rolls a friendly critter instead of a monster. `spawn.rs` instead scales
/// `spawnRate` by `3.0`, which does not correspond to anything this branch does.
///
/// Marked `#[ignore]`: fixing this needs a `spawnFriendly`-shaped outcome (a fork on *what* spawns,
/// not a further multiplier on *how often*), which is a modelling change to production code and out
/// of this lane's scope (tests only).
#[test]
#[ignore = "divergence: NPC.cs:903-923 leaves spawnRate at 1.0x for townNPCs>=3 in classic mode \
            (spawnFriendly replaces the roll instead of throttling it); spawn.rs:99 scales it 3.0x. \
            Needs a production change (a friendly-vs-hostile fork), out of scope for a test-only lane."]
fn town_suppression_at_three_residents_leaves_the_rate_unchanged_in_classic_mode() {
    let (base, _) = rates(plain());
    let (three, _) = rates(Conditions {
        town_npcs: 3,
        ..plain()
    });
    assert_eq!(
        three, base,
        "NPC.cs:917-921: classic mode always sets spawnFriendly instead of touching spawnRate"
    );
}
