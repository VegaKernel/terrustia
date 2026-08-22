use crate::{error::Result, reader::PacketReader, writer::Writer};

/// How a [`NetworkText`] should be interpreted by the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TextMode {
    /// The string is shown as-is.
    #[default]
    Literal = 0,
    /// The string is a format template filled from the substitutions.
    Formatted = 1,
    /// The string is a localisation key resolved client-side.
    LocalizationKey = 2,
}

impl TextMode {
    fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::Formatted,
            2 => Self::LocalizationKey,
            _ => Self::Literal,
        }
    }
}

/// Terraria's wire string-with-substitutions, used by kicks, status text, and chat.
///
/// A literal carries no substitution list at all — the count byte is only present for the other
/// two modes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetworkText {
    pub mode: TextMode,
    pub text: String,
    pub substitutions: Vec<NetworkText>,
}

impl NetworkText {
    pub fn literal(text: impl Into<String>) -> Self {
        Self {
            mode: TextMode::Literal,
            text: text.into(),
            substitutions: Vec::new(),
        }
    }

    /// A localisation key the client resolves in its own language.
    pub fn key(key: impl Into<String>, substitutions: Vec<NetworkText>) -> Self {
        Self {
            mode: TextMode::LocalizationKey,
            text: key.into(),
            substitutions,
        }
    }

    pub fn write(&self, out: &mut Writer) {
        out.u8(self.mode as u8).string(&self.text);
        if self.mode != TextMode::Literal {
            out.u8(self.substitutions.len() as u8);
            for sub in &self.substitutions {
                sub.write(out);
            }
        }
    }

    pub fn read(r: &mut PacketReader<'_>) -> Result<Self> {
        let mode = TextMode::from_byte(r.u8()?);
        let text = r.string()?;
        let mut substitutions = Vec::new();
        if mode != TextMode::Literal {
            let count = r.u8()?;
            substitutions.reserve(usize::from(count));
            for _ in 0..count {
                substitutions.push(Self::read(r)?);
            }
        }
        Ok(Self {
            mode,
            text,
            substitutions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &NetworkText) {
        let mut w = Writer::new();
        text.write(&mut w);
        let bytes = w.into_bytes();
        let mut r = PacketReader::new(&bytes);
        assert_eq!(&NetworkText::read(&mut r).unwrap(), text);
        assert!(r.is_empty());
    }

    #[test]
    fn literal_omits_the_substitution_count() {
        let text = NetworkText::literal("hello");
        let mut w = Writer::new();
        text.write(&mut w);
        // mode byte, length byte, five characters — and nothing else.
        assert_eq!(w.as_slice(), &[0, 5, b'h', b'e', b'l', b'l', b'o']);
        round_trip(&text);
    }

    #[test]
    fn keys_carry_nested_substitutions() {
        round_trip(&NetworkText::key(
            "Net.PlayerJoined",
            vec![NetworkText::literal("brooklyn")],
        ));
    }

    #[test]
    fn nesting_survives_two_levels() {
        round_trip(&NetworkText::key(
            "outer",
            vec![NetworkText::key(
                "inner",
                vec![NetworkText::literal("leaf")],
            )],
        ));
    }
}
