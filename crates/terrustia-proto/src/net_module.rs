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
