//! Load a `.wld`, save it, and load the result back, reporting any difference.
//!
//! ```text
//! cargo run --release --example roundtrip_wld -- input.wld /tmp/out.wld
//! ```

use std::{env, path::PathBuf, process::ExitCode};

use terrustia::world::{wld, wld_save};

/// Report whether a trailing section's bytes survived a save byte-for-byte, and count it as a
/// problem if they did not.
fn report_identical(label: &str, a: &[u8], b: &[u8], problems: &mut usize) {
    if a != b {
        println!(
            "  MISMATCH: {label} ({} bytes) differs after a save",
            a.len()
        );
        *problems += 1;
    } else {
        println!("  ok    {label}: {} bytes, byte-identical", a.len());
    }
}

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

    // File section index 6 onward (indices 2-5 of the trailing run; 0 and 1 above are the
    // townsfolk/tile-entity *fallback* bytes, only used if that section failed to decode, which
    // is a different path from this one). Every one of these is now written from this server's
    // own live state rather than carried through, so "byte-identical to the source file" is the
    // right expectation for only some of them — the rest are *supposed* to change on a real,
    // played-in world, and reporting that as a mismatch would be false alarm noise on exactly the
    // cases this fix exists for.
    if let (Some(orig_p), Some(reload_p)) = (&original.preserved, &reloaded.preserved) {
        if orig_p.trailing_sections.len() != reload_p.trailing_sections.len() {
            println!(
                "  MISMATCH: trailing section count {} -> {}",
                orig_p.trailing_sections.len(),
                reload_p.trailing_sections.len()
            );
            problems += 1;
        }
        for (i, (a, b)) in orig_p
            .trailing_sections
            .iter()
            .zip(reload_p.trailing_sections.iter())
            .enumerate()
        {
            let file_section = i + 4;
            match i {
                0 => report_identical("townsfolk fallback", a, b, &mut problems),
                1 => report_identical("tile-entity fallback", a, b, &mut problems),
                // Pressure plates and the bestiary always save as the genuinely empty shape now
                // (L3-21/L3-22) — a real file's own bytes here are exactly what must NOT survive.
                2 => {
                    let empty = 0i32.to_le_bytes();
                    if b.as_slice() == empty {
                        println!(
                            "  ok    pressure plates: saved empty, as a server-owned world must"
                        );
                    } else {
                        println!(
                            "  MISMATCH: pressure plates did not save empty ({} bytes)",
                            b.len()
                        );
                        problems += 1;
                    }
                }
                4 => {
                    let empty = [0i32.to_le_bytes(); 3].concat();
                    if *b == empty {
                        println!("  ok    bestiary: saved empty (no live kill/sight tracker yet)");
                    } else {
                        println!(
                            "  MISMATCH: bestiary did not save empty ({} bytes)",
                            b.len()
                        );
                        problems += 1;
                    }
                }
                // The town manager's room list (L3-20) is derived from the live residents this
                // same save just wrote into section 4, so it need not match the source file's own
                // bytes at all — only informational here, not a pass/fail check.
                3 => println!(
                    "  info  town manager rooms: {} -> {} bytes (derived from live residents, \
                     not compared)",
                    a.len(),
                    b.len()
                ),
                // Journey powers (L3-23) now round-trip for real, so this one genuinely should be
                // byte-identical when nothing else about the world's powers changed.
                5 => report_identical("Journey powers", a, b, &mut problems),
                _ => {
                    let label = format!("preserved section {file_section}");
                    report_identical(&label, a, b, &mut problems);
                }
            }
        }
    } else {
        println!("  MISMATCH: preserved header state missing after load or reload");
        problems += 1;
    }

    // The header is preserved verbatim and patched in place, so the only way to know a flag
    // really survives a save is to change one and read it back off the disk.
    {
        let mut changed = match wld::load(&input) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("reload failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        let before = changed.progress;
        changed.progress.downed_moon_lord = !before.downed_moon_lord;
        changed.progress.downed_ancient_cultist = !before.downed_ancient_cultist;
        changed.progress.tower_active_solar = !before.tower_active_solar;
        changed.progress.altar_count = before.altar_count + 1;
        changed.wind = 0.375;
        changed.raining = !changed.raining;
        changed.rain_time = 4321;
        // The two fields that were silently lost on every save of a loaded world. The ore tier is
        // the dangerous one: dropped, the header keeps reading -1 for "not chosen", so the next
        // altar smashed after a restart rolls a *second* tier and the world ends up with two.
        changed.ore_tiers[4] = 221;
        changed.ore_tiers[5] = 223;
        changed.ore_tiers[6] = 227;
        changed.banner_kills.insert(3, 4242);
        let probe = output.with_extension("flags.wld");
        if let Err(e) = wld_save::save(&changed, &probe) {
            eprintln!("probe save failed: {e}");
            return ExitCode::FAILURE;
        }
        let back = match wld::load(&probe) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("probe reload failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        let ok = back.progress.downed_moon_lord == changed.progress.downed_moon_lord
            && back.progress.downed_ancient_cultist == changed.progress.downed_ancient_cultist
            && back.progress.tower_active_solar == changed.progress.tower_active_solar
            && back.progress.altar_count == changed.progress.altar_count
            && (back.wind - 0.375).abs() < 1e-6
            && back.raining == changed.raining
            && back.rain_time == 4321
            && back.ore_tiers[4..7] == [221, 223, 227]
            && back.banner_kills.get(&3) == Some(&4242);
        println!(
            "flags       {} (moon lord {} -> {}, cultist {} -> {}, solar tower {} -> {}, \
             altars {} -> {}, wind {:.3}, rain {}/{}, ores {:?}, banner 3 = {:?})",
            if ok {
                "survive a save"
            } else {
                "DID NOT SURVIVE"
            },
            before.downed_moon_lord,
            back.progress.downed_moon_lord,
            before.downed_ancient_cultist,
            back.progress.downed_ancient_cultist,
            before.tower_active_solar,
            back.progress.tower_active_solar,
            before.altar_count,
            back.progress.altar_count,
            back.wind,
            back.raining,
            back.rain_time,
            &back.ore_tiers[4..7],
            back.banner_kills.get(&3),
        );
        std::fs::remove_file(&probe).ok();
        if !ok {
            return ExitCode::FAILURE;
        }
    }

    if problems == 0 {
        println!("\nround-trip is faithful: every tile, chest and sign survived");
        ExitCode::SUCCESS
    } else {
        println!("\n{problems} problem(s)");
        ExitCode::FAILURE
    }
}
