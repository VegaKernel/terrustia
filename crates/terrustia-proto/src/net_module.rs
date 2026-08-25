//! Packet `82` carries a "net module" identified by a leading `u16`.
//!
//! Module ids come from the registration order in `Terraria.Initializers.NetworkInitializer`, so
//! they shift whenever a module is inserted. Verified against the 1.4.5.7 build.

use crate::{
    error::{ProtoError, Result},
    id,
    net_text::NetworkText,
    reader::PacketReader,
    writer::PacketWriter,
};

pub const MODULE_LIQUID: u16 = 0;
pub const MODULE_TEXT: u16 = 1;
pub const MODULE_PING: u16 = 2;
/// `BannerSystem.NetBannersModule`, twelfth in the registration order.
pub const MODULE_BANNERS: u16 = 11;

/// How many banner slots the game's `killCount` array has.
///
/// Confirmed on the wire rather than counted from a table: a real server's full-state module opens
/// with `0x0125`, which is 293.
pub const BANNER_SLOTS: usize = 293;

/// One tile whose liquid has changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiquidChange {
    pub x: i32,
    pub y: i32,
    /// How full the tile is, 0 to 255.
    pub amount: u8,
    /// 0 water, 1 lava, 2 honey, 3 shimmer.
    pub kind: u8,
}

/// The most liquid changes that fit in one module-0 frame.
///
/// The count is a `u16`, and each entry is six bytes, so the ceiling is really the frame limit
/// rather than the counter. Kept well under it so a settling ocean splits across frames instead of
/// producing one the writer refuses.
pub const MAX_LIQUID_CHANGES: usize = 1000;

/// Module 0: liquid levels, as the game sends them.
///
/// This is the message the client expects for water moving, and it is a sixth the size of the tile
/// squares that would otherwise carry the same news — a settling pool dirties a stripe of tiles
/// every tick, so the difference is the difference between a trickle of traffic and a flood.
///
/// The coordinate is packed into one `i32` as `(x << 16) | y`, which is why a world wider than
/// 65535 tiles could not be described by it. Terraria's largest is 8400.
pub fn liquid_changes(changes: &[LiquidChange]) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_LIQUID).u16(changes.len() as u16);
    for change in changes {
        w.i32(((change.x & 0xFFFF) << 16) | (change.y & 0xFFFF))
            .u8(change.amount)
            .u8(change.kind);
    }
    w.finish()
}

/// `NetTeleportPylonModule`, ninth in the registration order.
pub const MODULE_PYLON: u16 = 8;

/// What a module-8 frame is saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PylonMessage {
    /// Server to client: this pylon exists, put it on the map.
    Added = 0,
    /// Server to client: it does not any more.
    Removed = 1,
    /// Client to server: take me to it.
    RequestTeleport = 2,
}

/// One pylon, as module 8 describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pylon {
    pub x: i16,
    pub y: i16,
    /// Which biome's pylon: 0 surface purity, 1 jungle, 2 hallow, 3 underground, 4 beach,
    /// 5 desert, 6 snow, 7 glowing mushroom, 8 victory.
    pub kind: u8,
}

impl Pylon {
    /// The Victory pylon, the one kind that needs no townsfolk around it.
    pub const VICTORY: u8 = 8;
}

/// Module 8: a pylon appeared or vanished.
///
/// The client keeps its own list and draws the travel map from it. A pylon it was never told about
/// is scenery: standing next to it opens a map with nowhere to go.
pub fn pylon_message(message: PylonMessage, pylon: Pylon) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_PYLON)
        .u8(message as u8)
        .i16(pylon.x)
        .i16(pylon.y)
        .u8(pylon.kind);
    w.finish()
}

/// Read a module-8 frame, returning `None` for any other module.
pub fn decode_pylon_message(payload: &[u8]) -> Result<Option<(PylonMessage, Pylon)>> {
    let mut r = PacketReader::new(payload);
    if r.u16()? != MODULE_PYLON {
        return Ok(None);
    }
    let message = match r.u8()? {
        0 => PylonMessage::Added,
        1 => PylonMessage::Removed,
        2 => PylonMessage::RequestTeleport,
        other => {
            return Err(ProtoError::OutOfRange {
                field: "pylon message type",
                value: i64::from(other),
            });
        }
    };
    Ok(Some((
        message,
        Pylon {
            x: r.i16()?,
            y: r.i16()?,
            kind: r.u8()?,
        },
    )))
}

/// Read a module-0 payload back into the changes it describes.
///
/// Returns `None` for any other module, so a caller can hand it every packet 82 it sees.
pub fn decode_liquid_changes(payload: &[u8]) -> Result<Option<Vec<LiquidChange>>> {
    let mut r = PacketReader::new(payload);
    if r.u16()? != MODULE_LIQUID {
        return Ok(None);
    }
    let count = usize::from(r.u16()?);
    let mut changes = Vec::with_capacity(count.min(MAX_LIQUID_CHANGES));
    for _ in 0..count {
        let packed = r.i32()?;
        changes.push(LiquidChange {
            x: (packed >> 16) & 0xFFFF,
            y: packed & 0xFFFF,
            amount: r.u8()?,
            kind: r.u8()?,
        });
    }
    Ok(Some(changes))
}

/// Module 11, message 0: every banner's kill count and claim count at once.
///
/// Sent as a player joins. Without it the client's bestiary shows nought kills for everything,
/// however many the world has recorded — the counts live only on the server, and there is no other
/// message that carries them.
///
/// `claimable` is what the game hands out when a threshold is crossed and the player has not
/// collected the banner yet. This server drops the banner as an item on the spot instead, so it
/// has nothing to claim later and sends zeroes; the counts, which are what the bestiary actually
/// displays, are real.
pub fn banners_full_state(
    kills: &[u32; BANNER_SLOTS],
    claimable: &[u16; BANNER_SLOTS],
) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_BANNERS).u8(0).i16(BANNER_SLOTS as i16);
    for count in kills {
        // The game stores these as signed ints, and a count large enough to wrap is one no world
        // will ever reach; saturating keeps the wire value sane if one somehow does.
        w.i32((*count).min(i32::MAX as u32) as i32);
    }
    w.i16(BANNER_SLOTS as i16);
    for count in claimable {
        w.u16(*count);
    }
    w.finish()
}

/// Module 11, message 1: one banner's kill count has changed.
///
/// Sent on every kill that counts towards a banner, so the bestiary's counter ticks up while the
/// player watches rather than only on their next join.
pub fn banner_kill_count(banner: u16, kills: u32) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_BANNERS)
        .u8(1)
        .i16(banner as i16)
        .i32(kills.min(i32::MAX as u32) as i32);
    w.finish()
}

/// A chat message as the client sends it: a command name, then the text.
///
/// The command is usually `Say`; `Emote` and the party/whisper commands use the same shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingChat {
    pub command: String,
    pub text: String,
}

impl IncomingChat {
    /// Parse a packet `82` payload, returning `None` when it is some other module.
    pub fn decode(payload: &[u8]) -> Result<Option<Self>> {
        let mut r = PacketReader::new(payload);
        if r.u16()? != MODULE_TEXT {
            return Ok(None);
        }
        Ok(Some(Self {
            command: r.string()?,
            text: r.string()?,
        }))
    }

    /// Whether this is ordinary chat rather than an emote or a party command.
    pub fn is_say(&self) -> bool {
        self.command.eq_ignore_ascii_case("Say")
    }
}

/// Build the server-to-client form of a chat line.
pub fn chat_broadcast(author: u8, text: &NetworkText, color: [u8; 3]) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::NET_MODULES);
    w.u16(MODULE_TEXT).u8(author);
    text.write(&mut w);
    w.rgb(color);
    w.finish()
}

/// The slot value the client renders as "from the server" rather than from a player.
pub const SERVER_AUTHOR: u8 = 255;

/// Reject a chat line that is empty or absurdly long before it reaches other players.
pub fn validate_chat(text: &str, max_len: usize) -> Result<()> {
    if text.is_empty() || text.len() > max_len {
        return Err(ProtoError::OutOfRange {
            field: "chat length",
            value: text.len() as i64,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Writer;

    /// Module 0 exactly as a real 1.4.5.8 server sent it.
    ///
    /// Two tiles of water settling near the surface of `ProbeTiny`, captured from the game's own
    /// dedicated server. The packing is the part worth pinning: the coordinate is one `i32` with
    /// **x in the high half**, which reads as a plausible position either way round on a square
    /// world and puts every splash in the wrong place on a real one.
    #[test]
    fn the_liquid_module_is_packed_the_way_a_real_server_packs_it() {
        const REAL: &[u8] = &[
            0x00, 0x00, 0x02, 0x00, 0x58, 0x01, 0xed, 0x09, 0xff, 0x00, 0x58, 0x01, 0xec, 0x09,
            0xff, 0x00,
        ];
        let frame = liquid_changes(&[
            LiquidChange {
                x: 2541,
                y: 344,
                amount: 255,
                kind: 0,
            },
            LiquidChange {
                x: 2540,
                y: 344,
                amount: 255,
                kind: 0,
            },
        ])
        .unwrap();
        assert_eq!(&frame[3..], REAL);
    }

    /// The banner module's full state, shaped exactly as a real 1.4.5.8 server sends it.
    ///
    /// Its frame was captured from the game's own dedicated server serving a fresh world, and came
    /// to 1765 bytes: the module id, a message byte, then two counted arrays of 293 entries — ints
    /// for kills and shorts for claims. Nothing about that is guessable from the field names, and
    /// a length read at the wrong width here desynchronises the client for the rest of the session.
    #[test]
    fn the_banner_full_state_is_the_shape_a_real_server_sends() {
        let kills = [0u32; BANNER_SLOTS];
        let claimable = [0u16; BANNER_SLOTS];
        let frame = banners_full_state(&kills, &claimable).unwrap();
        let payload = &frame[3..];

        assert_eq!(payload.len(), 1765, "a real server's frame was 1765 bytes");
        assert_eq!(u16::from_le_bytes([payload[0], payload[1]]), MODULE_BANNERS);
        assert_eq!(payload[2], 0, "message type 0 is the full state");
        assert_eq!(i16::from_le_bytes([payload[3], payload[4]]), 293);
        // The second length sits immediately after 293 four-byte kill counts.
        let at = 5 + BANNER_SLOTS * 4;
        assert_eq!(i16::from_le_bytes([payload[at], payload[at + 1]]), 293);
        assert_eq!(at + 2 + BANNER_SLOTS * 2, payload.len());
    }

    #[test]
    fn a_banner_kill_count_update_carries_the_banner_and_its_total() {
        let frame = banner_kill_count(7, 123).unwrap();
        let payload = &frame[3..];
        assert_eq!(u16::from_le_bytes([payload[0], payload[1]]), MODULE_BANNERS);
        assert_eq!(payload[2], 1, "message type 1 is a kill-count update");
        assert_eq!(i16::from_le_bytes([payload[3], payload[4]]), 7);
        assert_eq!(
            i32::from_le_bytes([payload[5], payload[6], payload[7], payload[8]]),
            123
        );
        assert_eq!(payload.len(), 9);
    }

    #[test]
    fn decodes_a_say_message() {
        let mut w = Writer::new();
        w.u16(MODULE_TEXT).string("Say").string("hello world");
        let chat = IncomingChat::decode(w.as_slice()).unwrap().unwrap();
        assert!(chat.is_say());
        assert_eq!(chat.text, "hello world");
    }

    #[test]
    fn ignores_other_modules() {
        let mut w = Writer::new();
        w.u16(MODULE_LIQUID).bytes(&[1, 2, 3]);
        assert_eq!(IncomingChat::decode(w.as_slice()).unwrap(), None);
    }

    #[test]
    fn broadcast_has_module_author_text_and_colour() {
        let frame = chat_broadcast(2, &NetworkText::literal("hi"), [255, 128, 0]).unwrap();
        assert_eq!(frame[2], id::NET_MODULES);
        let mut r = PacketReader::new(&frame[3..]);
        assert_eq!(r.u16().unwrap(), MODULE_TEXT);
        assert_eq!(r.u8().unwrap(), 2);
        assert_eq!(NetworkText::read(&mut r).unwrap().text, "hi");
        assert_eq!(r.rgb().unwrap(), [255, 128, 0]);
        assert!(r.is_empty());
    }

    #[test]
    fn empty_and_oversized_chat_is_refused() {
        assert!(validate_chat("", 500).is_err());
        assert!(validate_chat(&"x".repeat(501), 500).is_err());
        assert!(validate_chat("ok", 500).is_ok());
    }

    #[test]
    fn a_truncated_module_payload_is_an_error() {
        assert!(IncomingChat::decode(&[1]).is_err());
        let mut w = Writer::new();
        w.u16(MODULE_TEXT).string("Say"); // missing the text
        assert!(IncomingChat::decode(w.as_slice()).is_err());
    }
}
