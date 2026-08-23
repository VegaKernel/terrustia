//! What each statue does when a circuit reaches it.
//!
//! A statue is a two-by-three tile whose style is spread across its frame: the column group picks
//! one of fifty-five, and the row group adds another fifty-five per band. Most of them spawn an
//! enemy or a critter; a few drop an item; two of them fetch a townsperson instead.
//!
//! The table is here rather than in the wiring module because it is per-type data, not an
//! algorithm — the same reason the NPC and tile tables live in this crate.

/// What one statue produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statue {
    /// Spawn one of these NPC types, chosen at random.
    ///
    /// `offset` is in pixels from the statue's spawn point, which is the middle of its base.
    Npc {
        types: &'static [u16],
        offset: (i32, i32),
        /// Whether it needs clear ground around it, because what it spawns is large.
        needs_room: bool,
    },
    /// Drop an item, at a much longer interval than a monster statue runs at.
    Item { item: i32, offset_y: i32 },
    /// Fetch a townsperson from wherever they are. The king and queen statues.
    Lure { types: &'static [u16] },
    /// Turn into another tile. The lihzahrd statue, once, when its power is cut.
    Becomes { block: u16 },
}

impl Statue {
    /// How long this statue waits before it will fire again.
    ///
    /// A monster statue is a third of a second, which is what makes a statue farm work at all; an
    /// item statue is ten seconds, and the two that fetch people are five.
    pub fn cooldown(&self) -> i32 {
        match self {
            Statue::Npc { .. } => 30,
            Statue::Item { .. } => 600,
            Statue::Lure { .. } => 300,
            Statue::Becomes { .. } => 30,
        }
    }
}

/// The townsfolk a king statue will fetch.
const KINGS: &[u16] = &[
    17, 19, 22, 38, 54, 107, 108, 142, 160, 207, 209, 227, 228, 229, 368, 369, 550, 441, 588,
];
/// ...and a queen statue.
const QUEENS: &[u16] = &[18, 20, 124, 178, 208, 353, 633, 663];

/// What the statue of this style does, or `None` for one that is only decoration.
pub fn statue(style: i32) -> Option<Statue> {
    // The straightforward ones: a single NPC at the statue's own base.
    let plain = |types: &'static [u16]| {
        Some(Statue::Npc {
            types,
            offset: (0, 0),
            needs_room: false,
        })
    };
    // The ones that spawn a tile above their base, which is most of the enemies.
    let raised = |types: &'static [u16]| {
        Some(Statue::Npc {
            types,
            offset: (0, -12),
            needs_room: false,
        })
    };

    match style {
        // Item statues.
        2 => Some(Statue::Item {
            item: 184,
            offset_y: -16,
        }),
        17 => Some(Statue::Item {
            item: 166,
            offset_y: -20,
        }),
        37 => Some(Statue::Item {
            item: 58,
            offset_y: -16,
        }),

        // The statues that fetch people.
        40 => Some(Statue::Lure { types: KINGS }),
        41 => Some(Statue::Lure { types: QUEENS }),

        // The lihzahrd statue, which is a trap of its own kind: it becomes a different tile.
        34 => Some(Statue::Becomes { block: 349 }),

        // Critters and enemies that stand on the base itself.
        10 => plain(&[21]),
        30 => plain(&[6]),
        35 => plain(&[2]),
        5 => plain(&[73]),
        13 => plain(&[24]),
        27 => Some(Statue::Npc {
            types: &[85],
            offset: (-9, 0),
            needs_room: false,
        }),
        7 => Some(Statue::Npc {
            types: &[49],
            offset: (-4, -6),
            needs_room: false,
        }),

        // Enemies that appear a tile above the base.
        4 => raised(&[1]),
        8 => raised(&[55]),
        9 => raised(&[46]),
        18 => raised(&[67]),
        23 => raised(&[63]),
        28 => raised(&[74, 297, 298]),
        42 => raised(&[58]),

        // The two that need clear ground, because what they spawn is wide.
        16 => Some(Statue::Npc {
            types: &[42],
            offset: (0, -12),
            needs_room: true,
        }),
        50 => Some(Statue::Npc {
            types: &[65],
            offset: (0, -12),
            needs_room: true,
        }),
        64 => Some(Statue::Npc {
            types: &[86],
            offset: (0, 0),
            needs_room: true,
        }),
        71 => Some(Statue::Npc {
            types: &[170, 180, 171],
            offset: (0, 0),
            needs_room: true,
        }),

        // The critters, which are all in the upper band of styles.
        51 => plain(&[299, 538]),
        52 => plain(&[356]),
        53 => plain(&[357]),
        54 => plain(&[355, 358]),
        55 => plain(&[367, 366]),
        // A gold bird is one in five, which is what makes the statue worth wiring.
        56 => plain(&[359, 359, 359, 359, 360]),
        57 => plain(&[377]),
        58 => plain(&[300]),
        59 => plain(&[364, 362]),
        60 => plain(&[148]),
        61 => plain(&[361]),
        62 => plain(&[487, 486, 485]),
        63 => plain(&[164]),
        65 => plain(&[490]),
        66 => plain(&[82]),
        67 => plain(&[449]),
        68 => plain(&[167]),
        69 => plain(&[480]),
        70 => plain(&[48]),
        72 => plain(&[481]),
        73 => plain(&[482]),
        74 => plain(&[430]),
        75 => plain(&[489]),
        76 => plain(&[611]),
        77 => plain(&[602]),
        78 => plain(&[595, 596, 599, 597, 600, 598]),
        79 => plain(&[616, 617]),
        80 => plain(&[671, 672]),
        81 => plain(&[673]),
        82 => plain(&[674, 675]),
        _ => None,
    }
}

/// Which style a statue tile's frame names.
///
/// The frame carries both which statue it is and which of its six tiles this one is, so the
/// anchor has to be worked out before the style can be.
pub fn style_at(frame_x: i16, frame_y: i16) -> (i32, (i32, i32)) {
    let within = (
        i32::from(frame_x).rem_euclid(36) / 18,
        i32::from(frame_y).rem_euclid(54) / 18,
    );
    let band = (i32::from(frame_y) / 54).rem_euclid(3);
    (i32::from(frame_x) / 36 + band * 55, within)
}

/// Whether two NPC types count as the same thing for the spawn limit.
///
/// The game groups a handful of families so that, for instance, a bird statue cannot be worked
/// round by the gold bird it sometimes produces.
pub fn same_family(a: u16, b: u16) -> bool {
    if a == b {
        return true;
    }
    const FAMILIES: &[&[u16]] = &[
        &[74, 297, 298],
        &[46, 540, 303, 337],
        &[362, 363, 364, 365],
        &[602, 603],
        &[608, 609],
        &[616, 617],
        &[55, 230],
    ];
    FAMILIES
        .iter()
        .any(|family| family.contains(&a) && family.contains(&b))
}

/// Whether another statue spawn is welcome, given how far away the ones already out are.
///
/// Three within two hundred pixels, six within six hundred, or ten anywhere is the ceiling. It is
/// what stops a statue wired to a one-second timer filling the world.
pub fn spawn_allowed(distances: impl Iterator<Item = f32>) -> bool {
    let (mut all, mut near, mut middling) = (0, 0, 0);
    for d in distances {
        all += 1;
        if d < 200.0 {
            near += 1;
        }
        if d < 600.0 {
            middling += 1;
        }
    }
    near < 3 && middling < 6 && all < 10
}

/// The same ceiling for items, which is measured over a wider circle.
pub fn item_spawn_allowed(distances: impl Iterator<Item = f32>) -> bool {
    let (mut all, mut near, mut middling) = (0, 0, 0);
    for d in distances {
        all += 1;
        if d < 300.0 {
            near += 1;
        }
        if d < 800.0 {
            middling += 1;
        }
    }
    near < 3 && middling < 6 && all < 10
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame names both the statue and which of its six tiles this one is.
    #[test]
    fn a_frame_names_a_style_and_a_corner() {
        // The top-left tile of the first statue.
        assert_eq!(style_at(0, 0), (0, (0, 0)));
        // Its bottom-right tile: same style, a different corner.
        assert_eq!(style_at(18, 36), (0, (1, 2)));
        // The second statue in the first band.
        assert_eq!(style_at(36, 0), (1, (0, 0)));
        // The first statue of the second band, which is fifty-five along.
        assert_eq!(style_at(0, 54), (55, (0, 0)));
        assert_eq!(style_at(72, 108), (112, (0, 0)));
    }

    /// A bird statue lays a gold bird one time in five, which is the only reason to wire one.
    #[test]
    fn the_bird_statue_sometimes_lays_a_gold_one() {
        let Some(Statue::Npc { types, .. }) = statue(56) else {
            panic!("style 56 is the bird statue");
        };
        assert_eq!(types.len(), 5);
        assert_eq!(types.iter().filter(|&&t| t == 360).count(), 1);
        assert_eq!(types.iter().filter(|&&t| t == 359).count(), 4);
    }

    /// The three kinds of statue run at three different rates: a monster statue is a farm, an
    /// item statue is a trickle.
    #[test]
    fn each_kind_of_statue_has_its_own_pace() {
        assert_eq!(statue(4).unwrap().cooldown(), 30, "a slime statue");
        assert_eq!(statue(37).unwrap().cooldown(), 600, "a heart statue");
        assert_eq!(statue(40).unwrap().cooldown(), 300, "the king statue");
    }

    /// The spawn limit bites on crowding, not on the total alone.
    #[test]
    fn the_spawn_limit_counts_by_distance() {
        assert!(spawn_allowed(std::iter::empty()));
        assert!(
            spawn_allowed([100.0, 100.0].into_iter()),
            "two nearby is fine"
        );
        assert!(
            !spawn_allowed([100.0, 100.0, 100.0].into_iter()),
            "three is not"
        );
        assert!(
            spawn_allowed([5000.0; 9].into_iter()),
            "nine spread across the world is fine"
        );
        assert!(
            !spawn_allowed([5000.0; 10].into_iter()),
            "ten is never fine"
        );
    }

    /// A bird statue cannot be worked round by the gold bird it sometimes produces.
    #[test]
    fn families_count_as_one_for_the_limit() {
        assert!(same_family(74, 297));
        assert!(same_family(362, 365));
        assert!(same_family(55, 230));
        assert!(!same_family(1, 2));
    }
}
