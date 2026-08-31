//! What the per-tick section stream costs at the 255-player bar.
//!
//! `GameServer::check_player_sections` runs vanilla's own per-tick push (`Main.cs:65601` ->
//! `RemoteClient.CheckSection`, `RemoteClient.cs:152-190`) for every player on every tick, so it is
//! exactly the shape of per-player work that has to be shown to fit in 16.67ms before it can ship.
//!
//! This mirrors that loop rather than calling it: `GameServer`'s player table is private and there
//! is no public way to seat 255 players. The arithmetic, the types and the order of the checks are
//! the same ones the real loop uses, so what is timed here is what a tick pays.
//!
//! ```sh
//! cargo run --release -p terrustia --example section_stream_cost
//! ```

use std::{collections::HashSet, time::Instant};

use terrustia_proto::section::{SECTION_HEIGHT, SECTION_WIDTH};

/// How many sections either way a player is streamed, vanilla's `fluff = 1` and this server's
/// `SECTION_REACH`.
const REACH: i32 = 1;

const PLAYERS: usize = 255;
const TICKS: u64 = 10_000;

/// The world the performance notes size everything against.
const SECTIONS_X: i32 = 8400 / SECTION_WIDTH;
const SECTIONS_Y: i32 = 2400 / SECTION_HEIGHT;

/// `game::server::section_of`, which is two divides and nothing else.
fn section_of(at: (f32, f32)) -> (i32, i32) {
    (
        (at.0 / 16.0) as i32 / SECTION_WIDTH,
        (at.1 / 16.0) as i32 / SECTION_HEIGHT,
    )
}

/// One player, in the state the real loop reads: where they are, which section they were last
/// checked against, and what they already hold.
struct Seat {
    position: (f32, f32),
    last_section: Option<(i32, i32)>,
    sent: HashSet<(i32, i32)>,
}

impl Seat {
    fn new(index: usize) -> Self {
        // Spread across the world rather than piled at spawn, so no two share a cache line's worth
        // of luck.
        let sx = (index as i32 * 7) % (SECTIONS_X - 2) + 1;
        let sy = (index as i32 * 3) % (SECTIONS_Y - 2) + 1;
        let position = (
            (sx * SECTION_WIDTH * 16 + 16) as f32,
            (sy * SECTION_HEIGHT * 16 + 16) as f32,
        );
        let mut sent = HashSet::new();
        for x in (sx - REACH)..=(sx + REACH) {
            for y in (sy - REACH)..=(sy + REACH) {
                sent.insert((x, y));
            }
        }
        Self {
            position,
            last_section: Some((sx, sy)),
            sent,
        }
    }
}

/// The body of `check_player_sections`, minus the queueing and the status line: what every player
/// pays on every tick, and what a boundary crossing adds on top.
fn check(seat: &mut Seat, memoised: bool) -> usize {
    let at = section_of(seat.position);
    if memoised && seat.last_section == Some(at) {
        return 0;
    }
    seat.last_section = Some(at);

    let mut owed = 0;
    for sx in (at.0 - REACH)..=(at.0 + REACH) {
        for sy in (at.1 - REACH)..=(at.1 + REACH) {
            if sx < 0 || sy < 0 || sx >= SECTIONS_X || sy >= SECTIONS_Y {
                continue;
            }
            if seat.sent.contains(&(sx, sy)) {
                continue;
            }
            seat.sent.insert((sx, sy));
            owed += 1;
        }
    }
    owed
}

/// Run `TICKS` ticks over a full house, moving each player by `step` pixels of x per tick, and
/// report microseconds per tick.
fn run(step: f32, memoised: bool) -> (f64, usize) {
    let mut seats: Vec<Seat> = (0..PLAYERS).map(Seat::new).collect();
    let mut owed = 0usize;

    let began = Instant::now();
    for _ in 0..TICKS {
        for seat in &mut seats {
            seat.position.0 += step;
            owed += check(seat, memoised);
        }
        std::hint::black_box(&seats);
    }
    (
        began.elapsed().as_secs_f64() / TICKS as f64 * 1e6,
        std::hint::black_box(owed),
    )
}

fn main() {
    // `Player.maxRunSpeed` is 3 pixels a frame before accessories, so a sprinting player crosses a
    // 200-tile section about every eighteen seconds.
    let sprint = 3.0;
    // A section every tick per player: nobody can do this, it is the bound rather than the case.
    let teleport = (SECTION_WIDTH * 16) as f32;

    println!("{PLAYERS} players, {TICKS} ticks, {SECTIONS_X}x{SECTIONS_Y} sections\n");

    let (standing, _) = run(0.0, true);
    let (sprinting, sprint_owed) = run(sprint, true);
    let (teleporting, teleport_owed) = run(teleport, true);
    let (vanilla_shape, _) = run(0.0, false);

    let per_tick = |owed: usize| owed as f64 / TICKS as f64;
    println!("standing still      : {standing:.1} us per tick");
    println!(
        "all 255 sprinting   : {sprinting:.1} us per tick, {:.2} sections queued per tick",
        per_tick(sprint_owed)
    );
    println!(
        "all 255 teleporting : {teleporting:.1} us per tick, {:.2} sections queued per tick \
         (the bound, not a case)",
        per_tick(teleport_owed)
    );
    println!("without last_section: {vanilla_shape:.1} us per tick (vanilla's own shape)");
    println!("\ntick budget         : 16666.7 us");
    println!(
        "queued sections are encoded and sent by `drain_section_streams`, which is separately\n\
         bounded at {} us of a tick (`SECTION_STREAM_BUDGET`), joins and walkers sharing the one\n\
         budget, so this can never take more of a tick than a join already could.",
        4_000
    );
}
