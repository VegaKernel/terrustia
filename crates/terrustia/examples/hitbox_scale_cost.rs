//! What the C-wave npc_params fixes cost a tick. Dev tool, not part of the server.
//!
//! Three of the fixes touch code that runs every tick, so each one is measured against the
//! 16,666.7 us frame at the 255-player bar:
//!
//! 1. **B1, the hitbox scale.** `SetDefaults` folds a type's scale into its width and height
//!    (`NPC.cs:17842-17843`). The choice was between doing that inside `Npc::width()`/`height()`,
//!    which are among the hottest accessors here (every `center()`, every contact-damage test,
//!    every collision step), and doing it once in `Npc::new`. This measures both so the choice is
//!    a number rather than an opinion.
//! 2. **B6, the shimmerfly's avoidance radius.** The scan gained a distance test, which *removes*
//!    work: an entry outside the reach now skips a square root and two divisions.
//! 3. **M5, the town danger check.** One `hypot` per town NPC per tick.

use terrustia::game::npc::Npc;

/// A spread of types across the scale table: some above one, some below, some at exactly one.
const TYPES: [u16; 12] = [1, 16, 26, 50, 70, 71, 112, 113, 134, 141, 204, 304];

/// The NPC slot table is 200 entries, which is the real ceiling on any of these scans.
const NPCS: usize = 200;

fn micros(start: std::time::Instant, iterations: u64) -> f64 {
    start.elapsed().as_secs_f64() / iterations as f64 * 1e6
}

fn main() {
    // --- B1: construction, and the accessors it deliberately leaves alone -----------------------
    let runs: u64 = 200_000;
    for ty in TYPES {
        std::hint::black_box(Npc::new(ty, (0.0, 0.0), 1));
    }
    let start = std::time::Instant::now();
    for i in 0..runs {
        let ty = TYPES[(i as usize) % TYPES.len()];
        std::hint::black_box(Npc::new(ty, (0.0, 0.0), 1));
    }
    let construct = micros(start, runs);

    let npcs: Vec<Npc> = (0..NPCS)
        .map(|i| Npc::new(TYPES[i % TYPES.len()], (i as f32 * 32.0, 100.0), 1).expect("known type"))
        .collect();
    let ticks: u64 = 20_000;
    let start = std::time::Instant::now();
    for _ in 0..ticks {
        for n in &npcs {
            // What a contact-damage test reads: the box, twice over.
            std::hint::black_box((n.width(), n.height(), n.center()));
        }
    }
    let accessors = micros(start, ticks);

    // --- B6: the shimmerfly's avoid scan, with and without the radius test ----------------------
    // The list is every hostile NPC plus every target player, so 200 is the realistic worst case.
    let avoid: Vec<(f32, f32, f32)> = (0..NPCS)
        .map(|i| (i as f32 * 137.0, (i % 31) as f32 * 61.0, 100.0))
        .collect();
    let here = (avoid[NPCS / 2].0, avoid[NPCS / 2].1);

    let scans: u64 = 200_000;
    let start = std::time::Instant::now();
    for _ in 0..scans {
        let mut away = (0.0f32, 0.0f32);
        let mut crowd = 0.0f32;
        for &(kx, ky, _) in &avoid {
            let (dx, dy) = (here.0 - kx, here.1 - ky);
            let gap = dx.hypot(dy);
            if gap > 0.0 {
                crowd += 1.0;
                away.0 += dx / gap;
                away.1 += dy / gap;
            }
        }
        std::hint::black_box((away, crowd));
    }
    let unbounded = micros(start, scans);

    let start = std::time::Instant::now();
    for _ in 0..scans {
        let mut away = (0.0f32, 0.0f32);
        let mut crowd = 0.0f32;
        for &(kx, ky, reach) in &avoid {
            let (dx, dy) = (here.0 - kx, here.1 - ky);
            let gap2 = dx * dx + dy * dy;
            if gap2 > 0.0 && gap2 <= reach * reach {
                let gap = gap2.sqrt();
                crowd += 1.0;
                away.0 += dx / gap;
                away.1 += dy / gap;
            }
        }
        std::hint::black_box((away, crowd));
    }
    let bounded = micros(start, scans);

    // --- M5: the per-tick danger check a town NPC now makes -------------------------------------
    // Vanilla's own bar is one town NPC per house; a hundred is already an implausible town.
    let towns: Vec<(f32, f32)> = (0..100).map(|i| (i as f32 * 90.0, 200.0)).collect();
    let hostile = (4000.0f32, 260.0f32);
    let start = std::time::Instant::now();
    for _ in 0..ticks {
        for &(cx, cy) in &towns {
            std::hint::black_box(
                (hostile.0 - cx).hypot(hostile.1 - cy)
                    < terrustia_proto::npc_params::town_danger_range(22),
            );
        }
    }
    let danger = micros(start, ticks);

    println!("--- B1: the hitbox scale -----------------------------------------------");
    println!("Npc::new, per NPC              : {construct:.3} us");
    println!(
        "  a full 200-slot table refill : {:.1} us (a one-off, not a tick)",
        construct * 200.0
    );
    println!(
        "width/height/center, 200 NPCs  : {accessors:.2} us per tick  <- unchanged by this fix"
    );
    println!();
    println!("--- B6: the shimmerfly avoid scan, 200 entries --------------------------");
    println!("before, no radius              : {unbounded:.2} us per shimmerfly per check");
    println!("after, with the radius         : {bounded:.2} us per shimmerfly per check");
    println!("  and it only runs one tick in 15 (SHIMMERFLY_CHECK_EVERY)");
    println!();
    println!("--- M5: the town danger check ------------------------------------------");
    println!("100 town NPCs                  : {danger:.2} us per tick");
    println!();
    println!("tick budget                    : 16666.7 us");
}
