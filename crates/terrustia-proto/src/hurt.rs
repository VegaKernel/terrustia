//! Hurting and killing players: packets `117` (PlayerHurtV2) and `118` (PlayerDeathV2).
//!
//! Both carry a *death reason* rather than a plain number, because the game needs to name what
//! killed you. The reason is itself bit-packed: one flag byte says which of eight possible sources
//! are present, and only those are written. An NPC's claw is one field; a projectile fired by
//! another player is four.

use crate::{error::Result, id, reader::PacketReader, writer::PacketWriter};

/// What hurt somebody.
///
/// Every field is optional and absent by default, which is exactly how the wire treats them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeathReason {
    pub player: Option<i16>,
    pub npc: Option<i16>,
    pub projectile: Option<i16>,
    /// One of the game's fixed "other" causes: drowning, lava, falling and so on.
    pub other: Option<u8>,
    pub projectile_type: Option<i16>,
    pub item_type: Option<i16>,
    pub item_prefix: Option<u8>,
    pub custom: Option<String>,
}

impl DeathReason {
    /// Killed by an NPC, which is how nearly everything on this server kills you.
    pub fn from_npc(index: i16) -> Self {
        Self {
            npc: Some(index),
            ..Self::default()
        }
    }

    /// Killed by a projectile, naming both the shot and its type.
    pub fn from_projectile(index: i16, projectile_type: i16) -> Self {
        Self {
            projectile: Some(index),
            projectile_type: Some(projectile_type),
            ..Self::default()
        }
    }

    fn write(&self, w: &mut PacketWriter) {
        let mut flags = 0u8;
        for (bit, present) in [
            self.player.is_some(),
            self.npc.is_some(),
            self.projectile.is_some(),
            self.other.is_some(),
            self.projectile_type.is_some(),
            self.item_type.is_some(),
            self.item_prefix.is_some(),
            self.custom.is_some(),
        ]
        .into_iter()
        .enumerate()
        {
            if present {
                flags |= 1 << bit;
            }
        }
        w.u8(flags);
        if let Some(v) = self.player {
            w.i16(v);
        }
        if let Some(v) = self.npc {
            w.i16(v);
        }
        if let Some(v) = self.projectile {
            w.i16(v);
        }
        if let Some(v) = self.other {
            w.u8(v);
        }
        if let Some(v) = self.projectile_type {
            w.i16(v);
        }
        if let Some(v) = self.item_type {
            w.i16(v);
        }
        if let Some(v) = self.item_prefix {
            w.u8(v);
        }
        if let Some(v) = &self.custom {
            w.string(v);
        }
    }

    fn read(r: &mut PacketReader) -> Result<Self> {
        let flags = r.u8()?;
        let has = |bit: u8| flags & (1 << bit) != 0;
        Ok(Self {
            player: has(0).then(|| r.i16()).transpose()?,
            npc: has(1).then(|| r.i16()).transpose()?,
            projectile: has(2).then(|| r.i16()).transpose()?,
            other: has(3).then(|| r.u8()).transpose()?,
            projectile_type: has(4).then(|| r.i16()).transpose()?,
            item_type: has(5).then(|| r.i16()).transpose()?,
            item_prefix: has(6).then(|| r.u8()).transpose()?,
            custom: has(7).then(|| r.string()).transpose()?,
        })
    }
}

/// Packet `117`: somebody took a hit.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerHurt {
    pub player: u8,
    pub reason: DeathReason,
    pub damage: i16,
    /// Which way the hit throws them: -1, 0 or 1.
    pub direction: i8,
    pub crit: bool,
    pub pvp: bool,
    /// Which cooldown counter the hit is on. -1 is the ordinary one.
    pub cooldown: i8,
}

impl PlayerHurt {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::PLAYER_HURT_V2);
        w.u8(self.player);
        self.reason.write(&mut w);
        w.i16(self.damage);
        // The direction travels offset by one so it fits in a byte.
        w.u8((self.direction + 1) as u8);
        let mut flags = 0u8;
        if self.crit {
            flags |= 1 << 0;
        }
        if self.pvp {
            flags |= 1 << 1;
        }
        w.u8(flags);
        w.i8(self.cooldown);
        w.finish()
    }

    pub fn decode(body: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(body);
        let player = r.u8()?;
        let reason = DeathReason::read(&mut r)?;
        let damage = r.i16()?;
        let direction = r.u8()? as i8 - 1;
        let flags = r.u8()?;
        Ok(Self {
            player,
            reason,
            damage,
            direction,
            crit: flags & (1 << 0) != 0,
            pvp: flags & (1 << 1) != 0,
            cooldown: r.i8()?,
        })
    }
}

/// Packet `118`: somebody died.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerDeath {
    pub player: u8,
    pub reason: DeathReason,
    pub damage: i16,
    pub direction: i8,
    pub pvp: bool,
}

impl PlayerDeath {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::PLAYER_DEATH_V2);
        w.u8(self.player);
        self.reason.write(&mut w);
        w.i16(self.damage);
        w.u8((self.direction + 1) as u8);
        w.u8(u8::from(self.pvp));
        w.finish()
    }

    pub fn decode(body: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(body);
        let player = r.u8()?;
        let reason = DeathReason::read(&mut r)?;
        Ok(Self {
            player,
            reason,
            damage: r.i16()?,
            direction: r.u8()? as i8 - 1,
            pvp: r.u8()? != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claw_to_the_face_round_trips() {
        let hurt = PlayerHurt {
            player: 3,
            reason: DeathReason::from_npc(42),
            damage: 27,
            direction: -1,
            crit: false,
            pvp: false,
            cooldown: -1,
        };
        let bytes = hurt.encode().unwrap();
        assert_eq!(PlayerHurt::decode(&bytes[3..]).unwrap(), hurt);
    }

    #[test]
    fn a_projectile_names_both_the_shot_and_its_type() {
        let hurt = PlayerHurt {
            player: 0,
            reason: DeathReason::from_projectile(7, 44),
            damage: 21,
            direction: 1,
            crit: true,
            pvp: false,
            cooldown: -1,
        };
        let bytes = hurt.encode().unwrap();
        let back = PlayerHurt::decode(&bytes[3..]).unwrap();
        assert_eq!(back, hurt);
        assert_eq!(back.reason.projectile_type, Some(44));
    }

    /// The reason is bit-packed, so naming less costs less.
    #[test]
    fn a_simpler_reason_is_a_shorter_packet() {
        let plain = PlayerHurt {
            player: 0,
            reason: DeathReason::from_npc(1),
            damage: 10,
            direction: 0,
            crit: false,
            pvp: false,
            cooldown: -1,
        };
        let detailed = PlayerHurt {
            reason: DeathReason {
                player: Some(1),
                npc: Some(2),
                projectile: Some(3),
                other: Some(4),
                projectile_type: Some(5),
                item_type: Some(6),
                item_prefix: Some(7),
                custom: None,
            },
            ..plain.clone()
        };
        assert!(detailed.encode().unwrap().len() > plain.encode().unwrap().len() + 8);
    }

    #[test]
    fn a_death_round_trips() {
        let death = PlayerDeath {
            player: 1,
            reason: DeathReason::from_npc(5),
            damage: 100,
            direction: 1,
            pvp: false,
        };
        let bytes = death.encode().unwrap();
        assert_eq!(PlayerDeath::decode(&bytes[3..]).unwrap(), death);
    }

    #[test]
    fn the_direction_survives_its_trip_through_a_byte() {
        for direction in [-1i8, 0, 1] {
            let hurt = PlayerHurt {
                player: 0,
                reason: DeathReason::default(),
                damage: 1,
                direction,
                crit: false,
                pvp: false,
                cooldown: -1,
            };
            let bytes = hurt.encode().unwrap();
            assert_eq!(
                PlayerHurt::decode(&bytes[3..]).unwrap().direction,
                direction
            );
        }
    }
}
