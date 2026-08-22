//! The bosses, one module each.
//!
//! A boss is not a bigger enemy: it is a state machine with named phases, and each of these is a
//! transcription of one. They share almost nothing, which is why they are separate files rather
//! than a style table.

pub mod brain;
pub mod deerclops;
pub mod eye;
pub mod king_slime;
pub mod queen_bee;
pub mod skeletron;
pub mod wall;
