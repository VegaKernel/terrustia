//! The townsfolk you find tied up, and who they become when you free them.
//!
//! Six of the twenty-odd residents do not move in on a condition — they are found somewhere in the
//! world, bound or asleep or webbed, and freed by talking to them. Without this half of the system
//! the arrival table in [`super::arrivals`] can never fire for them, however far the world
//! progresses, because the flag it reads is only ever set by the rescue.
//!
//! That gap had a name: the Mechanic sells the only wire in the game, so the entire wiring
//! system — ported carefully, documented at length — was unreachable.
//!
//! The flags themselves live on the world's [`crate::world::progress::Progress`] and are already
//! read back from the save, so a world rescued in Terraria arrives here with its residents
//! already earned.

use crate::world::progress::Progress;

/// One rescue: who is tied up, who they become, and what to say.
pub struct Rescue {
    pub bound: u16,
    pub freed: u16,
    pub announcement: &'static str,
}

/// Every bound NPC in the game, with the townsperson each becomes.
///
/// The Tax Collector is deliberately absent, and always will be: he is a Tortured Soul turned by
/// Purification Powder rather than freed by talking, which is a different mechanic in a different
/// place. It lives in `Server::tick_powders`, and it sets the same `saved_tax_collector` flag
/// through [`remember`] below, which is why 441 has an arm there and no `Rescue` here.
///
/// The three bound town slimes are absent for the same reason, and each for a *different* reason:
/// no two of them are freed the same way. The Old Slime (685) wants a right-click while holding
/// Purification Powder, which reaches the server as packet 140 rather than as a talk
/// (`Main.cs:43751-43775`, `NPC.TransformElderSlime` at `NPC.cs:19172-19193`); the Purple Slime
/// (686) is freed by being *killed* (`NPC.HitEffect`, `NPC.cs:82596-82627`); the Yellow Slime (687)
/// by a thrown cloud of the powder (`Projectile.cs:14806-14824`). All three set their flag through
/// [`remember`], the same as the Tax Collector, so nothing else here has to know the difference.
pub const RESCUES: &[Rescue] = &[
    Rescue {
        bound: 105,
        freed: 107,
        announcement: "The Goblin Tinkerer is free!",
    },
    Rescue {
        bound: 106,
        freed: 108,
        announcement: "The Wizard is free!",
    },
    Rescue {
        bound: 123,
        freed: 124,
        announcement: "The Mechanic is free!",
    },
    Rescue {
        bound: 354,
        freed: 353,
        announcement: "The Stylist is free!",
    },
    Rescue {
        bound: 376,
        freed: 369,
        announcement: "The Angler is awake!",
    },
    Rescue {
        bound: 579,
        freed: 550,
        announcement: "The Tavernkeep is back on his feet!",
    },
    // The bound Golfer (589) is found in the underground desert and becomes the Golfer (588) when
    // freed (`NPC.cs:1693-1697` spawns 589; `NPC.cs:19883-19885` transforms 589 into 588). He was
    // missing entirely, so a world generated here could never gain a Golfer.
    Rescue {
        bound: 589,
        freed: 588,
        announcement: "The Golfer is free!",
    },
];

/// Who this one becomes, if talking to them frees anybody.
pub fn rescue_for(npc_type: u16) -> Option<&'static Rescue> {
    RESCUES.iter().find(|r| r.bound == npc_type)
}

/// Record the rescue on the world's history, so the freed resident can move in later.
pub fn remember(progress: &mut Progress, freed: u16) {
    match freed {
        107 => progress.saved_goblin = true,
        108 => progress.saved_wizard = true,
        124 => progress.saved_mechanic = true,
        353 => progress.saved_stylist = true,
        369 => progress.saved_angler = true,
        550 => progress.saved_bartender = true,
        588 => progress.saved_golfer = true,
        441 => progress.saved_tax_collector = true,
        // The three town slimes, keyed on what they become rather than on what they were, exactly
        // like every arm above. Each flag is what shuts its bound form's spawn arm off for good.
        679 => progress.unlocked_slime_old = true,
        680 => progress.unlocked_slime_purple = true,
        683 => progress.unlocked_slime_yellow = true,
        _ => {}
    }
}

/// Whether this world still has somebody to find.
pub fn still_bound(progress: &Progress, bound: u16) -> bool {
    match bound {
        105 => !progress.saved_goblin,
        106 => !progress.saved_wizard,
        123 => !progress.saved_mechanic,
        354 => !progress.saved_stylist,
        376 => !progress.saved_angler,
        579 => !progress.saved_bartender,
        589 => !progress.saved_golfer,
        // `NPC.cs:2095`'s own `!unlockedSlimeOldSpawn`. The bound Old Slime is found in the same
        // caverns as the Goblin Tinkerer and the Wizard, so it goes through the same underground
        // find; the other two bound slimes are not found down here and have no arm.
        685 => !progress.unlocked_slime_old,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Freeing the Mechanic is what puts wire in the game.
    #[test]
    fn the_mechanic_can_be_freed() {
        let rescue = rescue_for(123).expect("a bound mechanic is somebody");
        assert_eq!(rescue.freed, 124);

        let mut progress = Progress::default();
        assert!(still_bound(&progress, 123));
        remember(&mut progress, rescue.freed);
        assert!(progress.saved_mechanic, "no flag means no wire, ever");
        assert!(!still_bound(&progress, 123), "and she is not found twice");
    }

    /// Every rescue turns somebody into somebody else, and sets a flag doing it.
    #[test]
    fn every_rescue_is_wired_up() {
        for rescue in RESCUES {
            assert_ne!(rescue.bound, rescue.freed);
            let mut progress = Progress::default();
            remember(&mut progress, rescue.freed);
            assert!(
                !still_bound(&progress, rescue.bound),
                "freeing {} left it still bound, so it would be rescued forever",
                rescue.freed,
            );
        }
    }

    /// An ordinary townsperson is not a rescue.
    #[test]
    fn a_guide_frees_nobody() {
        assert!(rescue_for(22).is_none());
    }

    /// The three bound town slimes each set their own unlock flag when freed, and none of them is
    /// a talk rescue.
    ///
    /// Fails before the fix, when no unlock flag existed at all: `remember` had no arm for any of
    /// the three, so freeing one changed nothing a world could remember and the caverns, the sky
    /// and the jungle went on offering a bound slime beside the resident already in a house.
    #[test]
    fn freeing_a_town_slime_is_remembered() {
        for (freed, read) in [
            (
                679u16,
                (|p: &Progress| p.unlocked_slime_old) as fn(&Progress) -> bool,
            ),
            (680, |p: &Progress| p.unlocked_slime_purple),
            (683, |p: &Progress| p.unlocked_slime_yellow),
        ] {
            let mut progress = Progress::default();
            assert!(!read(&progress), "{freed} starts out there to be found");
            remember(&mut progress, freed);
            assert!(read(&progress), "freeing {freed} must be remembered");
        }

        // None of the three is freed by talking, so none may be in the talk table: adding one
        // there would let a player free it by opening a chat window with no powder and no fight.
        for bound in [685u16, 686, 687] {
            assert!(
                rescue_for(bound).is_none(),
                "{bound} is not freed by talking"
            );
        }

        // The bound Old Slime is still the one of the three found by the underground bound path,
        // so it needs a `still_bound` answer; the other two are found by their own producers.
        let mut progress = Progress::default();
        assert!(still_bound(&progress, 685));
        remember(&mut progress, 679);
        assert!(!still_bound(&progress, 685), "and is not found twice");
    }

    /// The bound Golfer (589) was missing entirely, so no world generated here could ever gain a
    /// Golfer. He is found in the underground desert (`NPC.cs:1693-1697`) and becomes 588 when
    /// freed (`NPC.cs:19883-19885`). Fails before the fix, when 589 was in no rescue table at all.
    #[test]
    fn the_golfer_can_be_freed() {
        let rescue = rescue_for(589).expect("a bound golfer is somebody");
        assert_eq!(rescue.freed, 588);

        let mut progress = Progress::default();
        assert!(
            still_bound(&progress, 589),
            "he starts out there to be found"
        );
        remember(&mut progress, rescue.freed);
        assert!(progress.saved_golfer);
        assert!(!still_bound(&progress, 589), "and is not found twice");
    }
}
