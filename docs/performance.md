# Performance

A tick has **16,666 µs**. Everything here is measured against that number; anything approaching
it is a bug rather than a tuning problem.

## Measuring

```sh
cargo run --release -p terrustia --example savecost -- world.wld    # what a save costs
cargo run --release -p terrustia --example stress   -- 127.0.0.1:7777
cargo run --release -p terrustia --example crowd    -- 127.0.0.1:7777
cargo run --release -p terrustia --example memreport
cargo run --release -p terrustia --example profile_ai
```

The server reports its own worst tick every ten seconds at `debug`, and warns at `info` when a
tick uses more than half its budget:

```
WARN ticks are using a lot of their budget worst_us=70905 budget_us=16666 phase=world ...
```

The phase in that line is the point. "A tick took 71 ms" is a mystery; "the world phase took
71 ms with no NPCs in it" is a bug report.

## Where the tick goes

Phases, in order: `World`, `Sections`, `Items`, `Npcs`, `Projectiles`, `Damage`, `Spawning`,
`Housing`, `Sync`.

With 24 players and a busy world, the worst tick sits around **350–520 µs** — about 3% of budget.
The roster is not the problem and has not been.

## Large world, 16 and 255 players

The measurement above is on this project's usual 4200×1200 world — every prior number on this page
is. To find an actual ceiling rather than re-measure the same size again, this ran against a real
8400×2400 world instead (vanilla Terraria's own "Large" preset, 20.16M tiles — this codebase has no
named `LARGE_WIDTH`/`HEIGHT` constant, only `SMALL_WIDTH`/`HEIGHT`, so this is the first time it has
generated one): 28.171 s to generate, `--release`, seed 999. `examples/crowd` against it, real
connections, `autosave_secs = 0` so the number reflects player count rather than the separately-
documented autosave cost below:

| Players | Worst tick (cpu_us) | % of budget | Dropped connections |
|---:|---:|---:|---:|
| 16 | 363–658 µs | 3.9% | 0 |
| 255 (the real protocol max) | 410–2,219 µs | 13.3% | 88 of 255 (34.5%) |

Tick cost passes cleanly even at the real 255-player ceiling, with better than 7× headroom.

The 88 drops are a different ceiling, not a tick-budget one: every one of them landed within the
first several seconds of the run, immediately after all 255 real connections completed their
handshake in under a second (`crowd.rs`'s own unmodified sequential join loop, against localhost's
near-zero latency) — a synchronized mass-join burst, not steady gameplay. `net/connection.rs`'s
outbound queue (`8,192 + 256 × max_players` = 73,472 frames at `max_players = 255`) is sized against
*one* newcomer's burst joining an already-settled population (see "outbound queue" below) — not
against 255 newcomers each triggering a presence-and-inventory relay on every other already-
connected client within the same second, which compounds far faster. Left for whoever next owns
`net/connection.rs` — this page exists so the reproduction (255 simultaneous joins, `autosave_secs
= 0`, watch for `outbound queue full`) does not have to be rediscovered.

## The autosave stall, and how it was found

The crowd test reported this:

```
worst_us=70905 budget_us=16666 phase=world npcs=0 projectiles=0
```

Seventy-one milliseconds, in the world phase, with **nothing in the world**. And exactly every
five minutes. That is the autosave, and it was running on the game task.

Every autosave dropped three or four ticks. Players see that as a stutter, and it gets worse the
larger the world.

### What it actually cost

`examples/savecost.rs` exists to answer that, because the first guess was wrong:

```
world       4200x1200, 5040000 tiles

  reading every tile and nothing else       8254 µs
  ...plus finding the runs                 30924 µs
  ...plus encoding them (the whole job)    54667 µs

serialise      54667 µs
write            906 µs
```

Two things fall out of this, and both are worth keeping:

**The write is nothing.** 906 µs of 55,573. Moving only the disk write off the thread — the
obvious first move — would have bought 1.5%.

**Reading the tiles is nothing either, and that is surprising.** The encoder walks *column-major
through a row-major array*, striding 50 KB per step on a large world. That looks exactly like a
cache disaster, so the first fix was to transpose bands of columns into a scratch buffer before
encoding them.

It changed nothing measurable — 54.8 ms against 55.6. Reading all five million tiles costs 8 ms,
1.6 ns each; the hardware prefetcher handles a constant stride perfectly well. The transpose was
reverted and the reason recorded in `write_tiles`, so the next person does not spend the same
afternoon on it.

The cost is split roughly evenly between spotting runs (23 ms) and encoding them (24 ms).

### The fix, and how far it got

Even a threefold faster encoder would still bust the budget, and a bigger world would bust it
again. So the save moved off the game task entirely:

1. The tick takes a **snapshot** — `World::snapshot()`.
2. A blocking task serialises and writes it.
3. The result comes back down a channel, which the tick polls. It cannot be awaited, because the
   tick is not async and should not become so for this.

The snapshot is **atomic with respect to the tick**, so a save can never catch the world halfway
through an edit. A torn save is much worse than a slow one.

Measured on a 4200×1200 world, with players connected:

| | Before | After |
|---|---:|---:|
| On the tick | 71 ms | **32 ms** |
| Off the tick | — | 63 ms |
| Ticks dropped per save | ~4 | ~2 |

Better, and honestly still over budget. The remaining 32 ms is the copy: eighty megabytes at
about 2.5 GB/s.

### The buffer experiment, which failed

The obvious next step is to keep the snapshot buffer between saves so its pages stay warm, and
copy into it rather than allocating. That was built, measured, and **reverted**: successive saves
went 33 ms, then 41, then 45.

The reason is visible in `ps`: an idle server's RSS is 27 MB even though its tile array is 80 MB,
because the OS pages a quiet world out. A second eighty-megabyte buffer makes that pressure
worse, so more of the live world is evicted, so each save faults more of it back in. The buffer
cost more than it saved and doubled the footprint doing it.

The measurement is the only reason this is known. The hypothesis was reasonable and wrong, which
is the third one on this page.

### What would actually finish it

Two options, neither small:

- **Shrink `Tile`.** It is fifteen bytes of fields padded to sixteen. Eight would halve the copy
  to about 16 ms and halve the world's memory with it. `examples/memreport` prints what six and
  eight bytes would buy.
- **Snapshot incrementally.** `World` already tracks dirty sections for the network. A persistent
  shadow copy updated a few sections per tick would make the snapshot at save time nearly free.
  This is the proper fix and the subtle one: the shadow has to be consistent at the moment the
  save takes it, not merely up to date on average.

Two rules fall out of that arrangement:

- **One save at a time.** A request arriving while one is running is *dropped*, not queued. Two
  saves racing for the same path is worse than a missed autosave, and a server whose disk cannot
  keep up should not build a backlog of sixty-megabyte snapshots.
- **Shutdown waits.** The shutdown save is synchronous and runs after any background save has
  finished. Both write through a temporary file and rename, so neither can leave a half-written
  world — but the shutdown save has the newer state and must land last, and two renames racing
  would settle that by scheduling rather than by which is newer.

## Other things that have been measured and are fine

- **NPC sync.** Sending every NPC to every player is thousands of frames a second per client.
  The game's rule is followed instead: skip an NPC for a client whose loaded sections do not
  cover it, but never more than four times in a row, so something far away still updates
  occasionally rather than freezing where it was last seen.
- **Section caching.** Encoded sections are cached and invalidated by tile writes. Tracking is
  off during generation and loading, where every tile is written once — five million set inserts
  there would cost more than the cache saves.
- **Per-tick scans.** Several AI routines want a survey of the roster. Each such list is built
  once per tick and only when something present actually reads it.
- **Buffs.** The whole buff pass returns immediately when nothing anywhere is buffed, which is
  the ordinary case.
- **Progress packets.** Pillar shields, invasion progress and the Moon Lord countdown are
  recomputed every tick and almost never change, so each is compared against what went out last.

## The outbound queue, which was a tenth of the size it needed to be

Each connection has a bounded queue of frames waiting to be written, and a client whose queue
fills is removed — the assumption being that anything that far behind has stopped reading.

It was a flat 512, chosen against "the initial world burst is around forty section packets". That
counted the sections and nothing else. The burst a joining player is actually sent is:

- around 39 sections,
- one frame per item on the ground, up to 400,
- one per live NPC, up to 200,
- for every *other* player already in the world: their presence frames, **and one frame per
  relayed inventory slot**, because equipment sent before the newcomer arrived has to be replayed
  for them. That is roughly two hundred frames per player.

So the burst scales with how busy the server is, and on a populated world it passes 512 easily.
The failure is not a slow client: it is the *newcomer* being dropped, mid-load, with no message,
intermittently, and looking exactly like a network problem at their end.

The depth is now `1024 + 256 × max_players`, sized from the config rather than guessed. The real
bound is memory rather than count — the sections dominate at tens of kilobytes each while the rest
are tens of bytes — but a count is what a channel takes.

## A section that failed to encode was lost for good

`send_section` recorded a section as delivered *before* encoding it, using the return of
`sent_sections.insert`. If the encode then failed, the section was marked sent anyway, and every
re-request the client made was dropped by that same dedupe.

The symptom is a 200×150 hole of untextured sky that never fills in however many times the player
walks back through it, and the only trace is one `debug!` line. It is reachable: a compressed
section over 65535 bytes cannot be framed, and because this server batches runs more strictly than
the game does, its sections are systematically larger than vanilla's for the same world.

Membership is now checked first and claimed only once the bytes exist, so a failure retries. The
log line was also raised to `warn!`, since the symptom is missing world rather than anything that
looks like an error.

## Memory

`examples/memreport`, on a 4200×1200 world:

```
baseline RSS          1.8 MB
size_of::<Tile>()      16 bytes
tile array           80.6 MB
process RSS          78.9 MB
```

The tile array is essentially all of it. A `Tile` is fifteen bytes of fields padded to sixteen;
memreport also reports what eight or six bytes would save, which is the obvious lever if memory
ever becomes the constraint.

A save briefly doubles it, since the snapshot is a second copy of the tiles. That copy is freed
as soon as the writer finishes, which is why the buffer-reuse experiment above — which would have
made the doubling permanent — was not worth its cost.

Note that RSS on an idle server reads far *below* the tile array's size, because the OS pages a
quiet world out. That is not a leak and not a saving; it is why a save on a long-idle server is
slower than one on a busy server.

## AI cost

`examples/profile_ai` runs every routine once and totals them:

```
685 routines, 108.3 µs if every one of them ran in the same tick
```

That is 0.65% of a tick for the entire roster acting at once, which cannot happen. The AI has
never been the constraint and this is the number that says so.

## Panic safety

A server that crashes is worse than one that stalls — and worse here than elsewhere, because the
game is a single actor task with no `catch_unwind` and the shutdown save runs *inside* it. A panic
does not degrade this server; it loses the world back to the last autosave.

Outside tests there are a small number of `unwrap`/`expect` calls, each on a proven invariant and
each carrying a message saying which: parsing the built-in default listen address, a buff-slot
search whose loop cannot exit without a slot, a boss routine that has just checked its target,
reading a fixed-width field out of a slice already length-checked, and the worldgen layout's
best-candidate search.

**The count is pinned by `tests/panic_budget.rs` rather than stated here.** This paragraph used to
say "three", and named three, when there were seven — nobody had lied, the sentence was simply
written once and never revisited. A number in prose has nothing keeping it true; a failing test
does. Adding a panic site is now a deliberate act with a test to update.

Everything else that can fail returns a `Result` and is logged. The fuzzer (`examples/fuzz`) throws
twenty thousand malformed packets at a running server and checks it is still answering afterwards.
