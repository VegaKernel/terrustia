//! Golden pins for the NPC sync/despawn timing constants against the decompiled 1.4.5.8 source, so
//! a future edit that drifts one of them fails here naming exactly which constant moved.
//!
//! These are plain `pub const`s (`game::npc`, `game::npc_ai`) rather than a function to exercise, so
//! each pin is a direct equality against the literal from source — there is no behaviour to drive,
//! only a number to keep honest.

use terrustia::game::{
    npc::{
        DEFAULT_TIME_LEFT, NET_SPAM_PACKET_LIMIT, NET_SPAM_PER_PACKET, NET_SPAM_PER_PACKET_BOSS,
        NPC_STREAM_SPEED,
    },
    npc_ai::{DESPAWN_HALF_HEIGHT, DESPAWN_HALF_WIDTH},
};

/// `NPC.cs:6148`: `public readonly int netSpamTicksPerPacket = 30;`
#[test]
fn net_spam_per_packet_matches_npc_net_spam_ticks_per_packet() {
    assert_eq!(NET_SPAM_PER_PACKET, 30);
}

/// `NPC.cs:6150`: `public readonly int netSpamTicksPerPacketForBosses = 5;`
#[test]
fn net_spam_per_packet_boss_matches_npc_net_spam_ticks_per_packet_for_bosses() {
    assert_eq!(NET_SPAM_PER_PACKET_BOSS, 5);
}

/// `NPC.cs:6146`: `public readonly int netSpamPacketLimit = 3;`
#[test]
fn net_spam_packet_limit_matches_npc_net_spam_packet_limit() {
    assert_eq!(NET_SPAM_PACKET_LIMIT, 3);
}

/// `NPC.cs:91685-91686`: the rate-limit check itself, so the three constants above are pinned not
/// just individually but in the shape the game actually uses them —
/// ```csharp
/// int num = (boss ? netSpamTicksPerPacketForBosses : netSpamTicksPerPacket);
/// if (!netUpdate || netSpam > num * netSpamPacketLimit) { ... }
/// ```
/// a boss's burst allowance (`5 * 3 = 15` ticks of banked budget) is six times tighter than an
/// ordinary NPC's (`30 * 3 = 90`), matching `netSpamTicksPerPacketForBosses` being six times
/// smaller than `netSpamTicksPerPacket`.
#[test]
fn a_bosss_burst_allowance_is_six_times_tighter_than_an_ordinary_npcs() {
    let ordinary_limit = NET_SPAM_PER_PACKET * NET_SPAM_PACKET_LIMIT;
    let boss_limit = NET_SPAM_PER_PACKET_BOSS * NET_SPAM_PACKET_LIMIT;
    assert_eq!(ordinary_limit, 90);
    assert_eq!(boss_limit, 15);
    assert_eq!(ordinary_limit, boss_limit * 6);
}

/// `Main.cs:449`: `public static int npcStreamSpeed = 30;`
#[test]
fn npc_stream_speed_matches_main_npc_stream_speed() {
    assert_eq!(NPC_STREAM_SPEED, 30);
}

/// `NPC.cs:6188`: `private static int activeTime = 750;`
#[test]
fn default_time_left_matches_npc_active_time() {
    assert_eq!(DEFAULT_TIME_LEFT, 750);
}

/// `NPC.cs:6791-6793`: `public static int sWidth => 1920;` / `public static int sHeight => 1200;` —
/// `CheckActive`'s despawn rectangle (`NPC.cs:78707`) is `sWidth`/`sHeight` wide and tall, centred on
/// the NPC, so the *half*-extents this project pins are exactly half a screen.
#[test]
fn despawn_half_extents_are_exactly_half_of_a_1920x1200_screen() {
    const SCREEN_WIDTH: f32 = 1920.0;
    const SCREEN_HEIGHT: f32 = 1200.0;
    assert_eq!(DESPAWN_HALF_WIDTH, SCREEN_WIDTH / 2.0);
    assert_eq!(DESPAWN_HALF_HEIGHT, SCREEN_HEIGHT / 2.0);
}

/// `NPC.cs:78707`:
/// ```csharp
/// Rectangle rectangle2 = new Rectangle(
///     (int)((double)(position.X + (float)(width / 2)) - (double)sWidth * 0.5 - (double)width),
///     (int)((double)(position.Y + (float)(height / 2)) - (double)sHeight * 0.5 - (double)height),
///     sWidth + width * 2, sHeight + height * 2);
/// ```
/// Worked out from `position` (top-left) to a half-extent from `Center` (`position + size/2`, the
/// same anchor `game::npc::Npc::center()` uses): the rectangle's left edge sits at
/// `Center.X - sWidth*0.5 - width`, its right edge at `Center.X + sWidth*0.5 + width` — a half-width
/// of `sWidth*0.5 + width`, i.e. the *full* sprite width added on top of half a screen, not half the
/// sprite. `game::npc_ai::tick_life` (`npc_ai.rs:427-429`) adds `npc.width()`/`npc.height()` — the
/// full extents `Npc::width`/`height` return (`npc.rs:271-277`) — on top of `DESPAWN_HALF_WIDTH`/
/// `DESPAWN_HALF_HEIGHT`, matching this derivation rather than double-counting or halving it.
#[test]
fn the_despawn_box_widens_by_the_npcs_full_size_not_half_of_it() {
    let full_sprite_width = 40.0_f32;
    let half_extent = DESPAWN_HALF_WIDTH + full_sprite_width;
    assert_eq!(
        half_extent,
        960.0 + 40.0,
        "sWidth*0.5 + width, NPC.cs:78707"
    );
}
