//! What a world has already been through.
//!
//! Terraria's world file records a long list of booleans about what has been killed, saved and
//! smashed. They are not trivia: they gate spawn pools, town NPC arrivals, shop stock, ore
//! generation and a good deal of enemy behaviour. A server that does not track them cannot behave
//! like the game even if every routine is perfect, because the routines ask.
//!
//! The order here is the order in the file, immediately after the crimson flag, because that is
//! the only way to read them back.

/// Everything the world remembers about its own history.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Eye of Cthulhu.
    pub downed_boss1: bool,
    /// Eater of Worlds or Brain of Cthulhu.
    pub downed_boss2: bool,
    /// Skeletron.
    pub downed_boss3: bool,
    pub downed_queen_bee: bool,
    /// The Destroyer.
    pub downed_mech1: bool,
    /// The Twins.
    pub downed_mech2: bool,
    /// Skeletron Prime.
    pub downed_mech3: bool,
    /// Any of the three, which is what unlocks the next tier rather than all of them.
    pub downed_mech_any: bool,
    pub downed_plantera: bool,
    pub downed_golem: bool,
    pub downed_king_slime: bool,
    pub saved_goblin: bool,
    pub saved_wizard: bool,
    pub saved_mechanic: bool,
    pub downed_goblins: bool,
    pub downed_clown: bool,
    pub downed_frost: bool,
    pub downed_pirates: bool,
    /// Whether a shadow orb or crimson heart has ever been broken.
    pub shadow_orb_smashed: bool,
    pub spawn_meteor: bool,
    /// How many have been broken, which is what decides when a meteor lands.
    pub shadow_orb_count: u8,
    /// How many demon altars have been smashed, which is what decides how much hardmode ore falls.
    pub altar_count: i32,
    /// The wall has fallen.
    pub hard_mode: bool,
    pub downed_fishron: bool,
    pub downed_martians: bool,
    /// The Lunatic Cultist, whose death tears the sky open.
    pub downed_ancient_cultist: bool,
    /// The Moon Lord, after which every lunar shield is half strength.
    pub downed_moon_lord: bool,
    pub downed_halloween_king: bool,
    pub downed_halloween_tree: bool,
    pub downed_christmas_ice_queen: bool,
    pub downed_christmas_santank: bool,
    pub downed_christmas_tree: bool,
    /// Which pillars have ever been beaten...
    pub downed_tower_solar: bool,
    pub downed_tower_vortex: bool,
    pub downed_tower_nebula: bool,
    pub downed_tower_stardust: bool,
    /// ...and which are standing right now, which is not the same question.
    pub tower_active_solar: bool,
    pub tower_active_vortex: bool,
    pub tower_active_nebula: bool,
    pub tower_active_stardust: bool,
    pub lunar_apocalypse_up: bool,
    pub saved_angler: bool,
    pub saved_stylist: bool,
    pub saved_tax_collector: bool,
    pub saved_golfer: bool,
    pub saved_bartender: bool,
}

impl Progress {
    /// Whether the world has reached the tier where a given mechanical boss's drops matter.
    ///
    /// The game asks this a lot, and always as "any of them", never "all".
    pub fn past_mechs(&self) -> bool {
        self.downed_mech_any
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    /// A fresh world has been through nothing, which is what makes `Default` the right starting
    /// point for a generated one.
    #[test]
    fn a_new_world_remembers_nothing() {
        let p = Progress::default();
        assert!(!p.hard_mode);
        assert!(!p.downed_boss1);
        assert_eq!(p.altar_count, 0);
        assert!(!p.past_mechs());
    }

    /// "Any mech" is its own flag rather than a derivation, because the game stores it that way
    /// and a world edited elsewhere can disagree with the three individual ones.
    #[test]
    fn any_mech_is_read_not_computed() {
        let mut p = Progress {
            downed_mech1: true,
            ..Progress::default()
        };
        assert!(!p.past_mechs(), "the file's own flag is what counts");
        p.downed_mech_any = true;
        assert!(p.past_mechs());
    }
}
