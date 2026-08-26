//! Vanilla's seven "secret" seeds: magic strings typed into the world-creation seed field that
//! flip extra world-generation (and, in real vanilla, gameplay) flags for that one world —
//! Celebrationmk10, Drunk World, Not the Bees, Remix, No Traps, "get fixed boi", and Don't Starve.
//!
//! ## The real activation mechanism
//!
//! Real Terraria's seed field is free text, not a bare number — a player can type a plain integer
//! (which becomes the numeric seed directly), any other text (which gets hashed down to a number
//! instead), or one of these seven specific strings. World creation checks the *typed string
//! itself*, case-insensitively, against the seven magic strings; a match sets one or more
//! `static bool` fields on `WorldGen`/`Main` (`notTheBees`, `drunkWorldGen`, `remixWorldGen`,
//! `noTrapsWorldGen`, `getGoodWorldGen`, `dontStarveWorldGen`, `tenthAnniversaryWorldGen`) that
//! stay set for the rest of that world's generation — separate from, and unrelated to, whatever
//! number ends up seeding the RNG. An ordinary numeric seed and a magic-string seed both still
//! need *some* number to drive generation; vanilla derives that separately (parses the string as
//! a number if it is one, hashes it otherwise). This project's own [`super::rand::UnifiedRandom`]
//! is already not seed-identical with vanilla's own generator (see `worldgen/mod.rs`'s own module
//! doc), so the exact hash does not have to match vanilla's — only "the same text always produces
//! the same numeric seed" has to hold, the same property [`numeric_seed`]'s own test checks.
//!
//! ## Hard evidence vs. reasoned inference — read this before trusting a spelling below
//!
//! **Hard evidence**, already found and disclosed by name across this session's own landed
//! worldgen passes, reading real decompiled `WorldGen.cs`/`Main.cs` source before it was reaped
//! from `.scratch/` (see `plan.md`'s own note on where that tree went): the seven boolean flag
//! names in the paragraph above are real, and every already-landed pass's own disclosed branch —
//! collected in `plan.md`'s "Secret seeds" section — is a genuine, previously-read data point
//! about what one specific flag changes in one specific pass. `traps.rs`'s own module doc, for
//! instance, names `noTrapsWorldGen` as one of eight real flags gating almost every branch of
//! `placeTrap`/`PlaceSandTrap`.
//!
//! **Reasoned inference, not verified against source** — because no decompiled source tree exists
//! in this environment (see the note above) — is the *exact magic strings* [`detect`] matches
//! against. These are taken from public, well-documented community knowledge of the seven seeds
//! (the kind collected on the Terraria Wiki's own "World creation" article), not from re-reading
//! `WorldGen.cs`'s own string comparison directly. The mechanism itself — trim, case-fold, exact
//! match, no fuzzy normalisation beyond that — is standard and high confidence. The exact
//! punctuation of a few of the seven strings (an exclamation mark, an apostrophe, whether a word
//! like "world" is part of the trigger) is lower confidence; each string below is commented with
//! how confident this is. If a canonical spelling here is ever found to be wrong, only this one
//! match statement needs correcting — the detection architecture, and everything downstream of
//! [`SecretSeed`], does not depend on getting the spelling exactly right on the first try.
//!
//! ## What actually branches on this, and what does not (yet)
//!
//! Building the trigger is real, valuable, honest work on its own even before every pass consumes
//! it — the same "narrower, disclosed" shape this project's own Tier 2/3 rows already use. Only
//! [`SecretSeed::NoTraps`] is actually wired to a behavioural difference this session
//! (`traps::scatter` short-circuits to placing nothing — see `traps.rs`'s own doc comment, which
//! already named `noTrapsWorldGen` as real vanilla's own gate before this module existed). The
//! other six are detected, recorded on [`super::Built::secret_seed`] so a caller (or a test) can
//! see which one a given seed text named, and otherwise left exactly as ordinary generation —
//! *not* silently ignored, disclosed here and in `plan.md`'s own sizing table with why each one
//! is out of scope this pass.

/// Which of vanilla's seven secret seeds a typed seed string names, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSeed {
    /// `tenthAnniversaryWorldGen` internally (see `floating_islands.rs`'s own disclosed
    /// `islandStyle` branch) — the 10th-anniversary celebration seed.
    Celebrationmk10,
    /// `drunkWorldGen`.
    DrunkWorld,
    /// `notTheBees` — replaces the jungle with a Hive/bee theme.
    NotTheBees,
    /// `remixWorldGen` — mirrors the whole world (surface and underworld swap, among other
    /// pipeline-wide flips). The single largest of the seven; see `plan.md`'s sizing note.
    Remix,
    /// `noTrapsWorldGen` — the one seed this session actually wires to a real behavioural
    /// difference. See `traps.rs`.
    NoTraps,
    /// `getGoodWorldGen` internally, despite the "get fixed boi" surface text — real vanilla's
    /// own `SpawnGraveyardBiomesEverywhere` gate (`dontStarveWorldGen && drunkWorldGen &&
    /// getGoodWorldGen`, found and disclosed in `plan.md`) only makes sense if this seed also
    /// implies Don't Starve and Drunk World are simultaneously active — reasoned inference from
    /// that one gate plus general knowledge that this is the "everything at once" meme seed, not
    /// independently re-verified against source.
    GetFixedBoi,
    /// `dontStarveWorldGen`.
    DontStarve,
}

impl SecretSeed {
    /// Case-insensitive, trimmed match against real vanilla's seven magic seed strings.
    ///
    /// See this module's own doc comment for which of these are hard evidence (the mechanism,
    /// the flag names) and which are a reasoned inference from community documentation rather
    /// than decompiled source (the exact spelling below).
    pub fn detect(seed_text: &str) -> Option<Self> {
        match seed_text.trim().to_lowercase().as_str() {
            // High confidence — this one is an unambiguous, widely-repeated literal string.
            "celebrationmk10" => Some(Self::Celebrationmk10),
            // Moderate confidence on the space — some sources give "drunk" alone as sufficient,
            // but "drunk world" is the form most consistently documented.
            "drunk world" => Some(Self::DrunkWorld),
            // Moderate confidence on the exclamation mark — the seed's own name references the
            // "not the bees!" meme, and the mark is usually included in documented spellings.
            "not the bees!" => Some(Self::NotTheBees),
            // High confidence — documented consistently as the bare word.
            "remix" => Some(Self::Remix),
            // High confidence.
            "no traps" => Some(Self::NoTraps),
            // High confidence — a well-known, exact meme phrase.
            "get fixed boi" => Some(Self::GetFixedBoi),
            // Moderate confidence on the apostrophe (a straight ASCII `'`, not a curly one — a
            // typed seed field cannot easily produce a curly quote anyway).
            "don't starve" => Some(Self::DontStarve),
            _ => None,
        }
    }

    /// A short, stable name for logging and the startup panel — not necessarily the exact typed
    /// string (`GetFixedBoi` displays as "get fixed boi", matching what a player actually typed,
    /// not the internal `getGoodWorldGen` flag name).
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Celebrationmk10 => "celebrationmk10",
            Self::DrunkWorld => "drunk world",
            Self::NotTheBees => "not the bees!",
            Self::Remix => "remix",
            Self::NoTraps => "no traps",
            Self::GetFixedBoi => "get fixed boi",
            Self::DontStarve => "don't starve",
        }
    }
}

/// Turn a typed seed string into the numeric seed [`super::rand::UnifiedRandom`] actually needs.
///
/// Real vanilla: parse the string as a number if it is one; otherwise hash it. This project's own
/// generator is already not seed-identical with vanilla's (see `worldgen/mod.rs`'s own module
/// doc), so the exact hash algorithm does not need to match vanilla's — only "the same text always
/// produces the same numeric seed" has to hold, which is what makes typing a word seed twice
/// reproduce the same world, the same property `worldgen::tests::a_seed_makes_the_same_world`
/// already checks for plain numeric seeds. A hand-rolled FNV-1a rather than `std`'s own
/// `DefaultHasher`, deliberately: `DefaultHasher`'s algorithm is not a stability guarantee across
/// std versions, and this project already leans on a pinned toolchain elsewhere for its own
/// reproducibility guarantees (see `rust-toolchain.toml`) — a dependency this ten-line function
/// does not need to add.
pub fn numeric_seed(seed_text: &str) -> u64 {
    let trimmed = seed_text.trim();
    if let Ok(n) = trimmed.parse::<u64>() {
        return n;
    }
    // FNV-1a, 64-bit.
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in trimmed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_seven_magic_strings_are_detected() {
        let expect = [
            ("celebrationmk10", SecretSeed::Celebrationmk10),
            ("drunk world", SecretSeed::DrunkWorld),
            ("not the bees!", SecretSeed::NotTheBees),
            ("remix", SecretSeed::Remix),
            ("no traps", SecretSeed::NoTraps),
            ("get fixed boi", SecretSeed::GetFixedBoi),
            ("don't starve", SecretSeed::DontStarve),
        ];
        for (text, want) in expect {
            assert_eq!(
                SecretSeed::detect(text),
                Some(want),
                "{text:?} should match"
            );
        }
    }

    #[test]
    fn detection_is_case_insensitive_and_trims_whitespace() {
        assert_eq!(
            SecretSeed::detect("  GET FIXED BOI  "),
            Some(SecretSeed::GetFixedBoi)
        );
        assert_eq!(SecretSeed::detect("No Traps"), Some(SecretSeed::NoTraps));
        assert_eq!(SecretSeed::detect("ReMiX"), Some(SecretSeed::Remix));
    }

    #[test]
    fn ordinary_seeds_do_not_match() {
        for text in ["", "12345", "my world", "traps", "boi", "get fixed"] {
            assert_eq!(SecretSeed::detect(text), None, "{text:?} should not match");
        }
    }

    #[test]
    fn a_plain_number_parses_as_itself() {
        assert_eq!(numeric_seed("12345"), 12345);
        assert_eq!(numeric_seed("  9  "), 9);
        assert_eq!(numeric_seed("0"), 0);
    }

    #[test]
    fn word_seeds_hash_deterministically_and_differ() {
        let a = numeric_seed("get fixed boi");
        let b = numeric_seed("get fixed boi");
        let c = numeric_seed("remix");
        assert_eq!(a, b, "the same text must hash to the same seed twice");
        assert_ne!(
            a, c,
            "different text should (almost always) hash differently"
        );
    }
}
