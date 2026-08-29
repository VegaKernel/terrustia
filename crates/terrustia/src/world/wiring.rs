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
//! * **Junction boxes**, which route the current through by frame rather than in every direction,
//!   which is what lets two circuits cross the same tile without joining into one.
//! * **Conveyor belts** and **Active/Inactive Stone Blocks**, which swap between their two states.
//! * **Traps** — darts, flames, spears, spiky balls and geysers — which hurt.
//! * **Land mines**, which explode on the spot rather than joining a circuit at all.
//! * **Statues**, which produce monsters, items or a fetched townsperson.
//! * **Doors, trapdoors and tall gates**, which change the world's shape — a shut door becomes a
//!   wider, different tile rather than merely a different frame of the same one.
//! * **Teleporters**, which swap whoever is standing on one pad with whoever is on the other.
//! * **Pumps**, which move liquid from every inlet cell a circuit reaches to every outlet.
//! * **Timers**, the one thing here that starts a circuit with nobody touching it.
//! * **Logic gates**, which read a stack of lamps, decide, and start a circuit of their own.
//!
//! The last two are what make wiring a machine rather than a switchboard. Almost every
//! contraption anybody builds runs off a timer, and almost every interesting one has a gate in
//! it; a server that only ran a circuit when a player hit a switch would run hardly any of them.
//!
//! Only the actuator, the junction box, the conveyor belt, the stone block, the pump, the lamp
//! and the timer are handled inside the flood, because they need nothing but the tiles. The rest
//! are *reported*: firing a trap needs a die roll, a cooldown and the projectile store; a statue
//! needs the NPC table; a teleporter needs the players; a gate needs to start a new circuit, which
//! cannot happen from inside the one that is running; a door, trapdoor or tall gate needs the real
//! function that reshapes it, which lives elsewhere in this module tree (doors, by way of
//! [`super::doors`]; trapdoors and tall gates, by way of [`super::trapdoors`] — see
//! [`Fired::trapdoors`] and [`Fired::gates`]). All of that lives on the caller, so the flood hands back which tiles it
//! reached and the caller does the work — [`trap_shot`] and [`check_logic_gate`] are the tables it
//! calls.
//!
//! What is left after all of that really is cosmetic: candles, chandeliers and the like change a
//! frame and nothing else, and a client does that for itself from the relayed hit.
//!
//! A tile the flood cannot act on still passes the current along, so a circuit through one is not
//! broken by it. A tile the circuit *started* from is not acted on at all, which is what stops a
//! timer switching itself off the first time it fires.

use std::collections::HashSet;

use terrustia_proto::tile::{Tile, TileFlags};

use super::doors::{DOOR_CLOSED, DOOR_OPEN};
use super::trapdoors::{TALL_GATE_CLOSED, TALL_GATE_OPEN, TRAPDOOR_CLOSED, TRAPDOOR_OPEN};

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
///
/// `HitSwitch` (`Wiring.cs:259`) handles several more tiles than the ones that flood a circuit in
/// the ordinary way, which is why they are listed here even though [`hit_switch`]'s own handling
/// of them is bespoke rather than a plain [`run_from`]:
/// * [`LEVER`] and [`DETONATOR`] are two tiles wide and flip their *own* frame — both cells of it
///   — before flooding from the pair, rather than flooding from the clicked tile alone.
/// * [`FAKE_CONTAINER`] and [`FAKE_CONTAINER2`] are a trapped chest's real footprint, found the
///   same way, but nothing about them changes: they only relay the click into a flood.
/// * [`LAND_MINE`] and [`GEYSER`] do not flood at all when clicked directly — they fire on the
///   spot, the same as a wire reaching one does.
pub fn is_trigger(block: u16) -> bool {
    matches!(
        block,
        // Switch, lever, a track switch, the pressure plates, and the Party Monolith.
        135 | 136
            | 144
            | MINECART_TRACK
            | 423
            | 428
            | 440
            | 442
            | 476
            | PARTY_MONOLITH
            | LEVER
            | DETONATOR
            | FAKE_CONTAINER
            | FAKE_CONTAINER2
            | LAND_MINE
            | GEYSER
    )
}

/// `TileID.Lever` — a switch two tiles wide, whose own frame remembers which way it is thrown
/// rather than using `frameY` the way [`flips`]'s tiles do. See [`hit_switch`]'s own case for it.
const LEVER: u16 = 132;
/// `TileID.Detonator` — clicked exactly like a [`LEVER`]; the only difference vanilla makes is a
/// cooldown record (`CheckMech`) this project has no use for, since nothing here reads it back.
const DETONATOR: u16 = 411;
/// `TileID.FakeContainers` — a chest that is really a trap, styled to look ordinary. Clicking it
/// finds its true footprint and floods from there; nothing about the chest itself changes.
const FAKE_CONTAINER: u16 = 441;
/// `TileID.FakeContainers2`, the other trapped-chest style, found and handled the same way.
const FAKE_CONTAINER2: u16 = 468;
/// `TileID.LandMine` — explodes the instant it is hit, whether by a click or by a circuit reaching
/// it; it is never itself a thing wired to something else the way a switch is.
const LAND_MINE: u16 = 210;

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

    /// The world's surface line, in tile `y`. [`actuation_allowed`]'s own Lihzahrd guard reads it
    /// to decide "below the surface", the same test `DeActive` makes against `Main.worldSurface`.
    ///
    /// Defaulted to `0` — every ordinary tile then counts as underground — rather than making
    /// every implementor supply a real value just to compile. That default is the *protective*
    /// side of the guard: paired with [`Self::downed_plantera`]'s own default, an implementation
    /// that does not override either one gets a Lihzahrd wall that can never be actuated away at
    /// all, which is strictly closer to vanilla than this project's previous behaviour (no guard
    /// whatsoever) rather than further from it. A real implementation should still override this
    /// with the world's actual surface line so the guard lifts below it and after Plantera falls.
    fn surface_y(&self) -> i32 {
        0
    }

    /// Whether this world has downed Plantera yet — `NPC.downedPlantBoss` in vanilla.
    ///
    /// Defaulted to `false` for the same reason [`Self::surface_y`] is: it is the conservative
    /// answer, keeping the temple's own walls protected until a real implementation overrides it.
    fn downed_plantera(&self) -> bool {
        false
    }
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
    /// Buried land mines that went off, whether from a direct click or a circuit reaching one.
    ///
    /// Kept apart from `mines`: a land mine (tile 210) is a different tile from the wired
    /// Explosives (141) `mines` reports, has already been cleared from the world by the time it is
    /// listed here (`ExplodeMine`'s own `KillTile`), and throws a different projectile at a
    /// different damage — the caller needs to tell the two apart to spawn the right one.
    pub land_mines: Vec<(i32, i32)>,
    /// Doors the current reached, whichever of their tiles it happened to touch — for the caller
    /// to resolve into `doors::open`/`doors::close`, exactly as a click does (`Wiring.cs:1464-
    /// 1491`), a shut one picking a random swing side and a closing one always forced.
    pub doors: Vec<(i32, i32)>,
    /// Trapdoors the current reached (`Wiring.cs:1443-1456`, `WorldGen.ShiftTrapdoor`).
    ///
    /// Reported rather than resolved here: shifting one is real domain logic — it moves between a
    /// vertical and a horizontal two-tile form depending on which side has room, and kills a
    /// cuttable plant in the way rather than refusing — which lives in
    /// [`super::trapdoors::shift_trapdoor`] now, and needs to know whether a player or NPC stands
    /// where the doorway would open, which only the caller (holding the entity table) can answer.
    pub trapdoors: Vec<(i32, i32)>,
    /// Tall gates the current reached (`Wiring.cs:1457-1463`, `WorldGen.ShiftTallGate`).
    ///
    /// Reported for the same reason `trapdoors` is: shifting one is real domain logic (a
    /// five-tile column swapping type, refused unforced while anything occupies it) that lives in
    /// [`super::trapdoors::shift_tall_gate`], which likewise needs the caller's own occupancy
    /// check.
    pub gates: Vec<(i32, i32)>,
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

    // A land mine explodes the instant it is hit — `ExplodeMine` (`Wiring.cs:3087`), which is a
    // `KillTile` and a projectile, not a circuit. No wire is involved when it is clicked directly,
    // so this returns before ever reaching `run_from`.
    if tile.block == LAND_MINE {
        world.set_tile(x, y, Tile::AIR);
        out.changed.push((x, y));
        out.land_mines.push((x, y));
        return out;
    }

    // A geyser trap fires the instant it is hit too (`GeyserTrap`, called directly from
    // `HitSwitch`'s own `case 443`) — reported the same way a wire reaching one is (`act`'s own
    // `TRAPS | GEYSER` case), so the caller's cooldown and projectile logic does not need to know
    // which path found it.
    if tile.block == GEYSER {
        out.traps.push((x, y));
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

    // A Lever or a Detonator is two tiles wide and remembers its own state in `frameX`, both
    // cells of it, rather than in `frameY` the way a Switch does. `Wiring.cs:345-377`: find the
    // pair's own anchor from whichever half was clicked, flip every cell of it that is really a
    // Lever or a Detonator (the 2x2 scan can land on a neighbour that is neither), and flood from
    // the pair rather than from the tile the player happened to click.
    if matches!(tile.block, LEVER | DETONATOR) {
        let (ax, ay, delta) = switch_anchor(tile.frame_x, tile.frame_y, x, y);
        for k in ax..ax + 2 {
            for l in ay..ay + 2 {
                let mut cell = world.tile(k, l);
                if matches!(cell.block, LEVER | DETONATOR) {
                    cell.frame_x += delta;
                    world.set_tile(k, l, cell);
                    out.changed.push((k, l));
                }
            }
        }
        run_from(world, ax, ay, 2, 2, &mut out);
        return out;
    }

    // A trapped chest is found the same way a Lever is, but nothing about the chest changes —
    // `Wiring.cs:312-325` only ever floods from its real footprint.
    if matches!(tile.block, FAKE_CONTAINER | FAKE_CONTAINER2) {
        let (ax, ay, _) = switch_anchor(tile.frame_x, tile.frame_y, x, y);
        run_from(world, ax, ay, 2, 2, &mut out);
        return out;
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

/// Find a Lever's, a Detonator's or a trapped chest's real footprint from whichever cell of it was
/// clicked, and — for the two that flip a frame — which way to flip it.
///
/// `Wiring.cs`'s own formula (`345-359` for the Lever/Detonator pair, `312-322` for a trapped
/// chest, identical but for the frame-flip step the chest never does): the clicked cell's own
/// column within the pair falls out of `frameX / 18`, taken negative and wrapped to `-1..=0` by a
/// modulo 4 that never actually reaches 4 in practice — a real two-column sprite only ever stores
/// `frameX` of `0`, `18` (off) or `36`, `54` (on) — so this recovers "which half" and "off or on"
/// in the same step: a clicked right-hand cell (`frameX` `18`/`54`) yields `-1`, wrapping the
/// anchor one tile left; a clicked "on" cell (`frameX` `36`/`54`) yields `-2`/`-3`, which the
/// `< -1` branch corrects back to `0`/`-1` while also flipping the flip direction negative, so a
/// second click turns the pair back off. The row offset has no such wrap: a chest or a Detonator
/// with a second row stacked underneath needs only the plain `frameY / 18`.
fn switch_anchor(frame_x: i16, frame_y: i16, x: i32, y: i32) -> (i32, i32, i16) {
    let mut dx = -(i32::from(frame_x) / 18);
    let mut delta: i16 = 36;
    dx %= 4;
    if dx < -1 {
        dx += 2;
        delta = -36;
    }
    let dy = -(i32::from(frame_y) / 18);
    (x + dx, y + dy, delta)
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

/// The four directions a step of the flood can take, numbered the way vanilla's own `HitWire`
/// does (`Wiring.cs:863-885`): `0` down, `1` up, `2` right, `3` left. The numbering itself is
/// only meaningful to [`junction_lets_through`] — nothing else here cares which number is which,
/// only that leaving in direction `k` and arriving from it are the same `k`.
const STEP: [(i32, i32); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];

/// Flood the current outward from a set of seeds and act on everything it reaches.
///
/// Every step remembers which of the four directions it *arrived* by, not just where it is — a
/// junction box's own frame decides which of the four is allowed to leave again, and without that
/// the box cannot do the one thing it exists for (see [`junction_lets_through`]).
///
/// **Simplified relative to vanilla's own flood**, and worth knowing about: `HitWire` tracks a
/// reference count per tile (`_toProcess`) that lets a tile already queued be reached again from a
/// second direction before it is finally visited, which matters for exactly how a wire diamond or
/// a loop back into a junction box settles. This keeps the simpler model the rest of the module
/// already used before junction boxes existed — a tile visited once by any direction is not
/// revisited by another — which is right for what a junction box is *for* (keeping two ordinary
/// crossing runs apart) and does not attempt the rarer shapes vanilla's ref-counting also handles.
fn trip(world: &mut impl WiredWorld, colour: Wire, seeds: Vec<(i32, i32)>, out: &mut Fired) {
    let mut seen: HashSet<(i32, i32)> = seeds.iter().copied().collect();
    // Every seed is treated as having "arrived" from direction 0, exactly as vanilla's own
    // `_wireDirectionList.PushBack(0)` seeds every tile the flood starts from.
    let mut queue: Vec<(i32, i32, u8)> = seeds.into_iter().map(|(x, y)| (x, y, 0)).collect();

    while let Some((x, y, arrived_via)) = queue.pop() {
        if seen.len() > MAX_CIRCUIT {
            out.truncated = true;
            break;
        }
        out.reached += 1;
        act(world, x, y, out);

        // A junction box is never itself acted on (no case for it in `act`), so its frame here is
        // still whatever it was before the line above.
        let here = world.tile(x, y);
        for (leaving_via, &(dx, dy)) in STEP.iter().enumerate() {
            let leaving_via = leaving_via as u8;
            if here.block == JUNCTION_BOX
                && !junction_lets_through(here.frame_x, arrived_via, leaving_via)
            {
                continue;
            }
            let (nx, ny) = (x + dx, y + dy);
            if nx < 2 || ny < 2 || nx >= world.width() - 2 || ny >= world.height() - 2 {
                continue;
            }
            if !colour.on(world.tile(nx, ny)) || !seen.insert((nx, ny)) {
                continue;
            }
            queue.push((nx, ny, leaving_via));
        }
    }
}

/// The junction box, `TileID.WirePipe` (`424`). Its three frame styles are three different pairs
/// of sides wired together, which is what lets two circuits cross the same tile without merging
/// into one — the whole reason anybody places one. Before this the flood had no notion of it at
/// all, so a junction box was simply open ground: every wire crossing through the same tile joined
/// into a single circuit, which is the opposite of what the piece is for.
const JUNCTION_BOX: u16 = 424;

/// Whether current arriving at a junction box from `arrived_via` may leave again via
/// `leaving_via` — transcribed from `Wiring.cs:900-928`'s own three-armed `switch` on
/// `frameX / 18`.
///
/// Style 0 (the straight frame) only ever lets current leave the way it was already travelling, so
/// a vertical run and a horizontal run cross the same tile without touching each other. Styles 1
/// and 2 are the two elbow frames: each is a pair of turns that, again, do not touch each other —
/// style 1 pairs *down* with *left* and *up* with *right*; style 2 pairs *down* with *right* and
/// *up* with *left*. A style outside 0..=2 (there is none in the real game, but a frame is just a
/// number) lets everything through rather than trapping a circuit that reaches it.
fn junction_lets_through(frame_x: i16, arrived_via: u8, leaving_via: u8) -> bool {
    match frame_x / 18 {
        0 => leaving_via == arrived_via,
        1 => matches!(
            (arrived_via, leaving_via),
            (0, 3) | (3, 0) | (1, 2) | (2, 1)
        ),
        2 => matches!(
            (arrived_via, leaving_via),
            (0, 2) | (2, 0) | (1, 3) | (3, 1)
        ),
        _ => true,
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
        // read elsewhere in `Minecart.cs` for cart collision — nothing to do with switching, and
        // `FlipSwitchTrack`'s own `switch` has no `case` for this group at all) is out of scope
        // here. The other two groups really are both switched by `FlipSwitchTrack`, in two
        // different ways: ordinary track (`trackType == 0`, vanilla's own array default) is
        // `case 0`'s simple front/back swap, handled directly below; booster-pad frames
        // (`trackType == 2`, frames 30-35) are `case 2`, a real, separate mechanism — see
        // [`booster_switch_target`]'s own doc for what it does and why.
        //
        // **Correction**: an earlier version of this comment claimed booster frames were "reframed
        // by a hammer, not wire" — that was wrong, confirmed directly against `Minecart.cs:1320-
        // 1323`, which calls `FrameTrack(i, j, pound: true, mute: true)` from inside
        // `FlipSwitchTrack` itself, reached only from `Wiring.cs`'s own tile-314 case above (never
        // from a hammer). A wire signal genuinely does reach and toggle a booster-pad track, the
        // same as it does an ordinary one.
        //
        // A frame in the ordinary group only actually has something to swap to if its own
        // `BackTrack()` (`frameY`) was ever set — not every track tile has a second track stacked
        // underneath it — matching vanilla's own `BackTrack() != -1` guard.
        if track_type(tile.frame_x) == 0 && tile.frame_y != -1 {
            if !out.skipped.insert((x, y)) {
                return;
            }
            let mut flipped = tile;
            flipped.frame_x = tile.frame_y;
            flipped.frame_y = tile.frame_x;
            world.set_tile(x, y, flipped);
            out.changed.push((x, y));
        } else if let Some(new_frame) = booster_switch_target(world, x, y, tile.frame_x) {
            if !out.skipped.insert((x, y)) {
                return;
            }
            let mut flipped = tile;
            flipped.frame_x = new_frame;
            world.set_tile(x, y, flipped);
            out.changed.push((x, y));
        }
        return;
    }
    // An actuator toggles its block between solid and passable. It runs whether or not the block
    // is active, which is the only way a block that has been actuated away can ever come back.
    //
    // Coming *back* (`ReActive`, `Wiring.cs:3238-3246`) has no guard at all in vanilla — it is
    // going the other way, hiding a solid block (`DeActive`, `3208-3236`), that is refused for a
    // handful of reasons; see [`actuation_allowed`]. Without this an actuator on a Lihzahrd temple
    // wall let a player walk in before Plantera the way the boss is meant to gate, and an
    // actuator on a door/gate/track/golf-hole did something vanilla never lets it do at all.
    if tile.flags.has(TileFlags::ACTUATOR)
        && (tile.flags.has(TileFlags::ACTUATED) || actuation_allowed(world, y, tile))
    {
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
    // A land mine a circuit reaches explodes exactly as one that is clicked directly does
    // (`Wiring.cs`'s own per-tile dispatch, `case 210: ExplodeMine(i, j);` — the same call
    // `hit_switch`'s own `LAND_MINE` case makes).
    if tile.is_active() && tile.block == LAND_MINE {
        if !out.skipped.insert((x, y)) {
            return;
        }
        world.set_tile(x, y, Tile::AIR);
        out.changed.push((x, y));
        out.land_mines.push((x, y));
        return;
    }
    // A conveyor belt swaps direction — `Wiring.cs:1017-1032`'s own `case 421`/`case 422`, a plain
    // type swap with no frame or anchor math at all. Vanilla skips the swap while the belt also
    // carries an actuator; this project's own actuator toggle above already ran this tick, so
    // `ACTUATOR` here means exactly what vanilla's guard checks.
    if tile.is_active() && matches!(tile.block, CONVEYOR_LEFT | CONVEYOR_RIGHT) {
        if !out.skipped.insert((x, y)) {
            return;
        }
        if !tile.flags.has(TileFlags::ACTUATOR) {
            let mut flipped = tile;
            flipped.block = if tile.block == CONVEYOR_LEFT {
                CONVEYOR_RIGHT
            } else {
                CONVEYOR_LEFT
            };
            world.set_tile(x, y, flipped);
            out.changed.push((x, y));
        }
        return;
    }
    // Active Stone Block hides itself; Inactive Stone Block always comes back solid —
    // `Wiring.cs:1426-1442`. Vanilla also refuses to hide a block whose absence would leave
    // something above it unsupported (`CanKillTile`, and a `PreventsActuationUnder` check on the
    // tile directly above); this project has neither piece of machinery yet, so it only checks
    // that there is *something* above to stand on, which is the common case that check exists for.
    if tile.is_active() && tile.block == ACTIVE_STONE {
        if !out.skipped.insert((x, y)) {
            return;
        }
        if world.tile(x, y - 1).is_active() {
            let mut hidden = tile;
            hidden.block = INACTIVE_STONE;
            world.set_tile(x, y, hidden);
            out.changed.push((x, y));
        }
        return;
    }
    if tile.is_active() && tile.block == INACTIVE_STONE {
        if !out.skipped.insert((x, y)) {
            return;
        }
        let mut shown = tile;
        shown.block = ACTIVE_STONE;
        world.set_tile(x, y, shown);
        out.changed.push((x, y));
        return;
    }
    // Doors, trapdoors and tall gates all change the *shape* of the world, not merely a frame —
    // opening a shut door replaces one tile column with two, for instance — which needs the real
    // functions this project already has for doors (`doors::open`/`doors::close`) or, for
    // trapdoors and tall gates, functions nobody has ported yet. Either way that is not something
    // a generic `WiredWorld` can resolve on the spot, so these are reported for the caller.
    if tile.is_active() && matches!(tile.block, DOOR_CLOSED | DOOR_OPEN) {
        out.doors.push((x, y));
    }
    if tile.is_active() && matches!(tile.block, TRAPDOOR_CLOSED | TRAPDOOR_OPEN) {
        out.trapdoors.push((x, y));
    }
    if tile.is_active() && matches!(tile.block, TALL_GATE_CLOSED | TALL_GATE_OPEN) {
        out.gates.push((x, y));
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

/// `TileID.Conveyorbelt` and its reverse — swapped for each other on a wire signal.
const CONVEYOR_LEFT: u16 = 421;
const CONVEYOR_RIGHT: u16 = 422;
/// `TileID.ActiveStoneBlock`, visible and solid.
const ACTIVE_STONE: u16 = 130;
/// `TileID.InactiveStoneBlock`, invisible and passable until a signal brings it back.
const INACTIVE_STONE: u16 = 131;
/// `TileID.LihzahrdBrick` — the temple's own walls, which `actuation_allowed` refuses to actuate
/// away before Plantera falls while still underground, exactly as `DeActive` does.
const LIHZAHRD_BRICK: u16 = 226;
/// `TileID.Bubble` — a decorative water/lava bubble; listed in `DeActive`'s own exclusion switch
/// even though it is not in `Main.tileSolid` to begin with, so excluding it here changes nothing
/// in practice but matches the source line for line.
const BUBBLE: u16 = 379;
/// `TileID.GolfHole` — also one of `DeActive`'s excluded types, and already one of [`is_trigger`]'s
/// own (a golf hole is hit directly to sink a ball, unrelated to this).
const GOLF_HOLE: u16 = 476;

/// Whether a solid tile with an actuator on it may be hidden — `Wiring.cs:3208-3236`'s own
/// `DeActive`, minus the two pieces this project has no equivalent of yet (`WorldGen.CanKillTile`
/// and `TileID.Sets.PreventsActuationUnder`, both about whether removing *this* tile would strand
/// something built on top of it — narrower checks than "is there solid ground" and not modelled
/// here, which is a real simplification, not an oversight: everything a manually-placed actuator
/// contraption cares about is covered by the checks that are here).
///
/// Coming back the other way (hidden to solid) has none of these guards in vanilla — `ReActive`
/// is unconditional — so this is only ever consulted before hiding a tile, never before showing
/// one again.
fn actuation_allowed(world: &impl WiredWorld, y: i32, tile: Tile) -> bool {
    // The temple's own walls cannot be actuated away before Plantera is down, while still
    // underground — the one thing standing between an early visit and the boss meant to gate it.
    if tile.block == LIHZAHRD_BRICK && y > world.surface_y() && !world.downed_plantera() {
        return false;
    }
    // A handful of types are never actuatable at all, whatever `tile_solid` says about them.
    if matches!(
        tile.block,
        MINECART_TRACK
            | BUBBLE
            | TRAPDOOR_CLOSED
            | TRAPDOOR_OPEN
            | TALL_GATE_CLOSED
            | TALL_GATE_OPEN
            | GOLF_HOLE
    ) {
        return false;
    }
    // Everything else has to actually be solid to have anything to hide.
    terrustia_proto::tile_solid::solid(tile.block)
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
/// array default, so every frame not explicitly listed below is this) is ordinary track, switched by
/// `FlipSwitchTrack`'s `case 0`. `1` (frames 20-23) is a small set of dead-end/bumper pieces
/// `Minecart.cs` reads for cart collision physics elsewhere — `FlipSwitchTrack`'s `switch` has no
/// case for this group, so a wire signal reaching one does nothing, and that is real vanilla
/// behaviour, not a gap. `2` (frames 30-35) is the six booster-pad frames — also genuinely switched
/// by wire, via `FlipSwitchTrack`'s `case 2`; see [`booster_switch_target`] for what that does.
fn track_type(frame: i16) -> u8 {
    match frame {
        20..=23 => 1,
        30..=35 => 2,
        _ => 0,
    }
}

/// One booster-frame pair: the two frame ids, and the two neighbour offsets (relative to the tile
/// itself) that must both be track for [`booster_switch_target`] to switch between them.
type BoosterPair = (i16, i16, (i32, i32), (i32, i32));

/// What `FlipSwitchTrack`'s `case 2` (`Minecart.cs:1320-1323`, `FrameTrack(i, j, pound: true, mute:
/// true)`) does to a booster-pad tile — derived by hand from the full algorithm rather than
/// transcribed line-for-line, and narrower than it by design; see the "Investigate proportionality"
/// reasoning this function departs from written up in `plan.md`'s corrections section for this row.
///
/// `FrameTrack` unpounded is vanilla's general track auto-tiling: a lookup keyed by which of a
/// tile's six diagonal/side neighbours (`GetNearbyTilesSetLookupIndex` — up-left, left, down-left,
/// up-right, right, down-right; never straight up or down) are themselves track, filtered against
/// every one of the 36 track frames' own `(leftSideConnection, rightSideConnection)` pair
/// (`_trackSwitchOptions`, built once in `Minecart.Initialize`). Read in full before deciding scope,
/// per the task's own instruction — porting it whole would mean carrying all of `_trackSwitchOptions`
/// and `_leftSideConnection`/`_rightSideConnection` for no real player benefit, since almost every
/// wired contraption anyone actually builds hits `case 0` (ordinary track), not this one.
///
/// For the six booster-pad frames specifically (`_trackType == 2`) the general algorithm collapses
/// to something much smaller, worked out directly from `Minecart.Initialize`'s own table rather than
/// guessed: all six have real (non-`-1`) connections on both sides, so none is ever an "end" piece,
/// and the three same-shape pairs — `(30, 31)` flat, `(32, 34)` down-right slope, `(33, 35)`
/// down-left slope — each share one identical `(leftSideConnection, rightSideConnection)` pair,
/// differing only by `_boostLeft` (which way the pad boosts). Because a pair's two members always
/// have identical connections, `FrameTrack`'s own "find a different-shaped candidate" search
/// (its `flag3` branch) can never succeed for a booster tile; it always falls through to "step to
/// the next matching entry in frame order", and since only a pair's own two members ever qualify as
/// candidates together, that step always means: swap to the other member of the pair. A pair
/// qualifies at all only when both of its own two required neighbour cells actually hold track —
/// `(30, 31)` needs track directly left and right; `(32, 34)` needs it up-left and down-right;
/// `(33, 35)` down-left and up-right — otherwise `FrameTrack` returns `false` and nothing changes,
/// which this returns `None` for.
///
/// Two things the general algorithm also does that this does not, both disclosed rather than
/// silently dropped: it resolves a `BackTrack` too, but the same derivation shows a booster pair's
/// own search for one can never succeed either (`num5` stays `-1` in vanilla's own code), so never
/// touching `frame_y` here matches the real outcome rather than shortcutting past it. And vanilla's
/// fallback for a tile whose own stored frame does not appear in its neighbour-derived candidate
/// list at all — a world where the frame and the actual surrounding tiles have gone out of sync,
/// which ordinary hammering or wiring cannot produce — resets `FrontTrack` to `0`, plain straight
/// track; that fallback is not reproduced, and a tile in that state (not reachable through normal
/// play) is left exactly as it is.
fn booster_switch_target(world: &impl WiredWorld, x: i32, y: i32, frame_x: i16) -> Option<i16> {
    // Each pair's two frames, and the two neighbour offsets (relative to the tile itself) that
    // must both be track for the pair to be switchable at all.
    const PAIRS: [BoosterPair; 3] = [
        (30, 31, (-1, 0), (1, 0)),
        (32, 34, (-1, -1), (1, 1)),
        (33, 35, (-1, 1), (1, -1)),
    ];
    let is_track = |dx: i32, dy: i32| world.tile(x + dx, y + dy).block == MINECART_TRACK;
    for (a, b, left, right) in PAIRS {
        if frame_x != a && frame_x != b {
            continue;
        }
        return if is_track(left.0, left.1) && is_track(right.0, right.1) {
            Some(if frame_x == a { b } else { a })
        } else {
            None
        };
    }
    None
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

    /// An actuator cannot hide a Lihzahrd temple wall before Plantera is down, while it is still
    /// underground — `DeActive`'s own guard (`Wiring.cs:3210`), which stands between an early
    /// temple visit and the boss meant to gate it.
    ///
    /// Fails before the fix: the actuator toggle had no guard at all, so it hid the wall the very
    /// first hit — a `Board` (this test's `WiredWorld`) never overrides `surface_y`/
    /// `downed_plantera`, so it gets the trait's own conservative defaults, exactly like a real
    /// implementation would before being wired up to the world's actual state.
    #[test]
    fn an_actuator_cannot_hide_lihzahrd_brick_pre_plantera() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut wall = wired(LIHZAHRD_BRICK, Wire::Red);
        wall.flags.set(TileFlags::ACTUATOR, true);
        board.set_tile(105, 100, wall);

        hit_switch(&mut board, 100, 100);
        assert!(
            !board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "the wall should still be solid"
        );
    }

    /// An actuator on something that is not solid to begin with — a torch, here — has nothing to
    /// hide, and `DeActive`'s own `flag` computation (`tileSolid && !NotReallySolid`) never lets
    /// it try.
    #[test]
    fn an_actuator_on_a_non_solid_tile_does_nothing() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut torch = wired(4, Wire::Red); // torch: not in the solid set.
        torch.flags.set(TileFlags::ACTUATOR, true);
        board.set_tile(105, 100, torch);

        hit_switch(&mut board, 100, 100);
        assert!(!board.tile(105, 100).flags.has(TileFlags::ACTUATED));
    }

    /// A minecart track is one of `DeActive`'s explicit exclusions — never actuatable, whatever
    /// `tile_solid` says about the type.
    #[test]
    fn an_actuator_on_a_minecart_track_does_nothing() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut track = wired(MINECART_TRACK, Wire::Red);
        track.frame_x = 1;
        track.frame_y = -1;
        track.flags.set(TileFlags::ACTUATOR, true);
        board.set_tile(105, 100, track);

        hit_switch(&mut board, 100, 100);
        assert!(!board.tile(105, 100).flags.has(TileFlags::ACTUATED));
    }

    /// Coming back the other way has no guard at all — an already-hidden tile always returns to
    /// solid, exactly the asymmetry `ReActive` has and `DeActive` does not.
    #[test]
    fn an_already_hidden_tile_always_reactivates() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut wall = wired(LIHZAHRD_BRICK, Wire::Red);
        wall.flags.set(TileFlags::ACTUATOR, true);
        wall.flags.set(TileFlags::ACTUATED, true); // already hidden
        board.set_tile(105, 100, wall);

        hit_switch(&mut board, 100, 100);
        assert!(
            !board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "should have come back solid with no guard to stop it"
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

    /// A dead-end/bumper piece (frame 20, `trackType == 1`, read elsewhere in `Minecart.cs` for
    /// cart collision, nothing to do with switching) is left alone even with a real value stored in
    /// its own back track: `FlipSwitchTrack`'s `switch` has no `case` for `trackType == 1` at all,
    /// unlike `trackType == 0` (`case 0`, the front/back swap above) and `trackType == 2` (`case 2`,
    /// booster pads — see the tests below, not "left alone" the way this project's own comment used
    /// to wrongly claim).
    #[test]
    fn a_bumper_track_frame_is_not_touched_by_a_wired_hit() {
        let mut board = Board(HashMap::new());
        let mut track = wired(MINECART_TRACK, Wire::Red);
        track.frame_x = 20;
        track.frame_y = 1; // a real stored value — still must not swap.
        board.set_tile(100, 100, track);

        hit_switch(&mut board, 100, 100);

        let after = board.tile(100, 100);
        assert_eq!(after.frame_x, 20);
        assert_eq!(after.frame_y, 1);
    }

    /// A wired booster-pad track (`trackType == 2`) genuinely is switched by wire — `Minecart.cs`'s
    /// `FlipSwitchTrack` has a real `case 2`, calling `FrameTrack(i, j, pound: true, mute: true)`.
    /// This project's own earlier comment claiming boosters were "reframed by a hammer, not wire"
    /// was wrong; this is the regression test for that gap. Frame 30 and 31 are the flat pair
    /// (`_boostLeft` false/true, sharing one connection shape) — properly connected with track
    /// directly left and right, the pair swaps.
    ///
    /// Fails on the code before this fix: without `booster_switch_target`, `act`'s own
    /// `MINECART_TRACK` case only ever handles `trackType == 0`, so a booster tile's `frame_x`
    /// would still read `30` after `hit_switch` instead of the swapped `31`.
    #[test]
    fn a_wired_booster_pad_swaps_between_its_pair_when_properly_connected() {
        let mut board = Board(HashMap::new());
        let mut booster = wired(MINECART_TRACK, Wire::Red);
        booster.frame_x = 30;
        board.set_tile(100, 100, booster);
        // Real track directly left and right, matching the flat pair's required neighbours.
        board.set_tile(99, 100, Tile::framed(MINECART_TRACK, 1, -1));
        board.set_tile(101, 100, Tile::framed(MINECART_TRACK, 1, -1));

        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(100, 100).frame_x, 31, "boosts the other way now");

        hit_switch(&mut board, 100, 100);
        assert_eq!(
            board.tile(100, 100).frame_x,
            30,
            "and flips back just as cleanly"
        );
    }

    /// The two sloped booster pairs swap the same way, given the neighbours their own shape needs:
    /// `(32, 34)` up-left and down-right; `(33, 35)` down-left and up-right.
    #[test]
    fn the_sloped_booster_pairs_swap_given_their_own_required_neighbours() {
        let cases: &[BoosterPair] = &[(32, 34, (-1, -1), (1, 1)), (33, 35, (-1, 1), (1, -1))];
        for &(a, b, left, right) in cases {
            let mut board = Board(HashMap::new());
            let mut booster = wired(MINECART_TRACK, Wire::Red);
            booster.frame_x = a;
            board.set_tile(100, 100, booster);
            board.set_tile(
                100 + left.0,
                100 + left.1,
                Tile::framed(MINECART_TRACK, 1, -1),
            );
            board.set_tile(
                100 + right.0,
                100 + right.1,
                Tile::framed(MINECART_TRACK, 1, -1),
            );

            hit_switch(&mut board, 100, 100);
            assert_eq!(board.tile(100, 100).frame_x, b, "pair ({a}, {b})");
        }
    }

    /// A booster pad with nothing around it does not swap at all: vanilla's own algorithm requires
    /// both of a pair's specific neighbour cells to actually be track before `FrameTrack` will
    /// change anything, and this is that guard, not a gap.
    #[test]
    fn an_unconnected_booster_pad_does_not_swap() {
        let mut board = Board(HashMap::new());
        let mut booster = wired(MINECART_TRACK, Wire::Red);
        booster.frame_x = 30;
        board.set_tile(100, 100, booster);
        // No neighbouring track tiles at all.

        hit_switch(&mut board, 100, 100);
        assert_eq!(
            board.tile(100, 100).frame_x,
            30,
            "nothing to swap to without the required neighbours"
        );
    }

    /// A booster pad on two wire colours swaps once, not twice — the same `skipped` guard that
    /// stops a two-colour lamp or ordinary track tile double-toggling back to where it started.
    #[test]
    fn a_booster_pad_on_two_colours_swaps_once() {
        let mut board = Board(HashMap::new());
        // Wire approaches from above, so the booster's own left/right track neighbours (its
        // switching requirement) stay free of the wire run itself.
        let mut lever = wired(136, Wire::Red);
        lever.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(100, 90, lever);
        for y in 91..100 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            wire.flags.set(TileFlags::WIRE_BLUE, true);
            board.set_tile(100, y, wire);
        }
        let mut booster = Tile::framed(MINECART_TRACK, 30, -1);
        booster.flags.set(TileFlags::WIRE_RED, true);
        booster.flags.set(TileFlags::WIRE_BLUE, true);
        board.set_tile(100, 100, booster);
        board.set_tile(99, 100, Tile::framed(MINECART_TRACK, 1, -1));
        board.set_tile(101, 100, Tile::framed(MINECART_TRACK, 1, -1));

        hit_switch(&mut board, 100, 90);
        assert_eq!(
            board.tile(100, 100).frame_x,
            31,
            "swapped once, not flipped back by the second colour's pass"
        );
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

    /// A real Lever (`TileID.Lever`, 132) is two tiles wide and flips *both* halves of its own
    /// frame before flooding from the pair — `Wiring.cs:345-377`, a different mechanism from the
    /// single-tile `frameY` flip a Switch (136) uses.
    ///
    /// Fails on the code before this fix: `is_trigger` did not recognise 132 at all, so
    /// `hit_switch` returned immediately without touching the lever's frame or the actuator it is
    /// wired to — "a Lever currently does NOTHING", as the audit finding put it.
    #[test]
    fn a_lever_flips_both_halves_and_actuates() {
        let mut board = Board(HashMap::new());
        let mut left = Tile::framed(LEVER, 0, 0);
        left.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(100, 100, left);
        let mut right = Tile::framed(LEVER, 18, 0);
        right.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(101, 100, right);
        for x in 102..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, actuated(Wire::Red));

        // Click the right-hand half; the anchor should still be found at the left one.
        hit_switch(&mut board, 101, 100);
        assert_eq!(board.tile(100, 100).frame_x, 36, "left half flipped on");
        assert_eq!(board.tile(101, 100).frame_x, 54, "right half flipped on");
        assert!(
            board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "the circuit it starts should have run"
        );

        // Click it again, from the same half, and it flips back off.
        hit_switch(&mut board, 101, 100);
        assert_eq!(board.tile(100, 100).frame_x, 0);
        assert_eq!(board.tile(101, 100).frame_x, 18);
        assert!(!board.tile(105, 100).flags.has(TileFlags::ACTUATED));
    }

    /// A land mine explodes the instant it is hit — `ExplodeMine` — rather than flooding a
    /// circuit: no wire is involved at all when it is clicked directly.
    ///
    /// Fails before the fix: 210 was not in `is_trigger`, so hitting a buried land mine did
    /// nothing.
    #[test]
    fn a_land_mine_explodes_on_a_direct_hit() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, Tile::framed(LAND_MINE, 0, 0));

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.land_mines, vec![(100, 100)]);
        assert!(!board.tile(100, 100).is_active(), "the mine tile is gone");
        assert!(fired.reached == 0, "no circuit ran for it");
    }

    /// A land mine a circuit reaches also explodes — `Wiring.cs`'s own per-tile dispatch has a
    /// `case 210` calling the very same `ExplodeMine`, separate from `HitSwitch`'s direct click.
    #[test]
    fn a_land_mine_reached_by_a_circuit_also_explodes() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut mine = Tile::framed(LAND_MINE, 0, 0);
        mine.flags.set(TileFlags::WIRE_RED, true);
        board.set_tile(105, 100, mine);

        let fired = hit_switch(&mut board, 100, 100);
        assert_eq!(fired.land_mines, vec![(105, 100)]);
        assert!(!board.tile(105, 100).is_active());
    }

    /// A geyser trap fires the instant it is hit directly, the same as a land mine — but unlike a
    /// land mine it is *reported*, not resolved on the spot, exactly the way a wire reaching one
    /// is (`act`'s own `TRAPS | GEYSER` case), so the caller's cooldown and projectile logic does
    /// not need to know which path found it.
    ///
    /// Fails before the fix: 443 was not in `is_trigger`, so clicking a geyser trap did nothing.
    #[test]
    fn a_geyser_fires_on_a_direct_hit() {
        let mut board = Board(HashMap::new());
        board.set_tile(300, 400, Tile::framed(GEYSER, 0, 0));

        let fired = hit_switch(&mut board, 300, 400);
        assert_eq!(fired.traps, vec![(300, 400)]);
        assert_eq!(
            board.tile(300, 400).block,
            GEYSER,
            "firing it does not change the tile"
        );
    }

    /// A trapped chest (`TileID.FakeContainers`) is styled to look like an ordinary one, but
    /// clicking it finds its real two-by-two footprint and floods from there — `Wiring.cs:312-325`
    /// — regardless of which of the four cells was actually clicked.
    ///
    /// Fails before the fix: 441 was not in `is_trigger`, so clicking a trapped chest did nothing.
    #[test]
    fn a_trapped_chest_floods_from_its_real_footprint() {
        let mut board = Board(HashMap::new());
        for dx in 0..2i32 {
            for dy in 0..2i32 {
                let mut cell = Tile::framed(FAKE_CONTAINER, dx as i16 * 18, dy as i16 * 18);
                cell.flags.set(TileFlags::WIRE_RED, true);
                board.set_tile(100 + dx, 100 + dy, cell);
            }
        }
        for x in 102..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, actuated(Wire::Red));

        // Click the bottom-right cell — the anchor should still be found at the top-left one.
        let fired = hit_switch(&mut board, 101, 101);
        assert!(
            board.tile(105, 100).flags.has(TileFlags::ACTUATED),
            "the flood should have started from the chest's real anchor"
        );
        for dx in 0..2i32 {
            for dy in 0..2i32 {
                assert_eq!(
                    board.tile(100 + dx, 100 + dy).block,
                    FAKE_CONTAINER,
                    "the chest itself never changes"
                );
            }
        }
        assert!(fired.changed.contains(&(105, 100)));
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

    /// A straight junction box (frame style 0) lets a horizontal run and a vertical run of the
    /// *same* colour cross the same tile without joining into one circuit — the whole reason
    /// anybody places one.
    ///
    /// Fails on the code before this fix: with no routing at all, the flood left every direction
    /// open at the junction box, so hitting either switch actuated *both* blocks instead of only
    /// the one on its own line.
    #[test]
    fn a_straight_junction_box_keeps_two_crossing_circuits_apart() {
        let mut board = Board(HashMap::new());
        // A horizontal line: switch A at x=80, through the box at (100,100), to a block at x=120.
        board.set_tile(80, 100, wired(136, Wire::Red));
        for x in 81..=120 {
            if x == 100 {
                continue;
            }
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(120, 100, actuated(Wire::Red));
        // A vertical line: switch B at y=80, through the same box, to a block at y=120.
        board.set_tile(100, 80, wired(136, Wire::Red));
        for y in 81..=120 {
            if y == 100 {
                continue;
            }
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(100, y, wire);
        }
        board.set_tile(100, 120, actuated(Wire::Red));
        // The box itself, straight style, carrying the colour both lines share.
        let mut junction = wired(JUNCTION_BOX, Wire::Red);
        junction.frame_x = 0;
        board.set_tile(100, 100, junction);

        hit_switch(&mut board, 80, 100);
        assert!(
            board.tile(120, 100).flags.has(TileFlags::ACTUATED),
            "switch A's own line should have run"
        );
        assert!(
            !board.tile(100, 120).flags.has(TileFlags::ACTUATED),
            "but not have leaked into the crossing vertical line"
        );

        hit_switch(&mut board, 100, 80);
        assert!(
            board.tile(100, 120).flags.has(TileFlags::ACTUATED),
            "switch B's own line should have run"
        );
    }

    /// An elbow junction box (frame style 1) connects exactly one pair of sides — arriving from
    /// above and leaving to the left, here — and nothing else: not straight through, not to the
    /// opposite elbow.
    ///
    /// Fails on the code before this fix, which had no routing at all: the current would have
    /// reached every one of the three other blocks, not only the one the elbow actually connects.
    #[test]
    fn an_elbow_junction_box_connects_only_its_own_pair_of_sides() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 80, wired(136, Wire::Red));
        for y in 81..100 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(100, y, wire);
        }
        let mut junction = wired(JUNCTION_BOX, Wire::Red);
        junction.frame_x = 18; // style 1: down pairs with left.
        board.set_tile(100, 100, junction);
        // The three sides that should each carry their own separate wire and block: left (the
        // one this style actually connects to "down"), right, and straight on down.
        for x in 90..100 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(90, 100, actuated(Wire::Red));
        for x in 101..110 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(110, 100, actuated(Wire::Red));
        for y in 101..110 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(100, y, wire);
        }
        board.set_tile(100, 110, actuated(Wire::Red));

        hit_switch(&mut board, 100, 80);
        assert!(
            board.tile(90, 100).flags.has(TileFlags::ACTUATED),
            "down should connect to left, the pair this style is"
        );
        assert!(
            !board.tile(110, 100).flags.has(TileFlags::ACTUATED),
            "but not to right"
        );
        assert!(
            !board.tile(100, 110).flags.has(TileFlags::ACTUATED),
            "nor straight on down"
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

    /// A conveyor belt reverses direction when a circuit reaches it — `Wiring.cs:1017-1032`'s own
    /// plain type swap between the two directions.
    ///
    /// Fails before the fix: the module doc used to call every tile past the reported ones
    /// "cosmetic", and the flood genuinely did nothing to a conveyor belt at all.
    #[test]
    fn a_conveyor_belt_reverses_direction() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, wired(CONVEYOR_LEFT, Wire::Red));

        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(105, 100).block, CONVEYOR_RIGHT);

        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(105, 100).block, CONVEYOR_LEFT, "and back again");
    }

    /// A conveyor belt with an actuator on it does not reverse — vanilla's own guard
    /// (`!tile.actuator()`) on the swap.
    #[test]
    fn an_actuated_conveyor_belt_does_not_reverse() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        let mut belt = wired(CONVEYOR_LEFT, Wire::Red);
        belt.flags.set(TileFlags::ACTUATOR, true);
        board.set_tile(105, 100, belt);

        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(105, 100).block, CONVEYOR_LEFT, "did not reverse");
    }

    /// Active Stone Block hides itself when a circuit reaches it, so long as there is something
    /// above it to stand on; Inactive Stone Block always comes back solid — `Wiring.cs:1426-1442`.
    #[test]
    fn stone_blocks_hide_and_reappear() {
        let mut board = Board(HashMap::new());
        board.set_tile(100, 100, wired(136, Wire::Red));
        for x in 101..105 {
            let mut wire = Tile::AIR;
            wire.flags.set(TileFlags::WIRE_RED, true);
            board.set_tile(x, 100, wire);
        }
        board.set_tile(105, 100, wired(ACTIVE_STONE, Wire::Red));
        board.set_tile(105, 99, Tile::block(1)); // something to stand on above it

        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(105, 100).block, INACTIVE_STONE, "hidden");

        hit_switch(&mut board, 100, 100);
        assert_eq!(board.tile(105, 100).block, ACTIVE_STONE, "and back again");
    }

    /// Doors, trapdoors and tall gates are reported to the caller rather than resolved on the
    /// spot — they change the world's *shape*, not just a frame, which a generic `WiredWorld`
    /// cannot do by itself.
    ///
    /// Fails before the fix: none of the three was recognised at all, so a wired door, trapdoor or
    /// gate did nothing — the exact gap the module's own doc used to call "cosmetic" and wave away.
    #[test]
    fn doors_trapdoors_and_gates_are_reported() {
        let mut board = Board(HashMap::new());
        // Three independent switch-and-target pairs, one per row, so each proves the report on
        // its own rather than relying on one flood happening to touch all three.
        for (row, block) in [
            (100i32, DOOR_CLOSED),
            (101, TRAPDOOR_CLOSED),
            (102, TALL_GATE_CLOSED),
        ] {
            board.set_tile(90, row, wired(136, Wire::Red));
            for x in 91..95 {
                let mut wire = Tile::AIR;
                wire.flags.set(TileFlags::WIRE_RED, true);
                board.set_tile(x, row, wire);
            }
            board.set_tile(95, row, wired(block, Wire::Red));
        }

        let doors = hit_switch(&mut board, 90, 100);
        assert!(doors.doors.contains(&(95, 100)));
        let trapdoors = hit_switch(&mut board, 90, 101);
        assert!(trapdoors.trapdoors.contains(&(95, 101)));
        let gates = hit_switch(&mut board, 90, 102);
        assert!(gates.gates.contains(&(95, 102)));
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
