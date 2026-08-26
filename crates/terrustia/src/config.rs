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
    /// Whether the web admin panel is running. Off by default — a new HTTP port is attack surface
    /// a CLI/systemd/Docker host did not ask for.
    pub panel_enabled: bool,
    /// Must be loopback (127.0.0.1 or ::1) — [`Config::validate`] refuses anything else. The panel
    /// is a materially more sensitive surface (account/server control) than gameplay traffic, so
    /// unlike `listen`, this is never allowed to face the network by configuration alone.
    pub panel_listen: SocketAddr,
    /// Check GitHub for a newer, signature-verified release on boot and say so (console log, plus
    /// an in-game notice to the first recognised admin who signs in afterward) — see the `update`
    /// module. On by default, unlike the panel: this makes one outbound, read-only request at
    /// most once a boot and opens no new attack surface of its own, so it does not need the same
    /// opt-in an admin/server-control HTTP port does. Still fully disable-able for an air-gapped
    /// host, or an operator who would rather not have this server talk to GitHub at all.
    pub update_check_enabled: bool,
    /// Attempt UPnP automatic port-mapping for `listen` on startup — see the `upnp` module. On by
    /// default; an operator who does not want this server changing their router's port-mapping
    /// table at all can turn it off. The panel is never affected either way: `panel_listen` is
    /// always loopback-only regardless (see above), so there is nothing for UPnP to forward there.
    pub upnp_enabled: bool,
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
            panel_enabled: false,
            panel_listen: "127.0.0.1:7778".parse().expect("valid default address"),
            update_check_enabled: true,
            upnp_enabled: true,
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

    /// Layer `TERRUSTIA_*` environment variables on top of whatever `load` already produced —
    /// Docker/automation config that needs no file on disk, and no shell around the process to
    /// pass flags either (an image's `ENTRYPOINT` already fixes the argv; env vars are the one
    /// thing every container orchestrator can set without touching that). Applied after the TOML
    /// file and before any CLI flag — `main`'s own `Args` still wins over both — matching the
    /// layering every other host convention uses: defaults < file < environment < explicit flag.
    ///
    /// Unset or empty variables are left alone rather than treated as "set to empty" — a container
    /// runtime that always defines a variable, blank or not, should not be able to blank out
    /// `motd` or `password` by accident.
    pub fn apply_env(&mut self) -> Result<(), ConfigError> {
        fn get(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|v| !v.is_empty())
        }
        fn parsed<T: std::str::FromStr>(name: &str, value: &str) -> Result<T, ConfigError> {
            value
                .parse()
                .map_err(|_| ConfigError::Invalid(format!("{name}: cannot parse {value:?}")))
        }

        if let Some(v) = get("TERRUSTIA_LISTEN") {
            self.listen = parsed("TERRUSTIA_LISTEN", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MAX_PLAYERS") {
            self.max_players = parsed("TERRUSTIA_MAX_PLAYERS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_WORLD_NAME") {
            self.world_name = v;
        }
        if let Some(v) = get("TERRUSTIA_WORLD_FILE") {
            self.world_file = Some(PathBuf::from(v));
        }
        if let Some(v) = get("TERRUSTIA_SAVE_FILE") {
            self.save_file = Some(PathBuf::from(v));
        }
        if let Some(v) = get("TERRUSTIA_AUTOSAVE_SECS") {
            self.autosave_secs = parsed("TERRUSTIA_AUTOSAVE_SECS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_WORLD_WIDTH") {
            self.world_width = parsed("TERRUSTIA_WORLD_WIDTH", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_WORLD_HEIGHT") {
            self.world_height = parsed("TERRUSTIA_WORLD_HEIGHT", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_SEED") {
            self.seed = parsed("TERRUSTIA_SEED", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MOTD") {
            self.motd = v;
        }
        if let Some(v) = get("TERRUSTIA_PASSWORD") {
            self.password = v;
        }
        if let Some(v) = get("TERRUSTIA_PANEL_ENABLED") {
            self.panel_enabled = parsed("TERRUSTIA_PANEL_ENABLED", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_PANEL_LISTEN") {
            self.panel_listen = parsed("TERRUSTIA_PANEL_LISTEN", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_UPDATE_CHECK_ENABLED") {
            self.update_check_enabled = parsed("TERRUSTIA_UPDATE_CHECK_ENABLED", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_UPNP_ENABLED") {
            self.upnp_enabled = parsed("TERRUSTIA_UPNP_ENABLED", &v)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        // The panel is account/server control, not gameplay — it never gets to face the network
        // just because someone typed a non-loopback address into `panel_listen`.
        if self.panel_enabled && !self.panel_listen.ip().is_loopback() {
            return Err(ConfigError::Invalid(format!(
                "panel_listen must be loopback (127.0.0.1 or ::1), got {}",
                self.panel_listen.ip()
            )));
        }
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

    /// Serializes every test in this file that mutates a `TERRUSTIA_*` process environment
    /// variable. `std::env::set_var`/`remove_var` are real process-wide state, not scoped to the
    /// calling test — `Config::apply_env` reads every `TERRUSTIA_*` name at once, so a second
    /// test's own unrelated-looking variable is just as visible to the first test's `apply_env()`
    /// call as if they shared a name. An earlier draft here relied on each test using a
    /// *different* variable name instead of a lock, which does not actually prevent that: found
    /// for real, as an intermittent failure under `cargo test`'s default parallel execution.
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// Real process-wide state: `std::env::set_var`/`remove_var` are not scoped to one test, and
    /// `apply_env()` reads *every* `TERRUSTIA_*` name at once, not just whichever one a given test
    /// nominally cares about. Using a different variable name per test (an earlier draft's own
    /// reasoning, corrected here) does not prevent the race it looks like it prevents: a second
    /// test's `set_var("TERRUSTIA_AUTOSAVE_SECS", "not-a-number")` is just as visible to this
    /// test's own `apply_env()` call as if it had set the same name — found for real, this test
    /// failing intermittently under `cargo test`'s default parallel execution with exactly that
    /// error. Every test in this file that touches a `TERRUSTIA_*` variable holds
    /// [`ENV_TEST_LOCK`] for its whole body, which is the actual fix — not the variable name.
    #[test]
    #[allow(unsafe_code)] // `set_var`/`remove_var` — see the SAFETY comments inline below.
    fn environment_variables_override_the_loaded_config() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: `ENV_TEST_LOCK` above serializes every test in this file that touches a
        // `TERRUSTIA_*` variable, so nothing else can be reading or writing one of these names
        // concurrently for as long as `_guard` is held.
        unsafe {
            std::env::set_var("TERRUSTIA_LISTEN", "127.0.0.1:9999");
            std::env::set_var("TERRUSTIA_MAX_PLAYERS", "42");
            std::env::set_var("TERRUSTIA_WORLD_NAME", "Env World");
            std::env::set_var("TERRUSTIA_MOTD", "hi from the environment");
            std::env::set_var("TERRUSTIA_PANEL_ENABLED", "true");
            // Deliberately left unset: proves an absent variable does not overwrite a value the
            // TOML file (or a default) already set.
            std::env::remove_var("TERRUSTIA_SEED");
        }

        let mut config = Config {
            seed: 777,
            ..Config::default()
        };
        config.apply_env().expect("valid environment overrides");

        assert_eq!(config.listen.port(), 9999);
        assert_eq!(config.max_players, 42);
        assert_eq!(config.world_name, "Env World");
        assert_eq!(config.motd, "hi from the environment");
        assert!(config.panel_enabled);
        assert_eq!(config.seed, 777, "an unset variable must not overwrite it");

        // SAFETY: same justification as above — cleaning up what this test itself set.
        unsafe {
            std::env::remove_var("TERRUSTIA_LISTEN");
            std::env::remove_var("TERRUSTIA_MAX_PLAYERS");
            std::env::remove_var("TERRUSTIA_WORLD_NAME");
            std::env::remove_var("TERRUSTIA_MOTD");
            std::env::remove_var("TERRUSTIA_PANEL_ENABLED");
        }
    }

    #[test]
    #[allow(unsafe_code)] // `set_var`/`remove_var` — see the SAFETY comments inline below.
    fn an_unparseable_environment_variable_is_reported_rather_than_ignored() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: see `ENV_TEST_LOCK`'s own doc comment and the override test above — `_guard`
        // holds the lock for this whole function, so no other `TERRUSTIA_*`-touching test can run
        // concurrently with this one.
        unsafe {
            std::env::set_var("TERRUSTIA_AUTOSAVE_SECS", "not-a-number");
        }
        let mut config = Config::default();
        assert!(config.apply_env().is_err());
        // SAFETY: cleaning up.
        unsafe {
            std::env::remove_var("TERRUSTIA_AUTOSAVE_SECS");
        }
    }
}
