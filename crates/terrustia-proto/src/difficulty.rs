//! How a world's difficulty and its player count change what an enemy is made of.
//!
//! Every NPC the game spawns goes through `NPC.ScaleStats`, which multiplies its life, its damage
//! and the coins it carries. Nothing here did that: `life_max` was assigned straight from the type
//! table, so **an expert world served classic-strength enemies and an eight-player fight was
//! exactly as hard as a solo one**. The numbers looked healthy the whole time, which is why it
//! went unnoticed — the fight was simply easier than it should have been.
//!
//! Two independent scalings, both from `Terraria/NPC.cs`:
//!
//! * **Difficulty** — a set of linear curves in `GameDifficultyData`, sampled at the world's
//!   difficulty. Classic is 1, expert 2, master 3, journey 0.5.
//! * **Player count** — `GetStatScalingFactors`, which is a specific accumulating series rather
//!   than anything as simple as "×N". It applies to bosses only.

/// The difficulty value a world's game mode samples the curves at.
///
/// `Main.Difficulty` in the game; the world file's `game_mode` numbering is
/// 0 classic, 1 expert, 2 master, 3 journey.
pub fn of_game_mode(game_mode: u8) -> f32 {
    match game_mode {
        1 => 2.0, // expert
        2 => 3.0, // master
        3 => 0.5, // journey
        _ => 1.0, // classic
    }
}

/// A piecewise-linear curve, sampled the way `GameDifficultyData.LinearCurve` samples.
///
/// Values below the first key return the first output; above the last, the final segment is
/// extrapolated — which is the game's behaviour and matters for the "for the worthy" seed.
fn sample(keys: &[(f32, f32)], at: f32) -> f32 {
    let mut previous = keys[0];
    let mut current = previous;
    for &key in keys {
        current = key;
        if at <= key.0 {
            break;
        }
        previous = key;
    }
    let span = current.0 - previous.0;
    if span == 0.0 {
        return previous.1;
    }
    (at - previous.0) * (current.1 - previous.1) / span + previous.1
}

/// `GameDifficultyData.EnemyMaxLifeMultiplier`.
pub fn life_multiplier(difficulty: f32) -> f32 {
    sample(&[(0.5, 0.5), (4.0, 4.0)], difficulty)
}

/// `GameDifficultyData.EnemyDamageMultiplier`.
pub fn damage_multiplier(difficulty: f32) -> f32 {
    sample(&[(0.5, 0.5), (3.0, 3.0), (4.0, 5.333_333_5)], difficulty)
}

/// `GameDifficultyData.HostileProjectileDamageMultiplier`.
///
/// Separate from [`damage_multiplier`] because the game keeps them separate: a projectile carries
/// its own `hostileDamageScaling` from `SetDefaults` rather than inheriting whatever fired it.
/// This only started mattering once the projectiles themselves existed — before that every boss
/// shot was dropped before it was created, so its damage was moot.
pub fn hostile_projectile_multiplier(difficulty: f32) -> f32 {
    sample(&[(0.5, 0.5), (3.0, 3.0)], difficulty)
}

/// `GameDifficultyData.EnemyMoneyDropMultiplier`.
pub fn money_multiplier(difficulty: f32) -> f32 {
    sample(&[(1.0, 1.0), (2.0, 2.5), (3.0, 2.5), (4.0, 3.5)], difficulty)
}

/// The boss life multiplier for a given number of players, from `NPC.GetStatScalingFactors`.
///
/// Not `1 + 0.35 * (n - 1)`, and not a decaying series either: `boost` climbs a third of the way
/// towards one on every step, so each additional player adds **more** than the one before,
/// approaching a full extra boss's worth of health each. Two players give 1.35, three 1.92, four
/// 2.63.
///
/// Above eight it is flattened to `(balance * 2 + 8) / 3`, which is what stops a crowded server
/// from turning a boss into something nobody can out-damage.
pub fn balance(players: u32) -> f32 {
    let mut balance = 1.0f32;
    let mut boost = 0.35f32;
    for _ in 1..players.max(1) {
        balance += boost;
        boost += (1.0 - boost) / 3.0;
    }
    if balance > 8.0 {
        balance = (balance * 2.0 + 8.0) / 3.0;
    }
    balance.min(1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three modes a server actually runs in.
    #[test]
    fn the_curves_hit_their_named_points() {
        assert_eq!(life_multiplier(of_game_mode(0)), 1.0, "classic");
        assert_eq!(life_multiplier(of_game_mode(1)), 2.0, "expert");
        assert_eq!(life_multiplier(of_game_mode(2)), 3.0, "master");
        assert_eq!(life_multiplier(of_game_mode(3)), 0.5, "journey");

        assert_eq!(damage_multiplier(of_game_mode(0)), 1.0);
        assert_eq!(damage_multiplier(of_game_mode(1)), 2.0);
        assert_eq!(damage_multiplier(of_game_mode(2)), 3.0);

        assert_eq!(money_multiplier(of_game_mode(0)), 1.0);
        assert_eq!(money_multiplier(of_game_mode(1)), 2.5);

        assert_eq!(hostile_projectile_multiplier(of_game_mode(0)), 1.0);
        assert_eq!(hostile_projectile_multiplier(of_game_mode(1)), 2.0);
        assert_eq!(hostile_projectile_multiplier(of_game_mode(2)), 3.0);
    }

    /// One player changes nothing, and each one after that adds more than the last.
    ///
    /// I wrote this test the other way round first, from the reasonable-sounding assumption that
    /// the series decays. It does not: `boost` climbs towards one, so the marginal player matters
    /// more, not less. The implementation was right and the expectation was wrong.
    #[test]
    fn each_extra_player_adds_more_than_the_last() {
        assert_eq!(balance(0), 1.0);
        assert_eq!(balance(1), 1.0, "a lone player fights the plain boss");
        assert!((balance(2) - 1.35).abs() < 1e-5);
        assert!((balance(3) - 1.916_666).abs() < 1e-4);
        assert!((balance(4) - 2.627_777).abs() < 1e-4);

        let steps: Vec<f32> = (1..8).map(|n| balance(n + 1) - balance(n)).collect();
        for pair in steps.windows(2) {
            assert!(
                pair[1] > pair[0],
                "each extra player must add more than the one before: {steps:?}",
            );
        }
        // And never more than a whole extra boss's worth.
        assert!(steps.iter().all(|s| *s < 1.0), "{steps:?}");
    }

    /// Past eight players the curve is bent down rather than left to run away.
    #[test]
    fn a_large_crowd_is_flattened() {
        let unbent = {
            let (mut balance, mut boost) = (1.0f32, 0.35f32);
            for _ in 1..30 {
                balance += boost;
                boost += (1.0 - boost) / 3.0;
            }
            balance
        };
        let actual = balance(30);
        assert!(actual < unbent, "thirty players must be bent down: {actual} vs {unbent}");
        assert!((actual - (unbent * 2.0 + 8.0) / 3.0).abs() < 1e-3);
    }
}
