# Wiring

Circuits, and the tools that lay them.

**Code:** [`world/wiring.rs`](../crates/terrustia/src/world/wiring.rs) (the simulation),
[`world/mass_wire.rs`](../crates/terrustia/src/world/mass_wire.rs) (the Grand Design's path).

## The Grand Design

Every wiring tool past the plain wrench works by **dragging**: one gesture lays or cuts a whole
run. Packet 109 asks for it; packet 110 is the bill.

It has to be the server's job rather than a stream of single-tile edits, for two reasons:

- the client does not know how much wire the player has left, and
- a run that stops halfway has to stop at the **same tile** for everybody, or two players end up
  looking at different circuits.

### The path is an L, not a line

All the way along one axis, then all the way along the other. Which axis goes first depends on
**which way the player is facing**:

```
facing right                     facing left
  from ●                           from ●─────────┐
       │                                          │
       │                                          │
       └─────────● to                             ● to
```

That is what lets one drag lay a corner. It is also why a run looks wrong if the order is
backwards — and it is the only reason this server tracks a player's facing at all.

### Running out

Stops the run **at that tile** and keeps everything laid before it. The player is then billed for
what was actually spent (packet 110, sent twice — once for wire, once for actuators), which is
what stops a client believing it still has wire the server has already used.

Wire already present costs nothing, so re-running a line is free. Four colours at once costs four
wire per tile. Cutting costs nothing and **refunds nothing**, which is why a mistake with the
Grand Design is expensive.

### Tool modes

A bit set, `WiresUI.Settings.MultiToolMode`:

| Bit | Meaning |
|---:|---|
| 1 | red |
| 2 | green |
| 4 | blue |
| 8 | yellow |
| 0x10 | actuator |
| 0x20 | **cutter** |

Cutter is a *modifier*, not a mode: with it set the colour bits say what to remove rather than
what to lay. That is how one tool does both jobs.

### A limit the game does not have

**Drags longer than 512 tiles are refused.**

The game relies on the client's own tool range, which is fine when the client is the game and not
fine when it is anything else. A drag across a large world is a hundred thousand tile edits
broadcast to every player — a denial of service dressed up as a wiring tool.

This is the only deliberate divergence from vanilla in the wiring code. It is documented here
rather than only in a comment because someone comparing against the game will otherwise find it
and assume it is a bug.

## Gem locks

A gem lock's locked state lives in the **tile's frame**, not in a flag: the lower band of its
sprite sheet is the locked form. Toggling one means moving all nine of its cells between bands,
which is why packet 105's handler walks a three-by-three rather than setting a bit.

## What else the wiring simulation covers

See [`world/wiring.rs`](../crates/terrustia/src/world/wiring.rs). In outline:

- **Traps** — dart, flame, spear, spiky ball, super dart, each with its own cooldown
- **Statues** — the ones that spawn, the ones that give items, the ones that do neither
- **Teleporters** — paired by wire colour, refusing pairs that go nowhere useful
- **Pumps** — inlet and outlet, moving liquid a bucket at a time
- **Timers** — 1s, 3s, 5s, and the 1/4s from 1.4, all firing on a shared window so two timers of
  the same kind stay in step however long they have been running
- **Logic gates and lamps** — AND, OR, XOR, NAND, NOR, XNOR, and the faulty gate
- **Announcement boxes, doors, platforms, actuators**

The mechanism cooldown table is capped the way the game caps it, at 999 entries.
