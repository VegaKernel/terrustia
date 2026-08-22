//! The bosses, one module each.
//!
//! A boss is not a bigger enemy: it is a state machine with named phases, and each of these is a
//! transcription of one. They share almost nothing, which is why they are separate files rather
//! than a style table.

pub mod brain;
pub mod cultist;
pub mod deerclops;
pub mod destroyer;
pub mod eye;
pub mod fishron;
pub mod golem;
pub mod king_slime;
pub mod moon;
pub mod moon_lord;
pub mod plantera;
pub mod prime;
pub mod queen_bee;
pub mod queen_slime;
pub mod skeletron;
pub mod tablet;
pub mod tree;
pub mod twins;
pub mod wall;
