//! Invasions.
//!
//! An invasion is not a spawn table swap. It is a countdown attached to the world: it has a size,
//! it comes from one edge, every member killed takes one off the size, and when the size reaches
//! zero it is over and the world says so. Everything else about it — which enemies, how often, how
//! close to spawn they appear — hangs off those four facts.
//!
//! The size is `80 + 40` per eligible player, where eligible means someone with two hundred
//! maximum life or more; a world of characters who have not found a life crystal cannot be
//! invaded at all. The Goblins and the Frost Legion share that base exactly, the Pirates are
//! larger and the Martians larger still, and the Martians are the one invasion that arrives
//! *at spawn* rather than from an edge, because the probe that called them in reported where
//! you were.

use rand::{Rng, rngs::SmallRng};

/// Which invasion is under way.
///
/// The numbering is the game's `Main.invasionType`, because it travels in the world file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invasion {
    Goblin = 1,
    FrostLegion = 2,
    Pirate = 3,
    Martian = 4,
}

impl Invasion {
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Goblin),
            2 => Some(Self::FrostLegion),
            3 => Some(Self::Pirate),
            4 => Some(Self::Martian),
            _ => None,
        }
    }

    /// What the server announces when it begins.
    pub fn arrival(self) -> &'static str {
        match self {
            Self::Goblin => "A goblin army is approaching from the",
            Self::FrostLegion => "A frost legion is approaching from the",
            Self::Pirate => "Pirates are approaching from the",
            Self::Martian => "Martians are invading!",
        }
    }

    /// ...and when it is beaten.
    pub fn defeat(self) -> &'static str {
        match self {
            Self::Goblin => "The goblin army has been defeated!",
            Self::FrostLegion => "The frost legion has been defeated!",
            Self::Pirate => "The pirates have been defeated!",
            Self::Martian => "The martian invasion has been defeated!",
        }
    }

    /// How many members it takes to see off, given how many players qualify.
    ///
    /// A player qualifies at two hundred maximum life. With nobody qualifying there is no
    /// invasion at all, which is why an early world cannot be invaded by accident.
    ///
    /// `Main.cs:65435-65443` `StartInvasion`: the base is `80 + 40 * num` for everything, type 3
    /// (`InvasionID.PirateInvasion`) adds `40 + 20 * num` on top, and type 4
    /// (`InvasionID.MartianMadness`) replaces the base outright with `160 + 40 * num`. Type 2
    /// (`InvasionID.SnowLegion`, the Frost Legion) gets no adjustment at all, so it is the same
    /// size as a Goblin army. `Main.cs:65471-65486` `FakeLoadInvasionStart` agrees: it groups
    /// `case 1:` and `case 2:` on one 80/40 arm and gives `case 3:` alone the 120/60 arm.
    pub fn size_for(self, qualifying_players: usize) -> i32 {
        if qualifying_players == 0 {
            return 0;
        }
        let players = qualifying_players as i32;
        match self {
            Self::Martian => 160 + 40 * players,
            Self::Pirate => 80 + 40 * players + 40 + 20 * players,
            _ => 80 + 40 * players,
        }
    }
}

/// An invasion in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvasionState {
    pub kind: Invasion,
    /// How many are left to kill.
    pub remaining: i32,
    /// How many it started with, which is what the progress bar and the wave rules divide by.
    pub started_with: i32,
    /// The tile column its front has reached, which moves.
    pub from_x: i32,
    /// Where it is marching to, which is spawn.
    pub toward_x: i32,
}

impl InvasionState {
    /// Begin an invasion, or `None` when nobody in the world qualifies to be invaded.
    ///
    /// `spawn_x` and `world_width` are in tiles.
    pub fn begin(
        kind: Invasion,
        qualifying_players: usize,
        spawn_x: i32,
        world_width: i32,
        rng: &mut SmallRng,
    ) -> Option<Self> {
        let size = kind.size_for(qualifying_players);
        if size <= 0 {
            return None;
        }
        // The Martians land on top of you; everything else marches in from one side or the other.
        let from_x = if kind == Invasion::Martian {
            spawn_x - 1
        } else if rng.random_range(0..2) == 0 {
            0
        } else {
            world_width
        };
        Some(Self {
            kind,
            remaining: size,
            started_with: size,
            from_x,
            // Everything marches on the town. The Martians begin there, so their front is already
            // where it is going and they never move.
            toward_x: spawn_x,
        })
    }

    /// How far through it is, from zero to one.
    pub fn progress(&self) -> f32 {
        if self.started_with <= 0 {
            return 1.0;
        }
        1.0 - self.remaining as f32 / self.started_with as f32
    }

    /// Whether it is finished.
    pub fn beaten(&self) -> bool {
        self.remaining <= 0
    }

    /// Which side it came from, for the announcement.
    pub fn side(&self) -> &'static str {
        if self.from_x <= self.toward_x {
            "west"
        } else {
            "east"
        }
    }

    /// How far an invasion's front moves in a tick.
    pub const MARCH: i32 = 1;
    /// How close to that front a player must be for invaders to arrive around them, in tiles.
    pub const FRONT: i32 = 187;

    /// Advance the front one tick. Returns whether it has just arrived at spawn.
    ///
    /// An invasion *marches*: it is not a column that spawns things forever, it is a line that
    /// closes on the town, and invaders appear around whichever player is near it. Left fixed at
    /// the edge it came from, an invasion is something that happens at the far side of the ocean
    /// and nowhere a player would ever stand.
    pub fn march(&mut self) -> bool {
        if self.from_x == self.toward_x {
            return false;
        }
        if self.from_x > self.toward_x {
            self.from_x = (self.from_x - Self::MARCH).max(self.toward_x);
        } else {
            self.from_x = (self.from_x + Self::MARCH).min(self.toward_x);
        }
        self.from_x == self.toward_x
    }

    /// Whether a player standing here is close enough to the front to be attacked.
    pub fn reaches(&self, player_x: i32) -> bool {
        (player_x - self.from_x).abs() <= Self::FRONT
    }

    /// Pick the next invader to send in.
    ///
    /// The nested rolls are the game's, and the order matters: an earlier roll that succeeds stops
    /// the later ones being made at all, so these are not independent probabilities and flattening
    /// them into a weighted table would change the mix.
    pub fn next_invader(&self, hardmode: bool, present: &[u16], rng: &mut SmallRng) -> Option<u16> {
        let absent = |ty: u16| !present.contains(&ty);
        let roll = |rng: &mut SmallRng, n: u32| rng.random_range(0..n) == 0;

        Some(match self.kind {
            Invasion::Goblin => {
                // The summoner only turns up in hardmode, and only one at a time.
                if hardmode && absent(471) && roll(rng, 30) {
                    471
                } else if roll(rng, 9) {
                    29
                } else if roll(rng, 5) {
                    26
                } else if roll(rng, 3) {
                    111
                } else if roll(rng, 3) {
                    27
                } else {
                    28
                }
            }
            Invasion::FrostLegion => {
                if roll(rng, 7) {
                    145
                } else if roll(rng, 3) {
                    143
                } else {
                    144
                }
            }
            Invasion::Pirate => {
                // The Flying Dutchman only appears once the invasion is half beaten.
                if self.remaining < self.started_with / 2 && roll(rng, 20) && absent(491) {
                    491
                } else if roll(rng, 30) && absent(216) {
                    216
                } else if roll(rng, 11) {
                    215
                } else if roll(rng, 9) {
                    252
                } else if roll(rng, 7) {
                    214
                } else if roll(rng, 3) {
                    213
                } else {
                    212
                }
            }
            Invasion::Martian => return self.next_martian(present, rng),
        })
    }

    /// The Martian pool, which is drawn in three tiers rather than as one list.
    ///
    /// A single roll of seven picks the tier — heavy, medium or light — and the saucer is only
    /// eligible once the invasion is nearly a third beaten, and only one at a time.
    fn next_martian(&self, present: &[u16], rng: &mut SmallRng) -> Option<u16> {
        let absent = |ty: u16| !present.contains(&ty);
        let saucer_due = self.progress() >= 0.3 && absent(395);
        let tier = rng.random_range(0..7);

        if saucer_due && rng.random_range(0..45) == 0 {
            return Some(395);
        }
        Some(if tier >= 6 {
            if saucer_due && rng.random_range(0..20) == 0 {
                395
            } else if rng.random_range(0..2) == 0 {
                390
            } else {
                386
            }
        } else if tier >= 4 {
            match rng.random_range(0..5) {
                0 | 1 => 382,
                4 => 388,
                _ => 381,
            }
        } else {
            let mut pick = rng.random_range(0..4);
            if pick == 3 {
                if absent(520) {
                    return Some(520);
                }
                pick = rng.random_range(0..3);
            }
            match pick {
                0 => 385,
                1 => 389,
                _ => 383,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(10)
    }

    /// A world of characters who have not found a life crystal cannot be invaded.
    #[test]
    fn nobody_qualified_means_no_invasion() {
        assert_eq!(Invasion::Goblin.size_for(0), 0);
        assert!(InvasionState::begin(Invasion::Goblin, 0, 100, 4200, &mut rng()).is_none());
    }

    /// More players means a longer invasion, and the Martians are the longest of them.
    ///
    /// The exact numbers are `Main.cs:65435-65443` `StartInvasion`, cross-checked against
    /// `Main.cs:65471-65486` `FakeLoadInvasionStart`, which reconstructs the same sizes from a
    /// saved world and groups Goblin and Frost Legion on one arm. Only the Pirates take the
    /// `40 + 20 * num` bonus; the Frost Legion is the same size as a Goblin army.
    #[test]
    fn the_size_grows_with_the_party() {
        assert_eq!(Invasion::Goblin.size_for(1), 120);
        assert_eq!(Invasion::Goblin.size_for(4), 240);
        assert_eq!(
            Invasion::FrostLegion.size_for(1),
            Invasion::Goblin.size_for(1),
            "vanilla gives the Frost Legion no bonus over a Goblin army"
        );
        assert_eq!(Invasion::FrostLegion.size_for(4), 240);
        assert_eq!(Invasion::Pirate.size_for(1), 180, "80 + 40 + 40 + 20");
        assert_eq!(Invasion::Pirate.size_for(4), 360, "80 + 160 + 40 + 80");
        assert_eq!(Invasion::Martian.size_for(1), 200);
        assert_eq!(Invasion::Martian.size_for(4), 320);
        assert!(Invasion::Martian.size_for(1) > Invasion::Pirate.size_for(1));
    }

    /// An invasion marches on the town, and arrives.
    #[test]
    fn an_invasion_marches_on_the_town() {
        let mut rng = SmallRng::seed_from_u64(1);
        let mut army =
            InvasionState::begin(Invasion::Goblin, 1, 2100, 4200, &mut rng).expect("an invasion");
        let started = army.from_x;
        assert_ne!(started, 2100, "it should begin at an edge");
        assert!(!army.reaches(2100), "and not reach the town yet");

        let mut arrived = None;
        for tick in 1..10_000 {
            if army.march() {
                arrived = Some(tick);
                break;
            }
        }
        assert_eq!(
            arrived,
            Some((started - 2100).abs()),
            "it should take a tick a tile"
        );
        assert_eq!(army.from_x, 2100, "and stop at the town");
        assert!(army.reaches(2100), "and reach whoever is standing there");
        assert!(!army.march(), "and stay arrived rather than marching past");
    }

    /// The front only reaches a player who is near it.
    #[test]
    fn the_front_only_reaches_who_is_near_it() {
        let mut rng = SmallRng::seed_from_u64(2);
        let army =
            InvasionState::begin(Invasion::Pirate, 1, 2100, 4200, &mut rng).expect("an invasion");
        assert!(army.reaches(army.from_x), "somebody standing on it");
        assert!(
            army.reaches(army.from_x + InvasionState::FRONT),
            "and somebody at the edge of its reach"
        );
        assert!(
            !army.reaches(army.from_x + InvasionState::FRONT + 1),
            "but not one step further"
        );
    }

    /// The Martians land on the town, so their front never has to move.
    #[test]
    fn the_martians_are_already_here() {
        let mut rng = SmallRng::seed_from_u64(3);
        let mut martians =
            InvasionState::begin(Invasion::Martian, 1, 2100, 4200, &mut rng).expect("an invasion");
        assert!(martians.reaches(2100), "they are already on the town");
        martians.march();
        assert!(!martians.march());
    }

    /// The Martians land where you are; everything else marches in from an edge.
    #[test]
    fn the_martians_arrive_at_spawn_and_the_rest_from_a_side() {
        let martians = InvasionState::begin(Invasion::Martian, 1, 2103, 4200, &mut rng()).unwrap();
        assert_eq!(martians.from_x, 2102, "right on top of spawn");

        // Whichever side is chosen, it is a side.
        for seed in 0..8 {
            let mut r = SmallRng::seed_from_u64(seed);
            let goblins = InvasionState::begin(Invasion::Goblin, 1, 2103, 4200, &mut r).unwrap();
            assert!(
                goblins.from_x == 0 || goblins.from_x == 4200,
                "got {}",
                goblins.from_x
            );
        }
    }

    /// Progress runs from nothing to finished, and finished is what ends it.
    #[test]
    fn an_invasion_ends_when_its_last_member_dies() {
        let mut state = InvasionState::begin(Invasion::Goblin, 1, 100, 4200, &mut rng()).unwrap();
        assert_eq!(state.progress(), 0.0);
        assert!(!state.beaten());
        state.remaining = state.started_with / 2;
        assert!((state.progress() - 0.5).abs() < 0.01);
        state.remaining = 0;
        assert!(state.beaten());
        assert_eq!(state.progress(), 1.0);
    }

    /// The goblin summoner is a hardmode-only member, and there is never more than one.
    #[test]
    fn the_summoner_only_marches_in_hardmode() {
        let state = InvasionState::begin(Invasion::Goblin, 1, 100, 4200, &mut rng()).unwrap();
        let mut r = rng();
        let classic: Vec<u16> = (0..600)
            .filter_map(|_| state.next_invader(false, &[], &mut r))
            .collect();
        assert!(!classic.contains(&471), "no summoner before the wall falls");

        let mut r = rng();
        let hard: Vec<u16> = (0..600)
            .filter_map(|_| state.next_invader(true, &[], &mut r))
            .collect();
        assert!(hard.contains(&471), "in hardmode he should turn up");

        // ...but not while one is already about.
        let mut r = rng();
        let with_one: Vec<u16> = (0..600)
            .filter_map(|_| state.next_invader(true, &[471], &mut r))
            .collect();
        assert!(!with_one.contains(&471), "only one summoner at a time");
    }

    /// The Flying Dutchman only shows up in the second half of a pirate invasion.
    #[test]
    fn the_dutchman_waits_until_the_pirates_are_losing() {
        let mut state = InvasionState::begin(Invasion::Pirate, 2, 100, 4200, &mut rng()).unwrap();
        let mut r = rng();
        let early: Vec<u16> = (0..2000)
            .filter_map(|_| state.next_invader(true, &[], &mut r))
            .collect();
        assert!(!early.contains(&491), "too early for the Dutchman");

        state.remaining = state.started_with / 4;
        let mut r = rng();
        let late: Vec<u16> = (0..2000)
            .filter_map(|_| state.next_invader(true, &[], &mut r))
            .collect();
        assert!(late.contains(&491), "it should sail in eventually");
    }

    /// Every invasion draws only from its own roster.
    #[test]
    fn each_invasion_sends_its_own() {
        let rosters: [(Invasion, &[u16]); 4] = [
            (Invasion::Goblin, &[26, 27, 28, 29, 111, 471]),
            (Invasion::FrostLegion, &[143, 144, 145]),
            (Invasion::Pirate, &[212, 213, 214, 215, 216, 252, 491]),
            (
                Invasion::Martian,
                &[381, 382, 383, 385, 386, 388, 389, 390, 395, 520],
            ),
        ];
        for (kind, roster) in rosters {
            let mut state = InvasionState::begin(kind, 3, 100, 4200, &mut rng()).unwrap();
            // Halfway through, so the members gated on progress are eligible too.
            state.remaining = state.started_with / 3;
            let mut r = rng();
            for _ in 0..3000 {
                let Some(ty) = state.next_invader(true, &[], &mut r) else {
                    continue;
                };
                assert!(
                    roster.contains(&ty),
                    "{kind:?} should not be sending type {ty}"
                );
            }
        }
    }

    /// The saucer holds off until the invasion is nearly a third beaten.
    #[test]
    fn the_martian_saucer_waits_its_turn() {
        let mut state = InvasionState::begin(Invasion::Martian, 1, 100, 4200, &mut rng()).unwrap();
        let mut r = rng();
        let early: Vec<u16> = (0..3000)
            .filter_map(|_| state.next_invader(true, &[], &mut r))
            .collect();
        assert!(!early.contains(&395), "too early for the saucer");

        state.remaining = state.started_with / 2;
        let mut r = rng();
        let late: Vec<u16> = (0..3000)
            .filter_map(|_| state.next_invader(true, &[], &mut r))
            .collect();
        assert!(late.contains(&395), "it should arrive eventually");
    }
}
