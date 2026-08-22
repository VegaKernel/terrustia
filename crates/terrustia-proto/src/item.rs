use crate::{error::Result, reader::PacketReader, writer::Writer};

/// One stack of items, as stored in a chest slot or dropped in the world.
///
/// Terraria treats "no item" as a zero stack rather than an absent slot, and the id of an empty
/// slot is meaningless, so [`ItemStack::EMPTY`] normalises both to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemStack {
    pub id: i32,
    pub stack: i16,
    /// Reforge prefix (0 = none).
    pub prefix: u8,
}

impl ItemStack {
    pub const EMPTY: Self = Self {
        id: 0,
        stack: 0,
        prefix: 0,
    };

    pub fn new(id: i32, stack: i16, prefix: u8) -> Self {
        if stack == 0 {
            Self::EMPTY
        } else {
            Self { id, stack, prefix }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stack == 0
    }

    /// Read a slot in the `.wld` chest encoding: a stack, then the item only if the stack is
    /// non-zero.
    pub fn read_save(r: &mut PacketReader<'_>) -> Result<Self> {
        let stack = r.i16()?;
        if stack == 0 {
            return Ok(Self::EMPTY);
        }
        let id = r.i32()?;
        let prefix = r.u8()?;
        Ok(Self { id, stack, prefix })
    }

    /// Write a slot in the `.wld` chest encoding.
    ///
    /// A negative stack is normalised to 1, exactly as the game's writer does. Its reader accepts
    /// negatives defensively, but writing one would produce bytes the game itself would never
    /// emit, and re-saving a world should not change how it round-trips.
    pub fn write_save(&self, w: &mut Writer) {
        let stack = if self.stack < 0 { 1 } else { self.stack };
        w.i16(stack);
        if stack > 0 {
            w.i32(self.id).u8(self.prefix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_slot_is_two_bytes() {
        let mut w = Writer::new();
        ItemStack::EMPTY.write_save(&mut w);
        assert_eq!(w.as_slice(), &[0, 0]);
    }

    #[test]
    fn a_filled_slot_round_trips() {
        let item = ItemStack::new(3507, 42, 58);
        let mut w = Writer::new();
        item.write_save(&mut w);
        let bytes = w.into_bytes();
        assert_eq!(bytes.len(), 7); // stack + id + prefix

        let mut r = PacketReader::new(&bytes);
        assert_eq!(ItemStack::read_save(&mut r).unwrap(), item);
        assert!(r.is_empty());
    }

    #[test]
    fn a_zero_stack_normalises_to_empty() {
        // A slot with an id but no count is not a half-present item; it is nothing.
        assert_eq!(ItemStack::new(3507, 0, 12), ItemStack::EMPTY);
        assert!(ItemStack::new(3507, 0, 12).is_empty());
    }

    #[test]
    fn a_negative_stack_is_written_as_one() {
        // The game's writer clamps before writing; its reader then accepts negatives defensively.
        // Matching the writer keeps our saves byte-comparable with the game's.
        let mut w = Writer::new();
        ItemStack::new(17, -5, 2).write_save(&mut w);
        let bytes = w.into_bytes();
        let decoded = ItemStack::read_save(&mut PacketReader::new(&bytes)).unwrap();
        assert_eq!(decoded, ItemStack::new(17, 1, 2));
    }

    #[test]
    fn a_negative_stack_read_from_a_save_keeps_its_item() {
        // Old saves can contain them; the item must not be lost.
        let mut w = Writer::new();
        w.i16(-3).i32(42).u8(1);
        let bytes = w.into_bytes();
        let decoded = ItemStack::read_save(&mut PacketReader::new(&bytes)).unwrap();
        assert_eq!(decoded.id, 42);
        assert_eq!(decoded.prefix, 1);
    }
}
