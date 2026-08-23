//! The six cavern enemies a particular world happens to have.
//!
//! Thirteen enemy types live in the caverns — the Crawdads, the Salamanders and the Giant Shellies
//! — but no single world has all of them. Each world draws **six**, and it draws them from its own
//! id rather than from the run's random generator, so the same world always has the same six no
//! matter how many times it is loaded.
//!
//! That is why two worlds feel different underground, and it is why the choice has to be
//! reproduced rather than approximated: a player who knows their world has Salamanders and no
//! Crawdads is right about that, permanently.
//!
//! `NPC.SetWorldSpecificMonstersByWorldID`.

use crate::world::worldgen::rand::UnifiedRandom;

/// The three families, as half-open type ranges.
///
/// Salamanders, Giant Shellies, Crawdads. The third is much the largest, which is why a world is
/// more likely to have several Crawdad variants than several of anything else.
const FAMILIES: [(u16, u16); 3] = [(494, 496), (496, 498), (498, 507)];

/// The six a world has: two families of three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CavernMonsters {
    pub types: [[u16; 3]; 2],
}

impl CavernMonsters {
    /// Work out which six a world with this id has.
    ///
    /// The two families are drawn without replacement — the game re-rolls the first until it
    /// differs from the second — so a world always has two *different* kinds of cavern enemy
    /// rather than six of one.
    pub fn for_world(world_id: i32) -> Self {
        let mut rand = UnifiedRandom::new(world_id);
        let mut first = rand.next_max(3);
        let second = rand.next_max(3);
        while first == second {
            first = rand.next_max(3);
        }

        let mut types = [[0u16; 3]; 2];
        for (row, family) in [first, second].into_iter().enumerate() {
            let (from, to) = FAMILIES[family.clamp(0, 2) as usize];
            for cell in &mut types[row] {
                *cell = rand.next_range(i32::from(from), i32::from(to)) as u16;
            }
        }
        Self { types }
    }

    /// The six, flattened, in the order the sync packet carries them.
    pub fn flat(&self) -> [u16; 6] {
        [
            self.types[0][0],
            self.types[0][1],
            self.types[0][2],
            self.types[1][0],
            self.types[1][1],
            self.types[1][2],
        ]
    }

    /// Pick one at random, as the cavern spawner does.
    pub fn pick(&self, rng: &mut impl rand::Rng) -> u16 {
        let row = rng.random_range(0..2);
        let column = rng.random_range(0..3);
        self.types[row][column]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same world always has the same six, however often it is opened.
    #[test]
    fn a_world_always_has_the_same_six() {
        let once = CavernMonsters::for_world(2_132_016_061);
        let again = CavernMonsters::for_world(2_132_016_061);
        assert_eq!(once, again);
    }

    /// ...and two worlds usually do not.
    #[test]
    fn different_worlds_usually_differ() {
        let a = CavernMonsters::for_world(1);
        let b = CavernMonsters::for_world(2);
        let c = CavernMonsters::for_world(3);
        assert!(
            a != b || b != c,
            "three worlds should not all have the same monsters"
        );
    }

    /// Every drawn type is one of the thirteen cavern enemies.
    #[test]
    fn every_pick_is_a_cavern_enemy() {
        for id in [1i32, 7, 100, -5, i32::MAX, 2_132_016_061] {
            for kind in CavernMonsters::for_world(id).flat() {
                assert!(
                    (494..507).contains(&kind),
                    "world {id} drew {kind}, which is not a cavern enemy"
                );
            }
        }
    }

    /// The two families differ, so a world is never six of one kind.
    #[test]
    fn the_two_families_differ() {
        for id in [1i32, 7, 100, 999, 2_132_016_061] {
            let monsters = CavernMonsters::for_world(id);
            let family = |kind: u16| FAMILIES.iter().position(|(a, b)| kind >= *a && kind < *b);
            let first = family(monsters.types[0][0]);
            let second = family(monsters.types[1][0]);
            assert_ne!(
                first, second,
                "world {id} drew the same family twice: {monsters:?}"
            );
        }
    }
}
