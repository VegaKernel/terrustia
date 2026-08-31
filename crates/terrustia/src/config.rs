use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
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
    /// Rotate the audit log (`admin::audit`) once its live file would reach this many bytes. See
    /// `admin::audit::DEFAULT_MAX_BYTES` for the default and why it is that size.
    pub audit_log_max_bytes: u64,
    /// How many rotated audit-log segments to keep beyond the live file before the oldest is
    /// dropped. See `admin::audit::DEFAULT_KEEP_SEGMENTS`.
    pub audit_log_keep_segments: usize,
    /// Escalate a repeat offender's mute automatically — a config-enabled mechanism, off by
    /// default. When on, a muted player who keeps talking while muted has their mute duration
    /// extended by `mute_escalation_secs` each time, up to `mute_escalation_max_secs`.
    pub mute_escalation_enabled: bool,
    /// How long each escalation step adds to an active mute, in seconds.
    pub mute_escalation_secs: u64,
    /// The longest an escalated mute may reach, in seconds. `0` means no ceiling.
    pub mute_escalation_max_secs: u64,
    /// A per-account cooldown between chat lines, in milliseconds — a config-enabled mechanism, off
    /// by default (`0`). When set, a line sent before the cooldown has elapsed since the account's
    /// last one is dropped rather than broadcast.
    pub chat_cooldown_ms: u64,
    /// Kick a client that edits tiles or moves liquid faster than the game's own ceilings allow.
    ///
    /// `Netplay.SpamCheck` (`Netplay.cs:65`), which vanilla declares `false` and turns on only for
    /// a server started with `secure=1` (`Main.cs:5200`) or `-secure`
    /// (`LaunchInitializer.cs:152`). With it off, `RemoteClient.SpamUpdate` zeroes all four
    /// counters every tick and returns before any of them can reach a ceiling
    /// (`RemoteClient.cs:70-80`), and the liquid counter is not even incremented
    /// (`MessageBuffer.cs:2415`), so a stock server never boots anybody for spam.
    ///
    /// Off by default for the same reason it is off in vanilla: the ceilings are tight enough that
    /// ordinary play reaches them. A stick of dynamite clears hundreds of tiles in one burst, which
    /// is over `spam_break`'s 500 long before the 5-a-tick decay catches up.
    pub spam_check: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Built rather than parsed. `"0.0.0.0:7777".parse().expect(..)` was correct and could
            // never have fired, but "could never have fired" is a claim about a string literal that
            // has to be re-checked every time somebody edits it, and it cost a line of the panic
            // budget to hold. `Ipv4Addr::UNSPECIFIED` is 0.0.0.0 and this is a `const fn` chain, so
            // there is no parse to fail.
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 7777),
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
            // Same reasoning as `listen` above; `Ipv4Addr::LOCALHOST` is 127.0.0.1, which
            // `validate` then insists on staying loopback.
            panel_listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7778),
            update_check_enabled: true,
            upnp_enabled: true,
            audit_log_max_bytes: crate::admin::audit::DEFAULT_MAX_BYTES,
            audit_log_keep_segments: crate::admin::audit::DEFAULT_KEEP_SEGMENTS,
            mute_escalation_enabled: false,
            mute_escalation_secs: 300,
            mute_escalation_max_secs: 3600,
            chat_cooldown_ms: 0,
            spam_check: false,
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
    ///
    /// Every field on [`Config`] has an entry here — `README.md` and `terrustia.toml.example` both
    /// say so, and that claim was false in both directions until it was checked field-by-field
    /// against this function: `max_chat_len`, `idle_timeout_secs`, `max_connections`,
    /// `max_connections_per_address` and `handshake_timeout_secs` had no variable at all, and
    /// `upnp_enabled`'s working variable was undocumented (missing from the example file). Adding a
    /// field to `Config` without adding its variable here silently reintroduces the same gap, so
    /// keep this in sync rather than treating the docs' "every key" claim as aspirational.
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
        if let Some(v) = get("TERRUSTIA_MAX_CHAT_LEN") {
            self.max_chat_len = parsed("TERRUSTIA_MAX_CHAT_LEN", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_IDLE_TIMEOUT_SECS") {
            self.idle_timeout_secs = parsed("TERRUSTIA_IDLE_TIMEOUT_SECS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MAX_CONNECTIONS") {
            self.max_connections = parsed("TERRUSTIA_MAX_CONNECTIONS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MAX_CONNECTIONS_PER_ADDRESS") {
            self.max_connections_per_address = parsed("TERRUSTIA_MAX_CONNECTIONS_PER_ADDRESS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_HANDSHAKE_TIMEOUT_SECS") {
            self.handshake_timeout_secs = parsed("TERRUSTIA_HANDSHAKE_TIMEOUT_SECS", &v)?;
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
        if let Some(v) = get("TERRUSTIA_AUDIT_LOG_MAX_BYTES") {
            self.audit_log_max_bytes = parsed("TERRUSTIA_AUDIT_LOG_MAX_BYTES", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_AUDIT_LOG_KEEP_SEGMENTS") {
            self.audit_log_keep_segments = parsed("TERRUSTIA_AUDIT_LOG_KEEP_SEGMENTS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MUTE_ESCALATION_ENABLED") {
            self.mute_escalation_enabled = parsed("TERRUSTIA_MUTE_ESCALATION_ENABLED", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MUTE_ESCALATION_SECS") {
            self.mute_escalation_secs = parsed("TERRUSTIA_MUTE_ESCALATION_SECS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_MUTE_ESCALATION_MAX_SECS") {
            self.mute_escalation_max_secs = parsed("TERRUSTIA_MUTE_ESCALATION_MAX_SECS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_CHAT_COOLDOWN_MS") {
            self.chat_cooldown_ms = parsed("TERRUSTIA_CHAT_COOLDOWN_MS", &v)?;
        }
        if let Some(v) = get("TERRUSTIA_SPAM_CHECK") {
            self.spam_check = parsed("TERRUSTIA_SPAM_CHECK", &v)?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        // The panel is account/server control, not gameplay — it never gets to face the network
        // just because someone typed a non-loopback address into `panel_listen`. Checked
        // unconditionally, not only when the panel is currently enabled: otherwise a config with
        // `panel_enabled = false` + a public `panel_listen` passes validation, and the runtime
        // `panel` toggle then binds that public address — exactly the exposure this refuses.
        if !self.panel_listen.ip().is_loopback() {
            return Err(ConfigError::Invalid(format!(
                "panel_listen must be loopback (127.0.0.1 or ::1), got {}",
                self.panel_listen.ip()
            )));
        }
        // These four all gate `net::listener::claim`/`connection::serve` directly, and each has a
        // zero value that is not merely useless but refuses every connection outright (verified by
        // reading the code that consumes them, not assumed): `claim` refuses once
        // `guard.total >= max_total` (true immediately when `max_total` is 0) and once
        // `*count >= max_per_address` (same, for 0), and `connection::serve` wraps every read in
        // `timeout(idle_timeout.min(..), ..)`, so an `idle`/`handshake` duration of zero elapses
        // before a real client can send a single byte. A config that passed this unbounded used to
        // start a server that would never admit a single player, silently. The upper bounds are a
        // typo guard rather than a technical ceiling (there is no protocol reason a bigger number
        // could not work), generous enough that no real deployment should ever reach them.
        // These four all gate `net::listener::claim`/`connection::serve` directly, and each has a
        // zero value that is not merely useless but refuses every connection outright (verified by
        // reading the code that consumes them, not assumed): `claim` refuses once
        // `guard.total >= max_total` (true immediately when `max_total` is 0) and once
        // `*count >= max_per_address` (same, for 0), and `connection::serve` wraps every read in
        // `timeout(idle_timeout.min(..), ..)`, so an `idle`/`handshake` duration of zero elapses
        // before a real client can send a single byte. A config that passed this unbounded used to
        // start a server that would never admit a single player, silently. The upper bounds are a
        // typo guard rather than a technical ceiling (there is no protocol reason a bigger number
        // could not work), generous enough that no real deployment should ever reach them.
        if self.max_connections == 0 || self.max_connections > 65_536 {
            return Err(ConfigError::Invalid(format!(
                "max_connections must be between 1 and 65536, got {}",
                self.max_connections
            )));
        }
        if self.max_connections_per_address == 0
            || self.max_connections_per_address > self.max_connections
        {
            return Err(ConfigError::Invalid(format!(
                "max_connections_per_address must be between 1 and max_connections ({}), got {}",
                self.max_connections, self.max_connections_per_address
            )));
        }
        if self.handshake_timeout_secs == 0 || self.handshake_timeout_secs > 3600 {
            return Err(ConfigError::Invalid(format!(
                "handshake_timeout_secs must be between 1 and 3600, got {}",
                self.handshake_timeout_secs
            )));
        }
        if self.idle_timeout_secs == 0 || self.idle_timeout_secs > 86_400 {
            return Err(ConfigError::Invalid(format!(
                "idle_timeout_secs must be between 1 and 86400, got {}",
                self.idle_timeout_secs
            )));
        }
        // `net_module::validate_chat` already refuses an empty line; a `max_chat_len` of 0 makes
        // that refusal unconditional and silently disables chat instead of reporting a bad config.
        // The upper bound is a real one, not a guess: `MAX_FRAME_LEN` is the `u16` length prefix
        // every frame carries (`net/codec.rs`), so a chat line longer than that could never fit in
        // one frame regardless of what this field says.
        if self.max_chat_len == 0 || self.max_chat_len > terrustia_proto::MAX_FRAME_LEN {
            return Err(ConfigError::Invalid(format!(
                "max_chat_len must be between 1 and {}, got {}",
                terrustia_proto::MAX_FRAME_LEN,
                self.max_chat_len
            )));
        }
        // `AuditLog::rotate_if_needed` treats 0 as "never rotate" (`admin/audit.rs`), not "rotate
        // every write": past `audit_log_max_bytes`, the live file is left exactly where it is and
        // keeps growing forever instead of rolling to `.1`. Unbounded disk growth from a config
        // value that looked like "keep nothing" is exactly the silent-self-DoS shape this function
        // exists to catch.
        if self.audit_log_keep_segments == 0 {
            return Err(ConfigError::Invalid(
                "audit_log_keep_segments must be at least 1, or the live audit log never rotates \
                 and grows without bound"
                    .into(),
            ));
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
            "max_connections = 0",
            "max_connections = 65537",
            "max_connections_per_address = 0",
            "max_connections_per_address = 9999",
            "handshake_timeout_secs = 0",
            "handshake_timeout_secs = 3601",
            "idle_timeout_secs = 0",
            "idle_timeout_secs = 86401",
            "max_chat_len = 0",
            "max_chat_len = 65536",
            "audit_log_keep_segments = 0",
        ] {
            let config: Config = toml::from_str(text).unwrap();
            assert!(config.validate().is_err(), "{text} should be rejected");
        }
    }

    /// L6-04, fail-then-pass: `max_connections = 0` used to pass `validate` outright and only bite
    /// at runtime, where `net::listener::claim` refuses `guard.total >= max_total` and 0 makes that
    /// true for the very first connection, forever. A server started with this config would run,
    /// bind its port, and never let a single player (or admin) in: a self-DoS indistinguishable from
    /// a healthy idle server until someone tried to join.
    #[test]
    fn max_connections_zero_is_refused_as_a_self_dos() {
        let config: Config = toml::from_str("max_connections = 0").unwrap();
        let err = config
            .validate()
            .expect_err("0 must be refused, not silently accepted");
        assert!(
            err.to_string().contains("max_connections"),
            "the error should name the offending field: {err}"
        );
    }

    /// The same shape of bug, for the handshake side: `handshake_timeout_secs = 0` used to pass
    /// `validate`, then made `connection::serve`'s handshake deadline `now + 0`, so every new
    /// connection timed out before it could finish handshaking.
    #[test]
    fn handshake_timeout_zero_is_refused_as_a_self_dos() {
        let config: Config = toml::from_str("handshake_timeout_secs = 0").unwrap();
        assert!(config.validate().is_err());
    }

    /// `audit_log_keep_segments = 0` used to pass `validate`, then made
    /// `AuditLog::rotate_if_needed` skip rotation forever (`self.keep_segments == 0` short-circuits
    /// before the size check even runs): the live audit file would grow without bound instead of
    /// rolling over, an unbounded-disk-growth self-DoS with the same "passes validate, bites later"
    /// shape as the connection-limit cases above.
    #[test]
    fn audit_log_keep_segments_zero_is_refused() {
        let config: Config = toml::from_str("audit_log_keep_segments = 0").unwrap();
        assert!(config.validate().is_err());
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
    fn a_public_panel_listen_is_refused_even_with_the_panel_off() {
        // panel_enabled defaults false; a public panel_listen must still be rejected, because the
        // runtime `panel` toggle could otherwise bind that public address to the network later.
        let public: Config =
            toml::from_str("panel_enabled = false\npanel_listen = \"0.0.0.0:7778\"").unwrap();
        assert!(
            public.validate().is_err(),
            "a non-loopback panel address must never validate, enabled or not"
        );

        let loopback: Config = toml::from_str("panel_listen = \"127.0.0.1:7778\"").unwrap();
        loopback
            .validate()
            .expect("a loopback panel address is fine");
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

    /// The five fields that were real `Config` keys with no `TERRUSTIA_*` equivalent at all until
    /// this test's own fix — see `apply_env`'s own doc comment for how that was found (a
    /// field-by-field check against the "every key has one" claim `README.md`/
    /// `terrustia.toml.example` both made). Doesn't re-prove the general override/precedence
    /// machinery — `environment_variables_override_the_loaded_config` above already does that —
    /// just that these five specific names are now wired up at all.
    #[test]
    #[allow(unsafe_code)] // `set_var`/`remove_var` — see the SAFETY comments inline below.
    fn the_previously_missing_environment_variables_now_apply() {
        let _guard = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: `ENV_TEST_LOCK` above serializes every test in this file that touches a
        // `TERRUSTIA_*` variable, so nothing else can be reading or writing one of these names
        // concurrently for as long as `_guard` is held.
        unsafe {
            std::env::set_var("TERRUSTIA_MAX_CHAT_LEN", "123");
            std::env::set_var("TERRUSTIA_IDLE_TIMEOUT_SECS", "45");
            std::env::set_var("TERRUSTIA_MAX_CONNECTIONS", "678");
            std::env::set_var("TERRUSTIA_MAX_CONNECTIONS_PER_ADDRESS", "9");
            std::env::set_var("TERRUSTIA_HANDSHAKE_TIMEOUT_SECS", "10");
        }

        let mut config = Config::default();
        config.apply_env().expect("valid environment overrides");

        assert_eq!(config.max_chat_len, 123);
        assert_eq!(config.idle_timeout_secs, 45);
        assert_eq!(config.max_connections, 678);
        assert_eq!(config.max_connections_per_address, 9);
        assert_eq!(config.handshake_timeout_secs, 10);

        // SAFETY: same justification as above — cleaning up what this test itself set.
        unsafe {
            std::env::remove_var("TERRUSTIA_MAX_CHAT_LEN");
            std::env::remove_var("TERRUSTIA_IDLE_TIMEOUT_SECS");
            std::env::remove_var("TERRUSTIA_MAX_CONNECTIONS");
            std::env::remove_var("TERRUSTIA_MAX_CONNECTIONS_PER_ADDRESS");
            std::env::remove_var("TERRUSTIA_HANDSHAKE_TIMEOUT_SECS");
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
