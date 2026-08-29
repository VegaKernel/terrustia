# Packet coverage

Terraria 1.4.5.7 (release 325) and 1.4.5.8 (release 326) share this wire format and define 162
message ids, of which **148 are live** (the rest are unused slots or deprecated). This is what this
server does with them, and what it does not. `terrustia-proto::id` accepts both releases and uses
`Terraria326` for current-client handshakes.

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
| Referenced anywhere (sent or received) | 139 |
| Dispatched inbound | 111 |
| Never touched | 11 |
| ...of those, ones a client actually sends | **1** |

For comparison, the audit that started this work found **76 never touched**, including NPC
debuffs, town NPC names, every tile entity, all five server-side teleports, the Angler's whole
quest system, the invasion progress bar, the lunar pillar shields and the Grand Design.

## Still missing, and what it costs

### A client sends this one (1)

`DevCommands` (94), and it will stay unhandled. It is a channel for a client to ask the server to
do developer things, and a public server that honours it is a public server anyone can rewrite.

### Server-to-client only (10)

Every one of these needs a system this build does not have, or does not apply to a dedicated
server at all.

**Needs a shop model** — `ShopOverride` (104). Town NPC inventories with prices. The Travelling
Merchant's own stock (72) *is* implemented; this is the rest of them.

**Not applicable to this build** — `TileFrameSection` (11) is legacy; `SocialHandshake` (93) is
Steam; `SpectatePlayer` (150) and `HostToken` (161) are host migration; `ClientSyncedInventory`
(138) is server-side characters.

**Client-side bookkeeping** — `SyncPlayerChestIndex` (80), `ExtraSpawnSectionLoaded` (158).

**Other** — `ItemTweaker` (88) is a modding hook. `TeleportNPCThroughPortal` (100) needs NPCs to
use portals, which they do not here.

`SetCountsAsHostForGameplay` (139) — a loopback-connected player counts as the host, the game's
own `DoesPlayerSlotCountAsAHost` rule — was already implemented (`introduce`'s own call to
`packets::counts_as_host`, sent to a newly-spawned local player and nobody else) but this page had
never been updated to say so.

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
8. **Quick stack** (85) — one of the most-used buttons in the game, and one of the few inventory
   operations that genuinely has to be the server's.
9. **World-specific cavern monsters** (136) — each world draws six of the thirteen from its own
   id, which is why two worlds feel different underground.
10. **Shimmer** (146) — the 1.4.4 transmutation pool, which did not exist here at all. See
    [shimmer.md](shimmer.md); decrafting is still missing and is recorded there.
11. **The Travelling Merchant** (72) — who did not exist here either. He arrives at random during
    the morning once the town has two other residents, carries four to six things chosen by the
    game's own chain of rolls, and leaves at dusk.
12. **Smaller ones** — chest names for the map (69), gem locks (105), portals (95/96), Nebula
    boosters (102), item ownership release (39), coin-loss revenge (92), the Old One's Army's
    skip, countdown and field-wipe (143/116/114), fished-up NPCs (130), the two made town slimes
    (140), Lucy (141), signs read aloud (107), item drift correction (160), and the cosmetic
    effects other clients would otherwise not see (77, 97, 98, 106, 108, 131, 132).
13. **PvP buff spread** (55) — a hostile-flagged player's own hit spreading one of `Main.pvpBuff`'s
    twenty real buffs onto another hostile-flagged player never reached anyone; every real
    trigger for it (certain thrown potions and traps with a PvP-specific effect) was silently
    inert in a real fight. Genuinely a relay, not a broadcast: sent to exactly the named target,
    the same way real vanilla's own server does it. The whitelist itself is generated
    (`terrustia_proto::buffs::PVP_BUFF`, from `Main.pvpBuff`) rather than hand-copied, matching
    every other per-buff table on this page.

## A note on the audit's method

`packet_audit.py` decides "the client sends this" by checking whether any `NetMessage.SendData`
call site for that id sits under a `netMode == 1` guard. That is a heuristic and it is
deliberately loose in the safe direction: it will occasionally flag something as client-sent that
is only sent by a host, which costs a second look, rather than missing one that matters.
