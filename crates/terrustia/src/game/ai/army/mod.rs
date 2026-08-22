//! The Old One's Army's own routines.
//!
//! Everything the event puts on the field either walks (style 107, the fighters), flies (108),
//! casts (109), or is furniture (105 and 106). The furniture is the interesting part: an Eternia
//! Crystal is an NPC you are defending rather than fighting, and a lane portal is an NPC whose
//! whole behaviour is a spawn timer. Neither has an attack.

pub mod bug;
pub mod crystal;
pub mod flyer;
