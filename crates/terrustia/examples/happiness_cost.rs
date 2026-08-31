//! What a town NPC's happiness costs. Dev tool, not part of the server.
//!
//! The claim this exists to check is that happiness adds *no* per-tick work. The game takes the
//! number once, in `Player.SetTalkNPC` (`Player.cs:4360-4375`), when a chat opens, and caches it
//! until the next chat; this server does the same thing on packet 40 and nowhere else. So the only
//! question is what one chat costs, and the worst case is every player on a full server opening
//! one on the same tick.

use terrustia_proto::happiness::{Resident, Zones, price_multiplier};

fn main() {
    // A big town: thirty-five residents, all housed within a few tiles of each other, which is the
    // most crowded the calculation ever sees (every one of them lands in the 25-tile "house" band,
    // so every neighbour opinion has to be looked up).
    let town: Vec<Resident> = (0..35)
        .map(|i| Resident {
            npc_type: [
                22, 17, 18, 19, 20, 38, 54, 107, 108, 124, 160, 178, 207, 208, 209,
            ][i % 15],
            home: Some((600 + i as i32 % 5, 100)),
            center: (600.0 + (i % 5) as f32, 100.0),
        })
        .collect();
    let shopper = town[0];
    let zones = Zones::default();

    for _ in 0..100 {
        std::hint::black_box(price_multiplier(&shopper, &town[1..], zones, false));
    }

    let runs = 200_000;
    let start = std::time::Instant::now();
    for _ in 0..runs {
        std::hint::black_box(price_multiplier(&shopper, &town[1..], zones, false));
    }
    let each = start.elapsed().as_secs_f64() / f64::from(runs) * 1e6;

    println!("residents in town   : {}", town.len());
    println!("one chat            : {each:.2} us");
    println!("255 chats in a tick : {:.2} us", each * 255.0);
    println!("per tick, at rest   : 0 us (nothing recomputes it)");
    println!("tick budget         : 16666.7 us");
}
