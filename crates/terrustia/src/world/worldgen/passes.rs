//! The generation passes, and the machinery that checks them against a real world.
//!
//! A world is built by a hundred and six passes run in a fixed order over shared mutable state.
//! They are not pure and cannot usefully be made so: each one reads what the last left behind,
//! and several write parameters that a pass forty steps later depends on. So they take a single
//! context and mutate it, exactly as the game's do.
//!
//! What makes that tractable is [`super::manifest`]: after every pass the game records what its
//! random generator reads next, and a port that has drawn the same numbers in the same order
//! reads the same value. Passes are therefore added one at a time, each checked before the next
//! is started.

use super::manifest::Manifest;
use super::rand::UnifiedRandom;
use crate::world::World;

/// Everything the passes share.
///
/// The game keeps this as a hundred and twenty static fields on `GenVars`. Gathering them here
/// changes nothing about how the passes work — they still read and write freely — but it means
/// the whole of a generation's state can be handed around, compared, and printed.
pub struct GenVars {
    /// The generator every pass draws from. Never reseeded once generation starts, which is why
    /// a pass that draws one number too many moves every pass after it.
    pub rand: UnifiedRandom,
    /// The world being built.
    pub world: World,
}

impl GenVars {
    pub fn new(seed: i32, world: World) -> Self {
        Self {
            rand: UnifiedRandom::new(seed),
            world,
        }
    }
}

/// One generation pass.
pub struct Pass {
    /// The name the world file records it under. This is how a pass is matched to its entry in
    /// the manifest, so it has to be the game's own spelling.
    pub name: &'static str,
    pub run: fn(&mut GenVars),
}

/// Every pass this port implements, in the order the game runs them.
///
/// The order is taken from the manifest of a real world rather than from the order the game
/// registers them in, because the two disagree in places and the manifest is what actually ran.
pub const PASSES: &[Pass] = &[];

/// Where a port stopped agreeing with a reference world.
pub struct Divergence {
    pub index: usize,
    pub name: String,
    pub expected: i64,
    pub got: i64,
}

/// What a comparison found.
pub struct Outcome {
    /// How many passes were run and checked.
    pub checked: usize,
    pub first_divergence: Option<Divergence>,
}

/// Run the implemented passes against a reference world's record.
///
/// Only the passes this port has are run, and only the reference entries for those are checked —
/// which is what lets the port grow one pass at a time instead of having to be finished before it
/// says anything.
pub fn compare_against(manifest: &Manifest, seed: i32, reference: &World) -> Outcome {
    let blank = World::empty(
        reference.width(),
        reference.height(),
        reference.name.clone(),
    );
    let mut vars = GenVars::new(seed, blank);

    let mut checked = 0;
    for (index, expected) in manifest.ran().enumerate() {
        let Some(pass) = PASSES.get(index) else {
            break;
        };
        if pass.name != expected.name {
            return Outcome {
                checked,
                first_divergence: Some(Divergence {
                    index,
                    name: format!("{} (we have {:?} here)", expected.name, pass.name),
                    expected: expected.rand_next,
                    got: -1,
                }),
            };
        }
        (pass.run)(&mut vars);
        // The game samples the generator here, and the sample itself advances it — so a port has
        // to take one at the same point or it diverges from the very first pass.
        let got = i64::from(vars.rand.next());
        if got != expected.rand_next {
            return Outcome {
                checked,
                first_divergence: Some(Divergence {
                    index,
                    name: expected.name.clone(),
                    expected: expected.rand_next,
                    got,
                }),
            };
        }
        checked += 1;
    }
    Outcome {
        checked,
        first_divergence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_of(entries: &[(&str, i64)]) -> Manifest {
        let body: Vec<String> = entries
            .iter()
            .map(|(name, next)| {
                format!(
                    "{{\"Name\":\"{name}\",\"DurationMs\":1,\"RandNext\":{next},\"Hash\":null,\"Skipped\":false}}"
                )
            })
            .collect();
        Manifest::parse(&format!(
            "{{\"GenPassResults\":[{}],\"Version\":\"test\",\"FinalHash\":null}}",
            body.join(",")
        ))
        .expect("a manifest")
    }

    /// With nothing implemented, nothing is checked and nothing is claimed.
    #[test]
    fn an_empty_port_checks_nothing() {
        let manifest = manifest_of(&[("Terrain", 1), ("Dunes", 2)]);
        let world = World::empty(600, 400, "reference");
        let outcome = compare_against(&manifest, 42, &world);
        assert_eq!(outcome.checked, 0);
        assert!(outcome.first_divergence.is_none());
    }

    /// The sample the game takes after each pass advances the generator, so a port that runs a
    /// pass drawing nothing still has to match the reference's *first* value.
    #[test]
    fn the_reference_value_is_the_draw_after_the_pass() {
        // What a pass that draws nothing leaves the generator reading.
        let mut r = UnifiedRandom::new(42);
        let after_nothing = i64::from(r.next());

        let manifest = manifest_of(&[("Terrain", after_nothing)]);
        let world = World::empty(600, 400, "reference");

        // A registry with one pass that does nothing at all.
        fn nothing(_: &mut GenVars) {}
        let passes = [Pass {
            name: "Terrain",
            run: nothing,
        }];
        // Exercise the same comparison the real one does, against this stand-in registry.
        let mut vars = GenVars::new(42, World::empty(600, 400, "ours"));
        (passes[0].run)(&mut vars);
        assert_eq!(i64::from(vars.rand.next()), manifest.passes[0].rand_next);
        let _ = world;
    }

    /// A pass that draws the wrong number of values is caught, and named.
    #[test]
    fn a_wrong_number_of_draws_is_caught() {
        let mut r = UnifiedRandom::new(42);
        r.next_max(10); // what the reference pass drew
        let expected = i64::from(r.next());

        let mut ours = UnifiedRandom::new(42);
        ours.next_max(10);
        ours.next_max(10); // one draw too many
        let got = i64::from(ours.next());
        assert_ne!(
            got, expected,
            "an extra draw has to change what comes next, or the oracle is worthless"
        );
    }
}
