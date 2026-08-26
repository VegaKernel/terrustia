//! Fuzzes every other hand-parsed packet decoder this crate's own tests already flag as
//! deliberately hardened against truncation (`grep -rln "truncated.*panic" crates/terrustia-proto`
//! names these exact files: items, net_module, npc, objects, square). Each `decode` takes the
//! payload directly, with no framing to strip first, so the same arbitrary bytes are handed to
//! all of them in one run — an independent, cheap way to explore many parsers per execution rather
//! than one target per struct, since a panic in any of them fails this target and libFuzzer's own
//! stack trace says which.
#![no_main]

use libfuzzer_sys::fuzz_target;
use terrustia_proto::{items, net_module, npc, objects, square};

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
});
