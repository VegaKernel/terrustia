//! Decoding packet 4 (`SYNC_PLAYER`, `PlayerInfo`): what a client says about how it looks.
//!
//! `game::server::on_sync_player` only reads as far as the name and relays the rest of the payload
//! verbatim — the server has never needed a player's skin/hair/gear colour for anything, so nothing
//! decoded past that point. The web panel's live world view does: it draws a stylized, procedural
//! avatar from real appearance data rather than a composited sprite (see `panel/mod.rs`'s module
//! doc), which means somebody has to actually parse the rest of the packet.
//!
//! The field order here is not a guess. It is transcribed from `terrustia-client`'s own
//! `appearance_packet` (`crates/terrustia-client/src/lib.rs`), a real client stand-in built and
//! exercised against this server over real sockets throughout this workspace's integration tests —
//! its doc comment says outright that it mirrors "the field order the 1.4.5.7 client uses." That is
//! the same authority `on_sync_player`'s own comment leans on for the fields it does read (slot,
//! skin variant, voice variant, voice pitch, hair, name), so this simply continues past where that
//! function stops.

use crate::{Result, reader::PacketReader};

/// The subset of a player's appearance a stylized avatar needs: real colour data, not a sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAppearance {
    pub skin_variant: u8,
    pub hair_style: u8,
    pub hair_color: [u8; 3],
    pub skin_color: [u8; 3],
    pub eye_color: [u8; 3],
    pub shirt_color: [u8; 3],
    pub undershirt_color: [u8; 3],
    pub pants_color: [u8; 3],
    pub shoe_color: [u8; 3],
}

impl PlayerAppearance {
    /// `payload` is a packet-4 body with the leading message id already stripped — the same bytes
    /// `game::server::Player::appearance` stores.
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        r.u8()?; // slot — the caller already knows who this is
        let skin_variant = r.u8()?;
        r.u8()?; // voice variant
        r.f32()?; // voice pitch offset
        let hair_style = r.u8()?;
        r.string()?; // name — already carried on `Player` itself
        r.u8()?; // hair dye
        r.u16()?; // hidden-accessory visibility bits
        r.u8()?; // hidden misc-slot bits
        let hair_color = r.rgb()?;
        let skin_color = r.rgb()?;
        let eye_color = r.rgb()?;
        let shirt_color = r.rgb()?;
        let undershirt_color = r.rgb()?;
        let pants_color = r.rgb()?;
        let shoe_color = r.rgb()?;
        Ok(Self {
            skin_variant,
            hair_style,
            hair_color,
            skin_color,
            eye_color,
            shirt_color,
            undershirt_color,
            pants_color,
            shoe_color,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id;
    use crate::writer::PacketWriter;

    /// Built the same way `terrustia-client`'s `appearance_packet` builds a real one — same field
    /// order, same call shape — so this is a real round trip through the wire format, not a decoder
    /// tested only against its own assumptions.
    fn a_real_looking_appearance_packet() -> Vec<u8> {
        let mut w = PacketWriter::new(id::SYNC_PLAYER);
        w.u8(3) // slot
            .u8(2) // skin variant
            .u8(1) // voice variant
            .f32(0.0) // voice pitch offset
            .u8(53) // hair
            .string("Brooklyn")
            .u8(0) // hair dye
            .u16(0) // accessory visibility
            .u8(0) // hidden misc slots
            .rgb([215, 90, 55]) // hair
            .rgb([255, 125, 90]) // skin
            .rgb([105, 90, 75]) // eyes
            .rgb([175, 165, 140]) // shirt
            .rgb([160, 180, 215]) // undershirt
            .rgb([255, 230, 175]) // pants
            .rgb([160, 105, 60]) // shoes
            .u8(0) // difficulty / extra accessory
            .u8(0) // torch flags
            .u8(0); // consumable flags
        w.finish().unwrap()
    }

    #[test]
    fn decodes_every_colour_from_a_real_shaped_packet() {
        let frame = a_real_looking_appearance_packet();
        // The frame is length-prefixed and carries the message id; `on_sync_player` only ever sees
        // what comes after both, so strip the same three bytes here.
        let payload = &frame[3..];
        let appearance = PlayerAppearance::decode(payload).expect("a well-formed packet decodes");
        assert_eq!(appearance.skin_variant, 2);
        assert_eq!(appearance.hair_style, 53);
        assert_eq!(appearance.hair_color, [215, 90, 55]);
        assert_eq!(appearance.skin_color, [255, 125, 90]);
        assert_eq!(appearance.eye_color, [105, 90, 75]);
        assert_eq!(appearance.shirt_color, [175, 165, 140]);
        assert_eq!(appearance.undershirt_color, [160, 180, 215]);
        assert_eq!(appearance.pants_color, [255, 230, 175]);
        assert_eq!(appearance.shoe_color, [160, 105, 60]);
    }

    #[test]
    fn a_truncated_packet_is_refused_rather_than_read_out_of_bounds() {
        let frame = a_real_looking_appearance_packet();
        let payload = &frame[3..frame.len() - 10];
        assert!(PlayerAppearance::decode(payload).is_err());
    }
}
