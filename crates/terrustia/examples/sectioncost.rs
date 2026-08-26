//! Measure what encoding a section actually costs, across every section of a real world.
//!
//! `bench.rs` already times a single section (the one at spawn), which is atypically sparse —
//! spawn is cleared. This instead walks every section in a real generated world and reports the
//! spread, because the question that matters for "does section (re-)encoding stall the tick" is
//! the *worst* section, not a convenient one.
//!
//! ```sh
//! cargo run --release -p terrustia --example sectioncost -- [--large]
//! ```
//!
//! With no argument it generates the usual 4200x1200 world; `--large` generates the 8400x2400
//! world `docs/performance.md`'s worldgen-timing section also uses.

use std::time::Instant;

use terrustia::world::worldgen;
use terrustia_proto::section::encode_section_packet;

fn main() {
    let large = std::env::args().any(|a| a == "--large");
    let (w, h) = if large { (8400, 2400) } else { (4200, 1200) };

    println!("generating a {w}x{h} world...");
    let began = Instant::now();
    let world = worldgen::generate(w, h, "sectioncost", 999);
    println!("  generated in {:.1} s", began.elapsed().as_secs_f64());

    let (sx_max, sy_max) = (world.sections_x(), world.sections_y());
    println!(
        "world       {w}x{h}, {sx_max}x{sy_max} sections ({} total)",
        sx_max * sy_max
    );

    let mut times_us: Vec<u64> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    let mut extras_us: Vec<u64> = Vec::new();

    for sy in 0..sy_max {
        for sx in 0..sx_max {
            let bounds = world.section_bounds(sx, sy);
            if bounds.width == 0 || bounds.height == 0 {
                continue;
            }

            let began = Instant::now();
            let extras = world.extras_for(bounds);
            extras_us.push(began.elapsed().as_micros() as u64);

            let began = Instant::now();
            let encoded =
                encode_section_packet(bounds, &extras, |x, y| world.tile(x, y)).expect("encode");
            times_us.push(began.elapsed().as_micros() as u64);
            sizes.push(encoded.len());
        }
    }

    times_us.sort_unstable();
    sizes.sort_unstable();
    extras_us.sort_unstable();

    let pct = |v: &[u64], p: f64| v[((v.len() - 1) as f64 * p) as usize];
    let pct_sz = |v: &[usize], p: f64| v[((v.len() - 1) as f64 * p) as usize];

    println!();
    println!("extras_for (chest/sign/tile-entity scan per section):");
    println!(
        "  min {:>6} µs   p50 {:>6} µs   p99 {:>6} µs   max {:>6} µs",
        extras_us[0],
        pct(&extras_us, 0.50),
        pct(&extras_us, 0.99),
        extras_us[extras_us.len() - 1]
    );

    println!();
    println!("encode_section_packet, {} real sections:", times_us.len());
    println!(
        "  min {:>6} µs   p50 {:>6} µs   p99 {:>6} µs   max {:>6} µs",
        times_us[0],
        pct(&times_us, 0.50),
        pct(&times_us, 0.99),
        times_us[times_us.len() - 1]
    );
    println!(
        "  bytes out: min {:>6}   p50 {:>6}   p99 {:>6}   max {:>6}",
        sizes[0],
        pct_sz(&sizes, 0.50),
        pct_sz(&sizes, 0.99),
        sizes[sizes.len() - 1]
    );

    let total_us: u64 = times_us.iter().sum();
    println!();
    println!(
        "total, every section encoded once: {:.1} ms across {} sections",
        total_us as f64 / 1000.0,
        times_us.len()
    );

    let budget_us = 16_666u64;
    let worst = times_us[times_us.len() - 1];
    println!();
    println!("tick budget {budget_us} µs");
    println!(
        "worst single section = {:.1}% of a tick",
        worst as f64 / budget_us as f64 * 100.0
    );
    if worst > budget_us / 2 {
        println!(
            "A single re-encode can eat more than half a tick's budget on its own — a real stall."
        );
    } else if worst > budget_us / 20 {
        println!(
            "A single re-encode is a measurable slice of a tick but not, on its own, a stall."
        );
    } else {
        println!("A single section re-encode costs nothing anybody would notice.");
    }
}
