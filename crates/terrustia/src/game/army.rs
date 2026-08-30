//! The Old One's Army — the game's one defence event.
//!
//! Every other invasion is a countdown: kill enough of them and it is over. This one is a siege.
//! There is a thing to protect, the Eternia Crystal, and two portals that pour enemies at it, and
//! the fight ends either when you have killed the wave quota or when the crystal is gone. So the
//! event's state is not "how many left" but "which wave, how many killed in it" — and a wave does
//! not simply have a size, it has a gate rate and a roster, and both change as the waves go up.
//!
//! There are three tiers, gated on progression: one from the start, two once a mechanical boss is
//! down, three once Golem is. A tier is not a difficulty multiplier; each has its own roster, its
//! own wave count, and its own boss at the end — the Dark Mage at tier one, the Ogre at tier two,
//! Betsy at tier three — and the last wave will not complete until that boss is dead.

use rand::{Rng, rngs::SmallRng};

use terrustia_proto::npc_params as ids;
use terrustia_proto::npc_params::{RAISE_CHECK_RANGE, RAISE_MINIMUM, RAISE_MOST, RAISE_RANGE};

/// Which tier is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    One = 1,
    Two = 2,
    Three = 3,
}

impl Tier {
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            _ => None,
        }
    }

    /// The last wave of this tier. Beating it wins the event.
    pub fn waves(self) -> i32 {
        match self {
            Self::One => 5,
            Self::Two | Self::Three => 7,
        }
    }

    /// How many kills the wave asks for.
    ///
    /// Tier three's last wave is the exception: there the count is not kills at all but how far
    /// through Betsy's health you are, which is why the portals keep feeding you enemies during it.
    pub fn required_kills(self, wave: i32) -> i32 {
        match (self, wave) {
            (_, ..=0) => 0,
            (_, 1) => 60,
            (_, 2) => 80,
            (_, 3) => 100,
            (_, 4) => 120,
            (_, 5) => 140,
            (Self::One, _) => 1,
            (_, 6) => 180,
            (Self::Two, 7) => 220,
            (Self::Three, 7) => 100,
            _ => 10,
        }
    }

    /// How often a gate lets one through, in ticks. Later waves are not merely bigger.
    pub fn lane_spawn_rate(self, wave: i32) -> i32 {
        match (self, wave) {
            (Self::One, 1) => 90,
            (Self::One, 2) => 60,
            (Self::One, 3) => 55,
            (Self::One, 4) => 50,
            (Self::One, 5) => 40,
            (Self::Two, 1) => 90,
            (Self::Two, 2) => 70,
            (Self::Two, 4) => 55,
            (Self::Two, 5) => 50,
            (Self::Two, 6) => 45,
            (Self::Two, 7) => 42,
            (Self::Three, 1) => 85,
            (Self::Three, 2) => 75,
            (Self::Three, 5) => 55,
            (Self::Three, 7) => 90,
            _ => 60,
        }
    }

    /// The one enemy whose death the last wave actually waits on.
    pub fn champion(self) -> u16 {
        match self {
            Self::One => ids::DD2_DARK_MAGE_T1,
            Self::Two => ids::DD2_OGRE_T2,
            Self::Three => ids::DD2_BETSY,
        }
    }

    /// What each kill is worth. Expert counts double, except on the wave that is waiting for the
    /// champion — there nothing but the champion counts at all.
    pub fn kill_worth(self, npc_type: u16, wave: i32, kills: i32, expert: bool) -> i32 {
        let last = wave == self.waves();
        let held = match self {
            Self::One => last && kills >= 139,
            Self::Two => last && kills >= 219,
            Self::Three => last,
        };
        if held {
            return i32::from(npc_type == self.champion() || npc_type == ids::DD2_DARK_MAGE_T3);
        }
        if !belongs(npc_type) {
            return 0;
        }
        // C7-08: the finishing kill of the FINAL wave counts one, not two, so an Expert double
        // cannot land exactly on the quota and complete the tier without the champion. Vanilla makes
        // this a single-point special case that fires only on the last wave at exactly `required-2`:
        // `waveNumber == 5 && waveKills == 138` (tier 1, `DD2Event.cs:1120`) and `waveNumber == 7 &&
        // waveKills == 218` (tier 2, `DD2Event.cs:1213`). Everywhere else an Expert kill is worth
        // two, so ordinary waves are not lengthened by a kill. The champion hold above `required-1`
        // is handled by `held` above (matching vanilla's `currentKillCount` clamp back to 139,
        // `DD2Event.cs:994-996`).
        let finishes_final_wave = last && kills == self.required_kills(wave) - 2;
        if !expert || finishes_final_wave {
            1
        } else {
            2
        }
    }
}

/// How many fallen goblins the field remembers at once.
const CORPSES_REMEMBERED: usize = 300;

/// What calling the next wave early cuts the wait down to, in ticks.
///
/// A second, not nothing. The arrival has an animation, and dropping the hold straight to zero
/// would put enemies through the gates before anybody saw them open.
const SKIP_TO: i32 = 60;

/// Whether killing this counts toward the wave at all.
pub fn belongs(npc_type: u16) -> bool {
    (551..=565).contains(&npc_type) || (568..=578).contains(&npc_type)
}

/// The event as the world sees it.
#[derive(Debug, Clone, Default)]
pub struct ArmyState {
    pub tier: Option<Tier>,
    pub wave: i32,
    pub kills: i32,
    /// Ticks left before the gates start letting enemies through again.
    pub hold: i32,
    /// Where the crystal stands, in tiles, so gates and arena can be placed around it.
    pub stand: (i32, i32),
    /// Whether the champion of the current tier has been beaten this run.
    pub champion_down: bool,
    /// Where goblins have died, for a Dark Mage to raise.
    ///
    /// Only the plain goblins leave anything worth raising — a javelinist or a drakin is gone for
    /// good — and a raising consumes the spots it uses, so the same corpse never comes back twice.
    pub corpses: Vec<(f32, f32)>,
}

impl ArmyState {
    pub fn ongoing(&self) -> bool {
        self.tier.is_some()
    }

    /// Nothing comes out of the gates while this is set: it is the gap between waves.
    pub fn spawning_on_hold(&self) -> bool {
        self.hold != 0
    }

    /// Start the event at a tier, around a crystal standing at these tile coordinates.
    pub fn start(&mut self, tier: Tier, stand: (i32, i32)) {
        self.tier = Some(tier);
        self.wave = 1;
        self.kills = 0;
        self.hold = 0;
        self.stand = stand;
        self.champion_down = false;
    }

    pub fn stop(&mut self) {
        *self = Self::default();
    }

    /// One tick of the countdown between waves.
    pub fn tick(&mut self) {
        if self.hold > 0 {
            self.hold -= 1;
        }
    }

    /// Call the next wave early, which is what the crystal's own button does.
    ///
    /// The gap between waves is generous on purpose — it is when you rebuild and re-arm — but a
    /// group that is ready should not have to stand about.
    ///
    /// It cuts the wait to a second rather than to nothing, which is the game's own figure and
    /// matters: the arrival has an animation, and dropping the hold straight to zero would put
    /// enemies through the gates before anyone saw them open. A request with less than a second
    /// left already is refused, so pressing it twice does not stack.
    ///
    /// Returns the new wait if there was one to skip, so the caller can tell clients.
    pub fn skip_wait(&mut self) -> Option<i32> {
        if !self.ongoing() || self.hold <= SKIP_TO {
            return None;
        }
        self.hold = SKIP_TO;
        Some(self.hold)
    }

    /// Record a kill. Returns the wave that just finished, if one did.
    pub fn note_kill(&mut self, npc_type: u16, expert: bool) -> Option<i32> {
        let tier = self.tier?;
        if npc_type == tier.champion() {
            self.champion_down = true;
            // Tier three's final wave is not a kill count at all — it is Betsy's health
            // (`Difficulty_3_GetRequiredWaveKills`: progress = 100 - life/lifeMax*100, reaching 100
            // as she dies). Every gate enemy there is worth 0 and Betsy herself only 1, against a
            // quota of 100, so her death could never complete the wave and the whole tier-3 event
            // was unwinnable. Her death is what ends it; mark the wave's quota met.
            if tier == Tier::Three && self.wave == tier.waves() {
                self.kills = tier.required_kills(self.wave);
            }
        }
        self.kills += tier.kill_worth(npc_type, self.wave, self.kills, expert);
        if self.kills < tier.required_kills(self.wave) {
            return None;
        }
        let finished = self.wave;
        self.kills = 0;
        self.wave += 1;
        // The gates go quiet for half a minute between waves, which is the only breathing room the
        // event ever gives you.
        self.hold = 1800;
        Some(finished)
    }

    /// Note where a goblin fell, if it was the kind that leaves anything behind.
    ///
    /// The oldest is forgotten once there are more than a wave's worth: a mage only ever raises
    /// eight at a time and only ones close by, so an unbounded list would be a slow leak across a
    /// long event and would never be read.
    pub fn note_corpse(&mut self, npc_type: u16, bottom: (f32, f32)) {
        if self.ongoing()
            && matches!(
                npc_type,
                ids::DD2_GOBLIN_T1 | ids::DD2_GOBLIN_T2 | ids::DD2_GOBLIN_T3
            )
        {
            if self.corpses.len() >= CORPSES_REMEMBERED {
                self.corpses.remove(0);
            }
            self.corpses.push(bottom);
        }
    }

    /// Whether there are enough corpses near a point to be worth a summoning.
    pub fn can_raise_at(&self, spot: (f32, f32)) -> bool {
        self.corpses
            .iter()
            .filter(|c| (c.0 - spot.0).hypot(c.1 - spot.1) <= RAISE_CHECK_RANGE)
            .count()
            >= RAISE_MINIMUM
    }

    /// Take the corpses near a point, up to the most a single summoning will raise.
    pub fn take_raisable(&mut self, spot: (f32, f32)) -> Vec<(f32, f32)> {
        let mut taken = Vec::new();
        self.corpses.retain(|c| {
            if (c.0 - spot.0).hypot(c.1 - spot.1) <= RAISE_RANGE {
                taken.push(*c);
                false
            } else {
                true
            }
        });
        taken.truncate(RAISE_MOST);
        taken
    }

    /// Whether the event has been won: the last wave of the tier is behind you.
    pub fn won(&self) -> bool {
        self.tier.is_some_and(|tier| self.wave > tier.waves())
    }
}

/// What a gate should let out this time, given the census of what is already on the field.
///
/// A `None` means the gate lets nothing through, because everything the wave wanted is already out
/// there. Two types can come out at once — a bomber alongside the goblin it walks with — which is
/// why this returns a list rather than one type.
pub fn from_gate(
    tier: Tier,
    wave: i32,
    left_gate: bool,
    kills: i32,
    count: &dyn Fn(u16) -> usize,
    players: usize,
    rng: &mut SmallRng,
) -> Vec<u16> {
    // Every cap grows by a third for each player past the first.
    let scale = |mut cap: usize| {
        for _ in 1..players.max(1) {
            cap = (cap as f64 * 1.3) as usize;
        }
        cap
    };
    let required = tier.required_kills(wave);
    let mut out = Vec::new();
    match tier {
        Tier::One => tier_one(
            wave, left_gate, kills, required, count, &scale, rng, &mut out,
        ),
        Tier::Two => tier_two(
            wave, left_gate, kills, required, count, &scale, rng, &mut out,
        ),
        Tier::Three => tier_three(wave, count, &scale, rng, &mut out),
    }
    out
}

type Cap<'a> = &'a dyn Fn(usize) -> usize;
type Census<'a> = &'a dyn Fn(u16) -> usize;

#[allow(clippy::too_many_arguments)]
fn tier_one(
    wave: i32,
    left_gate: bool,
    kills: i32,
    required: i32,
    count: Census<'_>,
    scale: Cap<'_>,
    rng: &mut SmallRng,
    out: &mut Vec<u16>,
) {
    let goblins = scale(50);
    let javelins = scale(match wave {
        5.. => 12,
        4 => 8,
        _ => 6,
    });
    let wyverns = scale(if wave > 4 { 8 } else { 6 });
    let footmen = count(ids::DD2_GOBLIN_T1) + count(ids::DD2_GOBLIN_BOMBER_T1);
    match wave {
        1 => {
            if footmen < goblins {
                out.push(ids::DD2_GOBLIN_T1);
            }
        }
        2 => {
            if footmen < goblins {
                out.push(if rng.random_range(0..7) != 0 {
                    ids::DD2_GOBLIN_T1
                } else {
                    ids::DD2_GOBLIN_BOMBER_T1
                });
            }
        }
        3 => {
            if rng.random_range(0..6) == 0 && count(ids::DD2_JAVELINST_T1) < javelins {
                out.push(ids::DD2_JAVELINST_T1);
            } else if footmen < goblins {
                out.push(if rng.random_range(0..5) != 0 {
                    ids::DD2_GOBLIN_T1
                } else {
                    ids::DD2_GOBLIN_BOMBER_T1
                });
            }
        }
        4 => {
            if rng.random_range(0..12) == 0 && count(ids::DD2_WYVERN_T1) < wyverns {
                out.push(ids::DD2_WYVERN_T1);
            } else if rng.random_range(0..5) == 0 && count(ids::DD2_JAVELINST_T1) < javelins {
                out.push(ids::DD2_JAVELINST_T1);
            } else if footmen < goblins {
                out.push(if rng.random_range(0..5) != 0 {
                    ids::DD2_GOBLIN_T1
                } else {
                    ids::DD2_GOBLIN_BOMBER_T1
                });
            }
        }
        _ => {
            // The Dark Mage comes out once the wave is half done, and only ever one of him.
            if (!left_gate || rng.random_range(0..2) == 0)
                && kills as f32 > required as f32 * 0.5
                && count(ids::DD2_DARK_MAGE_T1) == 0
            {
                out.push(ids::DD2_DARK_MAGE_T1);
            }
            if rng.random_range(0..10) == 0 && count(ids::DD2_WYVERN_T1) < wyverns {
                out.push(ids::DD2_WYVERN_T1);
            } else if rng.random_range(0..4) == 0 && count(ids::DD2_JAVELINST_T1) < javelins {
                out.push(ids::DD2_JAVELINST_T1);
            } else if footmen < goblins {
                out.push(if rng.random_range(0..4) != 0 {
                    ids::DD2_GOBLIN_T1
                } else {
                    ids::DD2_GOBLIN_BOMBER_T1
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tier_two(
    wave: i32,
    left_gate: bool,
    kills: i32,
    required: i32,
    count: Census<'_>,
    scale: Cap<'_>,
    rng: &mut SmallRng,
    out: &mut Vec<u16>,
) {
    let goblins = scale(50);
    let javelins = scale(match wave {
        6.. => 12,
        4 | 5 => 10,
        2 | 3 => 8,
        _ => 5,
    });
    let wyverns = if wave > 4 { 7 } else { 5 };
    let withers = 2;
    let kobolds = scale(if wave > 3 { 12 } else { 8 });
    let drakins = scale(if wave > 5 { 5 } else { 3 });
    let footmen = count(ids::DD2_GOBLIN_T2) + count(ids::DD2_GOBLIN_BOMBER_T2);
    // A pass of "roll for the rare thing, else fall through" — the order is the game's, and it is
    // what makes the later waves feel like they are stacking rather than swapping.
    let goblin_and_bomber =
        |out: &mut Vec<u16>, bomber_in: u32, invert: bool, rng: &mut SmallRng| {
            if footmen < goblins {
                let rolled = rng.random_range(0..bomber_in) == 0;
                if rolled != invert {
                    out.push(ids::DD2_GOBLIN_BOMBER_T2);
                }
                out.push(ids::DD2_GOBLIN_T2);
            }
        };
    match wave {
        1 => {
            if rng.random_range(0..20) == 0 && count(ids::DD2_JAVELINST_T2) < javelins {
                out.push(ids::DD2_JAVELINST_T2);
            } else if count(ids::DD2_GOBLIN_T2) < goblins {
                out.push(ids::DD2_GOBLIN_T2);
            }
        }
        2 => {
            if rng.random_range(0..3) == 0 && count(ids::DD2_KOBOLD_WALKER_T2) < kobolds {
                out.push(ids::DD2_KOBOLD_WALKER_T2);
            } else if rng.random_range(0..8) == 0 && count(ids::DD2_JAVELINST_T2) < javelins {
                out.push(ids::DD2_JAVELINST_T2);
            } else if count(ids::DD2_GOBLIN_T2) < goblins {
                out.push(ids::DD2_GOBLIN_T2);
            }
        }
        3 => {
            if rng.random_range(0..7) == 0 && count(ids::DD2_KOBOLD_WALKER_T2) < kobolds {
                out.push(ids::DD2_KOBOLD_WALKER_T2);
            } else if rng.random_range(0..10) == 0 && count(ids::DD2_WYVERN_T2) < wyverns {
                out.push(ids::DD2_WYVERN_T2);
            } else if rng.random_range(0..8) == 0 && count(ids::DD2_JAVELINST_T2) < javelins {
                out.push(ids::DD2_JAVELINST_T2);
            } else {
                goblin_and_bomber(out, 4, false, rng);
            }
        }
        4 => {
            if rng.random_range(0..10) == 0 && count(ids::DD2_DRAKIN_T2) < drakins {
                out.push(ids::DD2_DRAKIN_T2);
            } else if rng.random_range(0..12) == 0 && count(ids::DD2_WYVERN_T2) < wyverns {
                out.push(ids::DD2_WYVERN_T2);
            } else if rng.random_range(0..6) == 0 && count(ids::DD2_JAVELINST_T2) < javelins {
                out.push(ids::DD2_JAVELINST_T2);
            } else if rng.random_range(0..3) == 0 && count(ids::DD2_KOBOLD_WALKER_T2) < kobolds {
                out.push(ids::DD2_KOBOLD_WALKER_T2);
            } else if count(ids::DD2_GOBLIN_T2) < goblins {
                out.push(ids::DD2_GOBLIN_T2);
            }
        }
        5 => {
            if rng.random_range(0..7) == 0 && count(ids::DD2_DRAKIN_T2) < drakins {
                out.push(ids::DD2_DRAKIN_T2);
            } else if rng.random_range(0..10) == 0 && count(ids::DD2_WYVERN_T2) < wyverns {
                out.push(ids::DD2_WYVERN_T2);
            } else if rng.random_range(0..4) == 0
                && count(ids::DD2_KOBOLD_WALKER_T2) + count(ids::DD2_KOBOLD_FLYER_T2) < kobolds
            {
                out.push(if rng.random_range(0..2) != 0 {
                    ids::DD2_KOBOLD_FLYER_T2
                } else {
                    ids::DD2_KOBOLD_WALKER_T2
                });
            } else {
                goblin_and_bomber(out, 3, false, rng);
            }
        }
        6 => {
            if rng.random_range(0..7) == 0 && count(ids::DD2_DRAKIN_T2) < drakins {
                out.push(ids::DD2_DRAKIN_T2);
            } else if rng.random_range(0..17) == 0 && count(ids::DD2_WITHER_BEAST_T2) < withers {
                out.push(ids::DD2_WITHER_BEAST_T2);
            } else if rng.random_range(0..5) == 0
                && count(ids::DD2_KOBOLD_WALKER_T2) + count(ids::DD2_KOBOLD_FLYER_T2) < kobolds
            {
                out.push(if rng.random_range(0..2) == 0 {
                    ids::DD2_KOBOLD_FLYER_T2
                } else {
                    ids::DD2_KOBOLD_WALKER_T2
                });
            } else if rng.random_range(0..9) == 0 && count(ids::DD2_WYVERN_T2) < wyverns {
                out.push(ids::DD2_WYVERN_T2);
            } else if rng.random_range(0..3) == 0 && count(ids::DD2_JAVELINST_T2) < javelins {
                out.push(ids::DD2_JAVELINST_T2);
            } else {
                goblin_and_bomber(out, 3, true, rng);
            }
        }
        _ => {
            // The Ogre, once a tenth of the wave is down.
            if (!left_gate || rng.random_range(0..2) == 0)
                && kills as f32 > required as f32 * 0.1
                && count(ids::DD2_OGRE_T2) == 0
            {
                out.push(ids::DD2_OGRE_T2);
            } else if rng.random_range(0..7) == 0 && count(ids::DD2_DRAKIN_T2) < drakins {
                out.push(ids::DD2_DRAKIN_T2);
            } else if rng.random_range(0..17) == 0 && count(ids::DD2_WITHER_BEAST_T2) < withers {
                out.push(ids::DD2_WITHER_BEAST_T2);
            } else if rng.random_range(0..7) == 0
                && count(ids::DD2_KOBOLD_WALKER_T2) + count(ids::DD2_KOBOLD_FLYER_T2) < kobolds
            {
                out.push(if rng.random_range(0..3) == 0 {
                    ids::DD2_KOBOLD_FLYER_T2
                } else {
                    ids::DD2_KOBOLD_WALKER_T2
                });
            } else if rng.random_range(0..11) == 0 && count(ids::DD2_WYVERN_T2) < wyverns {
                out.push(ids::DD2_WYVERN_T2);
            } else {
                goblin_and_bomber(out, 2, false, rng);
            }
        }
    }
}

fn tier_three(
    wave: i32,
    count: Census<'_>,
    scale: Cap<'_>,
    rng: &mut SmallRng,
    out: &mut Vec<u16>,
) {
    let goblins = scale(60);
    let javelins = scale(match wave {
        6.. => 15,
        4 | 5 => 12,
        2 | 3 => 9,
        _ => 7,
    });
    let wyverns = if wave > 4 { 10 } else { 7 };
    let withers = if wave > 5 { 3 } else { 2 };
    let kobolds = scale(if wave > 3 { 18 } else { 12 });
    let drakins = scale(if wave > 5 { 6 } else { 4 });
    let bugs = scale(4);
    let footmen = count(ids::DD2_GOBLIN_T3) + count(ids::DD2_GOBLIN_BOMBER_T3);
    let goblin_and_bomber = |out: &mut Vec<u16>, bomber_in: u32, rng: &mut SmallRng| {
        if rng.random_range(0..bomber_in) == 0 {
            out.push(ids::DD2_GOBLIN_BOMBER_T3);
        }
        out.push(ids::DD2_GOBLIN_T3);
    };
    match wave {
        1 => {
            if rng.random_range(0..18) == 0 && count(ids::DD2_JAVELINST_T3) < javelins {
                out.push(ids::DD2_JAVELINST_T3);
            } else if count(ids::DD2_GOBLIN_T3) < goblins {
                goblin_and_bomber(out, 7, rng);
            }
        }
        2 => {
            if rng.random_range(0..3) == 0 && count(ids::DD2_LIGHTNING_BUG_T3) < bugs {
                out.push(ids::DD2_LIGHTNING_BUG_T3);
            } else if rng.random_range(0..7) == 0 && count(ids::DD2_JAVELINST_T3) < javelins {
                out.push(ids::DD2_JAVELINST_T3);
            } else if rng.random_range(0..3) == 0 && count(ids::DD2_KOBOLD_WALKER_T3) < kobolds {
                out.push(ids::DD2_KOBOLD_WALKER_T3);
            } else if count(ids::DD2_GOBLIN_T3) < goblins {
                goblin_and_bomber(out, 4, rng);
            }
        }
        3 => {
            if rng.random_range(0..13) == 0 && count(ids::DD2_DRAKIN_T3) < drakins {
                out.push(ids::DD2_DRAKIN_T3);
            } else if rng.random_range(0..7) == 0 && count(ids::DD2_KOBOLD_WALKER_T3) < kobolds {
                out.push(ids::DD2_KOBOLD_WALKER_T3);
            } else if rng.random_range(0..10) == 0 && count(ids::DD2_WYVERN_T3) < wyverns {
                out.push(ids::DD2_WYVERN_T3);
            } else if rng.random_range(0..8) == 0 && count(ids::DD2_JAVELINST_T3) < javelins {
                out.push(ids::DD2_JAVELINST_T3);
            } else if footmen < goblins {
                out.push(ids::DD2_GOBLIN_T3);
            }
        }
        4 => {
            if rng.random_range(0..24) == 0 && count(ids::DD2_DARK_MAGE_T3) == 0 {
                out.push(ids::DD2_DARK_MAGE_T3);
            } else if rng.random_range(0..12) == 0 && count(ids::DD2_DRAKIN_T3) < drakins {
                out.push(ids::DD2_DRAKIN_T3);
            } else if rng.random_range(0..15) == 0 && count(ids::DD2_WYVERN_T3) < wyverns {
                out.push(ids::DD2_WYVERN_T3);
            } else if rng.random_range(0..7) == 0 && count(ids::DD2_JAVELINST_T3) < javelins {
                out.push(ids::DD2_JAVELINST_T3);
            } else if rng.random_range(0..5) == 0
                && count(ids::DD2_KOBOLD_WALKER_T3) + count(ids::DD2_KOBOLD_FLYER_T3) < kobolds
            {
                out.push(if rng.random_range(0..3) == 0 {
                    ids::DD2_KOBOLD_FLYER_T3
                } else {
                    ids::DD2_KOBOLD_WALKER_T3
                });
            } else if count(ids::DD2_GOBLIN_T3) < goblins {
                out.push(ids::DD2_GOBLIN_T3);
            }
        }
        5 => {
            if rng.random_range(0..20) == 0 && count(ids::DD2_OGRE_T3) == 0 {
                out.push(ids::DD2_OGRE_T3);
            } else if rng.random_range(0..17) == 0 && count(ids::DD2_WITHER_BEAST_T3) < withers {
                out.push(ids::DD2_WITHER_BEAST_T3);
            } else if rng.random_range(0..8) == 0 && count(ids::DD2_DRAKIN_T3) < drakins {
                out.push(ids::DD2_DRAKIN_T3);
            } else if rng.random_range(0..7) == 0
                && count(ids::DD2_KOBOLD_WALKER_T3) + count(ids::DD2_KOBOLD_FLYER_T3) < kobolds
            {
                out.push(if rng.random_range(0..4) == 0 {
                    ids::DD2_KOBOLD_FLYER_T3
                } else {
                    ids::DD2_KOBOLD_WALKER_T3
                });
            } else if footmen < goblins {
                goblin_and_bomber(out, 3, rng);
            }
        }
        6 => {
            // Wave six is the one that rolls twice: a heavy from the first pass and something from
            // the second, which is why it feels like the wall it is.
            if rng.random_range(0..20) == 0 && count(ids::DD2_OGRE_T3) == 0 {
                out.push(ids::DD2_OGRE_T3);
            } else if rng.random_range(0..20) == 0 && count(ids::DD2_DARK_MAGE_T3) == 0 {
                out.push(ids::DD2_DARK_MAGE_T3);
            } else if rng.random_range(0..12) == 0 && count(ids::DD2_DRAKIN_T3) < drakins {
                out.push(ids::DD2_DRAKIN_T3);
            } else if rng.random_range(0..25) == 0 && count(ids::DD2_WITHER_BEAST_T3) < withers {
                out.push(ids::DD2_WITHER_BEAST_T3);
            }
            if rng.random_range(0..7) == 0 && count(ids::DD2_LIGHTNING_BUG_T3) < bugs {
                out.push(ids::DD2_LIGHTNING_BUG_T3);
            } else if rng.random_range(0..7) == 0
                && count(ids::DD2_KOBOLD_WALKER_T3) + count(ids::DD2_KOBOLD_FLYER_T3) < kobolds
            {
                out.push(if rng.random_range(0..3) == 0 {
                    ids::DD2_KOBOLD_FLYER_T3
                } else {
                    ids::DD2_KOBOLD_WALKER_T3
                });
            } else if rng.random_range(0..5) == 0 && count(ids::DD2_JAVELINST_T3) < javelins {
                out.push(ids::DD2_JAVELINST_T3);
            } else if footmen < goblins {
                goblin_and_bomber(out, 3, rng);
            }
        }
        _ => {
            // Wave seven is Betsy's, and she is not spawned from a gate — the portals only keep the
            // pressure on while you fight her.
            if rng.random_range(0..20) == 0 && count(ids::DD2_DRAKIN_T3) < drakins {
                out.push(ids::DD2_DRAKIN_T3);
            } else if rng.random_range(0..17) == 0 && count(ids::DD2_WITHER_BEAST_T3) < withers {
                out.push(ids::DD2_WITHER_BEAST_T3);
            } else if rng.random_range(0..10) == 0 && count(ids::DD2_JAVELINST_T3) < javelins {
                out.push(ids::DD2_JAVELINST_T3);
            } else if footmen < goblins {
                goblin_and_bomber(out, 5, rng);
            }
        }
    }
}

/// How far the arena walker will go each way from the crystal, in tiles.
///
/// It is a screen's width, which is why an arena bigger than a screen still only counts as one.
const WALKER_REACH: i32 = 1920 / 16;
/// The room a walker needs to pass: five tiles wide and ten tall.
const WALKER_HEIGHT: i32 = 10;

/// Where the arena's two ends are, found by walking the floor out from the crystal.
///
/// This is what decides where the gates go, and it is a real survey rather than a fixed width: the
/// walk stops at the first place a ten-tile-tall creature could not follow, so a platform ledge or
/// a low ceiling shortens the arena and brings the gates in. Building the arena is a real part of
/// preparing for the event, and this is the routine that judges it.
pub fn arena_ends(
    tiles: &impl crate::game::npc::TileView,
    from: (i32, i32),
) -> ((i32, i32), (i32, i32)) {
    let (_, floor) = expand_vertically(tiles, from.0, from.1, 0, 4);
    let mut left = walk(tiles, (from.0, floor), -1);
    let mut right = walk(tiles, (from.0, floor), 1);
    // Both ends are pulled one tile back in, so a gate never stands in the wall it stopped at.
    left.0 += 1;
    right.0 -= 1;
    (left, right)
}

/// How far up and down a column is open from a starting tile.
fn expand_vertically(
    tiles: &impl crate::game::npc::TileView,
    x: i32,
    y: i32,
    up: i32,
    down: i32,
) -> (i32, i32) {
    let solid = |y: i32| {
        let tile = tiles.tile(x, y);
        tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
    };
    let mut top = y;
    for _ in 0..up {
        if top < 10 || solid(top) {
            break;
        }
        top -= 1;
    }
    let mut bottom = y;
    for _ in 0..down {
        if solid(bottom) {
            break;
        }
        bottom += 1;
    }
    (top, bottom)
}

/// Walk the floor one way until something a walker could not pass.
fn walk(tiles: &impl crate::game::npc::TileView, start: (i32, i32), direction: i32) -> (i32, i32) {
    let solid = |x: i32, y: i32| {
        let tile = tiles.tile(x, y);
        tile.is_active() && terrustia_proto::tile_solid::solid(tile.block)
    };
    let mut at = (start.0, start.1 - 1);
    let mut last = at;
    for _ in 0..WALKER_REACH {
        // Step up out of anything it is standing in, but only a step or three.
        for _ in 0..3 {
            if !solid(at.0, at.1) {
                break;
            }
            at.1 -= 1;
        }
        let (top, bottom) = expand_vertically(tiles, at.0, at.1, WALKER_HEIGHT, 2);
        let (top, bottom) = (top + 1, bottom - 1);
        // Nothing underfoot: look a little further down for a floor, and stop if there is none.
        if !solid(at.0, bottom + 1) {
            let (_, floor) = expand_vertically(tiles, at.0, bottom, 0, 6);
            if !solid(at.0, floor) {
                break;
            }
        }
        // Not enough headroom to walk through.
        if bottom - top < WALKER_HEIGHT - 1 {
            break;
        }
        at = (at.0 + direction, bottom);
        last = at;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashSet;

    #[test]
    fn killing_betsy_wins_the_tier_three_finale() {
        let mut army = ArmyState::default();
        army.start(Tier::Three, (0, 0));
        army.wave = Tier::Three.waves(); // the final wave
        army.kills = 0;

        // Gate enemies on tier three's last wave are worth nothing — the wave is Betsy's health.
        assert_eq!(army.note_kill(ids::DD2_GOBLIN_T3, false), None);
        assert!(!army.won(), "gate kills do not advance the tier-3 finale");

        // Betsy's death is what ends it. Before the fix this added 1 toward a quota of 100 and the
        // event could never be won.
        let finished = army.note_kill(ids::DD2_BETSY, false);
        assert_eq!(finished, Some(7), "Betsy's death completes wave 7");
        assert!(army.won(), "the Old One's Army is won when Betsy falls");
    }

    fn empty(_: u16) -> usize {
        0
    }

    /// Every wave of every tier lets something out of an empty arena. A wave that could spawn
    /// nothing would stall the event forever.
    #[test]
    fn every_wave_has_something_to_send() {
        for tier in [Tier::One, Tier::Two, Tier::Three] {
            for wave in 1..=tier.waves() {
                let mut sent = false;
                for seed in 0..200u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    let out = from_gate(tier, wave, seed % 2 == 0, 0, &empty, 1, &mut rng);
                    sent |= !out.is_empty();
                }
                assert!(sent, "{tier:?} wave {wave} sends nothing");
            }
        }
    }

    /// Everything a gate sends belongs to the event, and is a type this build defines.
    #[test]
    fn gates_only_send_real_army_enemies() {
        for tier in [Tier::One, Tier::Two, Tier::Three] {
            for wave in 1..=tier.waves() {
                for seed in 0..300u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    for ty in from_gate(tier, wave, true, 999, &empty, 1, &mut rng) {
                        assert!(belongs(ty), "{tier:?} wave {wave} sent {ty}");
                        assert!(
                            terrustia_proto::npc_data::npc_stats(ty).is_some(),
                            "{ty} is not a type this build has"
                        );
                    }
                }
            }
        }
    }

    /// The champion only comes out once the wave is far enough along, and never twice.
    #[test]
    fn the_champion_waits_for_the_wave_to_be_half_done() {
        let mut early = HashSet::new();
        for seed in 0..300u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            early.extend(from_gate(Tier::One, 5, false, 10, &empty, 1, &mut rng));
        }
        assert!(
            !early.contains(&ids::DD2_DARK_MAGE_T1),
            "not at ten kills of a hundred and forty"
        );

        let mut late = HashSet::new();
        for seed in 0..300u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            late.extend(from_gate(Tier::One, 5, false, 100, &empty, 1, &mut rng));
        }
        assert!(late.contains(&ids::DD2_DARK_MAGE_T1), "by a hundred, yes");

        // ...and not while one is already standing.
        let standing = |ty: u16| usize::from(ty == ids::DD2_DARK_MAGE_T1);
        let mut again = HashSet::new();
        for seed in 0..300u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            again.extend(from_gate(Tier::One, 5, false, 100, &standing, 1, &mut rng));
        }
        assert!(!again.contains(&ids::DD2_DARK_MAGE_T1), "only ever one");
    }

    /// A full arena is a quiet one: with every cap met, the gates hold everything back.
    #[test]
    fn a_full_arena_stops_the_gates() {
        let packed = |_: u16| 500usize;
        for tier in [Tier::One, Tier::Two, Tier::Three] {
            for wave in 1..=tier.waves() {
                for seed in 0..100u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    let out = from_gate(tier, wave, true, 0, &packed, 1, &mut rng);
                    // Champions are capped by their own "none alive" check, which `packed` also
                    // satisfies, so nothing at all should come out.
                    assert!(
                        out.is_empty(),
                        "{tier:?} wave {wave} sent {out:?} into a full arena"
                    );
                }
            }
        }
    }

    /// More players means the caps rise, so a crowd that would have stalled one player does not.
    #[test]
    fn more_players_raises_the_caps() {
        let sixty = |ty: u16| usize::from(ty == ids::DD2_GOBLIN_T1) * 55;
        let solo: usize = (0..200u64)
            .map(|seed| {
                let mut rng = SmallRng::seed_from_u64(seed);
                from_gate(Tier::One, 1, true, 0, &sixty, 1, &mut rng).len()
            })
            .sum();
        let party: usize = (0..200u64)
            .map(|seed| {
                let mut rng = SmallRng::seed_from_u64(seed);
                from_gate(Tier::One, 1, true, 0, &sixty, 4, &mut rng).len()
            })
            .sum();
        assert_eq!(solo, 0, "fifty-five goblins is over a solo cap of fifty");
        assert!(party > 0, "four players should raise it past fifty-five");
    }

    /// Waves advance on kills, and the event is won when the last one is behind you.
    #[test]
    fn kills_advance_the_waves() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (100, 200));
        for wave in 1..=5 {
            assert_eq!(army.wave, wave);
            let quota = Tier::One.required_kills(wave);
            // The champion must die before the last wave will finish.
            for _ in 0..quota {
                army.note_kill(ids::DD2_GOBLIN_T1, false);
            }
            if wave == 5 {
                assert_eq!(army.wave, 5, "wave five waits for the Dark Mage");
                army.note_kill(ids::DD2_DARK_MAGE_T1, false);
            }
            army.tick();
        }
        assert!(army.won(), "five waves down is a win");
    }

    /// Killing something that is not part of the event does not advance it.
    #[test]
    fn a_passing_zombie_does_not_count() {
        let mut army = ArmyState::default();
        army.start(Tier::Two, (100, 200));
        for _ in 0..500 {
            army.note_kill(3, false);
        }
        assert_eq!(army.kills, 0);
        assert_eq!(army.wave, 1);
    }

    /// Expert kills count double all the way to the quota on an ordinary wave: the C7-08
    /// finishing-kill-counts-one rule fires only on the final wave, so an ordinary wave is not
    /// lengthened by a kill. Thirty doubles clear wave one exactly; the over-clamped guard that
    /// counted the finisher as one on every wave would have needed thirty-one.
    #[test]
    fn expert_counts_double_on_ordinary_waves() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        // Wave one asks for sixty. Twenty-nine doubles reach fifty-eight, none of them finishing.
        for _ in 0..29 {
            assert!(army.note_kill(ids::DD2_GOBLIN_T1, true).is_none());
        }
        assert_eq!(army.kills, 58, "twenty-nine kills at two apiece");
        // The thirtieth double lands exactly on sixty and finishes the wave. On an ordinary wave the
        // finisher keeps its full double, so it is the thirtieth kill that clears it, not a thirty-first.
        assert_eq!(
            army.note_kill(ids::DD2_GOBLIN_T1, true),
            Some(1),
            "the thirtieth double clears wave one"
        );
        assert_eq!(army.wave, 2, "and the event moves on to wave two");
        assert_eq!(army.kills, 0);
    }

    /// C7-08: on the final wave an Expert double must not land exactly on the quota and complete the
    /// wave without the champion. The count holds one short, at the champion gate, and only the
    /// champion's own death finishes it. On the pre-fix `<=` guard a regular kill from `required-2`
    /// jumped straight to `required` and won the tier without ever killing the Dark Mage.
    #[test]
    fn an_expert_double_cannot_skip_the_final_wave_champion() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        army.wave = Tier::One.waves(); // the final wave
        let required = Tier::One.required_kills(army.wave);

        let mut finished = None;
        for _ in 0..(required * 2) {
            if let Some(w) = army.note_kill(ids::DD2_GOBLIN_T1, true) {
                finished = Some(w);
                break;
            }
        }
        assert!(
            finished.is_none(),
            "regular kills alone never finish the final wave, however they are counted"
        );
        assert_eq!(
            army.kills,
            required - 1,
            "the count holds one short, waiting for the champion"
        );
        assert_eq!(
            army.note_kill(ids::DD2_DARK_MAGE_T1, true),
            Some(Tier::One.waves()),
            "and only the Dark Mage's death completes it"
        );
    }

    /// A flat floor with walls at both ends gives an arena those walls define.
    #[test]
    fn the_walker_finds_the_walls() {
        use terrustia_proto::tile::Tile;
        let mut world = std::collections::HashMap::new();
        // A flat floor from x=80 to x=140 at y=200, walled at both ends.
        for x in 80..=140 {
            for y in 200..210 {
                world.insert((x, y), Tile::block(1));
            }
        }
        for y in 180..200 {
            world.insert((80, y), Tile::block(1));
            world.insert((140, y), Tile::block(1));
        }
        struct Ground(std::collections::HashMap<(i32, i32), Tile>);
        impl crate::game::npc::TileView for Ground {
            fn tile(&self, x: i32, y: i32) -> Tile {
                self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
            }
        }
        let tiles = Ground(world);
        let (left, right) = arena_ends(&tiles, (110, 199));
        assert!(
            (81..=83).contains(&left.0),
            "the left end should stop at the left wall, not {left:?}"
        );
        assert!(
            (137..=139).contains(&right.0),
            "the right end should stop at the right wall, not {right:?}"
        );
        assert!(left.0 < right.0);
    }

    /// A hole in the floor stops the walk: an arena has to be somewhere a walker could cross.
    #[test]
    fn a_hole_shortens_the_arena() {
        use terrustia_proto::tile::Tile;
        let mut world = std::collections::HashMap::new();
        for x in 80..=140 {
            if (120..125).contains(&x) {
                continue;
            }
            for y in 200..210 {
                world.insert((x, y), Tile::block(1));
            }
        }
        struct Ground(std::collections::HashMap<(i32, i32), Tile>);
        impl crate::game::npc::TileView for Ground {
            fn tile(&self, x: i32, y: i32) -> Tile {
                self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
            }
        }
        let tiles = Ground(world);
        let (_, right) = arena_ends(&tiles, (110, 199));
        assert!(
            right.0 < 122,
            "the walk should have stopped at the hole, not reached {right:?}"
        );
    }

    /// A Dark Mage will not raise until there are three corpses to raise, and a raising uses
    /// them up rather than leaving them to be raised again.
    #[test]
    fn corpses_are_spent_when_they_are_raised() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        let spot = (1000.0, 1000.0);
        assert!(!army.can_raise_at(spot), "an empty field raises nothing");

        for i in 0..3 {
            army.note_corpse(ids::DD2_GOBLIN_T1, (1000.0 + i as f32 * 20.0, 1000.0));
        }
        assert!(army.can_raise_at(spot), "three is enough");

        let raised = army.take_raisable(spot);
        assert_eq!(raised.len(), 3);
        assert!(!army.can_raise_at(spot), "and they do not come back twice");
    }

    /// Only the plain goblins leave anything behind.
    #[test]
    fn a_javelinist_leaves_nothing_to_raise() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        for _ in 0..10 {
            army.note_corpse(ids::DD2_JAVELINST_T1, (1000.0, 1000.0));
            army.note_corpse(ids::DD2_WYVERN_T1, (1000.0, 1000.0));
        }
        assert!(!army.can_raise_at((1000.0, 1000.0)));
    }

    /// The field forgets the oldest corpses rather than remembering every one forever.
    #[test]
    fn the_field_only_remembers_so_many() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        for i in 0..(CORPSES_REMEMBERED * 3) {
            army.note_corpse(ids::DD2_GOBLIN_T1, (i as f32, 0.0));
        }
        assert_eq!(army.corpses.len(), CORPSES_REMEMBERED);
        // What it kept is the newest, which is what a mage standing in the fight would want.
        assert!(
            army.corpses[0].0 > CORPSES_REMEMBERED as f32,
            "it forgot the wrong end"
        );
    }

    /// A summoning raises at most eight at once, however many are lying about.
    #[test]
    fn a_summoning_raises_at_most_eight() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        for i in 0..30 {
            army.note_corpse(ids::DD2_GOBLIN_T1, (1000.0 + i as f32 * 10.0, 1000.0));
        }
        assert_eq!(army.take_raisable((1000.0, 1000.0)).len(), RAISE_MOST);
    }

    /// The gap between waves actually holds the gates shut.
    #[test]
    fn the_gates_go_quiet_between_waves() {
        let mut army = ArmyState::default();
        army.start(Tier::One, (0, 0));
        assert!(!army.spawning_on_hold(), "not before the first wave");
        for _ in 0..60 {
            army.note_kill(ids::DD2_GOBLIN_T1, false);
        }
        assert!(army.spawning_on_hold());
        for _ in 0..1800 {
            army.tick();
        }
        assert!(!army.spawning_on_hold(), "half a minute later, back on");
    }
}
