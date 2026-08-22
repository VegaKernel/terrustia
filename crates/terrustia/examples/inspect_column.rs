//! Print a vertical slice of tiles from a `.wld`, for checking that an edit reached the file.
//!
//! ```text
//! cargo run --release -p terrustia --example inspect_column -- world.wld 2122 320 340
//! ```

use std::{env, path::PathBuf, process::ExitCode};

use terrustia::world::wld;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: inspect_column <world.wld> <x> <y_from> <y_to>");
        return ExitCode::FAILURE;
    }
    let path = PathBuf::from(&args[0]);
    let (x, y0, y1) = (
        args[1].parse::<i32>().unwrap_or(0),
        args[2].parse::<i32>().unwrap_or(0),
        args[3].parse::<i32>().unwrap_or(0),
    );

    let world = match wld::load(&path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("load failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("{} at x={x}:", path.display());
    for y in y0..=y1 {
        let tile = world.tile(x, y);
        let block = if tile.is_active() {
            format!("block {}", tile.block)
        } else {
            "EMPTY".to_string()
        };
        println!("  y={y:<5} {block:<12} wall {}", tile.wall);
    }
    ExitCode::SUCCESS
}
