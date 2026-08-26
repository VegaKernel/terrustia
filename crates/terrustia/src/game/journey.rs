//! Journey (creative) mode powers — the state this server actually understands the effect of.
//!
//! Vanilla has 15 powers (`CreativePowerManager.cs:90-104`), in five different wire shapes. This
//! covers three of those shapes, eleven of the fifteen powers: the four one-shot buttons (day/
//! noon/night/midnight, handled entirely in `server.rs` — nothing to hold state for), the four
//! shared on/off toggles this struct holds (`FreezeTime`/`FreezeRain`/`FreezeWind`/
//! `StopBiomeSpread`), and the three shared sliders — but only one of those, `time_rate`, needs
//! state here. `ModifyWindDirectionAndStrength`/`ModifyRainPower` are `_syncToJoiningPlayers =
//! false` in source (unlike `ModifyTimeRate`'s own `true`) *and* neither implements
//! `IPersistentPerWorldContent` — real vanilla itself does not remember either past the moment
//! they're applied, so `server.rs`'s handler applies their effect straight to `Weather` and moves
//! on, nothing to hold onto here either.
//!
//! The remaining four — `Godmode`/`FarPlacementRange`/`SpawnRate` (per-player, bit-packed sync
//! across up to 255 players) and `Difficulty` (a slider on the wire, but a continuous 0–3
//! replacement for the discrete `world.game_mode` read at dozens of call sites throughout
//! `server.rs`) — are real, separately-sized follow-up work — see plan.md.
//!
//! **In-memory only, on purpose for now, disclosed rather than silent.** Every field this struct
//! actually holds (`freeze_time`/`freeze_rain`/`freeze_wind`/`stop_biome_spread`/`time_rate`) is
//! `IPersistentPerWorldContent` in real vanilla — they belong in the `.wld` file. That file's own
//! creative-powers section is one of the ones this project currently carries through opaquely
//! rather than parses (`wld.rs`'s own comment on `trailing_sections` — the same shape tile entities
//! and townsfolk started in before they were peeled out into sections this server actively models).
//! Doing that properly means writing a real parser/writer for that section's binary format, which
//! is follow-up work of its own, not a side effect of wiring up the toggles' live gameplay effect.
//! Until then: these powers work for the life of one server run and reset on restart, which is
//! wrong relative to vanilla but not silently wrong — a restart is the only way to lose them, there
//! is no ordinary path that drops a setting a player is relying on mid-session.

use terrustia_proto::net_module::power;

/// The shared on/off powers, plus `ModifyTimeRate`, whose gameplay effect this server applies. See
/// the module doc for which powers this does *not* cover yet and why.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct JourneyPowers {
    pub freeze_time: bool,
    pub freeze_rain: bool,
    pub freeze_wind: bool,
    /// A real vanilla power with nothing to gate yet: this project does not model corruption/
    /// crimson/hallow tile spread at all. The toggle itself still works — a client can flip it and
    /// see the state stick and sync to other players — it just has no effect to freeze, the same
    /// honest gap as everywhere else this project has found a mechanism with no counterpart here.
    pub stop_biome_spread: bool,
    /// `ModifyTimeRate`'s raw 0.0–1.0 slider position, exactly as the wire carries it and exactly
    /// as `_sliderCurrentValueCache` holds it in source — not the derived 1×–24× rate itself.
    /// Storing the raw value, not the computed rate, is what makes re-broadcasting it to a late
    /// joiner (`ASharedSliderPower::OnPlayerJoining` writes the cache, not the rate) exact rather
    /// than an inverse-remap guess. Default `0.0`, which [`time_rate`](Self::time_rate) resolves
    /// to `1` — matching `ModifyTimeRate::Reset`'s own explicit `TargetTimeRate = 1` alongside its
    /// `_sliderCurrentValueCache = 0f`, not a coincidence: `Remap(0, 0, 1, 1, 24)` is already `1`.
    pub time_rate_slider: f32,
}

impl JourneyPowers {
    /// The current state of one of the four modelled toggles, or `None` for any other power id
    /// (including every power this doesn't cover — see the module doc).
    pub fn get(&self, power_id: u16) -> Option<bool> {
        match power_id {
            power::FREEZE_TIME => Some(self.freeze_time),
            power::FREEZE_RAIN => Some(self.freeze_rain),
            power::FREEZE_WIND => Some(self.freeze_wind),
            power::STOP_BIOME_SPREAD => Some(self.stop_biome_spread),
            _ => None,
        }
    }

    /// Apply a toggle request. Returns whether `power_id` named one of the four this struct holds
    /// — `false` means nothing changed, the caller should not broadcast or persist anything.
    pub fn set(&mut self, power_id: u16, enabled: bool) -> bool {
        match power_id {
            power::FREEZE_TIME => self.freeze_time = enabled,
            power::FREEZE_RAIN => self.freeze_rain = enabled,
            power::FREEZE_WIND => self.freeze_wind = enabled,
            power::STOP_BIOME_SPREAD => self.stop_biome_spread = enabled,
            _ => return false,
        }
        true
    }

    /// `ModifyTimeRate::UpdateInfoFromSliderValueCache`'s own remap: `Utils.Remap(value, 0, 1, 1,
    /// 24)`, rounded to the nearest whole multiplier. The clock advances this many ticks per
    /// server tick instead of one — `World::tick_time`'s own `rate` parameter.
    pub fn time_rate(&self) -> i32 {
        (1.0 + self.time_rate_slider.clamp(0.0, 1.0) * 23.0).round() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_modelled_power_starts_off() {
        let powers = JourneyPowers::default();
        for id in [
            power::FREEZE_TIME,
            power::FREEZE_RAIN,
            power::FREEZE_WIND,
            power::STOP_BIOME_SPREAD,
        ] {
            assert_eq!(powers.get(id), Some(false));
        }
    }

    #[test]
    fn set_reports_whether_it_recognised_the_power_id() {
        let mut powers = JourneyPowers::default();
        assert!(powers.set(power::FREEZE_TIME, true));
        assert_eq!(powers.get(power::FREEZE_TIME), Some(true));

        assert!(!powers.set(power::GODMODE, true), "not one of the four");
        assert_eq!(
            powers.get(power::GODMODE),
            None,
            "and get() should agree it holds nothing for it"
        );
    }

    #[test]
    fn toggles_are_independent() {
        let mut powers = JourneyPowers::default();
        powers.set(power::FREEZE_WIND, true);
        assert_eq!(powers.get(power::FREEZE_TIME), Some(false));
        assert_eq!(powers.get(power::FREEZE_RAIN), Some(false));
        assert_eq!(powers.get(power::FREEZE_WIND), Some(true));
        assert_eq!(powers.get(power::STOP_BIOME_SPREAD), Some(false));
    }

    #[test]
    fn a_fresh_journey_state_ticks_time_at_one_times() {
        assert_eq!(JourneyPowers::default().time_rate(), 1);
    }

    #[test]
    fn the_time_rate_slider_remaps_across_its_whole_one_to_twenty_four_times_range() {
        let mut powers = JourneyPowers {
            time_rate_slider: 1.0,
            ..Default::default()
        };
        assert_eq!(powers.time_rate(), 24, "the top of the slider is 24x");

        powers.time_rate_slider = 0.5;
        assert_eq!(
            powers.time_rate(),
            13,
            "the midpoint rounds 12.5x to 13x, matching Math.Round's own away-from-zero rounding"
        );
    }

    /// A slider value outside 0.0–1.0 should never be trusted at face value — nothing on the wire
    /// stops a client (or a bug upstream of this call) from sending one.
    #[test]
    fn an_out_of_range_slider_value_is_clamped_rather_than_extrapolated() {
        let mut powers = JourneyPowers {
            time_rate_slider: 5.0,
            ..Default::default()
        };
        assert_eq!(powers.time_rate(), 24, "clamped to the slider's real top");

        powers.time_rate_slider = -3.0;
        assert_eq!(powers.time_rate(), 1, "clamped to the slider's real bottom");
    }
}
