//! An async Terraria server.
//!
//! The wire format lives in the `terrustia-proto` crate; this crate owns the async runtime, world
//! state, and game logic.
//!
//! Copyright (C) 2026 Brooklyn Halmstad.
//! Licensed under the GNU Affero General Public License v3.0 or later; see LICENSE.

pub mod admin;
pub mod config;
pub mod game;
pub mod net;
pub mod term;
pub mod world;
pub mod worlds;
