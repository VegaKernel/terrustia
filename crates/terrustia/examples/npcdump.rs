fn main() {
    println!("[");
    for t in 0..=691u16 {
        let Some(s) = terrustia_proto::npc_data::npc_stats(t) else {
            continue;
        };
        println!(
            "{{\"type\":{t},\"lifeMax\":{},\"damage\":{},\"defense\":{},\"value\":{},\"aiStyle\":{},\"width\":{},\"height\":{},\"kb\":{}}},",
            s.life_max,
            s.damage,
            s.defense,
            s.value,
            s.ai_style,
            s.width,
            s.height,
            s.knockback_resist
        );
    }
    println!("null]");
}
