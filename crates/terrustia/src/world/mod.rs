pub mod bulbs;
pub mod calendar;
pub mod census;
pub mod doors;
pub mod growth;
pub mod hardmode;
pub mod items;
pub mod liquid;
/// A test-only faithful transcription of vanilla's array-based liquid simulator, used purely as a
/// measurement probe for the FIX-1c liquid crux (see the module doc). Never compiled non-test.
#[cfg(test)]
mod liquid_faithful;
pub mod mass_wire;
pub mod meteor;
pub mod objects;
pub mod packed;
pub mod progress;
pub mod quick_stack;
pub mod trapdoors;
pub mod trees;
pub mod wiring;
pub mod wld;
pub mod wld_save;
#[allow(clippy::module_inception)]
pub mod world;
pub mod worldgen;

pub use items::{ItemStore, WorldItem};
pub use objects::{Chest, Sign};
pub use progress::Progress;
pub use wld::WldError;
pub use world::World;
