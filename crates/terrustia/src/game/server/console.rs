//! Commands, from the server's own terminal and from chat.
//!
//! Three entry points share this file because they share one vocabulary: [`GameServer::run_console`]
//! for a line typed at the server's terminal, [`GameServer::run_command`] for a chat line beginning
//! with `/`, and [`GameServer::run_admin_command`] for the subset that both of those hand off to,
//! which is every command that needs its argument's case left alone.

use terrustia_proto::{NetworkText, happiness, net_module, npc_data::npc_stats};
use tracing::info;

use crate::{
    game::housing,
    world::world::{DAY_LENGTH, NIGHT_LENGTH},
};

use super::{AuthOutcome, GameServer, SERVER_CHAT_COLOUR};

impl GameServer {
    /// A line typed at the server's own terminal.
    ///
    /// Whoever has the console already has the world file, so it is not gated: there is nothing a
    /// permission could protect them from. Output goes to the log rather than to chat, because the
    /// person who typed it is looking at the log.
    pub(super) fn run_console(&mut self, line: &str) {
        // Every `info!` inside this function that names `target: CONSOLE_REPLY` is a command's
        // own reply, not an ordinary log line — `TermLayer` prints those the way a REPL prints
        // its own output: no timestamp, no level tag, no target column. Only the replies
        // textually inside this function are tagged; `save`, `backups`, `rollback` and the admin
        // commands delegate to shared functions used by other call paths too, and keep the
        // ordinary log formatting rather than risk retagging something a non-console caller
        // also relies on.
        use crate::term::CONSOLE_REPLY_TARGET as CONSOLE_REPLY;
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let (name, argument) = line.split_once(char::is_whitespace).unwrap_or((line, ""));

        match name.to_ascii_lowercase().as_str() {
            // Only exists in a test build. There has to be *some* way to make the packet path
            // panic on purpose, or the guard around it is only believed rather than checked —
            // and "we catch panics" is exactly the sort of claim that is never true until tried.
            #[cfg(test)]
            "__panic_probe" => panic!("deliberate panic, to prove the packet path is guarded"),
            // Claiming from the console needs no token: whoever can type here can already read
            // the world file, so there is nothing left to prove.
            //
            // `password` is never logged below, at any level, not even inside an error branch:
            // matching `admin::mod`'s own "never logged" convention. That still holds even though
            // this line only ever reaches a trusted terminal: `info!` goes through the same
            // `tracing` pipeline as everything else, and this project does not assume every
            // deployment leaves that pipeline pointed only at a screen nobody else ever reads.
            "claim" => {
                let mut words = argument.split_whitespace();
                match (words.next(), words.next(), words.next()) {
                    (Some(name), Some(password), None) => {
                        if !self.admin.unclaimed() {
                            info!(target: CONSOLE_REPLY, "this server already has an owner; use /register or /group");
                        } else if password.len() < 6 {
                            info!(target: CONSOLE_REPLY, "that password is too short; use at least six characters");
                        } else {
                            match crate::admin::Account::new(name, password, "owner") {
                                Ok(account) => match self.admin.insert_account(account) {
                                    Ok(()) => {
                                        let _ = self.admin.save();
                                        self.claim_token = None;
                                        self.audit.record(
                                            name,
                                            crate::admin::AuditAction::Claim,
                                            name,
                                            "claimed from the console",
                                        );
                                        info!(target: CONSOLE_REPLY, account = name, "server claimed from the console");
                                    }
                                    Err(e) => info!(target: CONSOLE_REPLY, "{e}"),
                                },
                                Err(e) => info!(target: CONSOLE_REPLY, "{e}"),
                            }
                        }
                    }
                    _ => info!(target: CONSOLE_REPLY, "usage: claim <name> <password>"),
                }
            }
            "help" => {
                // One command per line, aligned, rather than a single run-on that wraps hard in any
                // ordinary terminal. This is often the first thing an operator ever types.
                for line in [
                    "commands:",
                    "  say <text>                          broadcast a message",
                    "  players                             who is connected",
                    "  save                                write the world now",
                    "  backups                             list the rotating backups",
                    "  rollback <n>                        restore backup n",
                    "  whitelist add|remove|list [name]    manage the whitelist",
                    "  claim <name> <password>             claim an unclaimed server",
                    "  kick <name> [reason]                disconnect a player",
                    "  ban <name|ip|uuid> <value> [reason] ban by name, address or uuid",
                    "  unban <value>                       lift a ban",
                    "  mute <name> [duration] [reason]     mute a player, e.g. 10m, 2h, 1d",
                    "  unmute <value>                      lift a mute",
                    "  group <account> <group>             set an account's group",
                    "  world undo <player> <duration>      revert a player's recent tile edits",
                    "  audit [n]                           show the last n audit-log entries",
                    "  panel                               toggle the web panel",
                    "  stop                                save and shut down",
                ] {
                    info!(target: CONSOLE_REPLY, "{line}");
                }
            }
            // Toggles the web panel: starts it if it is not running, stops it if it is.
            // `panel_toggle`'s other end (`crate::panel::supervise`) owns the actual bind/abort and
            // decides which of those this pulse means — this arm only ever sends one and reports
            // whether it could.
            "panel" => match &self.panel_toggle {
                Some(toggle) if toggle.send(()).is_ok() => {
                    info!(target: CONSOLE_REPLY, "panel toggled — see the log line just above for which way");
                }
                Some(_) => info!(target: CONSOLE_REPLY, "the panel supervisor is gone"),
                None => info!(target: CONSOLE_REPLY, "no panel supervisor is wired up in this run"),
            },
            "say" => {
                self.announce(argument);
            }
            "players" => {
                let names: Vec<&str> = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.is_playing())
                    .map(|p| p.name.as_str())
                    .collect();
                info!(target: CONSOLE_REPLY, online = names.len(), "{}", names.join(", "));
            }
            "save" => self.save_world_in_background("console"),
            "backups" => self.list_backups(),
            "whitelist" => {
                let mut words = argument.split_whitespace();
                match (words.next(), words.next()) {
                    (Some("add"), Some(name)) => {
                        if self.admin.add_to_whitelist(name) {
                            let _ = self.admin.save();
                            self.audit.record(
                                "console",
                                crate::admin::AuditAction::Whitelist,
                                name,
                                "added",
                            );
                            info!(target: CONSOLE_REPLY, name, "added to the guest list");
                        } else {
                            info!(target: CONSOLE_REPLY, name, "already on the guest list");
                        }
                    }
                    (Some("remove"), Some(name)) => {
                        if self.admin.remove_from_whitelist(name) {
                            let _ = self.admin.save();
                            self.audit.record(
                                "console",
                                crate::admin::AuditAction::Whitelist,
                                name,
                                "removed",
                            );
                            info!(target: CONSOLE_REPLY, name, "removed from the guest list");
                            // Take effect now rather than at their next join.
                            if let Some(slot) = self.slot_named(name) {
                                self.kick(slot, "You are no longer on this server's guest list.");
                            }
                        } else {
                            info!(target: CONSOLE_REPLY, name, "was not on the guest list");
                        }
                    }
                    (Some("list"), _) | (None, _) => {
                        if self.admin.whitelist_on() {
                            info!(
                                target: CONSOLE_REPLY,
                                names = %self.admin.whitelist.join(", "),
                                "the guest list is on"
                            );
                        } else {
                            info!(
                                target: CONSOLE_REPLY,
                                "the guest list is empty, so anyone may join. \
                                 `whitelist add <name>` turns it on."
                            );
                        }
                    }
                    _ => info!(target: CONSOLE_REPLY, "usage: whitelist add|remove|list [name]"),
                }
            }
            "rollback" => {
                let which: usize = argument.trim().parse().unwrap_or(1);
                match self.roll_back(which) {
                    Ok(message) => info!(target: CONSOLE_REPLY, "{message}"),
                    Err(message) => info!(target: CONSOLE_REPLY, "{message}"),
                }
            }
            "audit" => {
                let n: usize = argument.trim().parse().unwrap_or(20);
                for line in self.format_audit_tail(n) {
                    info!(target: CONSOLE_REPLY, "{line}");
                }
            }
            "stop" => {
                info!("stopping on console request");
                self.stopping = true;
            }
            // The player-facing ones do the same thing here, reporting to the log. Slot 255 is
            // "the server", which `tell` already knows how to address.
            "kick" | "ban" | "unban" | "mute" | "unmute" | "group" | "world" => {
                let _ = self.run_admin_command(net_module::SERVER_AUTHOR, name, argument);
            }
            other => {
                info!(target: CONSOLE_REPLY, "console: unknown command {other:?} (try 'help')")
            }
        }
    }

    /// The commands about people rather than the world.
    ///
    /// Kept apart from the rest because they are the ones that need the argument's case intact —
    /// a lowercased password is a different password, and `run_command` lowercases everything for
    /// the benefit of NPC-name lookup.
    ///
    /// `password` and `token` below are never logged, at any level: see `admin::mod`'s own
    /// "never logged" convention. `argument` itself is not logged either, for the same reason: it
    /// is the raw `/login`, `/register` or `/group` (etc.) line, which is exactly where those
    /// values live before they are split apart.
    pub(super) fn run_admin_command(
        &mut self,
        slot: u8,
        name: &str,
        argument: &str,
    ) -> terrustia_proto::Result<()> {
        use crate::admin::BanKind;

        let words: Vec<&str> = argument.split_whitespace().collect();
        match name {
            // The first account owns the server, so making it needs the token printed at
            // startup — otherwise, on a fresh public server, whoever connected first became the
            // owner. Every account after that is ordinary and needs nothing.
            "register" if self.admin.unclaimed() => match words.as_slice() {
                [account, password, token] => {
                    // Constant-time: a plain `!=` here would compare a real one-time secret byte
                    // by byte, exiting the moment a mismatch is found: the classic timing side
                    // channel. `self.claim_token` being `None` (nothing left to claim, or nothing
                    // generated yet) always refuses, same as before; the compare only runs at all
                    // once there is a real token to compare against.
                    let token_ok = self.claim_token.as_deref().is_some_and(|expected| {
                        crate::admin::constant_time_eq(expected.as_bytes(), token.as_bytes())
                    });
                    if !token_ok {
                        self.tell(
                            slot,
                            "that is not the claim token from the server's console.",
                        );
                        info!(slot, "refused a claim with the wrong token");
                        return Ok(());
                    }
                    self.begin_registration(slot, account, password, true);
                }
                [_, _] => self.tell(
                    slot,
                    "this server has not been claimed yet, and claiming it needs the claim token \
                     printed in the server's own console: /register <name> <password> <token>",
                ),
                _ => self.tell(slot, "usage: /register <name> <password> <token>"),
            },
            "register" => match words.as_slice() {
                [account, password] => self.begin_registration(slot, account, password, false),
                _ => self.tell(slot, "usage: /register <name> <password>"),
            },
            "login" => match words.as_slice() {
                [account, password] => {
                    // Checked before anything else touches the account store or a worker thread:
                    // a throttled attempt must cost exactly nothing, not even the lookup below.
                    // See `login_throttled`'s own doc comment.
                    let ip_key = self.player(slot).map(|p| p.addr.ip().to_string());
                    let account_key = account.to_ascii_lowercase();
                    if self.login_throttled(slot, ip_key.as_deref(), &account_key) {
                        return Ok(());
                    }
                    // The hash is fetched here and compared on a worker thread. An account that
                    // does not exist still pays a hash, deliberately: answering instantly for an
                    // unknown name and slowly for a known one tells an attacker which is which.
                    let stored = self.admin.account_hash(account);
                    if self.start_auth(slot) {
                        let (account, password) = (account.to_string(), password.to_string());
                        let report = self.auth_results.0.clone();
                        tokio::task::spawn_blocking(move || {
                            let correct = match &stored {
                                Some(hash) => crate::admin::Account::verify_hash(hash, &password),
                                // No account: hash against a throwaway anyway, so the two cases
                                // take the same time.
                                None => {
                                    let _ = crate::admin::Account::new("", &password, "");
                                    false
                                }
                            };
                            let _ = report.send(AuthOutcome::SignedIn {
                                slot,
                                account,
                                correct,
                                ip_key,
                            });
                        });
                    }
                }
                _ => self.tell(slot, "usage: /login <name> <password>"),
            },
            "logout" => {
                self.admin.sign_out(slot);
                self.tell(slot, "signed out.");
            }
            "whoami" => {
                let who = self
                    .admin
                    .signed_in_as(slot)
                    .unwrap_or("nobody")
                    .to_string();
                let group = self.admin.group_of(slot).name.clone();
                self.tell(slot, &format!("you are {who}, in group '{group}'."));
            }
            "kick" => match words.split_first() {
                Some((who, rest)) => {
                    let reason = if rest.is_empty() {
                        "kicked".to_string()
                    } else {
                        rest.join(" ")
                    };
                    match self.slot_named(who) {
                        Some(target) => {
                            self.announce(&format!("{who} was kicked: {reason}"));
                            self.kick(target, &reason);
                            let issuer = self.audit_issuer(slot);
                            self.audit.record(
                                &issuer,
                                crate::admin::AuditAction::Kick,
                                who,
                                &reason,
                            );
                        }
                        None => self.tell(slot, &format!("nobody here is called {who}.")),
                    }
                }
                None => self.tell(slot, "usage: /kick <name> [reason]"),
            },
            "ban" => match words.split_first() {
                Some((kind, rest)) if !rest.is_empty() => {
                    let Some(kind) = BanKind::parse(kind) else {
                        self.tell(slot, "usage: /ban <name|ip|uuid> <value> [reason]");
                        return Ok(());
                    };
                    let value = rest[0].to_string();
                    let reason = if rest.len() > 1 {
                        rest[1..].join(" ")
                    } else {
                        "banned".to_string()
                    };
                    let issuer = self.audit_issuer(slot);
                    self.admin.ban(kind.clone(), &value, &reason, &issuer);
                    self.announce(&format!("{value} is banned: {reason}"));
                    // And remove them if they are standing here.
                    if let Some(target) = self.slot_named(&value) {
                        self.kick(target, &reason);
                    }
                    self.audit.record(
                        &issuer,
                        crate::admin::AuditAction::Ban,
                        &value,
                        &format!("{kind:?}: {reason}"),
                    );
                    info!(value, reason, "ban added");
                }
                _ => self.tell(slot, "usage: /ban <name|ip|uuid> <value> [reason]"),
            },
            "unban" => match words.as_slice() {
                [value] => {
                    let removed = self.admin.unban(value);
                    if removed > 0 {
                        let issuer = self.audit_issuer(slot);
                        self.audit
                            .record(&issuer, crate::admin::AuditAction::Unban, value, "");
                    }
                    self.tell(slot, &format!("{removed} ban(s) lifted for {value}."));
                }
                _ => self.tell(slot, "usage: /unban <value>"),
            },
            // `<duration>` is optional and, when present, must parse (`parse_duration`, the same
            // `10m`/`2h`/`1d` grammar `/world undo` uses) — everything after the name is otherwise
            // taken whole as the reason, so a duration-shaped word never accidentally swallows part
            // of one (`/mute chatty 500 for spamming` mutes for 500 seconds with reason "for
            // spamming", not with a reason that starts "500 for").
            "mute" => match words.split_first() {
                Some((who, rest)) => {
                    let (duration, reason) = match rest.split_first() {
                        Some((maybe_duration, reason_words)) => {
                            match crate::game::tile_log::parse_duration(maybe_duration) {
                                Some(d) => (Some(d), reason_words.join(" ")),
                                None => (None, rest.join(" ")),
                            }
                        }
                        None => (None, String::new()),
                    };
                    let reason = if reason.is_empty() {
                        "muted".to_string()
                    } else {
                        reason
                    };
                    let issuer = self.audit_issuer(slot);
                    self.admin
                        .mute(who, &reason, duration.map(|d| d.as_secs()), &issuer);
                    self.audit
                        .record(&issuer, crate::admin::AuditAction::Mute, who, &reason);
                    self.tell(slot, &format!("{who} is muted: {reason}"));
                    info!(who, reason, "mute added");
                }
                None => self.tell(slot, "usage: /mute <name> [duration] [reason]"),
            },
            "unmute" => match words.as_slice() {
                [value] => {
                    let removed = self.admin.unmute(value);
                    if removed {
                        let issuer = self.audit_issuer(slot);
                        self.audit
                            .record(&issuer, crate::admin::AuditAction::Unmute, value, "");
                    }
                    let message = if removed {
                        format!("{value} is unmuted.")
                    } else {
                        "that name was not muted.".to_string()
                    };
                    self.tell(slot, &message);
                }
                _ => self.tell(slot, "usage: /unmute <value>"),
            },
            "group" => match words.as_slice() {
                [account, group] => {
                    if !self.admin.groups.iter().any(|g| &g.name == group) {
                        self.tell(slot, &format!("there is no group called {group}."));
                        return Ok(());
                    }
                    // The console (slot 255, "the server") is unconditionally trusted — see this
                    // module's own doc comment — so this reach check only applies to a real
                    // player's own `/group`, where it is what actually stops an `admin.accounts`
                    // holder promoting themselves (or anyone else) into a group that holds more
                    // than they do, `owner` above all. See `Admin::group_within_reach`.
                    if slot != net_module::SERVER_AUTHOR {
                        let actor_group = self.admin.group_of(slot).name.clone();
                        if !self.admin.group_within_reach(&actor_group, group) {
                            self.tell(
                                slot,
                                &format!(
                                    "you cannot move anyone into '{group}': it holds permissions \
                                     you do not have yourself."
                                ),
                            );
                            return Ok(());
                        }
                    }
                    match self
                        .admin
                        .accounts
                        .iter_mut()
                        .find(|a| a.name.eq_ignore_ascii_case(account))
                    {
                        Some(found) => {
                            found.group = (*group).to_string();
                            let _ = self.admin.save();
                            let issuer = self.audit_issuer(slot);
                            self.audit.record(
                                &issuer,
                                crate::admin::AuditAction::GroupChange,
                                account,
                                &format!("-> {group}"),
                            );
                            self.tell(slot, &format!("{account} is now in {group}."));
                            info!(account, group, "group changed");
                        }
                        None => self.tell(slot, &format!("there is no account called {account}.")),
                    }
                }
                _ => self.tell(slot, "usage: /group <account> <group>"),
            },
            "world" => match words.as_slice() {
                &["undo", player, duration_text] => {
                    let Some(within) = crate::game::tile_log::parse_duration(duration_text) else {
                        self.tell(
                            slot,
                            "could not parse that duration — try something like 10m, 2h or 1d.",
                        );
                        return Ok(());
                    };
                    let reverted = self.tile_log.take_recent(player, within);
                    let count = reverted.len();
                    for (x, y, before) in reverted {
                        self.world.set_tile(x, y, before);
                        self.liquids.disturb(x, y);
                        self.broadcast_tile(x, y);
                    }
                    self.tell(
                        slot,
                        &format!(
                            "reverted {count} tile edit(s) by {player} from the last {duration_text}."
                        ),
                    );
                    info!(slot, player, duration_text, count, "world undo");
                }
                _ => self.tell(slot, "usage: /world undo <player> <duration>"),
            },
            _ => {}
        }
        Ok(())
    }

    /// Who to blame an audit-log entry on, for an action taken by `slot`: `"console"` for the
    /// server's own trusted terminal (slot 255, "the server" — see this module's own doc comment),
    /// the signed-in account name if there is one, or the connected player's own name as a
    /// fallback (an unclaimed server lets anyone act before anyone has registered at all, and that
    /// still deserves an attributable line rather than a blank one).
    fn audit_issuer(&self, slot: u8) -> String {
        if slot == net_module::SERVER_AUTHOR {
            return "console".to_string();
        }
        if let Some(account) = self.admin.signed_in_as(slot) {
            return account.to_string();
        }
        self.player(slot)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("slot {slot}"))
    }

    /// The last `n` audit-log entries, one per line, for the `audit` command (console and chat
    /// alike). Timestamps are raw Unix seconds — this workspace has no date/time-formatting
    /// dependency, and inventing one just for a log line is not worth it.
    fn format_audit_tail(&self, n: usize) -> Vec<String> {
        let events = self.audit.tail(n);
        if events.is_empty() {
            return vec!["no audit events recorded yet.".to_string()];
        }
        events
            .iter()
            .map(|e| {
                let detail = if e.detail.is_empty() { "-" } else { &e.detail };
                format!(
                    "{} [{}] {} {} -> {detail}",
                    e.when,
                    e.issuer,
                    e.action.as_str(),
                    e.target,
                )
            })
            .collect()
    }

    /// The slot of whoever is playing under this name.
    pub(super) fn slot_named(&self, name: &str) -> Option<u8> {
        self.players
            .iter()
            .flatten()
            .find(|p| p.is_playing() && p.name.eq_ignore_ascii_case(name))
            .map(|p| p.slot)
    }

    /// Send a line of server text to one player.
    pub(super) fn tell(&mut self, slot: u8, text: &str) {
        if let Ok(frame) = net_module::chat_broadcast(
            net_module::SERVER_AUTHOR,
            &NetworkText::literal(text),
            SERVER_CHAT_COLOUR,
        ) {
            self.send(slot, frame);
        }
    }

    /// Handle a chat line beginning with `/`.
    ///
    /// Commands are gated per-command against the namespaced vocabulary in `admin::group::perm` —
    /// see the table below. Self-service (`register`/`login`/`logout`/`whoami`) and the read-only
    /// "look" commands (`help`/`players`/`npcs`/`house`/`happy`/`where`) need no permission at all, matching
    /// the behaviour before this system existed (`Permission::Look`, its predecessor, never gated
    /// anything in this dispatcher either). Until somebody registers, the server is unclaimed and
    /// every check passes regardless — see `Admin::unclaimed`.
    pub(super) fn run_command(&mut self, slot: u8, command: &str) -> terrustia_proto::Result<()> {
        use crate::admin::perm;

        let mut parts = command.split_whitespace();
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        // The whole rest of the line, not the first word of it: `/spawn Eater of Worlds Head` has
        // to reach the resolver intact or it looks up "eater" and finds nothing.
        let argument = parts.collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        // And the same line with its case intact, because a password, an account name and a
        // player's name are all case-sensitive and the lowercased form silently corrupts them.
        let raw_argument = command
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");

        // What each command costs. Anything absent needs nothing beyond being here.
        let needed = match name.as_str() {
            "time" => Some(perm::WORLD_TIME),
            "save" => Some(perm::WORLD_SAVE),
            "spawn" => Some(perm::WORLD_SPAWN),
            "butcher" => Some(perm::WORLD_BUTCHER),
            "kick" => Some(perm::SERVER_KICK),
            "ban" => Some(perm::SERVER_BAN),
            "unban" => Some(perm::SERVER_UNBAN),
            "mute" => Some(perm::SERVER_MUTE),
            "unmute" => Some(perm::SERVER_UNMUTE),
            // `world undo <player> <duration>` — the only subcommand `world` has as a chat command.
            "world" => Some(perm::SERVER_UNDO),
            // Moving an account between groups, which is exactly the lever a self-escalation would
            // pull — see `Admin::group_within_reach`, checked again inside `run_admin_command`'s own
            // `"group"` arm on top of this.
            "group" => Some(perm::ADMIN_ACCOUNTS),
            "audit" => Some(perm::ADMIN_AUDIT),
            _ => None,
        };
        if let Some(permission) = needed
            && !self.admin.may(slot, permission)
        {
            // Named rather than vague: "you may not" invites a second attempt, and the point is to
            // tell somebody how to become allowed.
            self.tell(
                slot,
                &format!(
                    "/{name} needs the '{}' permission. Sign in with /login <name> <password>.",
                    permission.as_str()
                ),
            );
            return Ok(());
        }

        match name.as_str() {
            "register" | "login" | "logout" | "kick" | "ban" | "unban" | "mute" | "unmute"
            | "group" | "whoami" | "world" => {
                return self.run_admin_command(slot, &name, &raw_argument);
            }
            "help" => {
                for line in [
                    "/help            this list",
                    "/players         who is online",
                    "/time <day|noon|night|midnight>",
                    "/save            write the world to disk",
                    "/where           your position and section",
                    "/spawn <npc>     spawn an NPC beside you",
                    "/npcs            what is alive right now",
                    "/butcher         remove every hostile NPC",
                    "/house           is the room you are standing in a valid house?",
                    "/happy           what the resident you are talking to thinks of the place",
                    "/register <name> <password>   make an account",
                    "/login <name> <password>      sign in",
                    "/logout          give up whatever you signed in for",
                    "/whoami          who the server thinks you are",
                    "/kick <name> [reason]",
                    "/ban <name|uuid|ip> <value> [reason]",
                    "/unban <value>",
                    "/mute <name> [duration] [reason]   e.g. 10m, 2h, 1d",
                    "/unmute <value>",
                    "/group <account> <group>      move somebody between groups",
                    "/world undo <player> <duration>   revert their tile edits from the last",
                    "                                   <duration> (e.g. 10m, 2h) — up to 72h back",
                    "/audit [n]       the last n audit-log entries",
                ] {
                    self.tell(slot, line);
                }
            }
            "audit" => {
                let n: usize = argument.trim().parse().unwrap_or(20);
                for line in self.format_audit_tail(n) {
                    self.tell(slot, &line);
                }
            }
            "players" => {
                let names: Vec<String> = self
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.is_playing())
                    .map(|p| p.name.clone())
                    .collect();
                let line = format!(
                    "{} of {} online: {}",
                    names.len(),
                    self.config.max_players,
                    if names.is_empty() {
                        "nobody".to_string()
                    } else {
                        names.join(", ")
                    }
                );
                self.tell(slot, &line);
            }
            "time" => {
                let set = match argument.as_str() {
                    "day" => Some((true, 0)),
                    "noon" => Some((true, DAY_LENGTH / 2)),
                    "night" => Some((false, 0)),
                    "midnight" => Some((false, NIGHT_LENGTH / 2)),
                    _ => None,
                };
                match set {
                    Some((day_time, time)) => {
                        self.set_time(day_time, time);
                        self.announce(&format!("Time set to {argument}."));
                    }
                    None => self.tell(slot, "usage: /time <day|noon|night|midnight>"),
                }
            }
            "save" => {
                if self.save_path.is_none() {
                    self.tell(
                        slot,
                        "This world has nowhere to be saved: start the server with --save <path>.",
                    );
                } else {
                    self.save_world_in_background("command");
                }
            }
            "spawn" => {
                // Accepts an id or an NPCID name, so `/spawn Zombie` and `/spawn 3` both work.
                let Some(npc_type) = resolve_npc(&argument) else {
                    self.tell(slot, "usage: /spawn <npc id or name>, e.g. /spawn Zombie");
                    return Ok(());
                };
                let Some(position) = self.player(slot).map(|p| p.position) else {
                    return Ok(());
                };
                // Drop it a little to the side so it does not appear inside the player.
                let at = (position.0 + 64.0, position.1 - 32.0);
                // Worm heads come with a body: spawning a bare head would be a floating face.
                let spawned = match self.worm_parts(npc_type) {
                    Some((body, tail, segments)) => {
                        self.npcs.spawn_worm(npc_type, body, tail, segments, at)
                    }
                    None => self.npcs.spawn(npc_type, at),
                };
                match spawned {
                    Some(index) => {
                        let name = npc_stats(npc_type).map(|s| s.name).unwrap_or("?");
                        self.broadcast_npc(index);
                        self.tell(slot, &format!("spawned {name} ({npc_type}) as npc {index}"));
                    }
                    None => self.tell(slot, "no free NPC slots"),
                }
            }
            "npcs" => {
                let total = self.npcs.len();
                let mut summary: Vec<String> = Vec::new();
                let mut counts: std::collections::BTreeMap<&str, usize> =
                    std::collections::BTreeMap::new();
                for (_, npc) in self.npcs.iter() {
                    *counts.entry(npc.stats.name).or_default() += 1;
                }
                for (name, n) in counts.iter().take(8) {
                    summary.push(format!("{name} x{n}"));
                }
                let line = format!(
                    "{total} NPCs ({:.1} spawn slots): {}",
                    self.npcs.used_slots(),
                    if summary.is_empty() {
                        "none".to_string()
                    } else {
                        summary.join(", ")
                    }
                );
                self.tell(slot, &line);
                if self.shots_thrown > 0 {
                    self.tell(
                        slot,
                        &format!(
                            "{} in flight, {} thrown since the server started",
                            self.projectiles.len(),
                            self.shots_thrown
                        ),
                    );
                }
            }
            "butcher" => {
                // Clear every hostile NPC, leaving town residents alone.
                let doomed: Vec<u8> = self
                    .npcs
                    .iter()
                    .filter(|(_, npc)| !npc.stats.town_npc)
                    .map(|(index, _)| index)
                    .collect();
                let killed = doomed.len();
                for index in doomed {
                    self.npcs.remove(index);
                    self.broadcast_npc_death(index);
                }
                self.announce(&format!("Butchered {killed} NPCs."));
            }
            "house" => {
                // Report on the room the player is standing in, with the reason if it is no good.
                let Some(position) = self.player(slot).map(|p| p.position) else {
                    return Ok(());
                };
                let (x, y) = ((position.0 / 16.0) as i32, (position.1 / 16.0) as i32);
                let line = match housing::check_room(&self.world, x, y) {
                    Ok(room) => format!(
                        "valid house: {} tiles, {}x{}",
                        room.tiles.len(),
                        room.right - room.left + 1,
                        room.bottom - room.top + 1
                    ),
                    Err(reason) => format!("not a house: {}", reason.describe()),
                };
                self.tell(slot, &line);
            }
            "happy" => {
                // What the server makes of the resident this player has open. The number the
                // *client* charges is its own (see `terrustia_proto::happiness`); this is here so
                // the two can be compared, which is the only way to tell they agree.
                let line = match self.player(slot).map(|p| (p.talking_to, p.shop_multiplier)) {
                    Some((Some(index), multiplier)) => {
                        let name = self
                            .npcs
                            .get(index)
                            .and_then(|npc| npc_stats(npc.npc_type))
                            .map_or("?", |stats| stats.name);
                        let mood = if multiplier <= happiness::MAX_HAPPINESS_MULTIPLIER {
                            "delighted"
                        } else if multiplier < 1.0 {
                            "content"
                        } else if multiplier >= happiness::HIGHEST_MULTIPLIER {
                            "furious"
                        } else {
                            "unimpressed"
                        };
                        format!("{name} is {mood}: prices x{multiplier:.2}")
                    }
                    _ => "you are not talking to anybody".to_string(),
                };
                self.tell(slot, &line);
            }
            "where" => {
                let line = self.player(slot).map(|p| {
                    let (tx, ty) = ((p.position.0 / 16.0) as i32, (p.position.1 / 16.0) as i32);
                    let (sx, sy) = self.world.section_of(tx, ty);
                    format!("tile ({tx}, {ty}) in section ({sx}, {sy})")
                });
                match line {
                    Some(line) => self.tell(slot, &line),
                    None => self.tell(slot, "unknown position"),
                }
            }
            other => self.tell(slot, &format!("unknown command: /{other}  (try /help)")),
        }
        Ok(())
    }
}

/// Look up an NPC by numeric id or by its `NPCID` name, case-insensitively.
fn resolve_npc(argument: &str) -> Option<u16> {
    if argument.is_empty() {
        return None;
    }
    if let Ok(id) = argument.parse::<u16>() {
        return npc_stats(id).is_some().then_some(id);
    }
    // The names come from `NPCID`, where they are run together: `EaterofWorldsBody`. Nobody types
    // that, so spaces and punctuation are ignored on both sides and the answer is the same
    // whether you write "Eater of Worlds Body", "eaterofworldsbody" or "Eater-of-Worlds-Body".
    let squashed = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let wanted = squashed(argument);
    if wanted.is_empty() {
        return None;
    }
    (0..terrustia_proto::npc_data::NPC_COUNT)
        .find(|id| npc_stats(*id).is_some_and(|s| squashed(s.name) == wanted))
}

/// The console's `panel` command sends exactly one pulse on `panel_toggle` — the other end
/// (`crate::panel::supervise`) decides what that pulse means and actually owns the bind/abort.
/// This only proves the command reaches the channel and never panics without one wired; the real
/// start/stop behaviour is covered end-to-end, over a real socket, in `tests/panel.rs`.
#[cfg(test)]
mod panel_toggle_command {
    use super::*;
    use crate::config::Config;
    use crate::game::server::{ServerEvent, Stopped};
    use tokio::sync::mpsc;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "panel toggle probe")
    }

    #[tokio::test]
    async fn the_panel_command_sends_one_pulse_when_a_toggle_channel_is_wired() {
        let (tx, rx) = mpsc::channel::<ServerEvent>(4);
        let (toggle_tx, mut toggle_rx) = mpsc::unbounded_channel();
        let server = GameServer::new(Config::default(), tiny_world()).with_panel_toggle(toggle_tx);
        let handle = tokio::spawn(server.run(rx));

        tx.send(ServerEvent::Console {
            line: "panel".into(),
        })
        .await
        .unwrap();
        drop(tx);
        assert_eq!(handle.await.unwrap(), Stopped::Cleanly);

        assert!(
            toggle_rx.try_recv().is_ok(),
            "the console command should have sent exactly one pulse"
        );
        assert!(
            toggle_rx.try_recv().is_err(),
            "and only one — not, say, one per tick"
        );
    }

    /// Every test that constructs a `GameServer` directly (all seventeen call sites, before this
    /// one) never calls `with_panel_toggle` — the command has to stay harmless there, not panic.
    #[tokio::test]
    async fn the_panel_command_does_not_panic_with_no_toggle_channel_wired() {
        let (tx, rx) = mpsc::channel::<ServerEvent>(4);
        let server = GameServer::new(Config::default(), tiny_world());
        let handle = tokio::spawn(server.run(rx));

        tx.send(ServerEvent::Console {
            line: "panel".into(),
        })
        .await
        .unwrap();
        drop(tx);
        assert_eq!(handle.await.unwrap(), Stopped::Cleanly);
    }
}

#[cfg(test)]
mod console_whitelist_audit {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "console whitelist audit probe")
    }

    /// A `GameServer` with a real, file-backed audit log rather than the in-memory one
    /// `Config::default()` gives: an in-memory `AuditLog` records nothing at all
    /// (`AuditLog::in_memory`'s own doc comment), which would make "an audit line was written"
    /// untestable rather than merely false.
    fn server_with_real_admin_files(name: &str) -> GameServer {
        let dir = crate::safe_write::tests::temp_dir(name);
        let config = Config {
            save_file: Some(dir.join("world.wld")),
            ..Config::default()
        };
        GameServer::new(config, tiny_world())
    }

    /// L6-05: `whitelist add|remove` typed at the server's own console used to change the guest
    /// list with no audit trail at all, unlike every other console moderation command (`kick`,
    /// `ban`, `mute`, `group` all record one via `run_admin_command`). Fail-then-pass: before the
    /// `whitelist` arm in `run_console` called `self.audit.record`, `server.audit.tail(10)` after
    /// this sequence was empty.
    #[test]
    fn whitelist_add_and_remove_from_the_console_are_audited() {
        let mut server = server_with_real_admin_files("console-whitelist-audit");

        server.run_console("whitelist add Brooklyn");
        server.run_console("whitelist remove Brooklyn");

        let tail = server.audit.tail(10);
        assert_eq!(
            tail.len(),
            2,
            "both the add and the remove must be audited: {tail:?}"
        );
        assert_eq!(tail[0].issuer, "console");
        assert_eq!(tail[0].target, "Brooklyn");
        assert_eq!(tail[0].action, crate::admin::AuditAction::Whitelist);
        assert_eq!(tail[0].detail, "added");
        assert_eq!(tail[1].action, crate::admin::AuditAction::Whitelist);
        assert_eq!(tail[1].detail, "removed");
    }
}
