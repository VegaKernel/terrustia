//! Load a `.wld`, save it, and load the result back, reporting any difference.
//!
//! ```text
//! cargo run --release --example roundtrip_wld -- input.wld /tmp/out.wld
//! ```

use std::{env, path::PathBuf, process::ExitCode};

use terrustia::world::{wld, wld_save};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: roundtrip_wld <input.wld> <output.wld>");
        return ExitCode::FAILURE;
    };
    let (input, output) = (PathBuf::from(input), PathBuf::from(output));

    let original = match wld::load(&input) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("load failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "loaded  {} ({}x{})",
        original.name,
        original.width(),
        original.height()
    );

    if let Err(e) = wld_save::save(&original, &output) {
        eprintln!("save failed: {e}");
        return ExitCode::FAILURE;
    }
    let saved_len = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    let source_len = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
    println!("saved   {saved_len} bytes (source was {source_len})");

    let reloaded = match wld::load(&output) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("reload failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "reloaded {} ({}x{})",
        reloaded.name,
        reloaded.width(),
        reloaded.height()
    );

    let mut problems = 0usize;
    let mut check = |what: &str, ok: bool| {
        if !ok {
            println!("  MISMATCH: {what}");
            problems += 1;
        }
    };
    check("name", original.name == reloaded.name);
    check(
        "size",
        original.width() == reloaded.width() && original.height() == reloaded.height(),
    );
    check(
        "spawn",
        (original.spawn_x, original.spawn_y) == (reloaded.spawn_x, reloaded.spawn_y),
    );
    check("surface", original.surface == reloaded.surface);
    check("rock layer", original.rock_layer == reloaded.rock_layer);
    check("world id", original.id == reloaded.id);
    check("crimson", original.crimson == reloaded.crimson);
    check("time", original.time == reloaded.time);
    check("day", original.day_time == reloaded.day_time);

    let chests_a: Vec<_> = original.chests.iter().flatten().collect();
    let chests_b: Vec<_> = reloaded.chests.iter().flatten().collect();
    check("chest count", chests_a.len() == chests_b.len());
    check("chest contents", chests_a == chests_b);

    let signs_a: Vec<_> = original.signs.iter().flatten().collect();
    let signs_b: Vec<_> = reloaded.signs.iter().flatten().collect();
    check("sign count", signs_a.len() == signs_b.len());
    check("sign contents", signs_a == signs_b);

    let mut differing = 0usize;
    let mut first = None;
    for y in 0..original.height() {
        for x in 0..original.width() {
            if original.tile(x, y) != reloaded.tile(x, y) {
                differing += 1;
                first.get_or_insert((x, y));
            }
        }
    }
    if differing > 0 {
        println!(
            "  MISMATCH: {differing} tiles differ, first at {:?}",
            first.unwrap()
        );
        if let Some((x, y)) = first {
            println!("    original {:?}", original.tile(x, y));
            println!("    reloaded {:?}", reloaded.tile(x, y));
        }
        problems += 1;
    }

    if problems == 0 {
        println!("\nround-trip is faithful: every tile, chest and sign survived");
        ExitCode::SUCCESS
    } else {
        println!("\n{problems} problem(s)");
        ExitCode::FAILURE
    }
}
