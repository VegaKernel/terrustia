use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use terrustia_proto::section::{SECTION_HEIGHT, SECTION_WIDTH};

/// Terraria addresses players by a byte slot, and slot 255 is reserved for "the server" in chat.
pub const MAX_PLAYERS: usize = 255;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub listen: SocketAddr,
    pub max_players: usize,
    pub world_name: String,
    /// Serve this `.wld` file instead of generating a world.
    pub world_file: Option<PathBuf>,
    /// Where to write saves. Defaults to `world_file`; set it to keep the original untouched.
    pub save_file: Option<PathBuf>,
    /// Seconds between automatic saves, or 0 to disable them.
    pub autosave_secs: u64,
    pub world_width: i32,
    pub world_height: i32,
    pub seed: u64,
    pub motd: String,
    /// Password clients must send before joining. Empty means the server is open.
    pub password: String,
    /// Longest chat line accepted from a client, in bytes.
    pub max_chat_len: usize,
    /// Drop a connection that sends nothing at all for this many seconds.
    ///
    /// A playing client sends control updates continuously, so this only catches dead sockets and
    /// connections that stall mid-handshake.
    pub idle_timeout_secs: u64,
    /// How many sockets may be open at once, before anyone has said who they are.
    ///
    /// Separate from `max_players`, and necessarily larger: a connection is accepted long before
    /// it has a slot, so the two count different things. Without a ceiling the accept loop is
    /// unconditional — every socket immediately gets two tasks, a read buffer and an outbound
    /// queue, none of which requires the other end to have spoken the protocol at all.
    pub max_connections: usize,
    /// How many sockets one address may hold open at once.
    ///
    /// The common case for hitting this is not an attack but a mistake — a script reconnecting in
    /// a loop — and either way the answer is the same.
    pub max_connections_per_address: usize,
    /// How long a connection has to finish the handshake before it is dropped.
    ///
    /// `idle_timeout_secs` wraps each individual read, so its timer resets on any byte: a
    /// connection trickling one byte a minute stays open for ever and costs a slot the whole time.
    /// This is the backstop that makes the slot finite.
    pub handshake_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:7777".parse().expect("valid default address"),
            max_players: 8,
            world_name: "Terrustia".into(),
            world_file: None,
            save_file: None,
            autosave_secs: 300,
            world_width: crate::world::worldgen::SMALL_WIDTH,
            world_height: crate::world::worldgen::SMALL_HEIGHT,
            seed: 0,
            motd: "Welcome to Terrustia".into(),
            password: String::new(),
            max_chat_len: 500,
            idle_timeout_secs: 60,
            // Generous next to any real player count, and still a ceiling. The point is that one
            // machine cannot open sockets until this one runs out of descriptors.
            max_connections: 512,
            max_connections_per_address: 8,
            handshake_timeout_secs: 30,
        }
    }
}

impl Config {
    /// Where saves should go, if saving is possible at all.
    pub fn save_target(&self) -> Option<&Path> {
        self.save_file.as_deref().or(self.world_file.as_deref())
    }

    /// Load from a TOML file, falling back to defaults when the file does not exist.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let config: Config = toml::from_str(&text)?;
                config.validate()?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // A loaded world brings its own dimensions, so the size limits below do not apply to it.
        if self.world_file.is_some() {
            if self.max_players == 0 || self.max_players > MAX_PLAYERS {
                return Err(ConfigError::Invalid(format!(
                    "max_players must be between 1 and {MAX_PLAYERS}, got {}",
                    self.max_players
                )));
            }
            return Ok(());
        }
        if self.max_players == 0 || self.max_players > MAX_PLAYERS {
            return Err(ConfigError::Invalid(format!(
                "max_players must be between 1 and {MAX_PLAYERS}, got {}",
                self.max_players
            )));
        }
        // Below one section the client has nothing to render around spawn.
        if self.world_width < 400 || self.world_height < 300 {
            return Err(ConfigError::Invalid(format!(
                "world must be at least 400x300, got {}x{}",
                self.world_width, self.world_height
            )));
        }
        // Dimensions travel as i16 in packet 7.
        if self.world_width > i32::from(i16::MAX) || self.world_height > i32::from(i16::MAX) {
            return Err(ConfigError::Invalid(
                "world dimensions must fit in an i16".into(),
            ));
        }
        // The client sizes its section grid with `maxTilesX / 200` and `maxTilesY / 150`, which
        // truncate. A world that is not a whole number of sections therefore has a strip along
        // its far edge that the client has nowhere to put and will never ask for — so refuse it
        // here rather than generate ground nobody can reach. Every size Terraria itself makes is
        // already a multiple of both.
        let ragged_x = self.world_width % SECTION_WIDTH;
        let ragged_y = self.world_height % SECTION_HEIGHT;
        if ragged_x != 0 || ragged_y != 0 {
            return Err(ConfigError::Invalid(format!(
                "world size must be a whole number of {SECTION_WIDTH}x{SECTION_HEIGHT} sections, \
                 got {}x{}; try {}x{}",
                self.world_width,
                self.world_height,
                self.world_width - ragged_x,
                self.world_height - ragged_y,
            )));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing config: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn partial_toml_keeps_defaults_for_the_rest() {
        let config: Config = toml::from_str("max_players = 4\nmotd = \"hi\"").unwrap();
        assert_eq!(config.max_players, 4);
        assert_eq!(config.motd, "hi");
        assert_eq!(config.world_width, crate::world::worldgen::SMALL_WIDTH);
    }

    #[test]
    fn a_typo_in_a_key_is_reported_rather_than_ignored() {
        assert!(toml::from_str::<Config>("max_playerz = 4").is_err());
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for text in [
            "max_players = 0",
            "max_players = 256",
            "world_width = 100",
            "world_height = 100",
            "world_width = 40000",
        ] {
            let config: Config = toml::from_str(text).unwrap();
            assert!(config.validate().is_err(), "{text} should be rejected");
        }
    }

    #[test]
    fn a_world_file_bypasses_the_generated_size_limits() {
        let config: Config =
            toml::from_str("world_file = \"/tmp/x.wld\"\nworld_width = 100\nworld_height = 100")
                .unwrap();
        config.validate().expect("a loaded world sets its own size");
    }

    #[test]
    fn a_world_file_still_validates_player_count() {
        let config: Config =
            toml::from_str("world_file = \"/tmp/x.wld\"\nmax_players = 0").unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn saves_default_to_the_loaded_world_file() {
        let config: Config = toml::from_str("world_file = \"/tmp/a.wld\"").unwrap();
        assert_eq!(config.save_target(), Some(Path::new("/tmp/a.wld")));
    }

    #[test]
    fn an_explicit_save_file_keeps_the_original_untouched() {
        let config: Config =
            toml::from_str("world_file = \"/tmp/a.wld\"\nsave_file = \"/tmp/b.wld\"").unwrap();
        assert_eq!(config.save_target(), Some(Path::new("/tmp/b.wld")));
    }

    #[test]
    fn a_generated_world_has_nowhere_to_save() {
        assert_eq!(Config::default().save_target(), None);
    }

    #[test]
    fn a_missing_file_yields_defaults() {
        let config = Config::load(Path::new("/nonexistent/terrustia.toml")).unwrap();
        assert_eq!(config.max_players, Config::default().max_players);
    }
}
