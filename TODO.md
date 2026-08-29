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

## Refactoring and dense-file splitting

A codebase-wide pass to clean up the code and break the overgrown files into cohesive modules. Run
it as one campaign with the `## Error handling` sweep above (the "unwrap saga") and the
`## Second audit wave`, over the same files: the densest files are also where the panic sites
concentrate, so split each heavy file, clear its non-test `.unwrap()`/`.expect()`/`panic!`, and tidy
its non-idiomatic code in a single visit rather than churning it three times.

Quick investigation (2026-08-29):

- `src` holds 239 hand-written `.rs` files, ~212k lines including the generated tables. The
  panic-site surface across `src` is **502 `.unwrap()` + 305 `.expect()` + 28 `panic!` = 835**. That
  is the raw upper bound; a large share sit inside `#[cfg(test)]` modules and are out of scope per
  the Error-handling note, so the real production count is lower and the sweep separates the two as
  it goes.
- **`game/server.rs` is the elephant: 16,058 lines and 108 panic sites**, the worst file on both
  axes and the natural centerpiece. Split it by responsibility into a `game/server/` module
  directory (packet/event dispatch, the tick loop, the panel-request handlers, the per-system update
  calls) rather than by an arbitrary line budget.
- Other hand-written files over ~1,000 lines, in rough priority: `world/wiring.rs` (2,575),
  `panel/mod.rs` (1,746), `world/world.rs` (1,636), `world/wld.rs` (1,511), `game/spawn.rs` (1,432),
  `world/worldgen/traps.rs` (1,404), `game/ai/mod.rs` (1,365), `world/worldgen/mod.rs` (1,281),
  `world/wld_save.rs` (1,233), `world/worldgen/structures.rs` (1,197), `game/npc.rs` (1,179),
  `game/npc_ai.rs` (1,164), `term.rs` (1,154), `game/ai/town.rs` (1,150), `game/buffs.rs` (1,136),
  `game/ai/critter.rs` (1,123), `game/army.rs` (1,088).
- Explicitly out of scope: the generated proto data tables (`recipes.rs` 25k, `npc_data.rs` 13k,
  `projectile_data.rs` 11k, `tile_object.rs`, `npc_drops.rs`, `placed_items.rs`, `town_names.rs`).
  Those are codegen output, never hand-edited; their size is fine and splitting is only for
  hand-written logic.

Approach: split along real seams (one module per cohesive responsibility), keep public paths stable
or update the call sites in the same change, and run the full test suite plus `clippy` and `fmt`
after each file. Because `warnings = "deny"`, a split cannot leave a dead-code or unused-import
warning behind, which is a useful forcing function. Pair every split with the panic-clearing and
idiomatic cleanup for that same file so it is only churned once.

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

## Dependency pruning

The default server build (the three default-members: `terrustia-proto`, `terrustia-client`,
`terrustia`) resolves **171 external crates**. `Cargo.lock` holds 228 and a full `cargo tree`
across dev-dependencies and the `terrustia-codegen` `regex` reaches into the 240s; 171 is the
number that actually compiles into the shipped server.

**Decision (2026-08-29): stability over crate count.** After measuring the whole tree, the only cut
being made is hand-rolling UPnP away from `igd-next` (below): it is the largest single win, adds no
new dependencies, and its worst failure is a non-fatal boot convenience that already falls back to a
logged manual-port-forward message. Everything else that could be cut is a working, in several cases
already-verified subsystem and is deliberately KEPT. A mature dependency is worth more than the
crates it costs, and rewriting one resets verification that has already been earned (the web panel
is Playwright-verified; hand-rolling its `axum` transport would reset that to zero). The rest of this
section keeps the full measurement so the trade stays visible, and records the larger levers that
were weighed and declined.

**How the numbers were measured.** Feature resolution matters here and the obvious tool lies about
it. Plain `cargo metadata` returns a feature-*unified maximal* graph: it speculatively turns on
`ureq`'s `cookies` feature, which drags `cookie_store` and `url` back in, and makes the ICU stack
look shared when it is not. The honest source is `cargo tree -e no-dev --workspace --no-dedupe`,
which reflects the features that actually compile. Every count below is from that tree. A first
pass off `cargo metadata` got this wrong and undercounted `igd-next` by 20 crates; do not trust
`cargo metadata` for feature-gated ownership questions.

**Exclusive ownership** (crates that leave the build if this one direct dependency is dropped):

| Direct dep | Crates | Purpose | Verdict |
| --- | --- | --- | --- |
| `igd-next` | 31 | UPnP auto-port-forward at boot | **hand-roll (decided)** |
| `ureq` | 13 (+`tempfile` 2) | `terrustia update` over TLS | keep |
| `rust-embed` | 8 | embed the web panel into the binary | keep |
| `crossterm` | 7 | console raw mode + key decoding, incl. Windows | keep |
| `toml` | 6 | config read + admin-store write | keep (deferred) |
| `tracing-subscriber` | 4 | log formatting layer | keep (deferred) |
| `argon2` | 4 | admin password hashing | keep |
| `axum` | 0 alone / 23 with `rust-embed` | panel HTTP + WebSocket server | keep (deferred) |

**The combined win.** `igd-next` and `axum` secretly share the same `hyper` + `hyper-util` +
`futures-*` + `http-body` machinery, and `url` (with its whole `idna` + `icu_*` + `zerovec`/`yoke`
subtree) is pulled by both `igd-next` and `axum`'s `matchit`/query path. So none of the three shows
that machinery as "exclusive," and dropping any one alone frees far less than expected. Dropping all
three together collapses it in one move:

- drop `igd-next`: 171 -> 140
- drop `igd-next` + `rust-embed`: 171 -> 132
- drop `igd-next` + `rust-embed` + `axum`: 171 -> **96** (a 44% cut)
- also hand-roll `toml` + `tracing-subscriber`: -> 86

The 75 crates that leave at the `igd-next` + `rust-embed` + `axum` step include the entire `hyper`
stack (`hyper`, `hyper-util`, `h2`, `http-body`, `http-body-util`, `want`, `try-lock`,
`atomic-waker`), the entire `futures` stack (`futures`, `futures-channel`, `futures-executor`,
`futures-io`, `futures-macro`, `futures-task`, `futures-util`), the entire `url`/IDNA/ICU stack
(`url`, `idna`, `idna_adapter`, `icu_normalizer`, `icu_properties`, `icu_collections`,
`icu_provider`, `icu_locale_core`, `zerovec`, `zerovec-derive`, `zerotrie`, `yoke`, `yoke-derive`,
`zerofrom`, `zerofrom-derive`, `tinystr`, `litemap`, `potential_utf`, `writeable`, `utf8_iter`,
`stable_deref_trait`, `synstructure`, `displaydoc`), `tungstenite`/`tokio-tungstenite`, `tower`,
`matchit`, `indexmap`/`hashbrown`/`equivalent`, `serde_urlencoded`/`form_urlencoded`, `sha1`/`sha2`,
`mime`/`mime_guess`, `walkdir`/`same-file`, `attohttpc`, `xml-rs`/`xmltree`, and more.

**On merging duplicate versions.** The tree carries duplicate majors (`base64` x2, `getrandom` x3,
`rustix` x2, `syn` x2, `winnow` x2, `rand_core` x2). These are upstream version pins, not something
this workspace can unify by hand. They collapse as a side effect of the removals above: dropping
`igd-next` removes the second `rand`/`rand_core`/`getrandom` and the `url` `base64`; dropping `toml`
removes both `winnow`s; dropping `axum` + `igd-next` removes the shared `futures`/`hyper` duplicates.
"Merging" here is realized by removing, not by editing versions.

### Hand-roll: UPnP (replaces `igd-next`, -31)

The only two `igd-next` operations used are `search_gateway` (SSDP discovery + fetch the device
description) and `gateway.add_port` (one `AddPortMapping` SOAP call); no external-IP query, no
unmapping. UPnP-IGD control traffic is plain HTTP/1.1 over the LAN with no TLS, and the "URLs"
involved are always raw `IP:port` literals, so none of `url`/`idna`/ICU is needed to parse them, and
only two fields (`serviceType`, `controlURL`) are needed out of the description XML. Hand-rolling it
adds **zero** new dependencies: `tokio` (already present) for the UDP M-SEARCH and the TCP HTTP
exchanges, plus a small pure module of:

- `build_msearch` (the SSDP `M-SEARCH` datagram to `239.255.255.250:1900`),
- `parse_location` (pull `LOCATION:` out of the SSDP response, case-insensitive),
- a tolerant tag scanner (`tag_inner`) that ignores namespace prefixes and attributes, used to find
  the `WANIPConnection`/`WANPPPConnection` service block and its `controlURL`, plus the SOAP fault
  `errorCode`/`errorDescription` on failure,
- URL splitting (`http://host:port/path`, relative vs absolute `controlURL` resolution),
- `build_soap_add_port` (the `AddPortMapping` envelope) and a minimal HTTP/1.1 GET/POST that sends
  `Connection: close` and reads to EOF.

Public API stays exactly `pub async fn attempt(listen: SocketAddr)`, so `main.rs`'s spawn site and
the lease-renewal loop are untouched. Effort: moderate. Risk: low to keep the capability, but the
live path cannot be tested without a real router, so all the parsing/formatting goes in pure,
unit-tested functions and the socket I/O stays a thin shell. The old module comment argued against
hand-rolling because router firmware varies; the variance is in namespaces, whitespace, and
relative-vs-absolute URLs, which the tolerant scanner and URL resolution handle directly.

### Deferred: asset embedding (`rust-embed`, -8) — kept

Kept for now with the rest; a working, mechanical serving path is not worth disturbing this pass.
The plan, if it is ever revisited: `rust-embed` only provides `#[derive(RustEmbed)]` over
`web-panel/dist/`; it drags in `sha2`,
`digest`, `block-buffer`, `crypto-common`, `mime_guess`, `unicase`, `walkdir`, and `same-file`. The
panel already has its own `content_type_for` match and its own `build.rs`. Replace the derive with a
`build.rs` that walks `dist/`, writes a generated `assets.rs` of `(&str path, &[u8] via include_bytes!)`
entries, and have `load_static_asset` look up that slice. MIME already comes from the local match.
The `embed-web` on/off feature and its disk-serving dev path stay as they are. No ETag/hash is
needed for a loopback admin panel; drop the `sha2` hash rust-embed computed. Effort: low. Risk: low,
since the assets are ours and the serving path is unchanged.

### Deferred: panel HTTP + WebSocket server (`axum` + `tungstenite`, -23 alone, -75 combined) — kept

The single biggest lever, and the one explicitly declined for stability: the panel is Playwright-
verified and a transport rewrite resets that verification to zero. Recorded here in full in case it
is ever revisited. The panel binds loopback only and needs
HTTP/1.1 (http2 is already disabled) plus a WebSocket. Scope observed in `panel/mod.rs`:

- ~30 `/api/*` routes (GET and POST) plus a static/SPA fallback. Extractors in use: JSON request
  body, query string (only `?session=` on the two WS routes), the `Authorization` bearer header, and
  the request path. No multipart. `serde`/`serde_json` stay (they are core, not part of `axum`), so
  request/response bodies are still `serde_json::from_slice`/`to_vec`.
- Two WebSocket routes, both server-to-client only: `stream_status` only ever sends `Text(json)` and
  never reads a client frame. So the RFC 6455 work is the `Sec-WebSocket-Accept` handshake
  (SHA-1 of key + magic GUID, then base64), a text-frame writer (server frames unmasked), and a
  minimal read side for `ping` -> `pong` and `close`. Dead-client detection already rides on
  send-failure every 2s.

To avoid re-adding `sha1`/`base64` for the handshake, hand-roll SHA-1 (a fixed, ~70-line algorithm
used here as a non-cryptographic handshake token, exactly as RFC 6455 specifies) and a ~15-line
base64 encoder. The server is a `tokio` accept loop; per connection: parse request line + headers +
Content-Length body, dispatch on `(method, path)`, write `(status, headers, body)`; on
`Upgrade: websocket` do the handshake and run the existing stream loop over the hand-rolled frame
codec. Open sub-decisions: connection model (keep-alive vs one-request-per-connection
`Connection: close`) and how much of the WS read side to implement. Effort: high. Risk: medium; the
panel is already browser-verified (Playwright), so this needs the same verification re-run after the
rewrite, plus unit tests over request parsing, the handshake accept value, and frame encode/decode.

### Keep (deliberate)

- `ureq` self-update (13 + `tempfile` 2). Hand-rolling a TLS 1.3 client is off the table; `ring`/
  `rustls` is the irreducible core of a secure `terrustia update`. Kept in the default build by
  decision.
- `argon2` (4). The one KDF this workspace intentionally does not hand-roll; a password hash is not
  the place to roll our own.
- `crossterm` (7). Cross-platform raw mode and key decoding including the Windows console path; a
  Unix-only `libc` termios hand-roll would regress Windows, which is on the roadmap.
- `tokio`, `serde`/`serde_json`, `flate2` (zlib-rs), `bytes`, `rand`, `libc`, `thiserror`,
  `tokio-util`. Core, tiny, or foundational; near-zero exclusive weight.

### Deferred (measured, kept for stability)

Both weighed and kept; the cut is not worth disturbing a working path for.

- `toml` -> hand-rolled config reader/writer (-6, also removes both `winnow` versions). Risk is
  faithfully round-tripping user-edited config (comments, quoting, arrays, tables), which is why the
  crate was chosen originally.
- `tracing-subscriber` -> a custom `tracing::Subscriber` (-4: `sharded-slab`, `thread_local`,
  `lazy_static`). We already own the `fmt`-style layer; this means implementing span registry
  storage ourselves for a modest cut.

## Release

- **Tag v0.0.1.** The last step, once the above is in a state worth cutting a first release for.
  Deliberately `0.0.1`, since worldgen is visibly unfinished.
