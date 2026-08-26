//! Wire circuits: what happens when a lever is pulled.
//!
//! A circuit is not a graph anybody builds — it is whatever happens to be connected. Hitting a
//! switch floods outward along wire of that colour, one tile at a time, and every tile the flood
//! touches is *acted on*. That is why a wire laid carelessly across somebody else's contraption
//! joins the two together: there is no notion of a separate circuit, only of connectedness.
//!
//! Four colours run independently, so the same tile can be on four circuits at once and do
//! something different on each.
//!
//! What a tile does when the current reaches it is a table sixty-odd entries long in the game.
//! The ones that change what the world *is*, rather than what it looks like, are here:
//!
//! * **Actuators**, which toggle their block between solid and passable.
//! * **Traps** — darts, flames, spears, spiky balls and geysers — which hurt.
//! * **Statues**, which produce monsters, items or a fetched townsperson.
//! * **Teleporters**, which swap whoever is standing on one pad with whoever is on the other.
//! * **Pumps**, which move liquid from every inlet cell a circuit reaches to every outlet.
//! * **Timers**, the one thing here that starts a circuit with nobody touching it.
//! * **Logic gates**, which read a stack of lamps, decide, and start a circuit of their own.
//!
//! The last two are what make wiring a machine rather than a switchboard. Almost every
//! contraption anybody builds runs off a timer, and almost every interesting one has a gate in
//! it; a server that only ran a circuit when a player hit a switch would run hardly any of them.
//!
//! Only the actuator, the pump, the lamp and the timer are handled inside the flood, because they
//! need nothing but the tiles. The rest are *reported*: firing a trap needs a die roll, a cooldown
//! and the projectile store; a statue needs the NPC table; a teleporter needs the players; a gate
//! needs to start a new circuit, which cannot happen from inside the one that is running. All of
//! that lives on the server, so the flood hands back which tiles it reached and the server does
//! the work — [`trap_shot`] and [`check_logic_gate`] are the tables it calls.
//!
//! The remaining entries are cosmetic: candles, chandeliers and the like change a frame and
//! nothing else, and a client does that for itself from the relayed hit.
//!
//! A tile the flood cannot act on still passes the current along, so a circuit through one is not
//! broken by it. A tile the circuit *started* from is not acted on at all, which is what stops a
//! timer switching itself off the first time it fires.

use std::collections::HashSet;

use terrustia_proto::tile::{Tile, TileFlags};

/// The four wire colours, which are four independent circuits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wire {
    Red,
    Blue,
    Green,
    Yellow,
}

impl Wire {
    pub const ALL: [Wire; 4] = [Wire::Red, Wire::Blue, Wire::Green, Wire::Yellow];

    fn flag(self) -> u16 {
        match self {
            Wire::Red => TileFlags::WIRE_RED,
            Wire::Blue => TileFlags::WIRE_BLUE,
            Wire::Green => TileFlags::WIRE_GREEN,
            Wire::Yellow => TileFlags::WIRE_YELLOW,
        }
    }

    /// Whether a tile carries this colour.
    pub fn on(self, tile: Tile) -> bool {
        tile.flags.has(self.flag())
    }
}

/// The tiles a player can hit to start a circuit.
///
/// A lever and a switch remember their state and flip it; a pressure plate and the rest simply
/// fire. Anything else is not a trigger and hitting it does nothing at all.
pub fn is_trigger(block: u16) -> bool {
    matches!(
        block,
        // Switch, lever, a track switch, the pressure plates, and the Party Monolith.
        135 | 136 | 144 | MINECART_TRACK | 423 | 428 | 440 | 442 | 476 | PARTY_MONOLITH
    )
}

/// Whether hitting this trigger flips a remembered state rather than only firing.
fn flips(block: u16) -> bool {
    matches!(block, 136 | 144)
}

/// How far a trigger's own footprint reaches, since a few are more than one tile.
fn footprint(block: u16) -> (i32, i32) {
    if block == 440 { (3, 3) } else { (1, 1) }
}

/// The most tiles one circuit will touch.
///
/// A circuit is whatever is connected, and what is connected can be the whole world: a player who
/// lays wire across a continent has built a circuit that a server must not spend a whole tick
/// walking. Stopping short is better than stalling — and a circuit this large is not a machine
/// anybody is using, it is a mistake or an attack.
pub const MAX_CIRCUIT: usize = 20_000;

/// What the world has to let a circuit do to it.
pub trait WiredWorld {
    fn tile(&self, x: i32, y: i32) -> Tile;
    fn set_tile(&mut self, x: i32, y: i32, tile: Tile);
    fn width(&self) -> i32;
    fn height(&self) -> i32;
}

/// What a circuit changed.
#[derive(Debug, Default)]
pub struct Fired {
    /// Tiles whose state changed and that clients have to be told about.
    pub changed: Vec<(i32, i32)>,
    /// Trap tiles the current reached, for the caller to resolve into shots.
    pub traps: Vec<(i32, i32)>,
    /// Buried land mines the current reached, for the caller to resolve into an explosion.
    ///
    /// Kept apart from `traps`: a mine is tile 141, not 137, and detonating is not a shot, so it
    /// cannot go through [`trap_shot`] without that function guessing at a kind that isn't there.
    pub mines: Vec<(i32, i32)>,
    /// Statues the current reached, by their top-left tile.
    pub statues: Vec<(i32, i32)>,
    /// The first two distinct teleporters the current reached, which are the pair it joins.
    ///
    /// A third makes no difference: the game keeps room for two and ignores the rest, so a
    /// circuit wired through three teleporters links the first two it happens to walk to.
    pub teleporters: Vec<(i32, i32)>,
    /// The cells of every inlet pump the current reached...
    pub pump_in: Vec<(i32, i32)>,
    /// ...and of every outlet.
    pub pump_out: Vec<(i32, i32)>,
    /// Logic-gate lamps the current toggled, for the caller to run the gates below them.
    pub lamps: Vec<(i32, i32)>,
    /// Timers the current switched on, which then run on their own until switched off.
    pub timers_started: Vec<(i32, i32)>,
    /// ...and the ones it switched off.
    pub timers_stopped: Vec<(i32, i32)>,
    /// Whether the current reached a Party Monolith — every placed monolith reflects the same
    /// single world-level toggle rather than having state of its own, so this is a `bool`, not a
    /// list of positions the way `statues`/`teleporters` are.
    pub party_monolith: bool,
    /// How many tiles the current reached, for the record.
    pub reached: usize,
    /// Whether the circuit was cut short by its size cap.
    pub truncated: bool,
    /// Tiles already acted on in this run, which are not acted on again.
    ///
    /// The four colours run one after another over the same world, so without this a lamp with
    /// two colours on it would toggle twice and end up where it started. The game keeps the same
    /// list and calls adding to it `SkipWire`.
    skipped: HashSet<(i32, i32)>,
}

/// Hit a trigger, and run whatever it is connected to.
///
/// Every colour on the trigger's own tiles runs, each as its own flood, because the four are
/// independent circuits that happen to share a switch.
pub fn hit_switch(world: &mut impl WiredWorld, x: i32, y: i32) -> Fired {
    let mut out = Fired::default();
    let tile = world.tile(x, y);
    if !tile.is_active() || !is_trigger(tile.block) {
        return out;
    }

    // A timer hit by hand is only switched on or off. It does not run its circuit there and then
    // — that is what it will do on its own, on its own schedule, from now on.
    if tile.block == TIMER {
        let mut flipped = tile;
        flipped.frame_y = if tile.frame_y == 0 { 18 } else { 0 };
        world.set_tile(x, y, flipped);
        out.changed.push((x, y));
        if flipped.frame_y == 0 {
            out.timers_stopped.push((x, y));
        } else {
            out.timers_started.push((x, y));
        }
        return out;
    }

    // A lever or a switch remembers which way it is thrown.
    if flips(tile.block) {
        let mut flipped = tile;
        flipped.frame_y = if tile.frame_y == 0 { 18 } else { 0 };
        world.set_tile(x, y, flipped);
        out.changed.push((x, y));
    }

    // A Party Monolith has no frame of its own to flip — the toggle is the world-level state a
    // direct click reaches immediately, matching `Player.cs`'s own click branch rather than
    // needing the flood below to reach it the way a wire-triggered one does.
    if tile.block == PARTY_MONOLITH {
        out.party_monolith = true;
    }

    let (w, h) = footprint(tile.block);
    run_from(world, x, y, w, h, &mut out);
    out
}

/// Run whatever is connected to a tile, without it having to be something a player can hit.
///
/// This is how a timer fires and how a logic gate passes its result on: both start a circuit from
/// their own tile, and neither is a switch.
pub fn trip_wire(world: &mut impl WiredWorld, x: i32, y: i32) -> Fired {
    let mut out = Fired::default();
    run_from(world, x, y, 1, 1, &mut out);
    out
}

/// Flood every colour present on a footprint, each as its own circuit.
fn run_from(world: &mut impl WiredWorld, x: i32, y: i32, w: i32, h: i32, out: &mut Fired) {
    // The tiles a circuit starts from are not acted on by it, with one exception: a track switch.
    // Every other trigger this protects (lever, switch, timer) either already had its own frame
    // toggled directly, before `run_from` was ever called (`hit_switch`'s own lever/switch step),
    // or fires on its own schedule and must not retrigger itself every time its own circuit reaches
    // it (`trip_wire`'s timer — without this a timer's circuit would reach the timer and switch it
    // straight back off, so every timer would fire exactly once). A track switch gets no such
    // direct step (see `act`'s own `MINECART_TRACK` case for why) — the flood reaching *itself* is
    // the only thing that ever flips one a player hits directly, wired to nothing else at all.
    for dx in 0..w {
        for dy in 0..h {
            let (tx, ty) = (x + dx, y + dy);
            if world.tile(tx, ty).block != MINECART_TRACK {
                out.skipped.insert((tx, ty));
            }
        }
    }
    for colour in Wire::ALL {
        let mut seeds = Vec::new();
        for dx in 0..w {
            for dy in 0..h {
                if colour.on(world.tile(x + dx, y + dy)) {
                    seeds.push((x + dx, y + dy));
                }
            }
        }
        if seeds.is_empty() {
            continue;
        }
        trip(world, colour, seeds, out);
    }
}

/// Flood the current outward from a set of seeds and act on everything it reaches.
fn trip(world: &mut impl WiredWorld, colour: Wire, seeds: Vec<(i32, i32)>, out: &mut Fired) {
    let mut seen: HashSet<(i32, i32)> = seeds.iter().copied().collect();
    let mut queue: Vec<(i32, i32)> = seeds;

    while let Some((x, y)) = queue.pop() {
        if seen.len() > MAX_CIRCUIT {
            out.truncated = true;
            break;
        }
        out.reached += 1;
        act(world, x, y, out);

        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 2 || ny < 2 || nx >= world.width() - 2 || ny >= world.height() - 2 {
                continue;
            }
            if !colour.on(world.tile(nx, ny)) || !seen.insert((nx, ny)) {
                continue;
            }
            queue.push((nx, ny));
        }
    }
}

/// What one tile does when the current reaches it.
///
/// A tile with nothing to do is not an error and does not stop the current: the flood is over
/// connectedness, not over things that respond.
fn act(world: &mut impl WiredWorld, x: i32, y: i32, out: &mut Fired) {
    if out.skipped.contains(&(x, y)) {
        return;
    }
    let tile = world.tile(x, y);
    if tile.is_active() && tile.block == LAMP {
        // A logic lamp toggles, unless it is the faulty one, which has no state to toggle. The
        // gate below it is then worth checking, because its inputs have changed.
        if !out.skipped.insert((x, y)) {
            return;
        }
        if tile.frame_x != LAMP_FAULTY {
            let mut flipped = tile;
            flipped.frame_x = if tile.frame_x == 0 { LAMP_ON } else { 0 };
            world.set_tile(x, y, flipped);
            out.changed.push((x, y));
        }
        out.lamps.push((x, y));
        return;
    }
    if tile.is_active() && tile.block == TIMER {
        // A timer on the circuit is toggled by it, the same as one hit by hand.
        if !out.skipped.insert((x, y)) {
            return;
        }
        let mut flipped = tile;
        flipped.frame_y = if tile.frame_y == 0 { 18 } else { 0 };
        world.set_tile(x, y, flipped);
        out.changed.push((x, y));
        if flipped.frame_y == 0 {
            out.timers_stopped.push((x, y));
        } else {
            out.timers_started.push((x, y));
        }
        return;
    }
    if tile.is_active() && tile.block == MINECART_TRACK {
        // `Minecart.FlipSwitchTrack` (`Minecart.cs:1302`), reached from `Wiring.cs`'s own per-tile
        // dispatch (`case 314: if (CheckMech(i, j, 5)) { Minecart.FlipSwitchTrack(i, j); }`) — a
        // wired track switch is a *separate* mechanic from `HitSwitch`'s frame toggle (that branch
        // for tile 314 only relays the current, `Wiring.cs`'s own `TripWire(i, j, 1, 1)`; it never
        // touches the tile), which is exactly why this project's own `is_trigger`/`flips` split
        // left tracks doing nothing on the way through: nothing ever called the piece that does.
        //
        // `FrontTrack()`/`BackTrack()` are themselves nothing but `frameX`/`frameY` in vanilla
        // (`Minecart.cs`'s own private extension methods alias them directly, no packed encoding)
        // — so no new tile field is needed here, only reading the two this project already has.
        // `_trackType`'s own table (`Minecart.cs::Initialize`) classifies every track frame into
        // one of three groups: frames 20-23 (`trackType == 1`, physics-only bumper/dead-end pieces
        // read elsewhere in `Minecart.cs` for cart collision — nothing to do with switching) and
        // frames 30-35 (`trackType == 2`, the six booster-pad frames a *hammer* reframes, not
        // wire) are both out of scope here; every other track frame (`trackType == 0`, vanilla's
        // own array default) is what `FlipSwitchTrack`'s `case 0` actually swaps. A frame in that
        // group only actually has something to swap to if its own `BackTrack()` (`frameY`) was
        // ever set — not every track tile has a second track stacked underneath it — matching
        // vanilla's own `BackTrack() != -1` guard.
        if track_type(tile.frame_x) == 0 && tile.frame_y != -1 {
            if !out.skipped.insert((x, y)) {
                return;
            }
            let mut flipped = tile;
            flipped.frame_x = tile.frame_y;
            flipped.frame_y = tile.frame_x;
            world.set_tile(x, y, flipped);
            out.changed.push((x, y));
        }
        return;
    }
    // An actuator toggles its block between solid and passable. It runs whether or not the block
    // is active, which is the only way a block that has been actuated away can ever come back.
    if tile.flags.has(TileFlags::ACTUATOR) {
        let mut toggled = tile;
        toggled
            .flags
            .set(TileFlags::ACTUATED, !tile.flags.has(TileFlags::ACTUATED));
        world.set_tile(x, y, toggled);
        out.changed.push((x, y));
    }
    if tile.is_active() && matches!(tile.block, TRAPS | GEYSER) {
        out.traps.push((x, y));
    }
    if tile.is_active() && tile.block == EXPLOSIVES {
        out.mines.push((x, y));
    }
    if tile.is_active() && tile.block == TELEPORTER {
        // A teleporter is three tiles wide and the anchor is its left one. Only the first two
        // distinct ones matter: they are the pair the circuit joins.
        let anchor = (x - i32::from(tile.frame_x) / 18, y);
        if out.teleporters.len() < 2 && !out.teleporters.contains(&anchor) {
            out.teleporters.push(anchor);
        }
    }
    if tile.is_active() && matches!(tile.block, PUMP_IN | PUMP_OUT) {
        // A pump is two by two, and all four of its cells take part.
        let anchor = (
            x - {
                let column = i32::from(tile.frame_x) / 18;
                if column > 1 { column - 2 } else { column }
            },
            y - i32::from(tile.frame_y) / 18,
        );
        let cells = [
            (anchor.0, anchor.1 + 1),
            (anchor.0 + 1, anchor.1 + 1),
            anchor,
            (anchor.0 + 1, anchor.1),
        ];
        let side = if tile.block == PUMP_IN {
            &mut out.pump_in
        } else {
            &mut out.pump_out
        };
        for cell in cells {
            if side.len() < PUMP_CELLS && !side.contains(&cell) {
                side.push(cell);
            }
        }
    }
    if tile.is_active() && tile.block == PARTY_MONOLITH {
        // Reached by a wire signal rather than clicked directly — `hit_switch`'s own direct-click
        // case above already covers the tile the flood started from (pre-skipped like any other
        // trigger, `run_from`'s own comment on why), so this is only a *different* monolith the
        // same circuit happens to also run through.
        out.party_monolith = true;
        return;
    }
    if tile.is_active() && tile.block == STATUE {
        // A statue is six tiles and the flood reaches all six; what it does belongs to the statue,
        // not to the tile, so it is reported once by its anchor.
        let (_, within) = terrustia_proto::statues::style_at(tile.frame_x, tile.frame_y);
        let anchor = (x - within.0, y - within.1);
        if !out.statues.contains(&anchor) {
            out.statues.push(anchor);
        }
    }
}

/// The tile every dart, flame, spear and spiky-ball trap is a frame of.
const TRAPS: u16 = 137;
/// The geyser, which is its own tile because it is two wide.
const GEYSER: u16 = 443;
/// A buried land mine, which is a different tile from every other trap and has no projectile —
/// it detonates rather than shooting. Reported separately from `traps` rather than folded in,
/// since [`trap_shot`] has no idea how to resolve it and should not be asked to.
const EXPLOSIVES: u16 = 141;
/// The tile every statue is a frame of.
const STATUE: u16 = 105;
/// The minecart track, `MinecartTrack` — also one of `is_trigger`'s own tiles (a player can hit it
/// by hand too), but its wired behaviour, [`act`]'s own case for it, is a different mechanism from
/// [`hit_switch`]'s frame toggle — see that block's own comment for why.
const MINECART_TRACK: u16 = 314;
/// `TileID.PartyMonolith` (`TileID.cs:1347`) — real vanilla toggles the world's manually-forced
/// birthday party both by a direct click (`Player.cs`'s own `tile.type == 455` branch, a sibling of
/// the celestial pillar monoliths right above it, not something `Wiring.HitSwitch` touches at all
/// in source) and by a wire signal reaching one (`Wiring.cs:2037`, inside the same per-tile
/// dispatch [`act`] is transcribed from). This project folds both paths through the same `hit_switch`
/// a lever or switch already uses, since a direct click already arrives as the same `HIT_SWITCH`
/// packet either way — see [`is_trigger`]'s own entry for it and [`Fired::party_monolith`].
const PARTY_MONOLITH: u16 = 455;

/// `Minecart._trackType`'s own frame classification (`Minecart.cs::Initialize`): `0` (vanilla's own
/// array default, so every frame not explicitly listed below is this) is ordinary track — the only
/// group `FlipSwitchTrack`'s `case 0` ever swaps. `1` (frames 20-23) is a small set of dead-end/
/// bumper pieces `Minecart.cs` reads for cart collision physics elsewhere, nothing to do with
/// switching. `2` (frames 30-35) is the six booster-pad frames, reframed by a hammer, not wire.
fn track_type(frame: i16) -> u8 {
    match frame {
        20..=23 => 1,
        30..=35 => 2,
        _ => 0,
    }
}

/// The teleporter, which is three wide.
const TELEPORTER: u16 = 235;
/// The timer, which is the one trigger that keeps firing on its own.
const TIMER: u16 = 144;
/// A logic gate's input lamp...
const LAMP: u16 = 419;
/// ...and the gate itself, which sits under a stack of them.
const GATE: u16 = 420;
/// A lamp or gate frame of 18 is on; of 36, faulty.
const LAMP_ON: i16 = 18;
const LAMP_FAULTY: i16 = 36;
/// The two pumps, which are two by two.
const PUMP_IN: u16 = 142;
const PUMP_OUT: u16 = 143;
/// The most pump cells one circuit run will pull from or push to.
///
/// The game keeps room for twenty and stops at nineteen, so a circuit wired through five pumps
/// only moves water through the first few it reaches.
const PUMP_CELLS: usize = 19;

/// How wide and tall a teleporter's catchment is, in pixels.
///
/// It reaches three tiles up from the teleporter's own row, which is why standing on one works
/// and walking past one at head height does not.
pub const TELEPORTER_BOX: f32 = 48.0;

/// Whether the two ends of a teleporter pair are far enough apart to be worth using.
///
/// Two within three tiles of each other would only shuffle whoever is standing on them, so the
/// game refuses the pair outright.
pub fn teleport_pair_is_useful(a: (i32, i32), b: (i32, i32)) -> bool {
    !(a.0 < b.0 + 3 && a.0 > b.0 - 3 && a.1 > b.1 - 3 && a.1 < b.1)
}

/// Move liquid from a set of inlet cells to a set of outlet cells.
///
/// Each inlet is emptied into the outlets in turn, and only into ones holding the same liquid —
/// an empty outlet takes on whatever arrives. A pump cannot mix water into lava; it simply skips
/// the outlets that would.
///
/// Returns the cells that changed, for the caller to broadcast and re-settle.
pub fn transfer_liquid(
    world: &mut impl WiredWorld,
    inlets: &[(i32, i32)],
    outlets: &[(i32, i32)],
) -> Vec<(i32, i32)> {
    let mut changed = Vec::new();
    for &(ix, iy) in inlets {
        let mut source = world.tile(ix, iy);
        if source.liquid == 0 {
            continue;
        }
        let kind = source.liquid_kind;
        let mut moved_any = false;
        for &(ox, oy) in outlets {
            if source.liquid == 0 {
                break;
            }
            let mut sink = world.tile(ox, oy);
            if sink.liquid == u8::MAX {
                continue;
            }
            // An empty outlet takes on whatever it is given; a full-enough one only accepts more
            // of what it already holds.
            if sink.liquid != 0 && sink.liquid_kind != kind {
                continue;
            }
            let room = u8::MAX - sink.liquid;
            let amount = source.liquid.min(room);
            if amount == 0 {
                continue;
            }
            sink.liquid += amount;
            sink.liquid_kind = kind;
            source.liquid -= amount;
            world.set_tile(ox, oy, sink);
            changed.push((ox, oy));
            moved_any = true;
        }
        if moved_any {
            if source.liquid == 0 {
                source.liquid_kind = terrustia_proto::tile::Liquid::Water;
            }
            world.set_tile(ix, iy, source);
            changed.push((ix, iy));
        }
    }
    changed
}

/// How often a timer fires, from the frame it is set to.
///
/// The five timers are a quarter of a second, half a second, one second, three and five. A timer
/// keeps a contraption running with nobody standing on it, which is what most wiring is actually
/// for, so a server that only runs a circuit when somebody hits a switch runs almost none of it.
pub fn timer_period(frame_x: i16) -> i32 {
    match frame_x / 18 {
        0 => 60,
        1 => 180,
        2 => 300,
        3 => 30,
        4 => 15,
        _ => 60,
    }
}

/// Whether this tile is a timer that is switched on.
pub fn timer_is_running(tile: Tile) -> bool {
    tile.is_active() && tile.block == TIMER && tile.frame_y != 0
}

/// What one logic gate decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateResult {
    /// Where the gate is.
    pub at: (i32, i32),
    /// Whether it should now pass current on.
    pub fires: bool,
}

/// Run the gate under a lamp the current just toggled.
///
/// A gate is a stack: one gate tile with a column of lamps directly above it. The gate reads all
/// of them at once and decides in one step — there is no notion of an input arriving before
/// another, which is why a logic gate in Terraria settles rather than oscillates.
///
/// A *faulty* lamp turns the whole gate into a coin toss weighted by how many of its lamps are
/// lit, which is the only source of randomness in the wire table.
pub fn check_logic_gate(
    world: &mut impl WiredWorld,
    lamp_x: i32,
    lamp_y: i32,
    already_fired: &HashSet<(i32, i32)>,
    rng: &mut impl rand::Rng,
) -> Option<GateResult> {
    // Walk down from the lamp through the rest of the stack to the gate under it.
    let mut y = lamp_y;
    let gate_y = loop {
        if y >= world.height() {
            return None;
        }
        let tile = world.tile(lamp_x, y);
        if !tile.is_active() {
            return None;
        }
        if tile.block == GATE {
            break y;
        }
        if tile.block != LAMP {
            return None;
        }
        y += 1;
    };

    let gate = world.tile(lamp_x, gate_y);
    let kind = i32::from(gate.frame_y) / 18;
    let was_on = gate.frame_x == LAMP_ON;
    let gate_is_faulty = gate.frame_x == LAMP_FAULTY;

    // Count the lamps above the gate, stopping at a faulty one.
    let (mut lamps, mut lit, mut faulty_lamp) = (0, 0, false);
    let mut above = gate_y - 1;
    while above > 0 {
        let tile = world.tile(lamp_x, above);
        if !tile.is_active() || tile.block != LAMP {
            break;
        }
        if tile.frame_x == LAMP_FAULTY {
            faulty_lamp = true;
            break;
        }
        lamps += 1;
        lit += i32::from(tile.frame_x == LAMP_ON);
        above -= 1;
    }

    let now_on = match kind {
        0 => lamps == lit, // and
        1 => lit > 0,      // or
        2 => lamps != lit, // nand
        3 => lit == 0,     // nor
        4 => lit == 1,     // xor
        5 => lit != 1,     // xnor
        _ => return None,
    };

    // A faulty gate with no faulty lamp is stuck: it changes nothing and passes nothing on.
    let stuck = !faulty_lamp && gate_is_faulty;
    // A faulty lamp at the top of the stack is what makes the gate roll a die instead.
    let rolls = faulty_lamp && world.tile(lamp_x, lamp_y).frame_x == LAMP_FAULTY;
    if now_on == was_on && !stuck && !rolls {
        return None;
    }

    let mut updated = gate;
    updated.frame_x = if faulty_lamp {
        LAMP_FAULTY
    } else {
        LAMP_ON * i16::from(now_on)
    };
    world.set_tile(lamp_x, gate_y, updated);

    let mut fires = !faulty_lamp || rolls;
    if rolls {
        fires = lamps > 0 && lit > 0 && rng.random_range(0..lamps) < lit;
    }
    if stuck {
        fires = false;
    }
    // A gate that has already fired in this pass has found a loop. The game puffs smoke at it and
    // refuses to go round again, which is what stops a wired ring locking the server up.
    if fires && already_fired.contains(&(lamp_x, gate_y)) {
        fires = false;
    }
    Some(GateResult {
        at: (lamp_x, gate_y),
        fires,
    })
}

/// A shot a wired trap wants to take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    pub projectile_type: u16,
    /// Where the projectile appears, in world pixels.
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub damage: i32,
    /// How long this tile must wait before it can fire again.
    pub cooldown: i32,
    /// Where the cooldown is recorded, which is not always the tile that fired: a geyser is two
    /// tiles wide and both halves share one.
    pub cools_at: (i32, i32),
}

/// What a trap tile throws, given its frame.
///
/// The kind is `frame_y / 18` and the direction is in `frame_x`, which is why a trap turned round
/// in the world is turned round here too rather than being a separate tile.
pub fn trap_shot(tile: Tile, x: i32, y: i32, rng: &mut impl rand::Rng) -> Option<Shot> {
    let (px, py) = (x as f32 * 16.0, y as f32 * 16.0);
    if tile.block == GEYSER {
        // The frame's left half tells us which of the two tiles is the anchor, and the top half
        // from the bottom half tells us which way the steam goes.
        let half = i32::from(tile.frame_x) / 36;
        let anchor_x = x - (i32::from(tile.frame_x) - half * 36) / 18;
        let up = half < 2;
        return Some(Shot {
            projectile_type: 654,
            position: (
                (anchor_x + 1) as f32 * 16.0,
                (y + i32::from(!up)) as f32 * 16.0,
            ),
            velocity: (0.0, if up { -8.0 } else { 8.0 }),
            damage: 20,
            cooldown: 200,
            cools_at: (anchor_x, y),
        });
    }

    let kind = i32::from(tile.frame_y) / 18;
    let cools_at = (x, y);
    match kind {
        // The darts and the flame: one tile, aimed by frame_x, ten pixels clear of the muzzle.
        0 | 1 | 2 | 5 => {
            let dx = match tile.frame_x {
                0 => -1,
                18 => 1,
                _ => 0,
            };
            let dy = if tile.frame_x >= 36 {
                if tile.frame_x >= 72 { 1 } else { -1 }
            } else {
                0
            };
            let (projectile_type, damage, speed) = match kind {
                0 => (98u16, 20, 12.0),
                1 => (184, 40, 12.0),
                2 => (187, 40, 5.0),
                _ => (980, 30, 12.0),
            };
            Some(Shot {
                projectile_type,
                position: (px + 8.0 + 10.0 * dx as f32, py + 8.0 + 10.0 * dy as f32),
                velocity: (dx as f32 * speed, dy as f32 * speed),
                damage,
                cooldown: 200,
                cools_at,
            })
        }
        // The spiky ball, which is thrown with a spread rather than aimed.
        3 => {
            let (dx, dy) = trap_facing(tile.frame_x);
            let mut spread = |d: i32| {
                let low = -20 + if d == 1 { 20 } else { 0 };
                let high = 21 - if d == -1 { 20 } else { 0 };
                4.0 * d as f32 + rng.random_range(low..high) as f32 * 0.05
            };
            Some(Shot {
                projectile_type: 185,
                position: (px + 8.0 + 14.0 * dx as f32, py + 8.0 + 14.0 * dy as f32),
                velocity: (spread(dx), spread(dy)),
                damage: 40,
                cooldown: 300,
                cools_at,
            })
        }
        // The spear, which is the only one that reaches back out of the wall it is set in.
        4 => {
            let (dx, dy) = trap_facing(tile.frame_x);
            Some(Shot {
                projectile_type: 186,
                position: (px + 8.0 + 18.0 * dx as f32, py + 8.0 + 18.0 * dy as f32),
                velocity: (8.0 * dx as f32, 8.0 * dy as f32),
                damage: 60,
                cooldown: 90,
                cools_at,
            })
        }
        _ => None,
    }
}

/// Which way a spiky-ball or spear trap points, which it states differently from the darts.
fn trap_facing(frame_x: i16) -> (i32, i32) {
    match frame_x / 18 {
        0 | 1 => (0, 1),
        2 => (0, -1),
        3 => (-1, 0),
        4 => (1, 0),
        _ => (0, 0),
    }
}

/// Whether another spiky ball is welcome, given how far away the ones already out are.
///
/// The game keeps a budget of two hundred and charges each ball against it on a sliding scale —
/// fifty for one within fifty pixels, one for one nearly a screen away. It is what stops a plate
/// held down by a slime from filling a corridor with several hundred of them.
pub fn spiky_ball_allowed(distances: impl Iterator<Item = f32>) -> bool {
    let mut budget = 200i32;
    for d in distances {
        budget -= match d {
            d if d < 50.0 => 50,
            d if d < 100.0 => 15,
            d if d < 200.0 => 10,
            d if d < 300.0 => 8,
            d if d < 400.0 => 6,
            d if d < 500.0 => 5,
            d if d < 700.0 => 4,
            d if d < 900.0 => 3,
            d if d < 1200.0 => 2,
            _ => 1,
        };
    }
    budget > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashMap;

    struct Board(HashMap<(i32, i32), Tile>);

    impl WiredWorld for Board {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
        fn set_tile(&mut self, x: i32, y: i32, tile: Tile) {
            self.0.insert((x, y), tile);
        }
        fn width(&self) -> i32 {
            500
        }
        fn height(&self) -> i32 {
            500
        }
    }

    fn wired(block: u16, colour: Wire) -> Tile {
        let mut tile = if terrustia_proto::tile_sets::frame_important(block) {
            Tile::framed(block, 0, 0)
        } else {
            Tile::block(block)
        };
        tile.flags.set(colour.flag(), true);
        tile
    }

    fn actuated(colour: Wire) -> Tile {
        let mut tile = Tile::block(1);
        tile.flags.set(colour.flag(), true);
        tile.flags.set(TileFlags::ACTUATOR, true);
        tile
    }

    /// A lever wired to an actuator toggles the block, and toggles it back.
    #[test]
    fn a_lever_actuates_a_block() {
        let mut board = Board(HashMap::new());
        // A lever at 100,100 with wire running to an actuated block at 105,100.
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, actuated(Wire::Red));

        assert!(!board.tile(105, 100).flags.has(TileFlags::ACTUATED));
        let fired = hit_switch(&mut board, 100, 100);
        assert!(fired.reached >= 6, "the current should have run the length");
        assert!(
            board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "the block should have been actuated away"
        );

        hit_switch(&mut board, 100, 100);
        assert!(
            !board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "and back again"
        );
    }

    /// A lever remembers which way it is thrown.
    #[test]
    fn a_lever_flips() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        assert_eq!(board.tile(100, 100).frame_y, 0);
        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_y, 18, "thrown");
        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_y, 0, "and thrown back");
    }

    /// A wired track switch swaps its front and back track — vanilla's `Minecart.FlipSwitchTrack`,
    /// reached from a *different* branch than `hit_switch`'s own frame toggle (`HitSwitch`'s own
    /// `type == 314` case only relays the current; it never touches the tile). Frame 1 (plain
    /// straight track) and frame 6 (one of the sloped connector frames) are both ordinary,
    /// `trackType == 0` frames — the only group `FlipSwitchTrack`'s `case 0` ever swaps; frame 6
    /// here stands in for "whatever second track was stacked underneath."
    ///
    /// Fails on the code before this fix: without `act`'s own `MINECART_TRACK` case, the current
    /// reaches the tile and does nothing to it at all, so `frame_x`/`frame_y` would still read
    /// `(1, 6)` after `hit_switch` instead of the swapped `(6, 1)`.
    #[test]
    fn a_wired_track_switch_swaps_its_stored_path() {
        let mut board = Board(HashMap::new());
        let mut track = wired(MINECART_TRACK, Wire::Red);
        track.frame_x = 1;
        track.frame_y = 6;
        board.set_tile(100, 100, track);

        hit_switch(&mut board, 100, 100);

        let after = board.tile(100, 100);
        assert_eq!(after.frame_x, 6, "front is now what back held");
        assert_eq!(after.frame_y, 1, "and back now holds what front held");

        // Flips back just as cleanly.
        hit_switch(&mut board, 100, 100);
        let back = board.tile(100, 100);
        assert_eq!(back.frame_x, 1);
        assert_eq!(back.frame_y, 6);
    }

    /// A frame `_trackType` classifies outside the ordinary group — a dead-end/bumper piece
    /// (frame 20, `trackType == 1`, read elsewhere in `Minecart.cs` for cart collision, nothing to
    /// do with switching) or a booster pad (frame 30, `trackType == 2`, reframed by a hammer, not
    /// wire) — is left alone even with a real value stored in its own back track: `FlipSwitchTrack`
    /// only has cases for `0` and `2`, and only `0` performs this swap at all.
    #[test]
    fn a_non_switchable_track_frame_is_not_touched_by_a_wired_hit() {
        let mut board = Board(HashMap::new());
        for bumper_or_booster in [20i16, 30] {
            let mut track = wired(MINECART_TRACK, Wire::Red);
            track.frame_x = bumper_or_booster;
            track.frame_y = 1; // a real stored value — still must not swap.
            board.set_tile(100, 100, track);

            hit_switch(&mut board, 100, 100);

            let after = board.tile(100, 100);
            assert_eq!(after.frame_x, bumper_or_booster);
            assert_eq!(after.frame_y, 1);
        }
    }

    /// An ordinary track frame with nothing stored in its back track (`BackTrack() == -1`, the
    /// state a plain track tile is in before anyone has stacked a second one underneath it) has
    /// nothing to swap to — vanilla's own guard, `FlipSwitchTrack`'s `BackTrack() != -1` check, not
    /// a gap this project invented.
    #[test]
    fn a_track_frame_with_no_stored_back_track_does_not_swap() {
        let mut board = Board(HashMap::new());
        let mut track = wired(MINECART_TRACK, Wire::Red);
        track.frame_x = 1;
        track.frame_y = -1;
        board.set_tile(100, 100, track);

        hit_switch(&mut board, 100, 100);

        let after = board.tile(100, 100);
        assert_eq!(after.frame_x, 1);
        assert_eq!(after.frame_y, -1);
    }

    /// A pressure plate fires without remembering anything.
    #[test]
    fn a_plate_does_not_flip() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(135, Wire::Red));
        board.set_tile(101, 100, actuated(Wire::Red));
        let before = board.tile(100, 100).frame_y;
        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_y, before, "a plate has no state");
        assert!(board.tile(101, 100).flags.has(TileFlags::ACTUATED));
    }

    /// The four colours are four circuits: red does not run what blue is wired to.
    #[test]
    fn the_colours_are_separate_circuits() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        // A red run to one block, a blue run to another, neither touching the other's wire.
        for x in 101..104 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(104, 100, actuated(Wire::Red));
        for x in 101..104 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_BLUE, true);
            board.set_tile(x, 105, wire);
        }
        board.set_tile(104, 105, actuated(Wire::Blue));

        hit_switch(&mut board, 100, 100);
        assert!(
            board.tile(104, 100).flags.has(TileFlags::ACTUATED),
            "the red circuit ran"
        );
        assert!(
            !board.tile(104, 105).flags.has(TileFlags::ACTUATED),
            "and the blue one, which the lever is not on, did not"
        );
    }

    /// ...but a switch carrying two colours runs both.
    #[test]
    fn a_switch_on_two_colours_runs_both() {
        let mut board = Board(HashMap::new());
        let mut lever = wired(136, Wire::Red);
        lever.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(100, 100, lever);
        board.set_tile(101, 100, actuated(Wire::Red));
        board.set_tile(100, 101, actuated(Wire::Blue));

        hit_switch(&mut board, 100, 100);
        assert!(board.tile(101, 100).flags.has(TileFlags::ACTUATED), "red");
        assert!(board.tile(100, 101).flags.has(TileFlags::ACTUATED), "blue");
    }

    /// Hitting something that is not a trigger does nothing at all.
    #[test]
    fn only_a_trigger_starts_a_circuit() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(1, Wire::Red));
        board.set_tile(101, 100, actuated(Wire::Red));
        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.reached, 0);
        assert!(!board.tile(101, 100).flags.has(TileFlags::ACTUATED));
    }

    /// A tile the circuit cannot act on still passes the current along.
    #[test]
    fn an_inert_tile_does_not_break_the_circuit() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        // A plain wired stone block in the middle of the run.
        board.set_tile(101, 100, wired(1, Wire::Red));
        board.set_tile(102, 100, actuated(Wire::Red));
        hit_switch(&mut board, 100, 100);
        assert!(
            board.tile(102, 100).flags.has(TileFlags::ACTUATED),
            "the current should have passed through the stone"
        );
    }

    /// A circuit big enough to be a mistake is cut short rather than stalling the tick.
    #[test]
    fn an_enormous_circuit_is_cut_short() {
        let mut board = Board(HashMap::new());
        board.set_tile(2, 100, wired(136, Wire::Red));
        // A wire running most of the width of the test world, folded so it fits.
        let mut laid = 0;
        'lay: for y in 100..400 {
            for x in 3..490 {
                let mut wire = Tile::AIR;
                wire.flags.set(TileFlags::WIRE_RED, true);
                board.set_tile(x, y, wire);
                laid += 1;
                if laid > MAX_CIRCUIT + 5_000 {
                    break 'lay;
                }
            }
            // Join the rows so it is one circuit.
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(489, y + 1, wire);
        }
        let fired = hit_switch(&mut board, 2, 100);
        assert!(fired.truncated, "it should have given up");
        assert!(
            fired.reached <= MAX_CIRCUIT + 1,
            "and stopped near the cap, not {} in",
            fired.reached
        );
    }

    /// A dart trap facing each way throws its dart that way, clear of its own tile.
    #[test]
    fn a_dart_trap_shoots_the_way_it_faces() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
        for (frame_x, want) in [
            (0i16, (-12.0f32, 0.0f32)),
            (18, (12.0, 0.0)),
            (36, (0.0, -12.0)),
            (72, (0.0, 12.0)),
        ] {
            let shot = trap_shot(Tile::framed(TRAPS, frame_x, 0), 100, 200, &mut rng)
                .expect("a dart trap fires");
            assert_eq!(shot.projectile_type, 98, "frame_x {frame_x}");
            assert_eq!(shot.velocity, want, "frame_x {frame_x}");
            assert_eq!(shot.damage, 20);
            assert_eq!(shot.cooldown, 200);
            // Ten pixels clear of the tile centre, in the direction it is pointing. `signum` is
            // no use here: it calls zero positive.
            let unit = |v: f32| if v == 0.0 { 0.0 } else { v.signum() };
            assert_eq!(
                shot.position,
                (1608.0 + 10.0 * unit(want.0), 3208.0 + 10.0 * unit(want.1)),
                "frame_x {frame_x}"
            );
        }
    }

    /// Each row of the trap tile is a different trap, and they do not share a projectile, a
    /// damage or a cooldown.
    #[test]
    fn every_row_of_the_trap_tile_is_a_different_trap() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(2);
        let seen: Vec<_> = (0..6)
            .filter_map(|row| {
                trap_shot(Tile::framed(TRAPS, 18, row * 18), 50, 60, &mut rng)
                    .map(|s| (s.projectile_type, s.damage, s.cooldown))
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (98, 20, 200),  // dart
                (184, 40, 200), // poison dart
                (187, 40, 200), // flamethrower
                (185, 40, 300), // spiky ball
                (186, 60, 90),  // spear
                (980, 30, 200), // venom dart
            ]
        );
    }

    /// A spiky ball is thrown with a spread, and only ever away from the trap: the random part
    /// can slow it but never turn it round.
    #[test]
    fn a_spiky_ball_is_thrown_with_a_spread_but_never_backwards() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(3);
        for _ in 0..200 {
            // frame_x 3 * 18 points left.
            let shot = trap_shot(Tile::framed(TRAPS, 54, 54), 10, 10, &mut rng).unwrap();
            assert!(shot.velocity.0 <= -3.0, "went {:?}", shot.velocity);
            assert!(shot.velocity.1.abs() <= 1.0, "wandered {:?}", shot.velocity);
        }
    }

    /// A geyser is two tiles wide and both halves cool down together, or one plate would fire it
    /// twice.
    #[test]
    fn both_halves_of_a_geyser_share_one_cooldown() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(4);
        let left = trap_shot(Tile::framed(GEYSER, 0, 0), 300, 400, &mut rng).unwrap();
        let right = trap_shot(Tile::framed(GEYSER, 18, 0), 301, 400, &mut rng).unwrap();
        assert_eq!(left.cools_at, right.cools_at);
        assert_eq!(left.cools_at, (300, 400));
        assert_eq!(left.velocity, (0.0, -8.0), "the top half blows upward");

        let below = trap_shot(Tile::framed(GEYSER, 72, 0), 300, 400, &mut rng).unwrap();
        assert_eq!(below.velocity, (0.0, 8.0), "the bottom half blows down");
    }

    /// The spiky-ball budget is spent fastest by the ones nearest the trap.
    #[test]
    fn spiky_balls_are_rationed_by_how_close_the_others_are() {
        assert!(spiky_ball_allowed(std::iter::empty()));
        assert!(
            spiky_ball_allowed(std::iter::repeat_n(2000.0, 100)),
            "a hundred of them across the map is still fine"
        );
        assert!(
            !spiky_ball_allowed(std::iter::repeat_n(10.0, 4)),
            "four underfoot is not"
        );
    }

    /// A trap the current reaches is reported rather than fired, because firing it needs a die
    /// roll and a cooldown the flood knows nothing about.
    #[test]
    fn the_flood_reports_traps_instead_of_firing_them() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, wired(TRAPS, Wire::Red));

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.traps, vec![(105, 100)]);
        // And the tile is untouched: a trap is not a thing the flood changes.
        assert_eq!(board.tile(105, 100).block, TRAPS);
    }

    /// A buried land mine is reported separately from `traps` — it is a different tile (141, not
    /// 137) and has no shot for `trap_shot` to resolve, so folding it into `traps` would hand the
    /// caller a tile that function cannot make sense of.
    #[test]
    fn the_flood_reports_mines_apart_from_traps() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(135, Wire::Red)); // pressure plate
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, wired(EXPLOSIVES, Wire::Red));

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.mines, vec![(105, 100)]);
        assert!(fired.traps.is_empty(), "a mine is not a trap");
    }

    /// A pump moves what the inlet holds into the outlet, up to what the outlet can take.
    #[test]
    fn a_pump_moves_liquid_from_inlet_to_outlet() {
        let mut board = Board(HashMap::new());
        let mut full = Tile::AIR;
        full.liquid = 200;
        full.liquid_kind = terrustia_proto::tile::Liquid::Water;
        board.set_tile(10, 10, full);

        let changed = transfer_liquid(&mut board, &[(10, 10)], &[(50, 50)]);
        assert_eq!(board.tile(10, 10).liquid, 0, "the inlet emptied");
        assert_eq!(board.tile(50, 50).liquid, 200, "and the outlet filled");
        assert_eq!(
            board.tile(50, 50).liquid_kind,
            terrustia_proto::tile::Liquid::Water,
            "an empty outlet takes on what arrives"
        );
        assert!(changed.contains(&(10, 10)) && changed.contains(&(50, 50)));
    }

    /// A pump will not mix water into lava: it skips the outlets that would and keeps the rest.
    #[test]
    fn a_pump_refuses_to_mix_liquids() {
        let mut board = Board(HashMap::new());
        let mut water = Tile::AIR;
        water.liquid = 100;
        water.liquid_kind = terrustia_proto::tile::Liquid::Water;
        board.set_tile(10, 10, water);
        let mut lava = Tile::AIR;
        lava.liquid = 50;
        lava.liquid_kind = terrustia_proto::tile::Liquid::Lava;
        board.set_tile(50, 50, lava);

        transfer_liquid(&mut board, &[(10, 10)], &[(50, 50)]);
        assert_eq!(board.tile(10, 10).liquid, 100, "nothing moved");
        assert_eq!(board.tile(50, 50).liquid, 50);
        assert_eq!(
            board.tile(50, 50).liquid_kind,
            terrustia_proto::tile::Liquid::Lava
        );
    }

    /// An outlet takes only what it has room for, and the inlet keeps the rest for the next one.
    #[test]
    fn a_pump_fills_its_outlets_in_turn() {
        let mut board = Board(HashMap::new());
        let mut full = Tile::AIR;
        full.liquid = 255;
        board.set_tile(10, 10, full);
        let mut nearly = Tile::AIR;
        nearly.liquid = 200;
        board.set_tile(50, 50, nearly);

        transfer_liquid(&mut board, &[(10, 10)], &[(50, 50), (51, 50)]);
        assert_eq!(
            board.tile(50, 50).liquid,
            255,
            "the first filled to the brim"
        );
        assert_eq!(
            board.tile(51, 50).liquid,
            200,
            "and the rest went to the next"
        );
        assert_eq!(board.tile(10, 10).liquid, 0);
    }

    /// A teleporter pair three tiles apart would only shuffle whoever is standing on it, so the
    /// game refuses it outright.
    #[test]
    fn a_teleporter_pair_has_to_go_somewhere() {
        assert!(teleport_pair_is_useful((100, 100), (400, 100)));
        assert!(teleport_pair_is_useful((100, 100), (100, 400)));
        assert!(!teleport_pair_is_useful((101, 100), (100, 101)));
    }

    /// The flood reports the first two teleporters it reaches, and only two: a third is ignored
    /// rather than replacing one of the pair.
    #[test]
    fn the_flood_pairs_the_first_two_teleporters() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..140 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        for x in [110i32, 120, 130] {
            let mut pad = wired(TELEPORTER, Wire::Red);
            pad.frame_x = 0;
            board.set_tile(x, 100, pad);
        }

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.teleporters.len(), 2, "only two make a pair");
        assert!(
            fired
                .teleporters
                .iter()
                .all(|p| [110, 120, 130].contains(&p.0))
        );
    }

    /// Build a gate with a stack of lamps above it. `lamps` is on/off from the top down.
    fn gate_stack(board: &mut Board, x: i32, gate_y: i32, kind: i16, lamps: &[bool]) {
        let mut gate = Tile::framed(GATE, 0, kind * 18);
        gate.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(x, gate_y, gate);
        for (i, &on) in lamps.iter().enumerate() {
            let y = gate_y - lamps.len() as i32 + i as i32;
            let mut lamp = Tile::framed(LAMP, if on { LAMP_ON } else { 0 }, 0);
            lamp.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, y, lamp);
        }
    }

    /// Each of the six gate kinds reads its whole stack at once and answers in one step.
    #[test]
    fn every_gate_kind_answers_the_way_it_should() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(1);
        let done = HashSet::new();
        // kind, lamps, expected output.
        let cases: &[(i16, &[bool], bool)] = &[
            (0, &[true, true], true), // and
            (0, &[true, false], false),
            (1, &[false, true], true), // or
            (1, &[false, false], false),
            (2, &[true, true], false), // nand
            (2, &[true, false], true),
            (3, &[false, false], true), // nor
            (3, &[true, false], false),
            (4, &[true, false], true), // xor: exactly one
            (4, &[true, true], false),
            (5, &[true, true], true), // xnor: not exactly one
            (5, &[true, false], false),
        ];
        for &(kind, lamps, want) in cases {
            let mut board = Board(HashMap::new());
            gate_stack(&mut board, 100, 100, kind, lamps);
            // Start the gate at the opposite state, so it always has something to say.
            let mut gate = board.tile(100, 100);
            gate.frame_x = if want { 0 } else { LAMP_ON };
            board.set_tile(100, 100, gate);

            let top = 100 - lamps.len() as i32;
            let result = check_logic_gate(&mut board, 100, top, &done, &mut rng)
                .unwrap_or_else(|| panic!("kind {kind} with {lamps:?} said nothing"));
            assert_eq!(result.at, (100, 100));
            assert_eq!(
                board.tile(100, 100).frame_x == LAMP_ON,
                want,
                "kind {kind} with {lamps:?}"
            );
            assert!(result.fires, "a gate that changed should pass it on");
        }
    }

    /// A gate whose answer has not changed says nothing, which is what stops a circuit running in
    /// circles through a stable machine.
    #[test]
    fn a_gate_that_did_not_change_stays_quiet() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(2);
        let done = HashSet::new();
        let mut board = Board(HashMap::new());
        // An OR gate with a lamp lit, already reading true.
        gate_stack(&mut board, 100, 100, 1, &[true, false]);
        let mut gate = board.tile(100, 100);
        gate.frame_x = LAMP_ON;
        board.set_tile(100, 100, gate);

        assert!(check_logic_gate(&mut board, 100, 98, &done, &mut rng).is_none());
    }

    /// A gate that has already fired in this pass has found a loop, and refuses to go round again.
    #[test]
    fn a_gate_will_not_go_round_twice() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(3);
        let mut board = Board(HashMap::new());
        gate_stack(&mut board, 100, 100, 1, &[true, false]);

        let done: HashSet<(i32, i32)> = [(100, 100)].into_iter().collect();
        let result = check_logic_gate(&mut board, 100, 98, &done, &mut rng).unwrap();
        assert!(!result.fires, "a gate already fired should not fire again");
        assert_eq!(
            board.tile(100, 100).frame_x,
            LAMP_ON,
            "though it still records what it worked out"
        );
    }

    /// The current toggles a lamp and reports it, rather than acting on the gate itself.
    #[test]
    fn the_flood_toggles_a_lamp_and_reports_it() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut lamp = Tile::framed(LAMP, 0, 0);
        lamp.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(105, 100, lamp);

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.lamps, vec![(105, 100)]);
        assert_eq!(board.tile(105, 100).frame_x, LAMP_ON, "the lamp came on");
    }

    /// A lamp on two colours is toggled once, not twice: the four floods share one skip list, or
    /// a two-colour lamp would end every circuit exactly where it started.
    #[test]
    fn a_lamp_on_two_colours_toggles_once() {
        let mut board = Board(HashMap::new());
        let mut lever = wired(136, Wire::Red);
        lever.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(100, 100, lever);
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            wire.flags.set(TileFlags::WIRE_BLUE, true);
            board.set_tile(x, 100, wire);
        }
        let mut lamp = Tile::framed(LAMP, 0, 0);
        lamp.flags.set(TileFlags::WIRE_RED, true);
        lamp.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(105, 100, lamp);

        hit_switch(&mut board, 100, 100);
        assert_eq!(
            board.tile(105, 100).frame_x,
            LAMP_ON,
            "on, not back off again"
        );
    }

    /// Hitting a timer switches it on, and hitting it again switches it off.
    #[test]
    fn a_timer_is_switched_on_and_off() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(TIMER, Wire::Red));

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.timers_started, vec![(100, 100)]);
        assert!(fired.timers_stopped.is_empty());
        assert!(timer_is_running(board.tile(100, 100)));

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.timers_stopped, vec![(100, 100)]);
        assert!(!timer_is_running(board.tile(100, 100)));
    }

    /// The five timers run at the five rates the game gives them.
    #[test]
    fn each_timer_has_its_own_rate() {
        assert_eq!(timer_period(0), 60, "one second");
        assert_eq!(timer_period(18), 180, "three seconds");
        assert_eq!(timer_period(36), 300, "five seconds");
        assert_eq!(timer_period(54), 30, "half a second");
        assert_eq!(timer_period(72), 15, "a quarter");
        // And the window they reset to is a multiple of all of them, so two of a kind stay in step.
        for frame in [0i16, 18, 36, 54, 72] {
            assert_eq!(18_000 % timer_period(frame), 0, "frame {frame}");
        }
    }

    /// The whole chain on a board: two levers, two lamps, an AND gate, and a trap beyond it.
    #[test]
    fn a_gate_passes_current_on_only_when_it_should() {
        let mut rng = rand::rngs::SmallRng::seed_from_u64(9);
        let mut board = Board(HashMap::new());
        let wire = |board: &mut Board, x: i32, y: i32, flag: u16| {
            let mut t = board.tile(x, y);
            t.flags.set(flag, true);
            board.set_tile(x, y, t);
        };

        // Red runs from its lever along the row of the lower lamp; blue along the row of the
        // upper one. Neither passes through the gate tile, which carries green and would
        // otherwise cut the wire that feeds it.
        board.set_tile(390, 318, wired(136, Wire::Red));
        board.set_tile(390, 317, wired(136, Wire::Blue));
        for x in 390..=400 {
            wire(&mut board, x, 318, TileFlags::WIRE_RED);
            wire(&mut board, x, 317, TileFlags::WIRE_BLUE);
        }

        let mut upper = Tile::framed(LAMP, 0, 0);
        upper.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(400, 317, upper);
        let mut lower = Tile::framed(LAMP, 0, 0);
        lower.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(400, 318, lower);
        let mut gate = Tile::framed(GATE, 0, 0);
        gate.flags.set(TileFlags::WIRE_GREEN, true);
        board.set_tile(400, 319, gate);
        for x in 401..=420 {
            wire(&mut board, x, 319, TileFlags::WIRE_GREEN);
        }
        let mut trap = Tile::framed(TRAPS, 0, 0);
        trap.flags.set(TileFlags::WIRE_GREEN, true);
        board.set_tile(420, 319, trap);

        let done = HashSet::new();
        // One lamp: an AND gate says nothing.
        let fired = hit_switch(&mut board, 390, 318);
        assert_eq!(
            fired.lamps,
            vec![(400, 318)],
            "the red lever reached the lower lamp"
        );
        assert!(
            check_logic_gate(&mut board, 400, 318, &done, &mut rng).is_none(),
            "one of two lamps is not an AND"
        );

        // Both: the gate flips and fires, and its own circuit reaches the trap.
        let fired = hit_switch(&mut board, 390, 317);
        assert_eq!(
            fired.lamps,
            vec![(400, 317)],
            "the blue lever reached the upper lamp"
        );
        let result = check_logic_gate(&mut board, 400, 317, &done, &mut rng)
            .expect("both lamps lit is an AND");
        assert!(result.fires);
        assert_eq!(result.at, (400, 319));

        let onward = trip_wire(&mut board, 400, 319);
        assert_eq!(
            onward.traps,
            vec![(420, 319)],
            "and the gate reached the trap"
        );
    }

    /// A direct click on a Party Monolith toggles it — `Player.cs`'s own `tile.type == 455`
    /// branch, folded here into the same `HIT_SWITCH` packet a lever or switch uses. No wire
    /// needed at all: `is_trigger` alone is what makes it reachable by a bare click.
    #[test]
    fn clicking_a_party_monolith_toggles_it() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, Tile::framed(PARTY_MONOLITH, 0, 0));

        let fired = hit_switch(&mut board, 100, 100);
        assert!(fired.party_monolith, "a direct click should reach it");
        assert!(
            fired.changed.is_empty(),
            "a monolith has no frame of its own to flip"
        );
    }

    /// A wire signal reaching a *different* Party Monolith than the one directly clicked also
    /// toggles it — `Wiring.cs:2037`'s own `act`-equivalent case, a separate path from the direct
    /// click above.
    #[test]
    fn a_wire_signal_reaching_a_party_monolith_toggles_it() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red)); // a lever
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut monolith = Tile::framed(PARTY_MONOLITH, 0, 0);
        monolith.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(105, 100, monolith);

        let fired = hit_switch(&mut board, 100, 100);
        assert!(fired.party_monolith, "the current should have reached it");
    }

    /// Hitting anything else at all leaves the flag alone, so a caller never has to guess whether
    /// an unrelated switch's own `Fired` happens to carry a stale `true` from elsewhere.
    #[test]
    fn an_unrelated_switch_does_not_report_a_party_monolith() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        let fired = hit_switch(&mut board, 100, 100);
        assert!(!fired.party_monolith);
    }
}
