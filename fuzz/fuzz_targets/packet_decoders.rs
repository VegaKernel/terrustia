//! Fuzzes the hand-parsed packet decoders `game/server.rs`'s inbound packet dispatch calls on raw,
//! untrusted client bytes: all 22 `T::decode(payload)` functions that dispatch calls, plus
//! `player_info::PlayerAppearance::decode`, reached there via `.ok()` on client-supplied bytes at
//! one call site. Each `decode` takes the payload directly, with no framing to strip first, so the
//! same arbitrary bytes are handed to all of them in one run — an independent, cheap way to explore
//! many parsers per execution rather than one target per struct, since a panic in any of them fails
//! this target and libFuzzer's own stack trace says which.
//!
//! This target originally covered only 14 decoders, scoped to five files (`items`, `net_module`,
//! `npc`, `objects`, `square`) this crate's own tests already flag as deliberately hardened against
//! truncation — real coverage of those five, but a parallel audit this session found the doc
//! comment's old claim to fuzz "every other hand-parsed packet decoder" misleading about total
//! coverage: 13 of the dispatch's 22 decoders were never fuzzed at all, including packet 13
//! (`PlayerControls`, the single highest-frequency inbound packet, sent roughly once a tick per
//! player) and packet 17 (`TileManipulation`, the packet behind this project's own headline
//! section-ownership fix). Those 13, plus the `PlayerAppearance` case, are added below — this now
//! covers every decoder the dispatch calls on raw client bytes, not just the five files it was
//! originally scoped to. `items::ItemOwner::decode`, `npc::SyncNpc::decode`, and
//! `net_module::decode_liquid_changes` are also fuzzed below even though `game/server.rs` never
//! calls them — they're decoded client-side, by `terrustia-client`'s own packet handling, and are
//! real hand-parsed decoders too, kept from the original scope rather than dropped.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terrustia_proto::{
    inventory, items, net_module, npc, objects, packets, player_info, projectile, square,
};

fuzz_target!(|data: &[u8]| {
    let _ = items::SyncItem::decode(data);
    let _ = items::ItemOwner::decode(data);
    let _ = items::decode_item_despawn(data);
    let _ = npc::SyncNpc::decode(data);
    let _ = npc::DamageNpc::decode(data);
    let _ = objects::RequestChestOpen::decode(data);
    let _ = objects::SyncChestItem::decode(data);
    let _ = objects::SyncPlayerChest::decode(data);
    let _ = objects::RequestSign::decode(data);
    let _ = objects::SignText::decode(data);
    let _ = objects::DoorToggle::decode(data);
    let _ = square::TileSquare::decode(data);
    let _ = net_module::decode_pylon_message(data);
    let _ = net_module::decode_liquid_changes(data);
    let _ = net_module::IncomingChat::decode(data);
    let _ = packets::Hello::decode(data);
    let _ = packets::SpawnTileData::decode(data);
    let _ = packets::PlayerSpawn::decode(data);
    let _ = packets::PlayerControls::decode(data);
    let _ = packets::PlayerHealth::decode(data);
    let _ = packets::PlayerMana::decode(data);
    let _ = packets::TileManipulation::decode(data);
    let _ = packets::AddNpcBuff::decode(data);
    let _ = packets::RemoveNpcBuff::decode(data);
    let _ = inventory::SyncEquipment::decode(data);
    let _ = projectile::SyncProjectile::decode(data);
    let _ = projectile::KillProjectile::decode(data);
    let _ = player_info::PlayerAppearance::decode(data);
});
