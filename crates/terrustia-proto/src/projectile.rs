//! Projectile networking: packets `27` (SyncProjectile) and `29` (KillProjectile).
//!
//! The identity of a projectile is not a slot number but a packed key: eight bits of owner, ten of
//! index and fourteen of generation, all in one `i32`. That generation counter is what stops a late
//! kill packet from destroying whatever has since taken the slot.
//!
//! Packet 27 is bit-packed in the same spirit as the NPC sync: two flag bytes say which of the AI
//! slots and which of the damage fields are non-zero, so an ordinary projectile with a single AI
//! value and no knockback costs a fraction of the worst case.

use crate::{error::Result, id, reader::PacketReader, writer::PacketWriter};

/// Terraria keeps a thousand projectile slots.
pub const MAX_PROJECTILES: usize = 1000;

/// AI slots a projectile carries. Only three are sent.
pub const MAX_AI: usize = 3;

/// The owner a server-spawned projectile carries: nobody.
pub const SERVER_OWNER: u8 = 255;

/// A projectile's identity on the wire.
///
/// The generation is the important part. Slots are reused constantly, so a kill packet that
/// arrives a moment late would otherwise destroy an innocent bystander; carrying the generation
/// means such a packet simply fails to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectileKey {
    pub owner: u8,
    pub index: u16,
    pub generation: u16,
}

impl ProjectileKey {
    /// Pack into the `i32` the wire carries.
    pub const fn pack(self) -> i32 {
        ((self.owner as u32)
            | ((self.index as u32 & 0x3FF) << 8)
            | ((self.generation as u32 & 0x3FFF) << 18)) as i32
    }

    /// Unpack from the wire.
    pub const fn unpack(bits: i32) -> Self {
        let bits = bits as u32;
        Self {
            owner: (bits & 0xFF) as u8,
            index: ((bits >> 8) & 0x3FF) as u16,
            generation: ((bits >> 18) & 0x3FFF) as u16,
        }
    }
}

/// Packet `27`: a projectile's full state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncProjectile {
    pub key: ProjectileKey,
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub projectile_type: i16,
    pub ai: [f32; MAX_AI],
    pub banner: u16,
    pub damage: i16,
    pub knockback: f32,
    pub original_damage: i16,
}

impl SyncProjectile {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::SYNC_PROJECTILE);
        w.i32(self.key.pack());
        w.f32(self.position.0);
        w.f32(self.position.1);
        w.f32(self.velocity.0);
        w.f32(self.velocity.1);
        w.i16(self.projectile_type);

        // The second flag byte only exists when something in it is set, and the first byte's bit 2
        // is what says so.
        let mut extra = 0u8;
        if self.ai[2] != 0.0 {
            extra |= 1 << 0;
        }
        let mut flags = 0u8;
        if self.ai[0] != 0.0 {
            flags |= 1 << 0;
        }
        if self.ai[1] != 0.0 {
            flags |= 1 << 1;
        }
        if extra != 0 {
            flags |= 1 << 2;
        }
        if self.banner != 0 {
            flags |= 1 << 3;
        }
        if self.damage != 0 {
            flags |= 1 << 4;
        }
        if self.knockback != 0.0 {
            flags |= 1 << 5;
        }
        if self.original_damage != 0 {
            flags |= 1 << 6;
        }
        w.u8(flags);
        if extra != 0 {
            w.u8(extra);
        }
        if flags & (1 << 0) != 0 {
            w.f32(self.ai[0]);
        }
        if flags & (1 << 1) != 0 {
            w.f32(self.ai[1]);
        }
        if flags & (1 << 3) != 0 {
            w.u16(self.banner);
        }
        if flags & (1 << 4) != 0 {
            w.i16(self.damage);
        }
        if flags & (1 << 5) != 0 {
            w.f32(self.knockback);
        }
        if flags & (1 << 6) != 0 {
            w.i16(self.original_damage);
        }
        if extra & (1 << 0) != 0 {
            w.f32(self.ai[2]);
        }
        w.finish()
    }

    pub fn decode(body: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(body);
        let key = ProjectileKey::unpack(r.i32()?);
        let position = (r.f32()?, r.f32()?);
        let velocity = (r.f32()?, r.f32()?);
        let projectile_type = r.i16()?;
        let flags = r.u8()?;
        let extra = if flags & (1 << 2) != 0 { r.u8()? } else { 0 };
        let mut ai = [0.0; MAX_AI];
        if flags & (1 << 0) != 0 {
            ai[0] = r.f32()?;
        }
        if flags & (1 << 1) != 0 {
            ai[1] = r.f32()?;
        }
        let banner = if flags & (1 << 3) != 0 { r.u16()? } else { 0 };
        let damage = if flags & (1 << 4) != 0 { r.i16()? } else { 0 };
        let knockback = if flags & (1 << 5) != 0 { r.f32()? } else { 0.0 };
        let original_damage = if flags & (1 << 6) != 0 { r.i16()? } else { 0 };
        if extra & (1 << 0) != 0 {
            ai[2] = r.f32()?;
        }
        Ok(Self {
            key,
            position,
            velocity,
            projectile_type,
            ai,
            banner,
            damage,
            knockback,
            original_damage,
        })
    }
}

/// Packet `29`: a projectile is gone, and where it was when it went.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KillProjectile {
    pub key: ProjectileKey,
    pub position: (f32, f32),
}

impl KillProjectile {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::KILL_PROJECTILE);
        w.i32(self.key.pack());
        w.f32(self.position.0);
        w.f32(self.position.1);
        w.finish()
    }

    pub fn decode(body: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(body);
        Ok(Self {
            key: ProjectileKey::unpack(r.i32()?),
            position: (r.f32()?, r.f32()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SyncProjectile {
        SyncProjectile {
            key: ProjectileKey {
                owner: 255,
                index: 37,
                generation: 9,
            },
            position: (1234.5, -678.25),
            velocity: (6.0, -2.5),
            projectile_type: 38,
            ai: [0.0, 0.0, 0.0],
            banner: 0,
            damage: 15,
            knockback: 0.0,
            original_damage: 0,
        }
    }

    #[test]
    fn a_key_survives_the_round_trip() {
        let key = ProjectileKey {
            owner: 255,
            index: 1000,
            generation: 16_383,
        };
        assert_eq!(ProjectileKey::unpack(key.pack()), key);
    }

    #[test]
    fn the_key_packs_into_the_fields_the_game_reads() {
        let key = ProjectileKey {
            owner: 7,
            index: 300,
            generation: 42,
        };
        let bits = key.pack() as u32;
        assert_eq!(bits & 0xFF, 7);
        assert_eq!((bits >> 8) & 0x3FF, 300);
        assert_eq!((bits >> 18) & 0x3FFF, 42);
    }

    #[test]
    fn a_plain_projectile_round_trips() {
        let p = sample();
        let bytes = p.encode().unwrap();
        assert_eq!(SyncProjectile::decode(&bytes[3..]).unwrap(), p);
    }

    #[test]
    fn every_optional_field_round_trips() {
        let p = SyncProjectile {
            ai: [1.5, -2.5, 3.5],
            banner: 12,
            knockback: 4.25,
            original_damage: 21,
            ..sample()
        };
        let bytes = p.encode().unwrap();
        assert_eq!(SyncProjectile::decode(&bytes[3..]).unwrap(), p);
    }

    /// The packing is the point: a bare projectile costs far less than a fully loaded one.
    #[test]
    fn an_empty_projectile_is_much_smaller_than_a_full_one() {
        let bare = SyncProjectile {
            damage: 0,
            ..sample()
        }
        .encode()
        .unwrap();
        let full = SyncProjectile {
            ai: [1.0, 2.0, 3.0],
            banner: 5,
            knockback: 1.0,
            original_damage: 9,
            ..sample()
        }
        .encode()
        .unwrap();
        assert!(
            full.len() >= bare.len() + 15,
            "bare {} against full {}",
            bare.len(),
            full.len()
        );
    }

    #[test]
    fn the_third_ai_slot_costs_a_whole_extra_flag_byte() {
        let without = SyncProjectile {
            ai: [1.0, 1.0, 0.0],
            ..sample()
        }
        .encode()
        .unwrap();
        let with = SyncProjectile {
            ai: [1.0, 1.0, 1.0],
            ..sample()
        }
        .encode()
        .unwrap();
        // One byte for the second flag byte, four for the value itself.
        assert_eq!(with.len(), without.len() + 5);
    }

    #[test]
    fn a_kill_round_trips() {
        let k = KillProjectile {
            key: ProjectileKey {
                owner: 255,
                index: 3,
                generation: 1,
            },
            position: (100.0, 200.0),
        };
        let bytes = k.encode().unwrap();
        assert_eq!(KillProjectile::decode(&bytes[3..]).unwrap(), k);
    }
}
