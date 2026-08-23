# Packet coverage

Terraria 1.4.5.7 defines 162 message ids, of which **148 are live** (the rest are unused slots or
deprecated). This is what this server does with them, and what it does not.

Regenerate this picture at any time:

```sh
python3 tools/packet_audit.py <path-to-decompiled-tree>
```

Without the tree argument it still reports coverage; with it, it also splits the gaps by whether
the *client* ever sends the message — a message only the server sends is not a gap on the
receiving side.

## Where it stands

| | |
|---|---:|
| Live messages | 148 |
| Referenced anywhere (sent or received) | 118 |
| Dispatched inbound | 97 |
| Never touched | 30 |
| ...of those, ones a client actually sends | **5** |

For comparison, the audit that started this work found **76 never touched**, including NPC
debuffs, town NPC names, every tile entity, all five server-side teleports, the Angler's whole
quest system, the invasion progress bar, the lunar pillar shields and the Grand Design.

## Still missing, and what it costs

### A client sends these (5)

| Id | Name | Cost of not handling |
|---:|---|---|
| 94 | `DevCommands` | none — developer-only, and handling it would be a liability |
| 107 | `SmartTextMessage` | signs and tombstones do not announce themselves in chat |
| 130 | `FishOutNPC` | an NPC caught while fishing does not appear |
| 140 | `SetMiscEventValues` | some event counters do not reach other clients |
| 141 | `RequestLucyPopup` | Lucy the Axe says nothing |

### Server-to-client only (25)

Grouped by what they would buy:

**Shops** — `TravelingMerchantItems` (72), `ShopOverride` (104). The Travelling Merchant's stock
and any overridden shop. Both need a shop model that does not exist yet.

**Storage** — `QuickStackChests` (85), `SyncPlayerChestIndex` (80), `ItemTweaker` (88). Quick
stack to nearby chests is the notable one; it is used constantly and is not hard, just not done.

**Shimmer** — `ShimmerActions` (146). Transmutation is a 1.4.4 system this server does not model
at all.

**The Old One's Army's timing** — `CrystalInvasionWipeAllTheThingssss` (114),
`CrystalInvasionSendWaitTime` (116). The event runs; clients are not told how long the gap
between waves has left.

**Cosmetic** — `TemporaryAnimation` (77), `PoofOfSmoke` (106), `PlayLegacySound` (132),
`WiredCannonShot` (108), `TamperWithNPC` (131). Effects other clients would otherwise not see.

**Achievements** — `AchievementMessageNPCKilled` (97), `AchievementMessageEventHappened` (98).

**Not applicable** — `TileFrameSection` (11) is legacy, `SocialHandshake` (93) is Steam,
`SpectatePlayer` (150) and `HostToken` (161) are host-migration, `SetCountsAsHostForGameplay` (139)
and `ClientSyncedInventory` (138) are journey-mode and server-side-character features this build
does not offer.

**Other** — `TeleportNPCThroughPortal` (100) needs NPCs to use portals, which they do not here.
`SyncCavernMonsterType` (136), `ExtraSpawnSectionLoaded` (158), `ItemPosition` (160).

## What was fixed, and in what order

Roughly by how much of the game each unblocked:

1. **NPC debuffs** (53/54/137/153) — no enemy in the world had ever been on fire. Also the reason
   ichor did nothing: clients compute armour penetration from a buff list they were never sent.
   See [buffs.md](buffs.md).
2. **Tile entities** (86/122/89/121/123/124/133/149/156) — pylons, item frames, mannequins, racks.
   All placed and then never mentioned again, and not read from the world file either. See
   [tile-entities.md](tile-entities.md).
3. **Server-side teleports** (73) — the Teleportation Potion, both conches, the Shellphone, and
   the crush rescue. See [teleports.md](teleports.md).
4. **The Grand Design** (109/110) — every wiring tool past the first. See [wiring.md](wiring.md).
5. **Town NPC names** (56) — a world full of people called "Guide".
6. **The Angler** (74/75/76/144) — a hundred and fifty quests deep, entirely inert.
7. **Progress feedback** (78/101/103) — the invasion bar, the pillar shields, the Moon Lord's
   countdown. All tracked correctly and told to nobody.
8. **Smaller ones** — chest names for the map (69), gem locks (105), portals (95/96), Nebula
   boosters (102), item ownership release (39), coin-loss revenge (92), the Old One's Army's skip
   (143).

## A note on the audit's method

`packet_audit.py` decides "the client sends this" by checking whether any `NetMessage.SendData`
call site for that id sits under a `netMode == 1` guard. That is a heuristic and it is
deliberately loose in the safe direction: it will occasionally flag something as client-sent that
is only sent by a host, which costs a second look, rather than missing one that matters.
