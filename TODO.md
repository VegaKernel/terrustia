# TODO

Work that is known and deferred, not hidden. Grouped by area. This is the single backlog; there is
no separate GAPS file.

## Integration pass (gameplay parity leftovers)

The wire, door, meteor and slime items already landed. These remain:

- **HC8: nebula headcrab buff.** A hit from the nebula headcrab should apply buff 163 to the player.
  Needs a player-buff channel on the AI `Effects`/`Outcome` and a consumer in `server.rs`.
- **HC9 / HC10: collision physics for two enemies.** Solar Sroller's multi-bounce and the Sand
  Shark's sand-swim, ported as `Collision_MoveSolarSroller` / `Collision_MoveSandshark` in `npc.rs`.
- **Drop gaps that need an AI-state condition.** Four boss/miniboss drops are gated on runtime NPC
  state the drop table has no way to read yet: Skeletron's RedHatSkeletron variant (items
  5624/5625/5626/5628/5737 when `ai[3] == 1`), Pumpking's weapon pool (1829/1831/1837/1845/1855),
  Mourning Wood (327), Mothron (477, item 1570). Needs a conditions field threaded into drop
  resolution.
- **L2: liquid destroys furniture.** `tick_liquids` should consume `Settled::drowned` and KillTile
  the tiles that actually die in that liquid. This needs the `tileLavaDeath` / `tileWaterDeath`
  classification (a per-tile table), so it pairs with the codegen work below. A partial table would
  destroy the wrong tiles, so it is left as a safe no-op until the table exists.
- **Trapdoor and tall-gate wiring.** `Fired::trapdoors` / `Fired::gates` are reported by the flood
  but not acted on. They need real `ShiftTrapdoor` / `ShiftTallGate` domain logic (a moving
  two/three-tile form), which is more than a wire-up.
- **Server MINORs.** NPC-buff broadcast scope, the summon combat books (-11/-17), a teleport guard
  on player controls, and the chest-open (packet 80) rigged-input check.
- **Persistence MINOR.** `wld.rs` should refuse a file whose section pointers are out of order
  rather than reading past them. Needs a corrupt-`.wld` fixture to test against.
- **BI8: slime facing.** A slime should re-target only during an active (flag3) hop, not on every
  hop. Small follow-on to the BI4 hop-rate fix.

## Codegen (finish moving the data generators off Python)

The Rust `terrustia-codegen` crate now generates `hurt_tiles` and `recipes`, both verified
byte-identical. The rest is deferred (no time to finish the full port now):

- **Port the remaining eight generators** into the codegen crate, one module each, each verified
  byte-identical against its committed `.rs`: `gen_drops`, `gen_projectiles`, `gen_banners`,
  `gen_buffs`, `gen_angler`, `gen_shimmer`, `gen_town_names`, `gen_travel_shop`. When all ten are
  ported, point `just regen` at `codegen all` and delete the last `tools/gen_*.py`.
- **Keep the three checker scripts in Python.** `check_drops.py`, `check_recipes.py` and
  `packet_audit.py` stay as Python: they only run in CI, never in the build or data path, and are
  genuinely useful there. Full Python removal is a longer-term goal, not this pass.
- **D1: unroll the loop-generated recipes.** `Recipe.SetupRecipes` builds families of recipes
  inside `for` loops (roughly 566 shimmer-decraft entries) that the regex extractor cannot see.
  Capturing them is a behavioural change that adds rows to `recipes.rs`, separate from the faithful
  port that is already done.

## Second audit wave

A full second pass over the whole codebase for bugs, performance, and idiomatic-Rust improvements,
then fixing what it finds. Not started.

## TUI and hosting

Landing under the current TUI/hosting work; anything not finished there moves here.

## Release

- **Tag v0.0.1.** The last step, once the above is in a state worth cutting a first release for.
  Deliberately `0.0.1`, since worldgen is visibly unfinished.
