//! The server side of the web panel.
//!
//! The panel itself lives in `crate::panel` and never touches the world: it asks for what it needs
//! over [`ServerEvent`](super::ServerEvent) and gets one of the snapshot types below back down a
//! `oneshot`. Everything here runs on the game task, so each of these is a read (or a small,
//! guarded write) that has to be cheap enough to sit between two ticks.

use std::net::SocketAddr;

use tracing::info;

use crate::game::player::Player;

use super::{
    GameServer,
    tick::{Phase, TICK},
};

/// One connected player, as the panel needs to show them: who they are, how they are doing, where
/// they are, and — for the live world view — enough of their real appearance data to draw a
/// stylized avatar rather than a sprite. See `panel/mod.rs`'s module doc for why nothing here is
/// (or ever will be) a composited Terraria asset.
#[derive(Debug, Clone)]
pub struct PanelPlayer {
    pub slot: u8,
    pub name: String,
    pub address: String,
    pub life: i16,
    pub life_max: i16,
    pub mana: i16,
    pub mana_max: i16,
    pub position: (f32, f32),
    pub pvp: bool,
    pub appearance: Option<terrustia_proto::player_info::PlayerAppearance>,
    /// Non-zero item ids currently worn in the armour/accessory slots (`inventory.rs`'s
    /// `SLOT_RUNS` run 2, slots 59..79) — real equipped gear, not decoration invented for the
    /// avatar.
    pub equipped: Vec<i32>,
    /// Whether this player's name is currently muted. See `Admin::is_muted`.
    pub muted: bool,
}

/// What [`ServerEvent::PanelWhitelist`](super::ServerEvent::PanelWhitelist) hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelWhitelist {
    pub on: bool,
    pub names: Vec<String>,
}

/// A coarse sample of the world's tiles for the panel's live world screen: one colour bucket per
/// sample point on a fixed-size grid, regardless of how large the actual world is. See
/// [`GameServer::world_tile_sample`] for how the grid is chosen and why sampling — not a full
/// tile dump — is the honest way to stream this over a websocket.
#[derive(Debug, Clone)]
pub struct PanelWorldTiles {
    pub world_width: i32,
    pub world_height: i32,
    pub sample_cols: u32,
    pub sample_rows: u32,
    /// `sample_cols * sample_rows` colour buckets, row-major, one per sample point.
    pub tiles: Vec<TileColor>,
}

/// A tile's colour bucket, for the panel's stylized (not sprite-accurate) world render. See
/// [`GameServer::tile_color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileColor {
    /// No active tile — open sky, or an unlit cave; the sample carries no depth information to
    /// tell those apart, so it does not pretend to.
    Empty,
    Dirt,
    Stone,
    Grass,
    Corruption,
    Crimson,
    Sand,
    Snow,
    Ice,
    Jungle,
    Ore,
    Gem,
    Water,
    Lava,
    Honey,
    Ash,
    /// Anything active with no bucket of its own — built structures, furniture, and the many tile
    /// ids this sampler does not have a named constant for.
    Other,
}

/// A read-only snapshot of [`Config`](crate::config::Config) for the panel's settings view. Never
/// carries the actual server password — only whether one is set.
#[derive(Debug, Clone)]
pub struct PanelConfigSnapshot {
    pub listen: SocketAddr,
    pub max_players: usize,
    pub world_width: i32,
    pub world_height: i32,
    pub motd: String,
    pub password_set: bool,
    pub max_chat_len: usize,
    pub idle_timeout_secs: u64,
    pub autosave_secs: u64,
    pub save_target: Option<String>,
    pub whitelist_on: bool,
    pub whitelist_count: usize,
}

/// What [`ServerEvent::PanelAuthLookup`](super::ServerEvent::PanelAuthLookup) hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelAuthLookup {
    pub unclaimed: bool,
    /// The currently-active one-time claim token, if the server is unclaimed and one has been
    /// printed to the console. `None` means either the server is claimed, or nobody has connected
    /// yet to trigger `announce_claim_token`.
    pub claim_token: Option<String>,
    /// `(hash, group)` for the named account, if one exists.
    pub hash_and_group: Option<(String, String)>,
    /// Whether that account's group grants `panel.view` — i.e. may sign in to the panel at all. A
    /// self-registered `default` account does not, by default; a specific route beyond login is
    /// then gated on its own permission (see `panel/mod.rs`'s module doc for the full mapping),
    /// not on this flag.
    pub panel_view: bool,
    /// The raw permission strings the account's group holds (or empty, for no account/unknown
    /// group). Handed back to the frontend on `/api/status` so it can decide which tabs and buttons
    /// to show — a UX convenience only; every route re-checks its own permission server-side
    /// regardless of what the panel chose to display.
    pub permissions: Vec<String>,
}

/// What [`ServerEvent::PanelStatus`](super::ServerEvent::PanelStatus) hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelStatus {
    pub player_count: usize,
    pub max_players: usize,
    pub world_name: String,
    /// The file stem of the world currently being served, if it has one, so the panel's world
    /// list can mark which entry is the running one.
    pub world_file: Option<String>,
    /// How many world saves have failed in a row: the saves-failing indicator.
    ///
    /// `0` is healthy and is what a panel should show nothing for. Anything else means the world on
    /// disk is older than the world being played, and by roughly this many autosave intervals - the
    /// one server-health fact a player-facing dashboard has no other way to learn, since a failing
    /// save is otherwise only a line in a log nobody has open. Crosses
    /// [`super::SAVE_FAILURES_BEFORE_ALARM`] and the players have been told in chat as well.
    pub save_failures: u32,
}

/// What [`ServerEvent::PanelMetrics`](super::ServerEvent::PanelMetrics) hands back — a live
/// snapshot for the panel's metrics graphs. Every duration is in microseconds, on the same clock
/// the tick budget itself uses. No history is kept here; the panel accumulates its own rolling
/// window client-side.
#[derive(Debug, Clone, Default)]
pub struct PanelMetrics {
    /// The tick budget, so the panel can draw the line a tick is measured against.
    pub budget_us: u64,
    /// The most recent tick's processor cost, and how long it actually took to happen.
    pub last_cpu_us: u64,
    pub last_wall_us: u64,
    /// The worst processor cost seen in the current reporting window, before it is reset.
    pub worst_cpu_us: u64,
    /// The last tick's per-phase processor breakdown, `(phase name, microseconds)`, in tick order.
    pub phases: Vec<(&'static str, u64)>,
    pub player_count: usize,
    pub npc_count: usize,
    pub projectile_count: usize,
    pub item_count: usize,
    /// Ticks elapsed since the process started — a monotonic clock the panel can label the x-axis
    /// with without needing wall-clock time.
    pub ticks: u64,
}

/// One world backup on disk, for the panel's backup/rollback view.
#[derive(Debug, Clone)]
pub struct PanelBackupEntry {
    /// `1` is the most recent — the number `rollback <n>` takes.
    pub index: usize,
    pub size_bytes: u64,
    /// Seconds since the backup was written, or `None` if the filesystem would not say.
    pub age_secs: Option<u64>,
}

/// What [`ServerEvent::PanelBackups`](super::ServerEvent::PanelBackups) hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelBackups {
    /// Whether this world is being saved at all — `false` means there is nothing to back up or roll
    /// back, and the panel says so rather than showing an empty list as if saves were merely absent.
    pub saving: bool,
    /// The world file the backups belong to, for display.
    pub world_file: Option<String>,
    /// How many backups the server keeps, so the panel can explain the rotation.
    pub kept: usize,
    pub backups: Vec<PanelBackupEntry>,
}

/// One admin group, for the panel's accounts view.
#[derive(Debug, Clone)]
pub struct PanelGroupInfo {
    pub name: String,
    /// The permission names the group holds (or the single entry `*`).
    pub permissions: Vec<String>,
    /// Whether this group can administer the server — i.e. change who may do what.
    pub can_admin: bool,
}

/// One account, for the panel's accounts view. Never carries the password hash.
#[derive(Debug, Clone)]
pub struct PanelAccountInfo {
    pub name: String,
    pub group: String,
    /// Whether the account's group actually resolves to one that can administer the server.
    pub can_admin: bool,
}

/// What [`ServerEvent::PanelAccounts`](super::ServerEvent::PanelAccounts) hands back.
#[derive(Debug, Clone, Default)]
pub struct PanelAccounts {
    pub groups: Vec<PanelGroupInfo>,
    pub accounts: Vec<PanelAccountInfo>,
}

impl GameServer {
    /// Whether an account's group can edit who is allowed to do what (`admin.groups`, or the
    /// wildcard). The last such account is the one the panel refuses to strip or delete — see
    /// [`ServerEvent::PanelSetAccountGroup`](super::ServerEvent::PanelSetAccountGroup) — because
    /// losing it would mean nobody could ever fix a permissions mistake from the panel again (the
    /// server's own console can always do so regardless; this guard is about not needing it to).
    fn account_can_admin(&self, account: &crate::admin::Account) -> bool {
        self.admin
            .groups
            .iter()
            .find(|g| g.name == account.group)
            .is_some_and(|g| g.may(crate::admin::perm::ADMIN_GROUPS))
    }

    /// How many accounts can still administer the server. Used to refuse the change that would take
    /// this to zero.
    fn admin_capable_accounts(&self) -> usize {
        self.admin
            .accounts
            .iter()
            .filter(|a| self.account_can_admin(a))
            .count()
    }

    /// A live snapshot for the panel's metrics view. See [`PanelMetrics`].
    pub(super) fn panel_metrics(&self) -> PanelMetrics {
        let phases = Phase::NAMES
            .iter()
            .zip(self.last_tick.phases.iter())
            .map(|(&name, dur)| (name, dur.as_micros() as u64))
            .collect();
        PanelMetrics {
            budget_us: TICK.as_micros() as u64,
            last_cpu_us: self.last_tick.cpu.as_micros() as u64,
            last_wall_us: self.last_tick.wall.as_micros() as u64,
            worst_cpu_us: self.worst_tick.cpu.as_micros() as u64,
            phases,
            player_count: self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing())
                .count(),
            npc_count: self.npcs.len(),
            projectile_count: self.projectiles.len(),
            item_count: self.items.len(),
            ticks: self.ticks,
        }
    }

    /// The world backups on disk, newest first. Mirrors [`Self::list_backups`]'s own scan, but hands
    /// the data back rather than logging it — the panel draws the same rows the console prints.
    pub(super) fn panel_backups(&self) -> PanelBackups {
        let kept = crate::world::wld_save::BACKUPS_KEPT;
        let Some(path) = self.save_path.clone() else {
            return PanelBackups {
                saving: false,
                world_file: None,
                kept,
                backups: Vec::new(),
            };
        };
        let mut backups = Vec::new();
        for n in 1..=kept {
            let bak = path.with_extension(format!("wld.bak{n}"));
            let Ok(meta) = std::fs::metadata(&bak) else {
                continue;
            };
            let age_secs = meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs());
            backups.push(PanelBackupEntry {
                index: n,
                size_bytes: meta.len(),
                age_secs,
            });
        }
        PanelBackups {
            saving: true,
            world_file: self.current_world_file_stem(),
            kept,
            backups,
        }
    }

    /// The groups and accounts, for the panel's accounts view. Never carries a password hash.
    pub(super) fn panel_accounts(&self) -> PanelAccounts {
        let groups = self
            .admin
            .groups
            .iter()
            .map(|g| PanelGroupInfo {
                name: g.name.clone(),
                permissions: g.permissions.iter().cloned().collect(),
                can_admin: g.may(crate::admin::perm::ADMIN_GROUPS),
            })
            .collect();
        let accounts = self
            .admin
            .accounts
            .iter()
            .map(|a| PanelAccountInfo {
                name: a.name.clone(),
                group: a.group.clone(),
                can_admin: self.account_can_admin(a),
            })
            .collect();
        PanelAccounts { groups, accounts }
    }

    /// Move an account into a different group. The console `group` command's own rule (the group
    /// must exist), the lock-out guard the panel needs, and the anti-escalation guard every
    /// account/group change needs: `actor` must already reach everything `group` holds (see
    /// `Admin::group_within_reach`), or this refuses — without it, an `admin.accounts` holder could
    /// promote themselves straight into `owner` through this very route.
    pub(super) fn panel_set_account_group(
        &mut self,
        actor: &str,
        name: &str,
        group: &str,
    ) -> Result<(), String> {
        if !self.admin.groups.iter().any(|g| g.name == group) {
            return Err(format!("there is no group called {group}"));
        }
        let Some(actor_group) = self.admin.account_group(actor) else {
            return Err("your own account no longer exists".into());
        };
        if !self.admin.group_within_reach(actor_group, group) {
            return Err(format!(
                "you cannot move anyone into '{group}': it holds permissions you do not have \
                 yourself"
            ));
        }
        let Some(account) = self
            .admin
            .accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        else {
            return Err(format!("there is no account called {name}"));
        };
        // Would this move strip the last account that can still administer the server?
        let target_can_admin_now = self.account_can_admin(account);
        let new_group_can_admin = self
            .admin
            .groups
            .iter()
            .find(|g| g.name == group)
            .is_some_and(|g| g.may(crate::admin::perm::ADMIN_GROUPS));
        if target_can_admin_now && !new_group_can_admin && self.admin_capable_accounts() <= 1 {
            return Err(
                "that is the only account that can still administer the server; make another \
                 account an admin before removing this one's access"
                    .into(),
            );
        }
        // The same lookup succeeded a few lines above and nothing between the two touches the
        // account list, so this cannot miss. It reports the miss rather than panicking anyway: a
        // panic on the game task loses the world back to the last autosave, and answering a panel
        // request with the error the first lookup would have given costs nothing.
        let Some(account) = self
            .admin
            .accounts
            .iter_mut()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        else {
            return Err(format!("there is no account called {name}"));
        };
        account.group = group.to_string();
        let _ = self.admin.save();
        self.audit.record(
            actor,
            crate::admin::AuditAction::GroupChange,
            name,
            &format!("-> {group}"),
        );
        info!(
            account = name,
            group, actor, "group changed from the web panel"
        );
        Ok(())
    }

    /// Create an account through the panel, guarded by the same anti-escalation rule
    /// [`Self::panel_set_account_group`] applies: `actor` must already reach everything the new
    /// account's chosen group holds.
    pub(super) fn panel_create_account(
        &mut self,
        actor: &str,
        account: crate::admin::Account,
    ) -> Result<(), String> {
        // The group has to exist — an account pointed at a group that is not there silently falls
        // back to `default`, which would be a confusing thing to have just created.
        if !self.admin.groups.iter().any(|g| g.name == account.group) {
            return Err(format!("there is no group called {}", account.group));
        }
        let Some(actor_group) = self.admin.account_group(actor) else {
            return Err("your own account no longer exists".into());
        };
        if !self.admin.group_within_reach(actor_group, &account.group) {
            return Err(format!(
                "you cannot create an account in '{}': it holds permissions you do not have \
                 yourself",
                account.group
            ));
        }
        let name = account.name.clone();
        let group = account.group.clone();
        let result = self.admin.insert_account(account);
        if result.is_ok() {
            let _ = self.admin.save();
            self.audit.record(
                actor,
                crate::admin::AuditAction::Register,
                &name,
                &format!("group: {group}"),
            );
            info!(
                account = name,
                group, actor, "account created from the web panel"
            );
        }
        result
    }

    /// Add or remove a permission on a group. `actor` must already hold `permission` themselves —
    /// the same reach rule as account/group changes, applied to a single permission instead of a
    /// whole group's worth, so the group editor cannot be used to grant a permission nobody making
    /// the change actually has. An unrecognised permission name is refused outright.
    pub(super) fn panel_set_group_permission(
        &mut self,
        actor: &str,
        group: &str,
        permission: &str,
        grant: bool,
    ) -> Result<(), String> {
        if !crate::admin::group::is_known(permission) {
            return Err(format!("'{permission}' is not a recognised permission"));
        }
        let Some(actor_group) = self.admin.account_group(actor) else {
            return Err("your own account no longer exists".into());
        };
        if grant && !self.admin.group_grants_str(actor_group, permission) {
            return Err(format!(
                "you cannot grant '{permission}': you do not hold it yourself"
            ));
        }
        let Some(target) = self.admin.groups.iter_mut().find(|g| g.name == group) else {
            return Err(format!("there is no group called {group}"));
        };
        let changed = if grant {
            target.permissions.insert(permission.to_string())
        } else {
            target.permissions.remove(permission)
        };
        if changed {
            let _ = self.admin.save();
            self.audit.record(
                actor,
                crate::admin::AuditAction::PermissionChange,
                group,
                &format!("{permission} grant={grant}"),
            );
            info!(
                group,
                permission, grant, actor, "group permissions changed from the web panel"
            );
        }
        Ok(())
    }

    /// Delete an account, guarded so the last admin-capable one cannot be removed, and by the same
    /// anti-escalation rule [`Self::panel_set_account_group`] applies: `actor` must already reach
    /// everything the target account's own group holds (see `Admin::group_within_reach`), or this
    /// refuses. Without it, an `admin.accounts` holder (which does not itself hold `admin.groups`)
    /// could delete an `owner` account outright: a strictly bigger escalation than anything the
    /// group-change route's own reach check stops, and one this route used to allow entirely
    /// unchecked.
    pub(super) fn panel_delete_account(&mut self, actor: &str, name: &str) -> Result<(), String> {
        let Some(account) = self
            .admin
            .accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
        else {
            return Err(format!("there is no account called {name}"));
        };
        let Some(actor_group) = self.admin.account_group(actor) else {
            return Err("your own account no longer exists".into());
        };
        if !self.admin.group_within_reach(actor_group, &account.group) {
            return Err(format!(
                "you cannot delete {name}: their group holds permissions you do not have yourself"
            ));
        }
        if self.account_can_admin(account) && self.admin_capable_accounts() <= 1 {
            return Err(
                "that is the only account that can still administer the server; it cannot be \
                 deleted, or nobody could sign in to the panel again"
                    .into(),
            );
        }
        self.admin
            .accounts
            .retain(|a| !a.name.eq_ignore_ascii_case(name));
        let _ = self.admin.save();
        self.audit
            .record(actor, crate::admin::AuditAction::DeleteAccount, name, "");
        info!(account = name, actor, "account deleted from the web panel");
        Ok(())
    }

    /// The file stem of the world currently being served, if it has one.
    pub(super) fn current_world_file_stem(&self) -> Option<String> {
        self.save_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(str::to_string)
    }

    /// Every connected player, in the shape the panel needs for the player list and the live world
    /// view. `appearance` decodes `Player::appearance`'s raw bytes on demand rather than caching a
    /// decoded copy on `Player` itself — this is the only consumer, and it is asked for at most a
    /// couple of times a second, not once per tick.
    pub(super) fn panel_players(&self) -> Vec<PanelPlayer> {
        self.players
            .iter()
            .flatten()
            .filter(|p| p.is_playing())
            .map(|p| PanelPlayer {
                slot: p.slot,
                name: p.name.clone(),
                address: p.addr.ip().to_string(),
                life: p.life,
                life_max: p.life_max,
                mana: p.mana,
                mana_max: p.mana_max,
                position: p.position,
                pvp: p.pvp,
                appearance: p.appearance.as_ref().and_then(|bytes| {
                    terrustia_proto::player_info::PlayerAppearance::decode(bytes).ok()
                }),
                equipped: Self::equipped_items(p),
                muted: self.admin.is_muted(&p.name),
            })
            .collect()
    }

    /// Non-zero item ids in the armour/accessory slot run — real gear, used to accent the panel's
    /// stylized avatar. See `terrustia_proto::inventory`'s `SLOT_RUNS` for the layout: 58 inventory
    /// slots, then 1 cursor slot, then this 20-slot armour/accessory run.
    fn equipped_items(player: &Player) -> Vec<i32> {
        const ARMOR_SLOTS_START: u16 = 59;
        const ARMOR_SLOTS_END: u16 = 79;
        let mut items: Vec<i32> = player
            .inventory
            .iter()
            .filter(|(slot, _)| (ARMOR_SLOTS_START..ARMOR_SLOTS_END).contains(slot))
            .filter(|(_, equip)| equip.item.id != 0)
            .map(|(_, equip)| equip.item.id)
            .collect();
        items.sort_unstable();
        items
    }

    /// How many sample points the panel's live world view gets along each axis, regardless of how
    /// large the actual world is. A full tile-for-tile dump of even a small 4200x1200 world is
    /// five million tiles — nothing a websocket should re-send every few seconds. This is dense
    /// enough to show real terrain shape at a glance and cheap enough to resample from scratch on
    /// every request: at most `WORLD_SAMPLE_COLS * WORLD_SAMPLE_ROWS` tile reads, each a plain
    /// array index.
    pub(super) fn world_tile_sample(&self) -> PanelWorldTiles {
        const WORLD_SAMPLE_COLS: u32 = 160;
        const WORLD_SAMPLE_ROWS: u32 = 90;

        let width = self.world.width();
        let height = self.world.height();
        let cols = WORLD_SAMPLE_COLS.min(width.max(1) as u32).max(1);
        let rows = WORLD_SAMPLE_ROWS.min(height.max(1) as u32).max(1);
        let mut tiles = Vec::with_capacity((cols * rows) as usize);
        for row in 0..rows {
            let y = ((row * height.max(1) as u32) / rows).min((height - 1).max(0) as u32) as i32;
            for col in 0..cols {
                let x = ((col * width.max(1) as u32) / cols).min((width - 1).max(0) as u32) as i32;
                tiles.push(Self::tile_color(self.world.tile(x, y)));
            }
        }
        PanelWorldTiles {
            world_width: width,
            world_height: height,
            sample_cols: cols,
            sample_rows: rows,
            tiles,
        }
    }

    /// Bucket a tile into a solid colour, not a sprite. Every id below is transcribed from
    /// `crate::world::worldgen::tiles`, the same table the generator itself is checked against —
    /// nothing here is invented. An id with no bucket falls into [`TileColor::Other`] rather than
    /// guessing.
    fn tile_color(tile: terrustia_proto::Tile) -> TileColor {
        use crate::world::worldgen::tiles as t;

        if tile.liquid > 0 {
            return match tile.liquid_kind {
                terrustia_proto::Liquid::Lava => TileColor::Lava,
                terrustia_proto::Liquid::Honey => TileColor::Honey,
                terrustia_proto::Liquid::Water | terrustia_proto::Liquid::Shimmer => {
                    TileColor::Water
                }
            };
        }
        if !tile.is_active() {
            return TileColor::Empty;
        }
        match tile.block {
            t::GRASS => TileColor::Grass,
            t::CORRUPT_GRASS | t::EBONSTONE | t::DEMON_ALTAR | t::SHADOW_ORB => {
                TileColor::Corruption
            }
            t::CRIMSON_GRASS | t::CRIMSTONE => TileColor::Crimson,
            t::JUNGLE_GRASS | t::MUD | t::HIVE | t::LIHZAHRD_BRICK | t::MUSHROOM_GRASS => {
                TileColor::Jungle
            }
            t::SAND | t::EBONSAND | t::CRIMSAND | t::SANDSTONE | t::HARDENED_SAND | t::SILT => {
                TileColor::Sand
            }
            t::SNOW => TileColor::Snow,
            t::ICE => TileColor::Ice,
            t::STONE | t::MARBLE | t::GRANITE | t::ASH | t::OBSIDIAN | t::CLAY => TileColor::Stone,
            t::IRON | t::COPPER | t::GOLD | t::SILVER | t::DEMONITE | t::CRIMTANE => TileColor::Ore,
            t::SAPPHIRE | t::RUBY | t::EMERALD | t::TOPAZ | t::AMETHYST | t::DIAMOND => {
                TileColor::Gem
            }
            t::DIRT => TileColor::Dirt,
            _ => TileColor::Other,
        }
    }
}

/// The web panel's kick/ban/whitelist/world-view/world-switch events. Each one is checked
/// directly against `handle_event`, the same entry point the real panel HTTP handlers reach
/// through `ServerEvent` — see `tests/panel.rs` for the same features exercised end to end over a
/// real socket instead.
#[cfg(test)]
mod panel_admin_events {
    use super::*;
    use crate::admin::BanKind;
    use crate::config::Config;
    use crate::game::player::ConnState;
    use crate::game::server::ServerEvent;
    use std::path::PathBuf;
    use terrustia_proto::TileFlags;
    use tokio::sync::{mpsc, oneshot};

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "panel events probe")
    }

    /// A player in `ConnState::Playing`, inserted directly into the slot — the shape every other
    /// test in this file that needs a "connected" player without a real socket already uses.
    fn seat_player(server: &mut GameServer, slot: u8, name: &str) {
        let (tx, _rx) = mpsc::channel(4);
        let mut player = Player::new(slot, "127.0.0.1:4000".parse().unwrap(), tx);
        player.name = name.to_string();
        player.state = ConnState::Playing;
        server.players[slot as usize] = Some(player);
    }

    fn oneshot_reply<T>() -> (oneshot::Sender<T>, oneshot::Receiver<T>) {
        oneshot::channel()
    }

    #[test]
    fn kicking_a_connected_player_removes_them_and_reports_success() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        seat_player(&mut server, 0, "Griefer");

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelKick {
            actor: "owner".into(),
            name: "griefer".into(), // case-insensitive, matching `/kick`
            reason: "wrecked spawn".into(),
            reply,
        });
        assert!(rx.try_recv().expect("a reply was sent").is_ok());
        assert!(
            server.players[0].is_none(),
            "the kicked player must actually be gone"
        );
    }

    #[test]
    fn kicking_nobody_reports_failure_without_touching_anyone() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelKick {
            actor: "owner".into(),
            name: "nobody-here".into(),
            reason: String::new(),
            reply,
        });
        assert!(rx.try_recv().expect("a reply was sent").is_err());
    }

    #[test]
    fn banning_a_connected_player_bans_and_kicks_them() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        seat_player(&mut server, 0, "Griefer");

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelBan {
            actor: "owner".into(),
            kind: BanKind::Name,
            value: "Griefer".into(),
            reason: "wrecked spawn".into(),
            reply,
        });
        rx.try_recv().expect("a reply was sent");
        assert!(server.players[0].is_none(), "a banned player is removed");
        assert!(
            server.admin.ban_for("Griefer", "1.2.3.4", None).is_some(),
            "the ban itself must be recorded, not just the kick"
        );
    }

    #[test]
    fn unbanning_lifts_a_real_ban() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server
            .admin
            .ban(BanKind::Name, "Griefer", "wrecked spawn", "owner");
        assert!(server.admin.ban_for("Griefer", "0.0.0.0", None).is_some());

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelUnban {
            actor: "owner".into(),
            value: "Griefer".into(),
            reply,
        });
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(server.admin.ban_for("Griefer", "0.0.0.0", None).is_none());
    }

    #[test]
    fn whitelist_add_and_remove_round_trip_through_the_events() {
        let mut server = GameServer::new(Config::default(), tiny_world());

        let (add_reply, mut add_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelistAdd {
            actor: "owner".into(),
            name: "Brooklyn".into(),
            reply: add_reply,
        });
        assert!(add_rx.try_recv().unwrap());

        let (list_reply, mut list_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelist { reply: list_reply });
        let list = list_rx.try_recv().unwrap();
        assert!(list.on);
        assert_eq!(list.names, vec!["Brooklyn".to_string()]);

        let (remove_reply, mut remove_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelistRemove {
            actor: "owner".into(),
            name: "brooklyn".into(), // case-insensitive, matching the console command
            reply: remove_reply,
        });
        assert!(remove_rx.try_recv().unwrap());

        let (list_reply2, mut list_rx2) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelist { reply: list_reply2 });
        assert!(!list_rx2.try_recv().unwrap().on, "an empty list is off");
    }

    /// Sanity check for the helper the account-deletion and whitelist audit tests below rely on: an
    /// in-memory admin store (no `save_file`, what `Config::default()` gives every other test in
    /// this module) also has an in-memory audit log, which records nothing at all and would make "no
    /// audit line was written" trivially true for the wrong reason. Those tests need a file-backed
    /// log instead, via `server_with_real_admin_files`, so a refusal that wrote nothing can be told
    /// apart from a store that could never have written anything.
    #[test]
    fn an_in_memory_admin_store_has_an_in_memory_audit_log_too() {
        let server = GameServer::new(Config::default(), tiny_world());
        assert!(
            server.audit.tail(10).is_empty(),
            "an in-memory audit log is always empty, by construction"
        );
    }

    /// A `GameServer` whose `admin`/`audit` are backed by real files in a fresh temp directory,
    /// rather than the in-memory stores `Config::default()` gives every other test in this module
    /// (see the sanity check just above): needed here because the account-deletion tests must be
    /// able to tell "refused, and correctly wrote no audit line" apart from "using a store that
    /// cannot write one regardless". The directory is not cleaned up; it is process- and
    /// thread-unique (`safe_write::tests::temp_dir`) and cheap enough to leave for the OS.
    fn server_with_real_admin_files(name: &str) -> GameServer {
        let dir = crate::safe_write::tests::temp_dir(name);
        let save_file = dir.join("world.wld");
        let config = Config {
            save_file: Some(save_file),
            ..Config::default()
        };
        GameServer::new(config, tiny_world())
    }

    /// L6-01: deleting an account used to skip the reach check every sibling account/group route
    /// enforces, so an `admin`-tier account (holds `admin.accounts` but not `admin.groups`) could
    /// delete an `owner` outright: a bigger escalation than `group_within_reach` was ever meant to
    /// let through by the group-change route. This is the fail-then-pass case: before
    /// `panel_delete_account` grew its own `group_within_reach` check, this refusal did not happen
    /// (the deletion went through) and no audit line existed to catch it either, since the whole
    /// route wrote none at all.
    ///
    /// Two `owner` accounts, deliberately: with only one, the pre-existing "do not strip the last
    /// admin-capable account" guard refuses the same deletion for an unrelated reason (verified by
    /// disabling the reach check locally and re-running this test: with a single owner it still
    /// passed, for the wrong reason, which is exactly the false confidence a real regression could
    /// hide behind). A second owner keeps that guard from firing, so a failure here can only mean
    /// the reach check itself stopped catching the escalation.
    #[test]
    fn deleting_an_owner_as_an_admin_tier_account_is_refused_and_unaudited() {
        let mut server = server_with_real_admin_files("panel-delete-account-refused");
        server
            .admin
            .insert_account(crate::admin::Account::new("boss", "hunter22", "owner").unwrap())
            .unwrap();
        server
            .admin
            .insert_account(crate::admin::Account::new("boss2", "hunter22", "owner").unwrap())
            .unwrap();
        server
            .admin
            .insert_account(crate::admin::Account::new("shady", "hunter22", "admin").unwrap())
            .unwrap();

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelDeleteAccount {
            actor: "shady".into(),
            name: "boss".into(),
            reply,
        });
        let outcome = rx.try_recv().expect("a reply was sent");
        assert!(
            outcome.is_err(),
            "an admin-tier actor must not be able to delete an owner, even when a second owner \
             means the last-admin-capable guard would not have caught it either"
        );
        assert!(
            server.admin.accounts.iter().any(|a| a.name == "boss"),
            "the refused deletion must not have touched the account"
        );
        assert!(
            server.audit.tail(10).is_empty(),
            "a refused deletion must not write an audit line either"
        );
    }

    /// The mirror of the refusal above: an actor whose group already reaches the target's group
    /// (here, the owner deleting a lesser account) succeeds, and is recorded in the audit log with
    /// the real actor as issuer, exactly as `panel_set_account_group`/`panel_create_account` are.
    #[test]
    fn deleting_an_account_within_the_actors_reach_succeeds_and_is_audited() {
        let mut server = server_with_real_admin_files("panel-delete-account-allowed");
        server
            .admin
            .insert_account(crate::admin::Account::new("boss", "hunter22", "owner").unwrap())
            .unwrap();
        server
            .admin
            .insert_account(crate::admin::Account::new("shady", "hunter22", "admin").unwrap())
            .unwrap();

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelDeleteAccount {
            actor: "boss".into(),
            name: "shady".into(),
            reply,
        });
        assert!(
            rx.try_recv().expect("a reply was sent").is_ok(),
            "an owner may delete an account within their reach"
        );
        assert!(
            !server.admin.accounts.iter().any(|a| a.name == "shady"),
            "the account must actually be gone"
        );
        let tail = server.audit.tail(10);
        assert_eq!(tail.len(), 1, "the deletion must be audited");
        assert_eq!(tail[0].issuer, "boss");
        assert_eq!(tail[0].target, "shady");
        assert_eq!(tail[0].action, crate::admin::AuditAction::DeleteAccount);
    }

    /// L6-05: whitelist changes made from the panel used to leave no trace in the audit log at all,
    /// unlike every other moderation action (kick, ban, mute, group change all record one). Fail-
    /// then-pass: before `PanelWhitelistAdd`/`PanelWhitelistRemove` threaded `actor` through to
    /// `self.audit.record`, `server.audit.tail(10)` after this sequence was empty.
    #[test]
    fn whitelist_changes_from_the_panel_are_audited() {
        let mut server = server_with_real_admin_files("panel-whitelist-audit");

        let (add_reply, mut add_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelistAdd {
            actor: "boss".into(),
            name: "Brooklyn".into(),
            reply: add_reply,
        });
        assert!(add_rx.try_recv().unwrap());

        let (remove_reply, mut remove_rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelWhitelistRemove {
            actor: "boss".into(),
            name: "Brooklyn".into(),
            reply: remove_reply,
        });
        assert!(remove_rx.try_recv().unwrap());

        let tail = server.audit.tail(10);
        assert_eq!(
            tail.len(),
            2,
            "both the add and the remove must be audited: {tail:?}"
        );
        assert_eq!(tail[0].issuer, "boss");
        assert_eq!(tail[0].target, "Brooklyn");
        assert_eq!(tail[0].action, crate::admin::AuditAction::Whitelist);
        assert_eq!(tail[0].detail, "added");
        assert_eq!(tail[1].action, crate::admin::AuditAction::Whitelist);
        assert_eq!(tail[1].detail, "removed");
    }

    #[test]
    fn a_switch_to_a_real_file_arms_the_pending_switch_and_starts_stopping() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let handle = server.world_switch_handle();
        let target = std::env::temp_dir().join(format!(
            "terrustia-panel-switch-test-{}.wld",
            std::process::id()
        ));
        std::fs::write(&target, b"not a real world, just needs to exist").unwrap();

        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelSwitchWorld {
            path: target.clone(),
            reply,
        });
        assert!(rx.try_recv().unwrap().is_ok());
        assert!(server.stopping, "a switch is a controlled shutdown");
        assert_eq!(
            handle.lock().unwrap().as_deref(),
            Some(target.as_path()),
            "main has to be able to read this back after `run` returns"
        );
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn a_switch_to_a_missing_file_is_refused_and_does_not_stop_the_server() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        let (reply, mut rx) = oneshot_reply();
        server.handle_event(ServerEvent::PanelSwitchWorld {
            path: PathBuf::from("/no/such/world/anywhere.wld"),
            reply,
        });
        assert!(rx.try_recv().unwrap().is_err());
        assert!(!server.stopping);
    }

    #[test]
    fn tile_colour_buckets_match_the_generators_own_ids() {
        use crate::world::worldgen::tiles as t;

        let dirt = terrustia_proto::Tile {
            block: t::DIRT,
            flags: TileFlags(TileFlags::ACTIVE),
            ..terrustia_proto::Tile::AIR
        };
        assert_eq!(GameServer::tile_color(dirt), TileColor::Dirt);

        let stone = terrustia_proto::Tile {
            block: t::STONE,
            flags: TileFlags(TileFlags::ACTIVE),
            ..terrustia_proto::Tile::AIR
        };
        assert_eq!(GameServer::tile_color(stone), TileColor::Stone);

        assert_eq!(
            GameServer::tile_color(terrustia_proto::Tile::AIR),
            TileColor::Empty,
            "an inactive tile has nothing to colour"
        );

        let mut lava = terrustia_proto::Tile::AIR;
        lava.liquid = 255;
        lava.liquid_kind = terrustia_proto::Liquid::Lava;
        assert_eq!(GameServer::tile_color(lava), TileColor::Lava);
    }

    #[test]
    fn the_world_sample_never_exceeds_the_actual_world_and_never_panics_on_a_tiny_one() {
        let server = GameServer::new(Config::default(), crate::world::World::empty(200, 150, "s"));
        let sample = server.world_tile_sample();
        assert!(sample.sample_cols as i32 <= sample.world_width);
        assert!(sample.sample_rows as i32 <= sample.world_height);
        assert_eq!(
            sample.tiles.len(),
            (sample.sample_cols * sample.sample_rows) as usize
        );
    }

    #[test]
    fn equipped_items_only_reads_the_armour_slot_run_and_ignores_empty_slots() {
        let (tx, _rx) = mpsc::channel(4);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), tx);
        // Slot 10 is ordinary inventory, not gear — must be ignored.
        player.inventory.insert(
            10,
            terrustia_proto::inventory::SyncEquipment {
                player: 0,
                slot: 10,
                item: terrustia_proto::ItemStack {
                    id: 999,
                    stack: 1,
                    prefix: 0,
                },
                favorited: false,
                blocked: false,
            },
        );
        // Slot 60 is inside the armour run and carries a real item.
        player.inventory.insert(
            60,
            terrustia_proto::inventory::SyncEquipment {
                player: 0,
                slot: 60,
                item: terrustia_proto::ItemStack {
                    id: 42,
                    stack: 1,
                    prefix: 0,
                },
                favorited: false,
                blocked: false,
            },
        );
        // Slot 61 is inside the run but empty (item id 0) — must be ignored too.
        player.inventory.insert(
            61,
            terrustia_proto::inventory::SyncEquipment {
                player: 0,
                slot: 61,
                item: terrustia_proto::ItemStack {
                    id: 0,
                    stack: 0,
                    prefix: 0,
                },
                favorited: false,
                blocked: false,
            },
        );

        assert_eq!(GameServer::equipped_items(&player), vec![42]);
    }
}
