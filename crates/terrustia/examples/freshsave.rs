fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fresh.wld".into());
    let mut world = terrustia::world::worldgen::generate(2100, 600, "Fresh", 42);
    // Something in every section that has one, so the writer is exercised rather than only the
    // empty case.
    world.progress.hard_mode = true;
    world.progress.downed_moon_lord = true;
    world.progress.downed_deerclops = true;
    world.progress.combat_book_two = true;
    world.raining = true;
    world.rain_time = 4321;
    world.max_rain = 0.6;
    world.wind = -0.375;
    world.sandstorm = true;
    world.sandstorm_time = 5000;
    let mut chest = terrustia::world::objects::Chest::empty_at(100, 200);
    chest.name = "loot".into();
    chest.items[0] = terrustia_proto::ItemStack::new(3507, 42, 58);
    world.chests = vec![Some(chest)];
    world.signs = vec![Some(terrustia::world::objects::Sign {
        x: 150,
        y: 210,
        text: "this way".into(),
    })];

    let bytes = terrustia::world::wld_save::serialize(&world).expect("a generated world saves");
    std::fs::write(&out, &bytes).unwrap();
    println!("wrote {} bytes to {out}", bytes.len());

    let back = terrustia::world::wld::parse(&bytes).expect("and reads back");
    let differing = (0..world.width())
        .flat_map(|x| (0..world.height()).map(move |y| (x, y)))
        .filter(|&(x, y)| world.tile(x, y) != back.tile(x, y))
        .count();
    println!(
        "{differing} differing tiles of {}",
        world.width() * world.height()
    );
    println!(
        "flags: hardmode {} moon lord {} deerclops {} book2 {}",
        back.progress.hard_mode,
        back.progress.downed_moon_lord,
        back.progress.downed_deerclops,
        back.progress.combat_book_two
    );
    println!(
        "weather: rain {}/{} at {} wind {} sandstorm {}/{}",
        back.raining, back.rain_time, back.max_rain, back.wind, back.sandstorm, back.sandstorm_time
    );
    let chest = back
        .chests
        .iter()
        .flatten()
        .next()
        .expect("the chest survived");
    println!(
        "chest {:?} at {},{} holding {:?}",
        chest.name, chest.x, chest.y, chest.items[0]
    );
    let sign = back
        .signs
        .iter()
        .flatten()
        .next()
        .expect("the sign survived");
    println!("sign {:?} at {},{}", sign.text, sign.x, sign.y);
}
