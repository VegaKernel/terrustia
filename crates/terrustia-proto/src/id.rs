//! Numeric message ids, transcribed from `Terraria.ID.MessageID` in the 1.4.5.7 build.
//!
//! Kept complete rather than trimmed to the ids we handle, so that logging an unexpected packet
//! can name it. See `docs/protocol-notes.md`.

/// The protocol release this server speaks.
///
/// 1.4.5.8 bumped this to 326 without changing the wire format — determined by asking a real
/// Terraria server, which rejected every other number and accepted this one. The decompiled tree
/// this project is written against is 1.4.5.7, release 325.
pub const CUR_RELEASE: u32 = 326;

/// The exact handshake string a current client opens with.
pub const VERSION_STRING: &str = "Terraria326";

/// Releases this server will talk to.
///
/// A range rather than one number, because the last two releases are the same protocol and
/// refusing the older one strands anybody who has not updated for no reason at all.
pub const SUPPORTED_RELEASES: &[u32] = &[325, 326];

pub const HELLO: u8 = 1;
pub const KICK: u8 = 2;
pub const PLAYER_INFO: u8 = 3;
pub const SYNC_PLAYER: u8 = 4;
pub const SYNC_EQUIPMENT: u8 = 5;
pub const REQUEST_WORLD_DATA: u8 = 6;
pub const WORLD_DATA: u8 = 7;
pub const SPAWN_TILE_DATA: u8 = 8;
pub const STATUS_TEXT_SIZE: u8 = 9;
pub const TILE_SECTION: u8 = 10;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const TILE_FRAME_SECTION: u8 = 11;
pub const PLAYER_SPAWN: u8 = 12;
pub const PLAYER_CONTROLS: u8 = 13;
pub const PLAYER_ACTIVE: u8 = 14;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const UNKNOWN15: u8 = 15;
pub const PLAYER_LIFE_MANA: u8 = 16;
pub const TILE_MANIPULATION: u8 = 17;
pub const SET_TIME: u8 = 18;
pub const TOGGLE_DOOR_STATE: u8 = 19;
pub const AREA_TILE_CHANGE: u8 = 20;
pub const SYNC_ITEM: u8 = 21;
pub const ITEM_OWNER: u8 = 22;
pub const SYNC_N_P_C: u8 = 23;
/// Dead. `NetMessage.SendData` case 24 still serialises a `(short, byte)` payload, but nothing in
/// the shipped game ever calls `SendData(24, ...)`, and `MessageBuffer.GetData`'s case 24 is
/// `Invariant.Assert(condition: false, "UnusedMeleeStrike")`: reaching it is treated as a bug, not
/// a real message. See `docs/packet-coverage.md`.
pub const UNUSED_MELEE_STRIKE: u8 = 24;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const UNUSED25: u8 = 25;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const UNUSED26: u8 = 26;
pub const SYNC_PROJECTILE: u8 = 27;
pub const DAMAGE_N_P_C: u8 = 28;
pub const KILL_PROJECTILE: u8 = 29;
pub const TOGGLE_P_V_P: u8 = 30;
pub const REQUEST_CHEST_OPEN: u8 = 31;
pub const SYNC_CHEST_ITEM: u8 = 32;
pub const SYNC_PLAYER_CHEST: u8 = 33;
pub const CHEST_UPDATES: u8 = 34;
pub const PLAYER_HEAL: u8 = 35;
pub const SYNC_PLAYER_ZONE: u8 = 36;
pub const REQUEST_PASSWORD: u8 = 37;
pub const SEND_PASSWORD: u8 = 38;
pub const RELEASE_ITEM_OWNERSHIP: u8 = 39;
pub const SYNC_TALK_N_P_C: u8 = 40;
pub const ITEM_ROTATION_AND_ANIMATION: u8 = 41;
pub const UNKNOWN42: u8 = 42;
pub const MANA_EFFECT: u8 = 43;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const UNKNOWN44: u8 = 44;
pub const TEAM_CHANGE: u8 = 45;
pub const OPEN_SIGN_REQUEST: u8 = 46;
pub const OPEN_SIGN_RESPONSE: u8 = 47;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const LIQUID_UPDATE: u8 = 48;
pub const INITIAL_SPAWN: u8 = 49;
pub const PLAYER_BUFFS: u8 = 50;
pub const MISC_DATA_SYNC: u8 = 51;
pub const LOCK_AND_UNLOCK: u8 = 52;
pub const ADD_N_P_C_BUFF: u8 = 53;
pub const N_P_C_BUFFS: u8 = 54;
pub const ADD_PLAYER_BUFF_PV_P: u8 = 55;
pub const UNIQUE_TOWN_N_P_C_INFO_SYNC_REQUEST: u8 = 56;
pub const UNKNOWN57: u8 = 57;
pub const INSTRUMENT_SOUND: u8 = 58;
pub const HIT_SWITCH: u8 = 59;
/// Where a town NPC lives. Sent both ways: the server announces it, and a client asks for a
/// change through the housing screen — dragging an NPC into a room, or evicting one.
pub const NPC_HOME: u8 = 60;
/// The name this id has in `MessageID.cs`, kept so the transcription stays greppable.
pub const UNKNOWN60: u8 = NPC_HOME;
pub const SPAWN_BOSS_USE_LICENSE_START_EVENT: u8 = 61;
pub const SYNC_DODGE: u8 = 62;
pub const SYNC_TILE_PAINT_OR_COATING: u8 = 63;
pub const SYNC_WALL_PAINT_OR_COATING: u8 = 64;
pub const TELEPORT_ENTITY: u8 = 65;
/// Not dead, unlike its neighbours either side. `Projectile.cs`'s `aiStyle == 52` (a heal-on-touch
/// projectile) calls `NetMessage.SendData(66, -1, -1, null, target, healAmount)`, and
/// `MessageBuffer.GetData`'s case 66 applies the heal to `player.statLife` and, when
/// `Main.netMode == 2`, relays it on to other clients. Unimplemented here: no dispatch arm and no
/// encoder, so a real client using that projectile heals silently as far as this server is
/// concerned. See `docs/packet-coverage.md`.
pub const UNKNOWN66: u8 = 66;
/// Dead. `NetMessage.SendData` has no `case 67:` at all, and `MessageBuffer.GetData` groups it
/// with the officially-deprecated ids 15/25/26/44/83/93 under a bare `break;` with no field reads.
/// See `docs/packet-coverage.md`.
pub const UNKNOWN67: u8 = 67;
pub const UNKNOWN68: u8 = 68;
pub const CHEST_NAME: u8 = 69;
pub const BUG_CATCHING: u8 = 70;
pub const BUG_RELEASING: u8 = 71;
pub const TRAVEL_MERCHANT_ITEMS: u8 = 72;
pub const REQUEST_TELEPORTATION_BY_SERVER: u8 = 73;
pub const ANGLER_QUEST: u8 = 74;
pub const ANGLER_QUEST_FINISHED: u8 = 75;
pub const QUESTS_COUNT_SYNC: u8 = 76;
pub const TEMPORARY_ANIMATION: u8 = 77;
pub const INVASION_PROGRESS_REPORT: u8 = 78;
pub const PLACE_OBJECT: u8 = 79;
pub const SYNC_PLAYER_CHEST_INDEX: u8 = 80;
pub const COMBAT_TEXT_INT: u8 = 81;
pub const NET_MODULES: u8 = 82;
/// Deprecated in 1.4.5; the client no longer needs it.
pub const UNUSED83: u8 = 83;
pub const PLAYER_STEALTH: u8 = 84;
pub const QUICK_STACK_CHESTS: u8 = 85;
pub const TILE_ENTITY_SHARING: u8 = 86;
pub const TILE_ENTITY_PLACEMENT: u8 = 87;
pub const ITEM_TWEAKER: u8 = 88;
pub const ITEM_FRAME_TRY_PLACING: u8 = 89;
pub const SPAWN_INSTANCED_ITEM: u8 = 90;
pub const SYNC_EMOTE_BUBBLE: u8 = 91;
pub const SYNC_EXTRA_VALUE: u8 = 92;
pub const SOCIAL_HANDSHAKE: u8 = 93;
pub const DEV_COMMANDS: u8 = 94;
pub const MURDER_SOMEONE_ELSES_PORTAL: u8 = 95;
pub const TELEPORT_PLAYER_THROUGH_PORTAL: u8 = 96;
pub const ACHIEVEMENT_MESSAGE_N_P_C_KILLED: u8 = 97;
pub const ACHIEVEMENT_MESSAGE_EVENT_HAPPENED: u8 = 98;
pub const MINION_REST_TARGET_UPDATE: u8 = 99;
pub const TELEPORT_N_P_C_THROUGH_PORTAL: u8 = 100;
pub const UPDATE_TOWER_SHIELD_STRENGTHS: u8 = 101;
pub const NEBULA_LEVELUP_REQUEST: u8 = 102;
pub const MOONLORD_HORROR: u8 = 103;
pub const SHOP_OVERRIDE: u8 = 104;
pub const GEM_LOCK_TOGGLE: u8 = 105;
pub const POOF_OF_SMOKE: u8 = 106;
pub const SMART_TEXT_MESSAGE: u8 = 107;
pub const WIRED_CANNON_SHOT: u8 = 108;
pub const MASS_WIRE_OPERATION: u8 = 109;
pub const MASS_WIRE_OPERATION_PAY: u8 = 110;
pub const TOGGLE_PARTY: u8 = 111;
pub const SPECIAL_F_X: u8 = 112;
pub const CRYSTAL_INVASION_START: u8 = 113;
pub const CRYSTAL_INVASION_WIPE_ALL_THE_THINGSSS: u8 = 114;
pub const MINION_ATTACK_TARGET_UPDATE: u8 = 115;
pub const CRYSTAL_INVASION_SEND_WAIT_TIME: u8 = 116;
pub const PLAYER_HURT_V2: u8 = 117;
pub const PLAYER_DEATH_V2: u8 = 118;
pub const COMBAT_TEXT_STRING: u8 = 119;
pub const EMOJI: u8 = 120;
pub const T_E_DISPLAY_DOLL_DATA_SYNC: u8 = 121;
pub const REQUEST_TILE_ENTITY_INTERACTION: u8 = 122;
pub const WEAPONS_RACK_TRY_PLACING: u8 = 123;
pub const T_E_HAT_RACK_ITEM_SYNC: u8 = 124;
pub const SYNC_TILE_PICKING: u8 = 125;
pub const SYNC_REVENGE_MARKER: u8 = 126;
pub const REMOVE_REVENGE_MARKER: u8 = 127;
pub const LAND_GOLF_BALL_IN_CUP: u8 = 128;
pub const FINISHED_CONNECTING_TO_SERVER: u8 = 129;
pub const FISH_OUT_N_P_C: u8 = 130;
pub const TAMPER_WITH_N_P_C: u8 = 131;
pub const PLAY_LEGACY_SOUND: u8 = 132;
pub const FOOD_PLATTER_TRY_PLACING: u8 = 133;
pub const UPDATE_PLAYER_LUCK_FACTORS: u8 = 134;
pub const DEAD_PLAYER: u8 = 135;
pub const SYNC_CAVERN_MONSTER_TYPE: u8 = 136;
pub const REQUEST_N_P_C_BUFF_REMOVAL: u8 = 137;
pub const CLIENT_SYNCED_INVENTORY: u8 = 138;
pub const SET_COUNTS_AS_HOST_FOR_GAMEPLAY: u8 = 139;
pub const SET_MISC_EVENT_VALUES: u8 = 140;
pub const REQUEST_LUCY_POPUP: u8 = 141;
pub const SYNC_PROJECTILE_TRACKERS: u8 = 142;
pub const CRYSTAL_INVASION_REQUESTED_TO_SKIP_WAIT_TIME: u8 = 143;
pub const REQUEST_QUEST_EFFECT: u8 = 144;
pub const SYNC_ITEMS_WITH_SHIMMER_DEPRECATED: u8 = 145;
pub const SHIMMER_ACTIONS: u8 = 146;
pub const SYNC_LOADOUT: u8 = 147;
pub const SYNC_ITEM_CANNOT_BE_TAKEN_BY_ENEMIES_DEPRECATED: u8 = 148;
pub const DEAD_CELLS_DISPLAY_JAR_TRY_PLACING: u8 = 149;
pub const SPECTATE_PLAYER: u8 = 150;
pub const SYNC_ITEM_DESPAWN: u8 = 151;
pub const ITEM_USE_SOUND: u8 = 152;
pub const N_P_C_DEBUFF_DAMAGE: u8 = 153;
pub const PING: u8 = 154;
pub const SYNC_CHEST_SIZE: u8 = 155;
pub const T_E_LEASHED_ENTITY_ANCHOR_PLACE_ITEM: u8 = 156;
pub const TEAM_CHANGE_FROM_U_I: u8 = 157;
pub const EXTRA_SPAWN_SECTION_LOADED: u8 = 158;
pub const REQUEST_SECTION: u8 = 159;
pub const ITEM_POSITION: u8 = 160;
pub const HOST_TOKEN: u8 = 161;
pub const DAMAGE_N_P_C_ACK: u8 = 162;

/// Human-readable name for a message id, for logs and error messages.
pub fn name(id: u8) -> &'static str {
    match id {
        0 => "NeverCalled",
        1 => "Hello",
        2 => "Kick",
        3 => "PlayerInfo",
        4 => "SyncPlayer",
        5 => "SyncEquipment",
        6 => "RequestWorldData",
        7 => "WorldData",
        8 => "SpawnTileData",
        9 => "StatusTextSize",
        10 => "TileSection",
        11 => "TileFrameSection",
        12 => "PlayerSpawn",
        13 => "PlayerControls",
        14 => "PlayerActive",
        15 => "Unknown15",
        16 => "PlayerLifeMana",
        17 => "TileManipulation",
        18 => "SetTime",
        19 => "ToggleDoorState",
        20 => "AreaTileChange",
        21 => "SyncItem",
        22 => "ItemOwner",
        23 => "SyncNPC",
        24 => "UnusedMeleeStrike",
        25 => "Unused25",
        26 => "Unused26",
        27 => "SyncProjectile",
        28 => "DamageNPC",
        29 => "KillProjectile",
        30 => "TogglePVP",
        31 => "RequestChestOpen",
        32 => "SyncChestItem",
        33 => "SyncPlayerChest",
        34 => "ChestUpdates",
        35 => "PlayerHeal",
        36 => "SyncPlayerZone",
        37 => "RequestPassword",
        38 => "SendPassword",
        39 => "ReleaseItemOwnership",
        40 => "SyncTalkNPC",
        41 => "ItemRotationAndAnimation",
        42 => "Unknown42",
        43 => "ManaEffect",
        44 => "Unknown44",
        45 => "TeamChange",
        46 => "OpenSignRequest",
        47 => "OpenSignResponse",
        48 => "LiquidUpdate",
        49 => "InitialSpawn",
        50 => "PlayerBuffs",
        51 => "MiscDataSync",
        52 => "LockAndUnlock",
        53 => "AddNPCBuff",
        54 => "NPCBuffs",
        55 => "AddPlayerBuffPvP",
        56 => "UniqueTownNPCInfoSyncRequest",
        57 => "Unknown57",
        58 => "InstrumentSound",
        59 => "HitSwitch",
        60 => "Unknown60",
        61 => "SpawnBossUseLicenseStartEvent",
        62 => "SyncDodge",
        63 => "SyncTilePaintOrCoating",
        64 => "SyncWallPaintOrCoating",
        65 => "TeleportEntity",
        66 => "Unknown66",
        67 => "Unknown67",
        68 => "Unknown68",
        69 => "ChestName",
        70 => "BugCatching",
        71 => "BugReleasing",
        72 => "TravelMerchantItems",
        73 => "RequestTeleportationByServer",
        74 => "AnglerQuest",
        75 => "AnglerQuestFinished",
        76 => "QuestsCountSync",
        77 => "TemporaryAnimation",
        78 => "InvasionProgressReport",
        79 => "PlaceObject",
        80 => "SyncPlayerChestIndex",
        81 => "CombatTextInt",
        82 => "NetModules",
        83 => "Unused83",
        84 => "PlayerStealth",
        85 => "QuickStackChests",
        86 => "TileEntitySharing",
        87 => "TileEntityPlacement",
        88 => "ItemTweaker",
        89 => "ItemFrameTryPlacing",
        90 => "SpawnInstancedItem",
        91 => "SyncEmoteBubble",
        92 => "SyncExtraValue",
        93 => "SocialHandshake",
        94 => "DevCommands",
        95 => "MurderSomeoneElsesPortal",
        96 => "TeleportPlayerThroughPortal",
        97 => "AchievementMessageNPCKilled",
        98 => "AchievementMessageEventHappened",
        99 => "MinionRestTargetUpdate",
        100 => "TeleportNPCThroughPortal",
        101 => "UpdateTowerShieldStrengths",
        102 => "NebulaLevelupRequest",
        103 => "MoonlordHorror",
        104 => "ShopOverride",
        105 => "GemLockToggle",
        106 => "PoofOfSmoke",
        107 => "SmartTextMessage",
        108 => "WiredCannonShot",
        109 => "MassWireOperation",
        110 => "MassWireOperationPay",
        111 => "ToggleParty",
        112 => "SpecialFX",
        113 => "CrystalInvasionStart",
        114 => "CrystalInvasionWipeAllTheThingsss",
        115 => "MinionAttackTargetUpdate",
        116 => "CrystalInvasionSendWaitTime",
        117 => "PlayerHurtV2",
        118 => "PlayerDeathV2",
        119 => "CombatTextString",
        120 => "Emoji",
        121 => "TEDisplayDollDataSync",
        122 => "RequestTileEntityInteraction",
        123 => "WeaponsRackTryPlacing",
        124 => "TEHatRackItemSync",
        125 => "SyncTilePicking",
        126 => "SyncRevengeMarker",
        127 => "RemoveRevengeMarker",
        128 => "LandGolfBallInCup",
        129 => "FinishedConnectingToServer",
        130 => "FishOutNPC",
        131 => "TamperWithNPC",
        132 => "PlayLegacySound",
        133 => "FoodPlatterTryPlacing",
        134 => "UpdatePlayerLuckFactors",
        135 => "DeadPlayer",
        136 => "SyncCavernMonsterType",
        137 => "RequestNPCBuffRemoval",
        138 => "ClientSyncedInventory",
        139 => "SetCountsAsHostForGameplay",
        140 => "SetMiscEventValues",
        141 => "RequestLucyPopup",
        142 => "SyncProjectileTrackers",
        143 => "CrystalInvasionRequestedToSkipWaitTime",
        144 => "RequestQuestEffect",
        145 => "SyncItemsWithShimmerDeprecated",
        146 => "ShimmerActions",
        147 => "SyncLoadout",
        148 => "SyncItemCannotBeTakenByEnemiesDeprecated",
        149 => "DeadCellsDisplayJarTryPlacing",
        150 => "SpectatePlayer",
        151 => "SyncItemDespawn",
        152 => "ItemUseSound",
        153 => "NPCDebuffDamage",
        154 => "Ping",
        155 => "SyncChestSize",
        156 => "TELeashedEntityAnchorPlaceItem",
        157 => "TeamChangeFromUI",
        158 => "ExtraSpawnSectionLoaded",
        159 => "RequestSection",
        160 => "ItemPosition",
        161 => "HostToken",
        162 => "DamageNPCAck",
        _ => "Unknown",
    }
}

/// Packet 42 is `Unknown42` in the shipped enum, but `NetMessage.SendData` case 42 writes
/// player mana. Aliased for readability at the call sites.
pub const PLAYER_MANA: u8 = UNKNOWN42;

/// Packet 68 is `Unknown68` in the shipped enum and the client reads a string it then discards.
/// It was `ClientUUID` in 1.4.4 and clients still send their UUID here.
pub const CLIENT_UUID: u8 = UNKNOWN68;

#[cfg(test)]
mod version_policy {
    use super::*;

    /// Fires when Terraria moves, so it is found here rather than by a user being turned away.
    ///
    /// **When this fails**, a new release exists and three things need checking, in this order:
    ///
    /// 1. `SUPPORTED_RELEASES` and `CUR_RELEASE` here.
    /// 2. Whether packet 7 changed. 325 -> 326 appended `dungeonX`/`dungeonY` as two `i16`s, which
    ///    is the entire wire difference between them; `WORLD_DATA_FIXED_LEN` and the pinned
    ///    `REAL_SERVER_PACKET_7` capture in `packets.rs` both encode that.
    /// 3. `wld::MAX_VERSION`, if the *save* format moved too. It did not between 325 and 326.
    ///
    /// The point is that the next release should be a morning's work for somebody who has never
    /// seen this code, rather than an archaeology project.
    #[test]
    fn the_supported_releases_are_the_ones_we_think() {
        assert_eq!(
            SUPPORTED_RELEASES,
            &[325, 326],
            "the set of releases this server speaks has changed; see this test's comment"
        );
        assert_eq!(CUR_RELEASE, 326, "the release we announce as our own");
        assert!(
            SUPPORTED_RELEASES.contains(&CUR_RELEASE),
            "we must accept the release we claim to be"
        );
        assert!(
            SUPPORTED_RELEASES.windows(2).all(|w| w[0] < w[1]),
            "kept in order, so the newest is always last"
        );
    }
}
