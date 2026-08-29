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
- **B13: Empress of Light damage.** The Empress's damage values still need a full re-derivation from
  vanilla's seven `case` blocks. The boss-AI pass left them alone rather than doing the column swap
  the other bosses took, since a wrong swap here is worse than the current placeholder. A boss-parity
  gap, not a wire-up.

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

## Error handling

- **Clear every non-test `.unwrap()` / `.expect()` from the production paths.** The server should
  never take a caller-triggered or environment-triggered fault out as a panic when it could return
  or log an explained error instead. Sweep the crates for `.unwrap()`, `.expect()`, panicking
  indexing, and integer casts that truncate on hostile input, and replace each production one with
  real propagation and an operator-facing message. Test-only unwraps (the `update.rs` fixture
  server, unit tests) are fine and out of scope. The `net::listener::bind` mapping added for the
  `os error 28` port-exhaustion case is the pattern to follow: keep the error kind, add advice that
  says what to do about it.
- **Back off the accept loop on a persistent error.** `net::listener::run` logs and retries on an
  `accept()` failure with no delay, so a sticky error (descriptor exhaustion, a broken listener)
  turns into a hot loop that pegs a core while filling the log. A short, capped backoff between
  repeated failures fixes that without slowing the normal one-off case.
- **Handle out-of-space and other storage errors on the write paths.** A full disk (ENOSPC), a
  read-only filesystem, or a vanished directory can hit any place the server writes: the world save
  and autosave, the rotating backups, the admin/account store, and the config the setup wizard
  writes. Today those surface as a bare OS error or, worse, risk a partial or truncated `.wld`. Each
  writer should fail with an explained, operator-facing message (the way `net::listener::bind` now
  does for `os error 28`), never lose the last good save to a half-written file (write to a temp
  path and rename into place), and keep the server running where the failure is recoverable (an
  autosave that could not write should warn and retry, not take the process down). Pairs with the
  `.unwrap()` sweep above.

## TUI and hosting

The wrap-corruption bug, Ctrl-D, the flat boot, the status footer, the worlds/ directory and the
`--headless` flag all landed. Two lower-impact polish items from the TUI audit remain:

- **Hanging indent for wrapped log lines.** A long operational log line wraps back to column 0,
  misaligned from where its message started (around column 38). Padding continuation lines to the
  message column would make a wrapped line read as intentional. Needs manual wrapping at the terminal
  width rather than relying on the terminal's own wrap.
- **Narrow-terminal awareness.** Nothing consults the terminal width when laying out the boot block,
  so in a terminal narrower than the content the info lines wrap mid-value. Low priority now that the
  boxes are gone, but a documented minimum width or a narrower fallback layout would be tidy.

## Docs

- **De-slop the remaining docs.** The em-dash and AI-slop cleanup so far covered `README.md` only.
  `AUDIT.md`, `docs/*.md` and `plan.md` still carry em-dashes and the same tells (aphoristic reveals,
  rule-of-three lists, "not X, it's Y"). The house style is now plain prose everywhere, so the rest
  of the docs should get the same pass. De-em-dashing code comments across the whole codebase is a
  much larger, lower-priority sweep, optional rather than committed to here.

## Release

- **Tag v0.0.1.** The last step, once the above is in a state worth cutting a first release for.
  Deliberately `0.0.1`, since worldgen is visibly unfinished.
