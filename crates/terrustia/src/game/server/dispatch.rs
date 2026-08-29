//! What a connected client can ask for.
//!
//! [`GameServer::handle_packet`] is the one match every byte from an untrusted socket passes
//! through; everything below it is one packet id's handler, plus the few helpers that exist only to
//! serve them (the join handshake, the section stream, and the presence frames a new arrival needs).
//! Nothing here is trusted: a handler validates first and gives up quietly rather than panicking,
//! because the loop that calls it owns the world.

// The parent module's prelude, wholesale, rather than a copy of it. Sixty-odd packet handlers
// between them name most of what `server/mod.rs` imports plus about twenty of its own private
// constants and helpers, and restating all of that here would be a second list to keep in step
// with the first. The smaller siblings (`console`, `panel`, `tick`) each name what they use.
use super::*;

impl GameServer {
    // ---------------------------------------------------------------- packets

    pub(super) fn handle_packet(&mut self, slot: u8, frame: Frame) {
        let payload = frame.payload;
        let result = match frame.id {
            id::HELLO => self.on_hello(slot, &payload),
            id::SYNC_PLAYER => self.on_sync_player(slot, &payload),
            id::SYNC_EQUIPMENT => self.on_equipment(slot, &payload),
            id::SPAWN_BOSS_USE_LICENSE_START_EVENT => self.on_summon(slot, &payload),
            id::REQUEST_WORLD_DATA => self.on_request_world_data(slot),
            id::SPAWN_TILE_DATA => self.on_spawn_tile_data(slot, &payload),
            id::PLAYER_SPAWN => self.on_player_spawn(slot, &payload),
            id::PLAYER_CONTROLS => self.on_player_controls(slot, &payload),
            id::PLAYER_LIFE_MANA => self.on_health(slot, &payload),
            id::PLAYER_MANA => self.on_mana(slot, &payload),
            id::CLIENT_UUID => self.on_uuid(slot, &payload),
            id::SEND_PASSWORD => self.on_password(slot, &payload),
            id::TEAM_CHANGE | id::TEAM_CHANGE_FROM_U_I => self.on_team(slot, &payload),
            id::TOGGLE_P_V_P => self.on_pvp(slot, &payload),
            id::ADD_PLAYER_BUFF_PV_P => self.on_pvp_buff_spread(slot, &payload),
            id::PLAYER_BUFFS => self.on_buffs(slot, &payload),
            id::SYNC_PLAYER_ZONE => self.on_zone(slot, &payload),
            // Damage and death are not simulated; relaying keeps every client's view of another
            // player's health and death messages consistent with the client that took the hit.
            id::PLAYER_HURT_V2 | id::PLAYER_DEATH_V2 | id::DEAD_PLAYER => {
                self.relay_player_packet(slot, frame.id, &payload)
            }
            // Everything a player does that only other clients need to be told about: a heal, a
            // mana burst, the angle of the item they are holding, a ninja dodge, their stealth, a
            // flute note, which NPC their minions are on, which tile they are mining, and which
            // loadout they just switched to. Each is the same shape — the owner byte is rewritten
            // to the connection it arrived on, so nobody can act as anybody else — and none of
            // them is something the server has an opinion about.
            id::PLAYER_HEAL
            | id::ITEM_ROTATION_AND_ANIMATION
            | id::MANA_EFFECT
            | id::INSTRUMENT_SOUND
            | id::SYNC_DODGE
            | id::PLAYER_STEALTH
            | id::MINION_ATTACK_TARGET_UPDATE
            | id::SYNC_TILE_PICKING
            | id::SYNC_LOADOUT => self.relay_player_packet(slot, frame.id, &payload),
            id::CRYSTAL_INVASION_START => self.on_crystal_placed(slot, &payload),
            id::SYNC_TILE_PAINT_OR_COATING | id::SYNC_WALL_PAINT_OR_COATING => {
                self.on_paint(slot, frame.id, &payload)
            }
            id::MISC_DATA_SYNC => self.on_misc_data(slot, &payload),
            id::LOCK_AND_UNLOCK => self.on_lock(slot, &payload),
            id::CHEST_UPDATES => self.on_chest_update(slot, &payload),
            id::TILE_ENTITY_PLACEMENT => self.on_tile_entity_placed(slot, &payload),
            id::HIT_SWITCH => self.on_hit_switch(slot, &payload),
            id::NPC_HOME => self.on_npc_home(slot, &payload),
            id::BUG_CATCHING => self.on_bug_caught(slot, &payload),
            id::BUG_RELEASING => self.on_bug_released(slot, &payload),
            id::LIQUID_UPDATE => self.on_liquid(slot, &payload),
            // Social chatter and cosmetic effects: nothing to keep, but everyone else has to see
            // it or the world looks different from each side.
            id::SYNC_EMOTE_BUBBLE
            | id::EMOJI
            | id::TOGGLE_PARTY
            | id::PING
            | id::SPECIAL_F_X
            | id::ITEM_USE_SOUND
            | id::MINION_REST_TARGET_UPDATE
            | id::SYNC_PROJECTILE_TRACKERS
            | id::UPDATE_PLAYER_LUCK_FACTORS
            | id::SYNC_REVENGE_MARKER
            | id::REMOVE_REVENGE_MARKER
            | id::LAND_GOLF_BALL_IN_CUP
            | id::COMBAT_TEXT_INT
            // Effects nobody but the sender would otherwise see: a temporary animation, a puff
            // of smoke, a legacy sound, a wired cannon firing, an NPC being interfered with, and
            // the two achievement announcements.
            | id::TEMPORARY_ANIMATION
            | id::POOF_OF_SMOKE
            | id::PLAY_LEGACY_SOUND
            | id::WIRED_CANNON_SHOT
            | id::TAMPER_WITH_N_P_C
            | id::ACHIEVEMENT_MESSAGE_N_P_C_KILLED
            | id::ACHIEVEMENT_MESSAGE_EVENT_HAPPENED
            | id::COMBAT_TEXT_STRING => {
                if self.player(slot).is_some_and(Player::is_playing)
                    && let Ok(relayed) = packets::verbatim(frame.id, &payload)
                {
                    self.broadcast(relayed, Some(slot));
                }
                Ok(())
            }
            id::PLACE_OBJECT => self.on_place_object(slot, &payload),
            id::TELEPORT_ENTITY => self.on_teleport(slot, &payload),
            // Which town NPC a player is talking to. The owner byte is first, so the ordinary
            // relay handles it, and remembering it is what a shop will need.
            id::SYNC_TALK_N_P_C => self.on_talk_npc(slot, &payload),
            id::REQUEST_SECTION => self.on_request_section(slot, &payload),
            id::AREA_TILE_CHANGE => self.on_tile_square(slot, &payload),
            id::SYNC_ITEM | id::SPAWN_INSTANCED_ITEM => self.on_sync_item(slot, &payload),
            id::SYNC_ITEM_DESPAWN => self.on_item_despawn(slot, &payload),
            id::DAMAGE_N_P_C => self.on_damage_npc(slot, &payload),
            id::TOGGLE_DOOR_STATE => self.on_door(slot, &payload),
            id::REQUEST_CHEST_OPEN => self.on_chest_open(slot, &payload),
            id::SYNC_CHEST_ITEM => self.on_chest_item(slot, &payload),
            id::SYNC_PLAYER_CHEST => self.on_player_chest(slot, &payload),
            id::OPEN_SIGN_REQUEST => self.on_sign_request(slot, &payload),
            id::OPEN_SIGN_RESPONSE => self.on_sign_write(slot, &payload),
            id::TILE_MANIPULATION => self.on_tile_manipulation(slot, &payload),
            id::NET_MODULES => self.on_net_module(slot, &payload),
            id::SYNC_PROJECTILE => self.on_client_projectile(slot, &payload),
            id::KILL_PROJECTILE => self.on_client_projectile_kill(slot, &payload),
            // Four different ids for one message: putting an item into a frame, onto a weapon
            // rack, onto a food platter, or into a display jar.
            id::ITEM_FRAME_TRY_PLACING
            | id::WEAPONS_RACK_TRY_PLACING
            | id::FOOD_PLATTER_TRY_PLACING
            | id::DEAD_CELLS_DISPLAY_JAR_TRY_PLACING => self.on_display_item(slot, &payload),
            id::T_E_LEASHED_ENTITY_ANCHOR_PLACE_ITEM => self.on_anchor_item(slot, &payload),
            id::REQUEST_TELEPORTATION_BY_SERVER => self.on_server_teleport(slot, &payload),
            id::QUICK_STACK_CHESTS => self.on_quick_stack(slot, &payload),
            id::FISH_OUT_N_P_C => self.on_fished_out_npc(slot, &payload),
            id::SET_MISC_EVENT_VALUES => self.on_misc_event_value(slot, &payload),
            id::REQUEST_LUCY_POPUP => self.on_lucy_popup(slot, &payload),
            // Chat a client asks the server to put in front of everybody: a sign read aloud, a
            // tombstone's epitaph. Relayed rather than modelled, but relayed *to everybody*,
            // which is the part that was missing.
            id::SMART_TEXT_MESSAGE => {
                if self.player(slot).is_some_and(Player::is_playing)
                    && let Ok(relayed) = packets::verbatim(frame.id, &payload)
                {
                    self.broadcast(relayed, Some(slot));
                }
                Ok(())
            }
            id::RELEASE_ITEM_OWNERSHIP => self.on_release_item(slot, &payload),
            id::MURDER_SOMEONE_ELSES_PORTAL => self.on_close_portal(slot, &payload),
            id::TELEPORT_PLAYER_THROUGH_PORTAL => self.on_portal_teleport(slot, &payload),
            id::NEBULA_LEVELUP_REQUEST => self.on_nebula_booster(slot, &payload),
            id::SYNC_EXTRA_VALUE => self.on_extra_value(slot, &payload),
            id::CRYSTAL_INVASION_REQUESTED_TO_SKIP_WAIT_TIME => self.on_skip_army_wait(slot),
            id::REQUEST_QUEST_EFFECT => self.on_quest_effect(slot),
            id::MASS_WIRE_OPERATION => self.on_mass_wire(slot, &payload),
            id::CHEST_NAME => self.on_chest_name_request(slot, &payload),
            id::GEM_LOCK_TOGGLE => self.on_gem_lock(slot, &payload),
            id::ANGLER_QUEST_FINISHED => self.on_angler_finished(slot),
            id::QUESTS_COUNT_SYNC => self.on_quest_count(slot, &payload),
            id::T_E_DISPLAY_DOLL_DATA_SYNC => self.on_display_doll_slot(slot, &payload),
            id::T_E_HAT_RACK_ITEM_SYNC => self.on_hat_rack_slot(slot, &payload),
            id::REQUEST_TILE_ENTITY_INTERACTION => {
                self.on_tile_entity_interaction(slot, &payload)
            }
            id::ADD_N_P_C_BUFF => self.on_add_npc_buff(slot, &payload),
            id::REQUEST_N_P_C_BUFF_REMOVAL => self.on_remove_npc_buff(slot, &payload),
            id::UNIQUE_TOWN_N_P_C_INFO_SYNC_REQUEST => self.on_town_npc_name_request(slot, &payload),
            other => {
                debug!(slot, id = other, name = id::name(other), "ignoring packet");
                Ok(())
            }
        };

        if let Err(e) = result {
            debug!(slot, id = frame.id, name = id::name(frame.id), error = %e, "malformed packet");
        }
    }

    fn on_hello(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if self.player(slot).map(|p| p.state) != Some(ConnState::Greeting) {
            return Ok(()); // a second hello is not a way to restart the handshake
        }

        let hello = Hello::decode(payload)?;
        if !hello.is_supported() {
            // Name both sides. A refusal that says only what the server speaks leaves the person
            // on the other end guessing which of the two needs updating — and this exact check
            // once refused *every* current client, because it matched the string "Terraria325"
            // while the installed game announced 326.
            info!(slot, version = %hello.version, "rejecting unsupported client");
            self.kick(
                slot,
                &format!(
                    "Your client speaks {}; this server speaks {}. \
                     Whichever is older needs updating.",
                    hello.version,
                    id::SUPPORTED_RELEASES
                        .iter()
                        .map(|r| format!("Terraria{r}"))
                        .collect::<Vec<_>>()
                        .join(" and "),
                ),
            );
            return Ok(());
        }

        if let Some(player) = self.player_mut(slot) {
            player.greeted = true;
        }

        // With a password set, the slot is withheld until the client proves it knows it.
        if !self.config.password.is_empty() {
            self.send(slot, packets::empty(id::REQUEST_PASSWORD)?);
            return Ok(());
        }

        self.accept_player(slot)
    }

    /// Assign the slot and let the client proceed.
    fn accept_player(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        if let Some(player) = self.player_mut(slot) {
            player.password_ok = true;
            player.advance_to(ConnState::SlotAssigned);
        }
        // The trailing bool is new in 1.4.5; see docs/protocol-notes.md.
        let frame = packets::player_info(slot, false)?;
        self.send(slot, frame);
        Ok(())
    }

    /// Packet 38: the client's answer to a password prompt.
    fn on_password(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        // Only a connection that has already passed the version check may offer a password.
        let ready = self
            .player(slot)
            .is_some_and(|p| p.greeted && p.state == ConnState::Greeting);
        if self.config.password.is_empty() || !ready {
            return Ok(());
        }
        let offered = PacketReader::new(payload).string()?;
        if constant_time_eq(offered.as_bytes(), self.config.password.as_bytes()) {
            self.accept_player(slot)
        } else {
            info!(slot, "wrong password");
            self.kick(slot, "Incorrect password.");
            Ok(())
        }
    }

    /// Relay a packet that describes the sender, stamping our slot over whatever they claimed.
    fn relay_player_packet(
        &mut self,
        slot: u8,
        message: u8,
        payload: &[u8],
    ) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let frame = packets::rewrite_owner(message, payload, slot)?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    fn on_sync_player(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        // The name sits after slot, skin variant, voice variant, voice pitch and hair.
        let mut r = PacketReader::new(payload);
        r.bytes(1 + 1 + 1)?;
        r.f32()?;
        r.u8()?;
        let name = r.string()?;

        // Two players sharing a name is not merely confusing. `angler_finished_today` is keyed by
        // name on purpose, so a duplicate shares one daily reward with the original and either can
        // shed the cooldown by renaming. Refuse the collision at the door instead.
        let wanted = name.trim().to_string();
        if !wanted.is_empty()
            && self
                .players
                .iter()
                .flatten()
                .any(|p| p.slot != slot && p.name.eq_ignore_ascii_case(&wanted))
        {
            info!(slot, name = %wanted, "rejecting a duplicate name");
            self.kick(slot, "Someone is already playing under that name.");
            return Ok(());
        }

        if let Some(player) = self.player_mut(slot) {
            if !wanted.is_empty() {
                player.name = wanted;
            }
            player.appearance = Some(Bytes::copy_from_slice(payload));
            player.advance_to(ConnState::Identified);
        }

        // The name is known now, so a name or address ban can be enforced before the world is
        // sent. A UUID ban has to wait for packet 68, which arrives later.
        self.enforce_ban(slot);
        if self.player(slot).is_none() {
            return Ok(());
        }

        // Relay live appearance changes; a first-time sync reaches others at spawn instead.
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::SYNC_PLAYER, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_request_world_data(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let frame = self.world_data().encode()?;
        self.send(slot, frame);
        if let Some(player) = self.player_mut(slot) {
            player.advance_to(ConnState::WorldSent);
        }
        Ok(())
    }

    fn on_spawn_tile_data(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let request = SpawnTileData::decode(payload)?;

        // Vanilla re-sends world data here before the tiles; mirroring it keeps the client's
        // loading sequence identical to the one it was written against.
        let world_data = self.world_data().encode()?;
        self.send(slot, world_data);

        let sections = self.sections_for(request);
        // The key rather than the English, which is what vanilla sends here (`Lang.inter[44]`):
        // a literal would put "Receiving tile data" on the loading screen of a client playing in
        // any other language.
        let status = packets::status_text(
            sections.len() as i32,
            &NetworkText::key("LegacyInterface.44", Vec::new()),
            0,
        )?;
        self.send(slot, status);

        // Queued and drained a few at a time off the tick (`drain_section_streams`) rather than
        // sent in one synchronous loop here: a first join can want up to ~39 sections, and this
        // packet handler runs inline on the single-writer game task like everything else — sending
        // them all in one call blocked every other player's tick for the whole burst.
        let empty = sections.is_empty();
        if let Some(player) = self.player_mut(slot) {
            player.pending_sections = VecDeque::from(sections);
        }
        if empty {
            self.finish_join_stream(slot);
        }
        Ok(())
    }

    /// The rest of a client's initial world stream, run once every section it asked for has
    /// actually gone out: the entities that were alive when it joined, and the packet that tells
    /// the client the tile stream itself is done.
    ///
    /// Split out of `on_spawn_tile_data` so `drain_section_streams` can call it too, the moment a
    /// player's own queue empties on whichever tick that happens to land on.
    fn finish_join_stream(&mut self, slot: u8) {
        // Vanilla sends the live entities after the tiles and before StartPlaying; without this a
        // joining player sees an empty world where everyone else sees dropped loot.
        let existing: Vec<(i16, (f32, f32), ItemStack)> = self
            .items
            .iter()
            .map(|(index, item)| (index, item.position, item.item))
            .collect();
        for (index, position, stack) in existing {
            match SyncItem::dropped(index, position, stack).encode() {
                Ok(frame) => self.send(slot, frame),
                Err(e) => {
                    warn!(slot, error = %e, "could not encode a dropped item for a joining player");
                    return;
                }
            }
        }

        if let Err(e) = self.send_npcs(slot) {
            warn!(slot, error = %e, "could not send npcs to a joining player");
            return;
        }

        if let Some(player) = self.player_mut(slot) {
            player.advance_to(ConnState::TilesSent);
        }
        match packets::empty(id::INITIAL_SPAWN) {
            Ok(frame) => self.send(slot, frame),
            Err(e) => warn!(slot, error = %e, "could not encode InitialSpawn"),
        }
    }

    /// Advance every player's own queued initial-join section stream by a bounded slice of a
    /// tick's budget.
    ///
    /// This used to be a single synchronous loop inside `on_spawn_tile_data`'s own packet handler
    /// — up to ~39 sections, each up to ~3ms on the largest world this project benchmarks, so one
    /// player joining could block every other player's tick for 15–115ms straight. Running it here
    /// instead spreads the same total work across many ticks, a few sections at a time, the same
    /// way every other per-tick system already shares the budget.
    ///
    /// The budget is shared across every player still streaming, not given to each one
    /// separately — a per-player budget would let a burst of simultaneous joins reproduce the
    /// exact stall this exists to fix, just triggered by many joiners at once instead of one.
    /// Slots are drained in ascending order each tick, so under a mass-simultaneous-join burst the
    /// earlier-numbered ones finish loading first rather than everyone progressing in lockstep — a
    /// disclosed ordering bias, not a fairness guarantee, since the problem this fixes (one join
    /// stalling everyone already playing) does not depend on joiners being served evenly.
    pub(super) fn drain_section_streams(&mut self) {
        let slots: Vec<u8> = self
            .players
            .iter()
            .flatten()
            .filter(|p| !p.pending_sections.is_empty())
            .map(|p| p.slot)
            .collect();
        let began = Instant::now();
        for slot in slots {
            while let Some((sx, sy)) = self
                .player_mut(slot)
                .and_then(|p| p.pending_sections.pop_front())
            {
                let _ = self.send_section(slot, sx, sy);
                let drained = self
                    .player(slot)
                    .is_some_and(|p| p.pending_sections.is_empty());
                if drained {
                    self.finish_join_stream(slot);
                    break;
                }
                if began.elapsed() >= SECTION_STREAM_BUDGET {
                    return;
                }
            }
        }
    }

    /// Stream one section, unless this client already has it.
    fn send_section(&mut self, slot: u8, sx: i32, sy: i32) -> terrustia_proto::Result<()> {
        if sx < 0 || sy < 0 || sx >= self.world.sections_x() || sy >= self.world.sections_y() {
            return Ok(());
        }
        // Membership is only *checked* here, and claimed further down once the bytes exist.
        // Claiming it up front meant a section that failed to encode was marked delivered anyway,
        // and every re-request for it was then dropped by this same dedupe — leaving a 200x150
        // hole of sky that no amount of walking back through would fill in.
        if self
            .player(slot)
            .is_none_or(|player| player.sent_sections.contains(&(sx, sy)))
        {
            return Ok(());
        }

        let bounds = self.world.section_bounds(sx, sy);
        if bounds.width == 0 || bounds.height == 0 {
            return Ok(());
        }
        self.flush_dirty_sections();
        let frame = match self.section_cache.get(&(sx, sy)) {
            Some(cached) => cached.clone(),
            None => {
                let extras = self.world.extras_for(bounds);
                let encoded =
                    match encode_section_packet(bounds, &extras, |x, y| self.world.tile(x, y)) {
                        Ok(bytes) => Bytes::from(bytes),
                        Err(e) => {
                            // Loud, because the symptom is a missing piece of world rather than
                            // anything that looks like an error to whoever is playing.
                            warn!(slot, sx, sy, error = %e, "could not encode a section");
                            return Err(e);
                        }
                    };
                self.section_cache.insert((sx, sy), encoded.clone());
                encoded
            }
        };
        if let Some(player) = self.player_mut(slot) {
            player.sent_sections.insert((sx, sy));
        }
        self.send_bytes(slot, frame);
        self.send_chest_contents_for_section(slot, bounds)?;
        Ok(())
    }

    /// Send the contents of every chest inside a section that has just gone out.
    ///
    /// The section itself only announces each chest's id, position and name — enough to draw it,
    /// and nothing more. The game follows every section with the contents as well
    /// (`NetMessage.SyncChestContentsForSection`), and the client needs them for the things it
    /// does without opening a chest: crafting from what is nearby, quick-stacking into it, and the
    /// item search. Without this a room full of stocked chests looks, to all three of those, like
    /// a room full of empty ones.
    fn send_chest_contents_for_section(
        &mut self,
        slot: u8,
        bounds: terrustia_proto::section::SectionBounds,
    ) -> terrustia_proto::Result<()> {
        let right = bounds.x + i32::from(bounds.width);
        let bottom = bounds.y + i32::from(bounds.height);
        let inside: Vec<(i16, Vec<terrustia_proto::ItemStack>)> = self
            .world
            .chests
            .iter()
            .enumerate()
            // A chest's id is its slot in the table, so the index has to survive the gaps that
            // deleted chests leave behind.
            .filter_map(|(id, slot)| slot.as_ref().map(|chest| (id, chest)))
            .filter(|(_, chest)| {
                let (x, y) = (i32::from(chest.x), i32::from(chest.y));
                x >= bounds.x && x < right && y >= bounds.y && y < bottom
            })
            .map(|(id, chest)| (id as i16, chest.items.clone()))
            .collect();

        for (id, items) in inside {
            self.send(slot, objects::sync_chest_size(id, items.len() as i16)?);
            for (index, item) in items.iter().enumerate() {
                let frame = SyncChestItem {
                    chest: id,
                    slot: index as u8,
                    item: *item,
                }
                .encode()?;
                self.send(slot, frame);
            }
        }
        Ok(())
    }

    /// Packet 159: the client asking for one section as it moves.
    ///
    /// New in 1.4.5 — previously the server pushed sections from the player's position. Without
    /// this a player can walk out of the area streamed at spawn and see nothing but sky.
    fn on_request_section(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if self
            .player(slot)
            .is_none_or(|p| p.state < ConnState::TilesSent)
        {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let sx = i32::from(r.u16()?);
        let sy = i32::from(r.u16()?);
        self.send_section(slot, sx, sy)
    }

    /// The sections vanilla streams on a tile request: a block around spawn, plus one around the
    /// requested position when it is a real location.
    fn sections_for(&self, request: SpawnTileData) -> Vec<(i32, i32)> {
        let mut wanted = HashSet::new();
        let (max_x, max_y) = (self.world.sections_x(), self.world.sections_y());

        // The block is *slid* inside the world rather than clipped against it. Clipping loses a
        // row or column whenever a player is near an edge — a player who spawns in the topmost
        // section used to get one fewer section beneath them than intended, which left the world
        // simply absent a hundred and fifty tiles below their feet. It only showed up when the
        // generator started putting the surface high enough to reach section zero.
        let mut add_block = |cx: i32, cy: i32, w: i32, h: i32| {
            let first_x = (cx - 2).clamp(0, (max_x - w).max(0));
            let first_y = (cy - 1).clamp(0, (max_y - h).max(0));
            for sx in first_x..(first_x + w).min(max_x) {
                for sy in first_y..(first_y + h).min(max_y) {
                    wanted.insert((sx, sy));
                }
            }
        };

        let (spawn_sx, spawn_sy) = self
            .world
            .section_of(i32::from(self.world.spawn_x), i32::from(self.world.spawn_y));
        add_block(spawn_sx, spawn_sy, 5, 3);

        let valid = request.x >= 10
            && request.y >= 10
            && request.x < self.world.width() - 10
            && request.y < self.world.height() - 10;
        if valid {
            let (sx, sy) = self.world.section_of(request.x, request.y);
            add_block(sx, sy, 6, 4);
        }

        let mut sections: Vec<(i32, i32)> = wanted.into_iter().collect();
        // Deterministic order keeps logs and tests reproducible.
        sections.sort_unstable();
        sections
    }

    fn on_player_spawn(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let spawn = PlayerSpawn::decode(payload)?;
        let was_playing = self.player(slot).is_some_and(Player::is_playing);

        if let Some(player) = self.player_mut(slot) {
            if player.state < ConnState::TilesSent {
                return Ok(()); // spawning before the world arrived is not a valid sequence
            }
            player.team = spawn.team;
            // A respawn puts you back on your feet. Without this the server keeps thinking you
            // are dead, and every routine that checks whether anyone is alive ignores you for the
            // rest of the session.
            if player.life <= 0 {
                player.life = player.life_max.max(1);
                player.immune_ticks = 0;
            }
            player.advance_to(ConnState::Playing);
        } else {
            return Ok(());
        }

        // Always relay the spawn so respawns after death are visible.
        let relay = packets::rewrite_owner(id::PLAYER_SPAWN, payload, slot)?;
        self.broadcast(relay, Some(slot));

        if !was_playing {
            self.introduce(slot)?;
            // What everyone already here is wearing. This waits until they are actually playing
            // rather than going out with the world: a client is still working through its
            // handshake when the tiles arrive, and anything sent then is read by the handshake
            // rather than by the game.
            self.send_existing_equipment(slot);
        }
        Ok(())
    }

    /// Exchange presence between a newly spawned player and everyone already in the world.
    fn introduce(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        let others: Vec<u8> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.is_playing() && p.slot != slot)
            .map(|p| p.slot)
            .collect();

        // Tell the newcomer about everyone else.
        for other in &other_slots(&others) {
            for frame in self.presence_frames(*other)? {
                self.send(slot, frame);
            }
        }

        // Tell everyone else about the newcomer.
        for frame in self.presence_frames(slot)? {
            self.broadcast(frame, Some(slot));
        }

        // A player joining on the same machine the server runs on counts as the host, which is
        // exactly the rule the game uses — `DoesPlayerSlotCountAsAHost` asks the socket whether the
        // far end is the loopback address and nothing else. Only sent when true, as the game only
        // sends it when true.
        if self.player(slot).is_some_and(Player::is_local) {
            self.send(slot, packets::counts_as_host(slot, true)?);
        }

        // How much of the world has gone over to each side. The client cannot work this out from
        // the sections it holds, so without it the Dryad reports a world that is nought per cent
        // of everything however far the corruption has spread.
        self.send(
            slot,
            packets::world_evil_tally(
                self.census.percent_hallow,
                self.census.percent_corrupt,
                self.census.percent_crimson,
            )?,
        );

        // Where every town NPC lives. This is what the housing screen draws its banners from; a
        // client never told has an empty housing menu no matter how many villagers it can see.
        for frame in self.npc_home_frames() {
            self.send(slot, frame);
        }

        // Every banner's kill count. The world has been recording these since §26; nothing was
        // ever telling a client about them, so the bestiary showed nought kills for everything.
        self.send(slot, self.banner_state_frame()?);

        // Every pylon. The client keeps its own list and draws the travel map from it, so one it
        // was never told about is scenery: standing beside it opens a map with nowhere to go.
        for pylon in self.pylons() {
            self.pylon_kinds.insert((pylon.x, pylon.y), pylon.kind);
            self.send(
                slot,
                net_module::pylon_message(net_module::PylonMessage::Added, pylon)?,
            );
        }

        self.send(slot, packets::empty(id::FINISHED_CONNECTING_TO_SERVER)?);

        // What the Travelling Merchant is carrying, if he is here. A client that joins mid-visit
        // and is not told finds him with nothing to sell.
        if !self.travel_shop.is_empty() {
            let mut w = terrustia_proto::PacketWriter::new(id::TRAVEL_MERCHANT_ITEMS);
            for at in 0..TRAVEL_SHOP_SLOTS {
                w.i16(self.travel_shop.get(at).copied().unwrap_or(0) as i16);
            }
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }

        // Which six cavern enemies this world has. Fixed for the world's life, so it is sent once
        // on joining rather than kept up to date.
        {
            let mut w = terrustia_proto::PacketWriter::new(id::SYNC_CAVERN_MONSTER_TYPE);
            for kind in self.cavern_monsters.flat() {
                w.u16(kind);
            }
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }

        // Journey mode's four shared toggles this server models — `ASharedTogglePower::
        // OnPlayerJoining`'s own effect. A client never told assumes every power starts off, which
        // is wrong the moment an operator has frozen time or the weather before this player joined.
        for id in [
            net_module::power::FREEZE_TIME,
            net_module::power::FREEZE_RAIN,
            net_module::power::FREEZE_WIND,
            net_module::power::STOP_BIOME_SPREAD,
        ] {
            if let Some(enabled) = self.journey.get(id)
                && let Ok(frame) = net_module::creative_power_toggle(id, enabled)
            {
                self.send(slot, frame);
            }
        }
        // `ModifyTimeRate` and `Difficulty` are the two shared sliders that sync to a joining
        // player (`_syncToJoiningPlayers = true`, the base `ASharedSliderPower` default that
        // neither constructor overrides — `ModifyWind`/`ModifyRain` are both `false` in source,
        // see `journey.rs`'s own module doc for why there is nothing to send for those two here).
        for (id, value) in [
            (
                net_module::power::MODIFY_TIME_RATE,
                self.journey.time_rate_slider,
            ),
            (
                net_module::power::DIFFICULTY,
                self.journey.difficulty_slider,
            ),
        ] {
            if let Ok(frame) = net_module::creative_power_slider(id, value) {
                self.send(slot, frame);
            }
        }
        // `Godmode`/`FarPlacementRange`'s full per-player state — `APerPlayerTogglePower::
        // OnPlayerJoining`'s own `SyncEveryone`, bit-packed. `SpawnRate` sends nothing here on
        // purpose: `APerPlayerSliderPower::OnPlayerJoining` only resets the *new* player's own
        // local cache to the default, no network message at all — another player's slider
        // position was never anyone else's business in the first place (see the slider handler's
        // own comment on why a change to it is never broadcast either).
        for (id, states) in [
            (net_module::power::GODMODE, self.journey.godmode),
            (
                net_module::power::FAR_PLACEMENT_RANGE,
                self.journey.far_placement_range,
            ),
        ] {
            if let Ok(frame) = net_module::creative_power_toggle_full_state(id, &states) {
                self.send(slot, frame);
            }
        }

        // What the Angler wants today. A client that is never told shows no quest at all, so a
        // player who joins after dawn would find the Angler had nothing to say until midnight.
        {
            let name = self
                .player(slot)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let done = self.angler_finished_today.contains(&name);
            let mut w = terrustia_proto::PacketWriter::new(id::ANGLER_QUEST);
            w.u8(self.angler_quest).bool(done);
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }

        let name = self
            .player(slot)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        // `LegacyMultiplayer.19` is `"{0} has joined."`.
        let who = NetworkText::literal(&name);
        self.announce_key("LegacyMultiplayer.19", vec![who]);

        let motd = self.config.motd.clone();
        if !motd.is_empty()
            && let Ok(frame) = net_module::chat_broadcast(
                net_module::SERVER_AUTHOR,
                &NetworkText::literal(&motd),
                SERVER_CHAT_COLOUR,
            )
        {
            self.send(slot, frame);
        }
        Ok(())
    }

    /// Everything another client needs in order to draw a player.
    fn presence_frames(&self, slot: u8) -> terrustia_proto::Result<Vec<Vec<u8>>> {
        let Some(player) = self.player(slot) else {
            return Ok(Vec::new());
        };

        let mut frames = vec![packets::player_active(slot, true)?];
        if let Some(appearance) = &player.appearance {
            frames.push(packets::rewrite_owner(id::SYNC_PLAYER, appearance, slot)?);
        }
        frames.push(
            PlayerHealth {
                player: slot,
                life: player.life,
                life_max: player.life_max,
            }
            .encode()?,
        );
        frames.push(
            PlayerMana {
                player: slot,
                mana: player.mana,
                mana_max: player.mana_max,
            }
            .encode()?,
        );
        if let Some(buffs) = &player.buffs {
            frames.push(packets::rewrite_owner(id::PLAYER_BUFFS, buffs, slot)?);
        }
        if let Some(zone) = &player.zone {
            frames.push(packets::rewrite_owner(id::SYNC_PLAYER_ZONE, zone, slot)?);
        }
        if player.team != 0 {
            let mut w = terrustia_proto::PacketWriter::new(id::TEAM_CHANGE);
            w.u8(slot).u8(player.team);
            frames.push(w.finish()?);
        }
        if player.pvp {
            let mut w = terrustia_proto::PacketWriter::new(id::TOGGLE_P_V_P);
            w.u8(slot).bool(true);
            frames.push(w.finish()?);
        }
        if let Some(controls) = &player.last_controls {
            frames.push(packets::rewrite_owner(id::PLAYER_CONTROLS, controls, slot)?);
        }
        Ok(frames)
    }

    fn on_player_controls(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let controls = PlayerControls::decode(payload)?;

        if let Some(player) = self.player_mut(slot) {
            // Velocity is what actually changed since the last update, not what the client
            // claims: the routines that lead a running player want the real thing.
            player.velocity = (
                controls.position.0 - player.position.0,
                controls.position.1 - player.position.1,
            );
            player.position = controls.position;
            // Which way they are looking. Only one thing reads it — a wiring tool's L turns the
            // other way depending on it — but that one thing is visible the moment it is wrong.
            player.facing_right = controls.facing_right();
            player.sitting = controls.sitting();
            player.selected_item = controls.selected_item;
            player.last_controls = Some(Bytes::copy_from_slice(payload));
            if !player.is_playing() {
                return Ok(());
            }
        } else {
            return Ok(());
        }

        // Relayed verbatim: the payload has optional trailing blocks the server does not model.
        let frame = packets::rewrite_owner(id::PLAYER_CONTROLS, payload, slot)?;
        self.broadcast(frame, Some(slot));

        // Checked every real control update while sitting, matching real vanilla's own cadence
        // (`PlayerSittingHelper.UpdateSitting`, called every frame a player sits) rather than a
        // periodic tick — the whole point is that it responds the moment a player who is already
        // sitting selects the doll, not up to `OLD_MAN_CHECK_INTERVAL` seconds later.
        if controls.sitting() {
            self.check_red_hat_skeletron(slot);
        }
        Ok(())
    }

    /// The Clothier's own red-hatted Skeletron: a repeatable, vanity-only re-fight available once
    /// Skeletron has been defeated for real at least once (`NPC.cs:81216-81241`,
    /// `RegisterBoss_Skeletron`'s own five `RedHatSkeletronAdjustmentsEnabled`-gated items).
    ///
    /// Real vanilla's own condition, transcribed exactly rather than approximated now that both
    /// prerequisites this project was once missing — which hotbar slot is selected, and what the
    /// player is sitting on — are both tracked: the sitting player's own currently-selected item
    /// is the Clothier Voodoo Doll (`Player.killClothier`, reset every frame and set true only
    /// while that item is selected — modelled here as a direct check rather than a persisted flag,
    /// since nothing else needs the intermediate state), it is night, they are seated on the one
    /// specific chair frame the event uses (`tile.type == 89`, `frameX` in `2322..=2358`), and a
    /// real, active Clothier (54) — not the Old Man, who only stands in for him before Skeletron's
    /// first, real defeat — is close enough to see them.
    fn check_red_hat_skeletron(&mut self, slot: u8) {
        const CHAIR: u16 = 89;
        const CHAIR_FRAME_MIN: i16 = 2322;
        const CHAIR_FRAME_MAX: i16 = 2358;
        const CLOTHIER_VOODOO_DOLL: i32 = 1307;
        const CLOTHIER: u16 = 54;
        const SKELETRON: u16 = 35;
        /// How close the Clothier has to be — real vanilla's own `Collision.CanHit` is a
        /// line-of-sight check this project has no equivalent for on an NPC-to-player pair; a
        /// flat distance is the same narrowing this project's own `RedHatSkeletron` plan.md entry
        /// already flagged as the honest substitute, not a silent approximation.
        const CLOTHIER_REACH: f32 = 400.0;

        if self.world.day_time || !self.world.progress.downed_boss3 {
            return;
        }
        if self.npcs.iter().any(|(_, n)| n.npc_type == SKELETRON) {
            return;
        }
        let Some(player) = self.player(slot) else {
            return;
        };
        let holding_doll = player
            .inventory
            .get(&u16::from(player.selected_item))
            .is_some_and(|e| e.item.id == CLOTHIER_VOODOO_DOLL && e.item.stack > 0);
        if !holding_doll {
            return;
        }
        let (px, py) = player.position;
        // Real vanilla checks the tile under `player.Bottom + (0, -2)`
        // (`PlayerSittingHelper.UpdateSitting`), not the hitbox's own top-left corner: `position`
        // here is that top-left corner (matching every other tile lookup against it in this file),
        // so it needs the same width/2 and height-2 offsets vanilla's own `Bottom` applies before
        // either coordinate means anything as a tile index.
        let (tx, ty) = (
            ((px + crate::game::ai::PLAYER_WIDTH as f32 / 2.0) / crate::game::npc::TILE) as i32,
            ((py + crate::game::ai::PLAYER_HEIGHT as f32 - 2.0) / crate::game::npc::TILE) as i32,
        );
        let tile = self.world.tile(tx, ty);
        if tile.block != CHAIR || tile.frame_x < CHAIR_FRAME_MIN || tile.frame_x > CHAIR_FRAME_MAX {
            return;
        }
        let clothier = self.npcs.iter().find(|(_, n)| {
            n.npc_type == CLOTHIER
                && n.is_alive()
                && (n.center().0 - px).abs() < CLOTHIER_REACH
                && (n.center().1 - py).abs() < CLOTHIER_REACH
        });
        let Some((index, at)) = clothier.map(|(index, n)| (index, n.center())) else {
            return;
        };

        self.spawn_skeletron_from(index, at, true);
    }

    /// Shared by both real triggers: consume the cursed NPC at `index` and raise Skeletron in its
    /// place — the ordinary Old Man/Clothier curse (`red_hat = false`) and the Clothier's own
    /// repeatable vanity re-fight (`red_hat = true`, `SpawnSkeletron`'s own `redHatMode` argument,
    /// `NPC.cs:81232-81233`) differ only in that one flag.
    pub(super) fn spawn_skeletron_from(&mut self, index: u8, at: (f32, f32), red_hat: bool) {
        const SKELETRON: u16 = 35;

        self.npcs.remove(index);
        self.broadcast_npc_death(index);
        if let Some(spawned) = self.npcs.spawn(SKELETRON, at) {
            if red_hat && let Some(npc) = self.npcs.get_mut(spawned) {
                // What `RedHatSkeletronAdjustmentsEnabled` reads back at drop time.
                npc.ai[3] = 1.0;
            }
            self.announce("Skeletron has awoken!");
            self.broadcast_npc(spawned);
        }
    }

    fn on_health(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let health = PlayerHealth::decode(payload)?;
        if let Some(player) = self.player_mut(slot) {
            player.life = health.life;
            player.life_max = health.life_max;
        }
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::PLAYER_LIFE_MANA, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_mana(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mana = PlayerMana::decode(payload)?;
        if let Some(player) = self.player_mut(slot) {
            player.mana = mana.mana;
            player.mana_max = mana.mana_max;
        }
        if self.player(slot).is_some_and(Player::is_playing) {
            let frame = packets::rewrite_owner(id::PLAYER_MANA, payload, slot)?;
            self.broadcast(frame, Some(slot));
        }
        Ok(())
    }

    fn on_team(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        r.u8()?;
        let team = r.u8()?;
        if let Some(player) = self.player_mut(slot) {
            player.team = team;
        }
        self.relay_player_packet(slot, id::TEAM_CHANGE, payload)
    }

    /// Packet 60 inbound: a player using the housing screen.
    ///
    /// This id is sent both ways. The server announces where each town NPC lives, which it already
    /// did — but the *client* sends the same packet to ask for a change, and that half was falling
    /// through to the ignore arm. So dragging an NPC into a room, or evicting one, did nothing at
    /// all on this server while looking like it had worked locally.
    ///
    /// Vanilla's server half (`MessageBuffer.cs` case 60, the `netMode != 1` branches) is two
    /// cases: a status byte of 1 evicts, anything else assigns the room at the given tile. It also
    /// boots a client whose NPC index is out of range as a cheat attempt; we decline the packet
    /// instead, since the transport is not the place to decide somebody is cheating.
    fn on_npc_home(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let home_x = r.i16()?;
        let home_y = r.i16()?;
        let evicting = r.u8()? == 1;

        let Ok(index) = u8::try_from(index) else {
            debug!(
                slot,
                index, "housing request for an npc slot that cannot exist"
            );
            return Ok(());
        };
        // Only town NPCs have homes; anything else is a client asking for something meaningless.
        let Some(npc) = self.npcs.get(index) else {
            return Ok(());
        };
        if !npc.stats.town_npc || !npc.is_alive() {
            return Ok(());
        }

        if evicting {
            if let Some(npc) = self.npcs.get_mut(index) {
                npc.home = None;
            }
            info!(slot, index, "town npc evicted");
        } else {
            // The room has to be one the game would accept, or a client could house a merchant
            // inside solid rock and the server would agree.
            match crate::game::housing::check_room(
                &self.world,
                i32::from(home_x),
                i32::from(home_y),
            ) {
                Ok(_) => {
                    if let Some(npc) = self.npcs.get_mut(index) {
                        npc.home = Some((i32::from(home_x), i32::from(home_y)));
                    }
                    info!(slot, index, home_x, home_y, "town npc moved in");
                }
                Err(why) => {
                    debug!(slot, index, ?why, "housing request refused");
                    // Tell the asker what it actually is, so their screen stops showing the move.
                    if let Some(frame) = self.npc_home_frame(index) {
                        self.send(slot, frame);
                    }
                    return Ok(());
                }
            }
        }
        // Everyone's housing screen has to agree, including the one that asked.
        self.broadcast_npc_home(index);
        Ok(())
    }

    fn on_pvp(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        r.u8()?;
        let hostile = r.bool()?;
        if let Some(player) = self.player_mut(slot) {
            player.pvp = hostile;
        }
        self.relay_player_packet(slot, id::TOGGLE_P_V_P, payload)
    }

    fn on_buffs(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if let Some(player) = self.player_mut(slot) {
            player.buffs = Some(Bytes::copy_from_slice(payload));
        }
        self.relay_player_packet(slot, id::PLAYER_BUFFS, payload)
    }

    /// Packet 55: a PvP-flagged player's own hit spreads a buff onto another PvP-flagged player.
    ///
    /// Unlike every other player packet on this page, this one is not a broadcast: real vanilla's
    /// own server relays it to exactly the named target, and only that player's own client
    /// actually calls `AddBuff` on receiving it — everyone else is never told. The payload itself
    /// carries no sender identity to rewrite (`target: u8, buff: u16, duration: i32`, the whole of
    /// it); the real sender is `slot`, read from the connection the way every other packet here
    /// already trusts it, not from anything in the payload.
    fn on_pvp_buff_spread(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        let target = r.u8()?;
        let buff = r.u16()?;

        let both_hostile =
            self.player(slot).is_some_and(|p| p.pvp) && self.player(target).is_some_and(|p| p.pvp);
        if !both_hostile || !terrustia_proto::buffs::is_pvp_spreadable(buff) {
            return Ok(());
        }

        self.send(
            target,
            packets::verbatim(id::ADD_PLAYER_BUFF_PV_P, payload)?,
        );
        Ok(())
    }

    fn on_zone(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if let Some(player) = self.player_mut(slot) {
            player.zone = Some(Bytes::copy_from_slice(payload));
        }
        self.relay_player_packet(slot, id::SYNC_PLAYER_ZONE, payload)
    }

    /// A player teleported: a magic mirror, a teleporter, a recall potion.
    ///
    /// The server has to move its own idea of the player as well as relaying, because every
    /// routine that hunts a target reads that position. A teleport the server does not apply
    /// leaves every enemy in the world attacking where the player used to be.
    fn on_teleport(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        let flags = r.u8()?;
        let _claimed = r.i16()?;
        let x = r.f32()?;
        let y = r.f32()?;
        let style = r.u8()?;

        // Bits 0 and 1 together say what is being teleported; only zero — a player — is ours.
        let what = (flags & 1) + ((flags & 2) >> 1) * 2;
        if what != 0 {
            return Ok(());
        }
        // Bit 2 means "where they already are", which is how a client asks for the effect without
        // moving anything.
        let stay = flags & 4 != 0;
        let extra = if flags & 8 != 0 { r.i32()? } else { 0 };

        let at = if stay {
            match self.player(slot) {
                Some(player) => player.position,
                None => return Ok(()),
            }
        } else {
            (x, y)
        };
        if !at.0.is_finite() || !at.1.is_finite() {
            return Ok(());
        }
        if let Some(player) = self.player_mut(slot) {
            player.position = at;
            player.velocity = (0.0, 0.0);
        }

        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_ENTITY);
        w.u8(flags);
        w.i16(i16::from(slot));
        w.f32(at.0);
        w.f32(at.1);
        w.u8(style);
        if flags & 8 != 0 {
            w.i32(extra);
        }
        let frame = w.finish()?;
        self.broadcast(frame, Some(slot));
        debug!(slot, x = at.0, y = at.1, "player teleported");
        Ok(())
    }

    /// Packet 73: a client asking the server to move it somewhere.
    ///
    /// Five items work this way, and the reason they are the server's business rather than the
    /// client's is that all five have to *search the world* for somewhere safe to land — which
    /// means seeing tiles the client may not have loaded. None of them was handled, so a
    /// Teleportation Potion, a Magic Conch, a Demon Conch and a Shellphone were all inert.
    ///
    /// A search that finds nowhere leaves the player where they are. That is the game's own
    /// behaviour and the right one: a conch that fails is a wasted item, but a conch that drops
    /// you into a lava lake is a lost character.
    fn on_server_teleport(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use crate::game::teleport::{self, Gates, Wants};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let kind = r.u8()?;

        let (width, height) = (self.world.width(), self.world.height());
        let gates = Gates {
            downed_plantera: self.world.progress.downed_plantera,
            downed_skeletron: self.world.progress.downed_boss3,
            surface: i32::from(self.world.surface),
            width,
            height,
        };
        let Some(here) = self.player(slot).map(|p| p.position) else {
            return Ok(());
        };

        // The underworld begins here. The game keeps it as a fraction of the world's height.
        let underworld = height - 200;

        let spot = match kind {
            TELEPORT_POTION => {
                // Anywhere at all, above the underworld.
                let tiles = WorldTiles(&self.world);
                teleport::find_spot(
                    &tiles,
                    &mut self.rng,
                    (100, width - 200),
                    (100, underworld - 100),
                    &Wants::default(),
                    &gates,
                )
            }
            MAGIC_CONCH => {
                // The ocean on the far side of the world from wherever you are, which is what
                // makes the conch a way of crossing the map rather than a local shuffle.
                let far_side_is_left = here.0 / crate::game::npc::TILE >= (width / 2) as f32;
                let start = if far_side_is_left {
                    BEACH_MARGIN
                } else {
                    width - BEACH_DISTANCE
                };
                let tiles = WorldTiles(&self.world);
                teleport::find_spot(
                    &tiles,
                    &mut self.rng,
                    (start, BEACH_DISTANCE - BEACH_MARGIN),
                    (100, i32::from(self.world.surface) + 100),
                    &Wants {
                        avoid_any_liquid: true,
                        max_fall: 300,
                        ..Default::default()
                    },
                    &gates,
                )
            }
            DEMON_CONCH => {
                // The underworld, near the middle first and then further out if that fails.
                let middle = width / 2;
                let tiles = WorldTiles(&self.world);
                let wants = Wants {
                    avoid_any_liquid: true,
                    avoid_walls: true,
                    allow_platform_floor: true,
                    ..Default::default()
                };
                teleport::find_spot(
                    &tiles,
                    &mut self.rng,
                    (middle - 50, 100),
                    (underworld + 20, 80),
                    &wants,
                    &gates,
                )
                .or_else(|| {
                    // Failing the middle, anywhere in the underworld at all.
                    teleport::find_spot(
                        &tiles,
                        &mut self.rng,
                        (100, width - 200),
                        (underworld + 20, 80),
                        &wants,
                        &gates,
                    )
                })
            }
            // The Shellphone's spawn setting, and the rescue that fires when a player is crushed
            // with nowhere to stand. Both go to the world's spawn point, which is always valid.
            SHELLPHONE_SPAWN | NO_SPACE_RESCUE => Some((
                f32::from(self.world.spawn_x) * crate::game::npc::TILE - PLAYER_HALF_WIDTH,
                f32::from(self.world.spawn_y) * crate::game::npc::TILE - PLAYER_HEIGHT,
            )),
            _ => return Ok(()),
        };

        let Some(at) = spot else {
            debug!(slot, kind, "no safe landing spot; the player stays put");
            return Ok(());
        };
        if let Some(player) = self.player_mut(slot) {
            player.position = at;
            player.velocity = (0.0, 0.0);
        }

        // Style 2 is the potion's swirl, 11 the phone's. They are only an effect, but a client
        // that is not told which one plays the wrong animation.
        let style = if kind == TELEPORT_POTION { 2u8 } else { 11u8 };
        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_ENTITY);
        w.u8(0).i16(i16::from(slot)).f32(at.0).f32(at.1).u8(style);
        let frame = w.finish()?;
        // To everyone including the mover: the client asked to be moved and does not move itself.
        self.broadcast(frame, None);
        debug!(slot, kind, x = at.0, y = at.1, "server-side teleport");
        Ok(())
    }

    /// A player started or stopped talking to a town NPC.
    fn on_talk_npc(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let npc = r.i16()?;
        if let Some(player) = self.player_mut(slot) {
            player.talking_to = if npc >= 0 { Some(npc as u8) } else { None };
        }
        if npc >= 0 {
            self.try_rescue(npc as u8);
        }
        self.relay_player_packet(slot, id::SYNC_TALK_N_P_C, payload)
    }

    /// Talking to somebody tied up frees them.
    ///
    /// Six residents are found rather than earned, and the flag their arrival waits on is only
    /// ever set here. Without this the Mechanic could never appear, and she sells the only wire in
    /// the game — so an entire implemented subsystem sat unreachable behind one missing
    /// interaction.
    fn try_rescue(&mut self, index: u8) {
        let Some(npc) = self.npcs.get(index) else {
            return;
        };
        let Some(rescue) = crate::game::rescues::rescue_for(npc.npc_type) else {
            return;
        };

        if let Some(npc) = self.npcs.get_mut(index) {
            npc.become_type(rescue.freed);
        }
        crate::game::rescues::remember(&mut self.world.progress, rescue.freed);
        self.announce(rescue.announcement);
        self.broadcast_npc(index);
        self.broadcast_world_data();
        info!(freed = rescue.freed, "a bound townsperson was rescued");
    }

    /// A player placed a multi-tile object: a chest, a door, a bed, a workbench.
    ///
    /// This has to happen on the server as well as on the clients. Every other client is told and
    /// places it locally, but if the server does not write the tiles too the object is gone the
    /// moment the world is saved, invisible to anyone who joins later, and does not count toward a
    /// house.
    fn on_place_object(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        // Same gates every other tile-writing handler has, and that `MessageBuffer.cs`'s own
        // `case 79` applies: a placement only counts from a client that has actually joined, it is
        // charged against that client's block-spam budget (`SpamAddBlock++`), and it is refused
        // outright unless the client owns the section it is placing into
        // (`Netplay.Clients[whoAmI].TileSections`, mirrored by `Player::sent_sections`). Without
        // these, any socket — handshake incomplete included — could scatter furniture, chests and
        // tile entities anywhere in the world at an unthrottled rate, growing `world.tile_entities`
        // without bound: exactly the illegitimate-edit class the packet-17 fix already closed.
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }

        let mut r = PacketReader::new(payload);
        let x = i32::from(r.i16()?);
        let y = i32::from(r.i16()?);
        let block = r.i16()?;
        let style = i32::from(r.i16()?);
        let _alternate = r.u8()?;
        let random = i32::from(r.i8()?);
        let _direction = r.bool()?;

        if self.note_tile_spam(slot, TileAction::PlaceTile) {
            return Ok(());
        }

        let Ok(block) = u16::try_from(block) else {
            return Ok(());
        };
        let Some(object) = terrustia_proto::tile_object::tile_object(block) else {
            debug!(
                slot,
                block, "ignoring a placement of something that is not an object"
            );
            return Ok(());
        };
        // Ten tiles clear of the world's edge, as the game requires.
        if x < 10 || y < 10 || x >= self.world.width() - 10 || y >= self.world.height() - 10 {
            return Ok(());
        }
        // The section the object is placed into must be one this client was actually sent. Vanilla
        // `break`s here — dropping the placement — rather than relaying a suppressed edit the way
        // case 17 does.
        let (sx, sy) = self.world.section_of(x, y);
        if !self
            .player(slot)
            .is_some_and(|p| p.sent_sections.contains(&(sx, sy)))
        {
            return Ok(());
        }

        // The packet gives the cursor tile; the object's own origin says where its corner goes.
        let (left, top) = (x - object.origin.0, y - object.origin.1);
        // Nothing is placed over anything already there — the game refuses the whole object
        // rather than filling in the gaps.
        for dx in 0..object.width {
            for dy in 0..object.height {
                if self.world.tile(left + dx, top + dy).is_active() {
                    return Ok(());
                }
            }
        }

        let (frame_x, frame_y) = object.frame_of(style, random);
        for dx in 0..object.width {
            let fx = frame_x + dx * (object.coord_width + object.padding);
            let mut fy = frame_y;
            for dy in 0..object.height {
                // A framed tile, which is what marks it active — setting the block alone leaves an
                // inactive tile that every client draws as empty air.
                let was = self.world.tile(left + dx, top + dy);
                let tile = terrustia_proto::tile::Tile::framed(block, fx as i16, fy as i16)
                    .with_wall(was.wall);
                self.world.set_tile(left + dx, top + dy, tile);
                self.liquids.disturb(left + dx, top + dy);
                fy += object.coord_heights.get(dy as usize).copied().unwrap_or(16) + object.padding;
            }
        }

        // A chest is not only tiles: it needs somewhere to keep what is put in it.
        if block == CHEST_BLOCK {
            let anchor = (left as i16, top as i16);
            if self.world.chest_at(anchor.0, anchor.1).is_none() {
                self.world
                    .add_chest(crate::world::Chest::empty_at(anchor.0, anchor.1));
            }
        }

        // Nor is an item frame, a mannequin, a hat rack or a food platter. These are never asked
        // for by packet — the game's placement request does nothing for them — so placing the
        // tile is the *only* moment they can come into existence. Without this an item frame is
        // scenery: it can be built and never holds anything.
        if let Some(kind) = terrustia_proto::tile_entity::EntityKind::for_tile(block) {
            self.add_tile_entity(kind, left as i16, top as i16);
        }

        // Everyone else places it themselves from the same packet.
        self.broadcast(
            terrustia_proto::packets::place_object(x, y, block, style, random)?,
            Some(slot),
        );
        debug!(slot, block, x, y, "object placed");
        Ok(())
    }

    /// `World::world_data`, with the ambient events that live on `GameServer` rather than `World`
    /// patched in — `PartyIsUp` (`self.party`), the same shape `self.army`'s own tier flags would
    /// need if `ArmyOngoing` were wired up (it is not, a real pre-existing gap this project already
    /// disclosed — `World::world_data`'s own comment on `DownedArmyTier1..3`). Every caller that
    /// sends packet 7 should go through this rather than `self.world.world_data()` directly, or a
    /// joining client learns everything about the world except whether a party is happening in it
    /// right now.
    pub(super) fn world_data(&self) -> terrustia_proto::packets::WorldData {
        use terrustia_proto::packets::WorldFlag;
        let mut data = self.world.world_data();
        data.flags
            .set_flag(WorldFlag::PartyIsUp, self.party.is_up());
        data.flags
            .set_flag(WorldFlag::SlimeRain, self.slime_rain.is_active());
        data.flags
            .set_flag(WorldFlag::LanternNight, self.lantern_night.is_up());
        data
    }

    /// Tell everyone the world itself has changed — an eclipse begun, a blood moon risen.
    pub(super) fn broadcast_world_data(&mut self) {
        if let Ok(frame) = self.world_data().encode() {
            self.broadcast(frame, None);
        }
    }

    /// A player used a summoning item.
    ///
    /// This is the only way a boss enters the world, so it is also the only place a client gets to
    /// name an NPC type. What it may name is the game's own list and nothing else: without that
    /// check a crafted packet could ask for anything in the roster, in any number.
    ///
    /// Negative types are events rather than bosses — a pumpkin moon, an eclipse, an invasion.
    fn on_summon(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut r = PacketReader::new(payload);
        // The player the client claims; ignored in favour of the connection it arrived on.
        let _claimed = r.i16()?;
        let what = r.i16()?;

        if what >= 0 {
            let npc_type = what as u16;
            if !terrustia_proto::npc_params::summonable(npc_type) {
                debug!(
                    slot,
                    npc_type, "refusing to summon something that is not summonable"
                );
                return Ok(());
            }
            // One at a time. A second Eye of Cthulhu is not a thing the game allows.
            if self.npcs.iter().any(|(_, n)| n.npc_type == npc_type) {
                return Ok(());
            }
            self.summon_on_player(slot, npc_type);
            return Ok(());
        }

        match what {
            // A pumpkin or frost moon, which only rise at night.
            -4 | -5 => {
                let moon = if what == -4 {
                    crate::game::moons::Moon::Pumpkin
                } else {
                    crate::game::moons::Moon::Frost
                };
                self.start_moon(moon, slot);
            }
            // A solar eclipse, which only happens by day.
            -6 => {
                if self.world.day_time && !self.world.eclipse {
                    self.world.eclipse = true;
                    self.announce_key("LegacyMisc.20", Vec::new());
                    self.broadcast_world_data();
                }
            }
            -7 => self.start_invasion(Invasion::Martian),
            // A blood moon, which only rises at night and not twice in one night.
            -10 => {
                if !self.world.day_time && !self.world.blood_moon {
                    self.world.blood_moon = true;
                    self.announce_key("LegacyMisc.8", Vec::new());
                    self.broadcast_world_data();
                }
            }
            // The rest of the negative range is the invasions, numbered from -1.
            other => {
                if let Some(kind) = Invasion::from_id(i32::from(-other)) {
                    self.start_invasion(kind);
                } else {
                    debug!(slot, what = other, "ignoring an unrecognised summon");
                }
            }
        }
        Ok(())
    }

    /// Put a boss somewhere near a player: on the ground, out of arm's reach, or overhead when
    /// there is no ground to be found.
    pub(super) fn summon_on_player(&mut self, slot: u8, npc_type: u16) {
        use terrustia_proto::npc_params::{
            SUMMON_ABOVE, SUMMON_ATTEMPTS, SUMMON_RANGE_X, SUMMON_RANGE_Y, SUMMON_SAFE_X,
            SUMMON_SAFE_Y,
        };
        // Copied out, because the search below needs the generator and the player at once.
        let Some(at_player) = self.player(slot).map(|p| p.position) else {
            return;
        };
        let (px, py) = (
            (at_player.0 / crate::game::npc::TILE) as i32,
            (at_player.1 / crate::game::npc::TILE) as i32,
        );

        let mut at = None;
        for _ in 0..SUMMON_ATTEMPTS {
            let x = px + rand::Rng::random_range(&mut self.rng, -SUMMON_RANGE_X..=SUMMON_RANGE_X);
            let y = py + rand::Rng::random_range(&mut self.rng, -SUMMON_RANGE_Y..=SUMMON_RANGE_Y);
            // Not right on top of the player.
            if (x - px).abs() < SUMMON_SAFE_X && (y - py).abs() < SUMMON_SAFE_Y {
                continue;
            }
            if self.world.tile(x, y).is_active() {
                continue;
            }
            let Some(ground) = crate::game::spawn::find_ground(&self.world, x, y) else {
                continue;
            };
            at = Some((
                x as f32 * crate::game::npc::TILE,
                (ground - 1) as f32 * crate::game::npc::TILE,
            ));
            break;
        }
        // Nowhere to stand — a Moon Lord, or a player in mid-air. Overhead it is.
        let at = at.unwrap_or((at_player.0, at_player.1 - SUMMON_ABOVE));

        // A worm head spawned alone is a floating face: this is the same real trigger the evil
        // biome's own third-orb-break uses (`smash_orb`) and the one a real summon item's packet
        // reaches (`on_summon`), so a bodyless Eater of Worlds or Destroyer here is not a cosmetic
        // gap — it is the whole fight missing, since both bosses' own real damage/behaviour depend
        // on having a body at all. `/spawn`'s own admin command already knew to do this for the
        // four ordinary worm monsters; this was the one real path that never did.
        let spawned = match terrustia_proto::npc_params::worm_body(npc_type) {
            Some((body, tail, segments)) => {
                self.npcs.spawn_worm(npc_type, body, tail, segments, at)
            }
            None => self.npcs.spawn(npc_type, at),
        };
        if let Some(index) = spawned {
            let name = self
                .npcs
                .get(index)
                .map(|n| n.stats.name)
                .unwrap_or("Something");
            // `Announcement.HasAwoken` is `"{0} has awoken!"`, and its argument is itself a
            // keyed text — the NPC's name. Our internal names are exactly the game's `NPCName.*`
            // keys (`npc_data.rs` calls it "the NPCID constant name"), so the two line up without
            // a translation table.
            let who = NetworkText::key(format!("NPCName.{name}"), Vec::new());
            self.announce_key("Announcement.HasAwoken", vec![who]);
            self.broadcast_npc(index);
            info!(slot, npc_type, name, "boss summoned");
        }
    }

    /// One slot of a player's inventory.
    ///
    /// The slot is remembered whatever it is — the server is the authority on what a player is
    /// carrying — but only the public ones are passed on. A player's safe is their own business.
    ///
    /// The owner byte the client sends is not trusted: it is overwritten with the slot the packet
    /// actually arrived on, which is what stops one client rewriting another's inventory.
    fn on_equipment(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let mut equipment = terrustia_proto::inventory::SyncEquipment::decode(payload)?;
        equipment.player = slot;
        if equipment.slot >= terrustia_proto::inventory::SLOT_COUNT {
            debug!(
                slot,
                requested = equipment.slot,
                "ignoring an out-of-range inventory slot"
            );
            return Ok(());
        }
        if let Some(player) = self.player_mut(slot) {
            player.inventory.insert(equipment.slot, equipment);
        }
        if terrustia_proto::inventory::relayed(equipment.slot) {
            self.broadcast(equipment.encode()?, Some(slot));
        }
        Ok(())
    }

    /// Tell one player what everybody else is carrying.
    ///
    /// Without this a player who joins a running server sees everyone else naked: the equipment
    /// packets went out before they arrived and are never repeated.
    fn send_existing_equipment(&mut self, to: u8) {
        let frames: Vec<Bytes> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.slot != to && p.is_playing())
            .flat_map(|p| p.inventory.values())
            .filter(|e| terrustia_proto::inventory::relayed(e.slot))
            .filter_map(|e| e.encode().ok())
            .map(Bytes::from)
            .collect();
        for frame in frames {
            self.send_bytes(to, frame);
        }
    }

    fn on_uuid(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let uuid = PacketReader::new(payload).string()?;
        if let Some(player) = self.player_mut(slot) {
            player.uuid = Some(uuid);
        }
        // The last of the three identities arrives here, so this is the first moment a UUID ban
        // can be enforced. Name and address are checked earlier, at the handshake; a UUID cannot
        // be, because packet 68 comes after the slot is already assigned.
        self.enforce_ban(slot);
        Ok(())
    }

    /// Turn somebody away if any of their three identities is banned.
    ///
    /// Name, address and client UUID. `Player::uuid` was stored by this server and read by nothing
    /// at all until now; this is what it was for.
    fn enforce_ban(&mut self, slot: u8) {
        let Some(player) = self.player(slot) else {
            return;
        };
        let (name, address) = (player.name.clone(), player.addr.ip().to_string());
        let uuid = player.uuid.clone();

        // The guest list first, when there is one. Checked here rather than earlier because it is
        // keyed by name, and the name only arrives with the player's appearance.
        if !self.admin.welcome(&name) {
            info!(slot, %name, %address, "refusing somebody not on the guest list");
            self.kick(slot, "You are not on this server's guest list.");
            return;
        }

        let Some(ban) = self.admin.ban_for(&name, &address, uuid.as_deref()) else {
            return;
        };
        let reason = ban.reason.clone();
        info!(slot, %name, %address, reason, "refusing a banned player");
        self.kick(slot, &format!("You are banned: {reason}"));
    }

    /// Count one tile edit against this player's spam budget, and say whether to stop.
    ///
    /// Vanilla's, transcribed from `RemoteClient`: a counter per kind, bumped once per edit
    /// packet, decayed every tick, and the connection booted past a ceiling. Placing is the
    /// tightest (100, decaying 0.3 a tick, so ~18 a second sustained); breaking is deliberately
    /// loose (500, decaying 5 a tick) because mining is fast and legitimate.
    ///
    /// Not having this at all was a regression *from* vanilla rather than a place where we simply
    /// match how trusting vanilla is — which is why it sits inside "match vanilla's trust model"
    /// rather than being the TShock-style validation that stays deferred.
    fn note_tile_spam(&mut self, slot: u8, kind: TileAction) -> bool {
        let (counter, ceiling, why): (fn(&mut Player) -> &mut f32, f32, &str) = match kind {
            TileAction::KillTile | TileAction::KillTileNoItem | TileAction::KillWall => (
                |p| &mut p.spam_break,
                SPAM_BREAK_MAX,
                "breaking tiles too fast",
            ),
            _ => (
                |p| &mut p.spam_place,
                SPAM_PLACE_MAX,
                "placing tiles too fast",
            ),
        };
        let Some(player) = self.player_mut(slot) else {
            return true;
        };
        let count = counter(player);
        *count += 1.0;
        if *count <= ceiling {
            return false;
        }
        // Vanilla boots with `Net.CheatingTileSpam`; the reason travels as our own text because
        // it is the server talking, not the game.
        info!(slot, why, "disconnecting a client for edit spam");
        self.kick(slot, why);
        true
    }

    /// Let every player's spam budget recover, once a tick.
    pub(super) fn tick_tile_spam(&mut self) {
        for player in self.players.iter_mut().flatten() {
            player.spam_place = (player.spam_place - SPAM_PLACE_DECAY).max(0.0);
            player.spam_break = (player.spam_break - SPAM_BREAK_DECAY).max(0.0);
            player.spam_liquid = (player.spam_liquid - SPAM_LIQUID_DECAY).max(0.0);
        }
    }

    fn on_tile_manipulation(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }

        let edit = TileManipulation::decode(payload)?;
        // Counted before anything else, so an edit that is refused still costs its budget. A
        // client hammering out-of-bounds coordinates is spamming just as hard as one hammering
        // valid ones.
        if self.note_tile_spam(slot, edit.kind()) {
            return Ok(());
        }
        let (x, y) = (i32::from(edit.x), i32::from(edit.y));
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }

        // Vanilla parity: `MessageBuffer.cs`'s packet-17 handler (`case 17`) starts its own local
        // `flag14` from the client's own claimed failure bit, then forces it to `true` — never back
        // to `false` — the moment the edit's section isn't in `Netplay.Clients[whoAmI].TileSections`
        // (`RemoteClient.cs:31`, the exact state `Player::sent_sections` already mirrors here). That
        // combined flag is what every `WorldGen.KillTile`/`KillWall` call in that packet passes as
        // its own `fail` argument: a client editing a tile it was never sent a section for still
        // gets the swing animation, but the edit itself never actually lands — the same shape the
        // check below reproduces by folding into `changed` rather than dropping the packet outright.
        // Relaying regardless (below, unconditional on `changed`) is also load-bearing and already
        // vanilla-shaped: it is exactly how vanilla's own "this edit failed" state reaches every
        // other client too, not something this check needs to special-case.
        let (sx, sy) = self.world.section_of(x, y);
        let section_owned = self
            .player(slot)
            .is_some_and(|p| p.sent_sections.contains(&(sx, sy)));

        let mut tile = self.world.tile(x, y);
        // Snapshotted before any match arm below touches it, so `/world undo` can put back the
        // tile's whole state — not just the field this particular edit happened to change.
        let before = tile;
        let mut changed = true;
        let mut broke = None;

        match edit.kind() {
            TileAction::KillTile | TileAction::KillTileNoItem => {
                // A pickaxe swing that only damages a block also arrives here; only a real break
                // clears the tile.
                if edit.destroyed() {
                    if tile.is_active() && matches!(edit.kind(), TileAction::KillTile) {
                        // The frames go with the block. Everything that decides what a broken
                        // object is worth — which evil an orb belongs to, which chair a chair is
                        // — lives in the frame, and the frame is about to be cleared.
                        broke = Some((tile.block, tile.frame_x, tile.frame_y));
                    }
                    tile.flags.set(TileFlags::ACTIVE, false);
                    tile.block = 0;
                    tile.frame_x = -1;
                    tile.frame_y = -1;
                    tile.slope = 0;
                    tile.flags.set(TileFlags::HALF_BRICK, false);
                } else {
                    changed = false;
                }
            }
            TileAction::PlaceTile => {
                let block = edit.arg.max(0) as u16;
                if frame_important(block) {
                    // Multi-tile objects need placement and framing rules the slice does not
                    // implement. The edit is still relayed so clients agree with each other.
                    debug!(slot, block, "not modelling framed tile placement");
                    changed = false;
                } else {
                    tile.block = block;
                    tile.frame_x = -1;
                    tile.frame_y = -1;
                    tile.flags.set(TileFlags::ACTIVE, true);
                    tile.flags.set(TileFlags::HALF_BRICK, false);
                    tile.slope = 0;
                }
            }
            TileAction::KillWall => {
                if edit.destroyed() {
                    tile.wall = 0;
                    tile.wall_color = 0;
                } else {
                    changed = false;
                }
            }
            TileAction::PlaceWall => tile.wall = edit.arg.max(0) as u16,
            TileAction::PoundTile => {
                // Hammering cycles a block through half-brick and the slopes; the client does the
                // same walk, so mirroring just the half-brick step keeps the common case right.
                tile.slope = 0;
                let half = tile.flags.has(TileFlags::HALF_BRICK);
                tile.flags.set(TileFlags::HALF_BRICK, !half);
            }
            TileAction::SlopeTile => {
                tile.slope = edit.arg.clamp(0, 4) as u8;
                tile.flags.set(TileFlags::HALF_BRICK, false);
            }
            TileAction::PlaceWire => tile.flags.set(TileFlags::WIRE_RED, true),
            TileAction::KillWire => tile.flags.set(TileFlags::WIRE_RED, false),
            TileAction::PlaceWire2 => tile.flags.set(TileFlags::WIRE_BLUE, true),
            TileAction::KillWire2 => tile.flags.set(TileFlags::WIRE_BLUE, false),
            TileAction::PlaceWire3 => tile.flags.set(TileFlags::WIRE_GREEN, true),
            TileAction::KillWire3 => tile.flags.set(TileFlags::WIRE_GREEN, false),
            TileAction::PlaceWire4 => tile.flags.set(TileFlags::WIRE_YELLOW, true),
            TileAction::KillWire4 => tile.flags.set(TileFlags::WIRE_YELLOW, false),
            TileAction::PlaceActuator => tile.flags.set(TileFlags::ACTUATOR, true),
            TileAction::KillActuator => tile.flags.set(TileFlags::ACTUATOR, false),
            TileAction::Other(action) => {
                debug!(slot, action, "unmodelled tile action; relaying only");
                changed = false;
            }
        }

        // Never turns a rejected edit back into an accepted one — only ever suppresses one the
        // match above already decided should apply, matching `flag14`'s own one-way OR in source.
        changed = changed && section_owned;
        if changed {
            self.world.set_tile(x, y, tile);
            // Mining a block is the commonest way liquid starts moving.
            self.liquids.disturb(x, y);
            if let Some(name) = self.player(slot).map(|p| p.name.clone()) {
                self.tile_log.record(x, y, before, &name);
            }
        }
        // Gated on `section_owned`, not `changed`: a rejected edit still leaves `broke` set to
        // whatever the match arm above decided a real kill would drop, and applying these side
        // effects (a real item drop, an altar smash, waking a boss) without the tile ever actually
        // having been removed is exactly the exploit this whole check exists to close.
        if section_owned && let Some((block, frame_x, frame_y)) = broke {
            self.spawn_tile_drop(block, frame_x, frame_y, x, y);
            // A demon altar is the only way hardmode ore gets into a world, and it always costs
            // something to break.
            if block == DEMON_ALTAR {
                self.smash_altar(x, y, slot);
            }
            // The handful of tiles that are worth more than the item they leave behind.
            if block == terrustia_proto::orbs::ORB_TILE {
                self.smash_orb(x, y, frame_x);
            }
            // Neither of these has a summon item: breaking the thing *is* the summon.
            if block == crate::world::bulbs::BULB {
                self.wake_from_tile(x, y, PLANTERA);
                // And another grows, so a world cannot be left with no way back to her.
                if !self.world.progress.downed_plantera {
                    self.grow_plantera_bulb();
                }
            }
            if block == BEE_LARVA {
                self.wake_from_tile(x, y, QUEEN_BEE);
            }
        }

        // Relay regardless: even an edit the server does not model must reach other clients, or
        // their view of the world silently diverges from the sender's.
        self.broadcast(edit.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 20: a rectangle of tiles pushed as one unit.
    ///
    /// Clients send this for anything spanning more than a single tile — furniture, trees, a door
    /// swinging open. Applying it is what keeps the server's world in step with multi-tile
    /// operations without reimplementing the game's placement and framing rules.
    fn on_tile_square(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }

        let square = TileSquare::decode(payload)?;
        let (x0, y0) = (i32::from(square.x), i32::from(square.y));

        // A square is at most 255 on a side, so a hostile one cannot cost much; still refuse any
        // that reaches outside the world rather than clamping it into somewhere unintended.
        if !self.world.in_bounds(x0, y0)
            || !self.world.in_bounds(
                x0 + i32::from(square.width) - 1,
                y0 + i32::from(square.height) - 1,
            )
        {
            debug!(
                slot,
                x = square.x,
                y = square.y,
                "tile square out of bounds"
            );
            return Ok(());
        }

        for dx in 0..usize::from(square.width) {
            for dy in 0..usize::from(square.height) {
                if let Some(tile) = square.tile(dx, dy) {
                    let (x, y) = (x0 + dx as i32, y0 + dy as i32);
                    self.world.set_tile(x, y, tile);
                    // Anything a client rewrites might have been holding liquid up.
                    self.liquids.disturb(x, y);
                }
            }
        }

        self.broadcast(square.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 19: a door opening or closing.
    ///
    /// The tile change itself is not modelled — a door swings between a 1x3 closed tile and a 2x3
    /// open one with recomputed frames, which is placement logic this server does not implement.
    /// Relaying keeps every client in agreement with the one that acted; the server's own copy of
    /// those tiles stays as it was until a client pushes a tile square over them.
    fn on_door(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let door = DoorToggle::decode(payload)?;
        if !self.world.in_bounds(i32::from(door.x), i32::from(door.y)) {
            return Ok(());
        }
        self.broadcast(door.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 31: a client asking to open a chest.
    fn on_chest_open(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let request = RequestChestOpen::decode(payload)?;

        let Some((id, chest)) = self.world.chest_at(request.x, request.y) else {
            return Ok(());
        };
        // Vanilla refuses a chest someone else is already inside, so two players cannot both edit
        // the same slots and clobber each other.
        if self
            .players
            .iter()
            .flatten()
            .any(|p| p.slot != slot && p.open_chest == id)
        {
            debug!(slot, chest = id, "chest is already open elsewhere");
            return Ok(());
        }

        let name = chest.name.clone();
        let (x, y, slots) = (chest.x, chest.y, chest.items.len());
        let items: Vec<_> = chest.items.clone();

        self.send(slot, objects::sync_chest_size(id, slots as i16)?);
        for (index, item) in items.iter().enumerate() {
            let frame = SyncChestItem {
                chest: id,
                slot: index as u8,
                item: *item,
            }
            .encode()?;
            self.send(slot, frame);
        }
        self.send(
            slot,
            SyncPlayerChest {
                chest: id,
                x,
                y,
                name: Some(name).filter(|n| !n.is_empty()),
            }
            .encode()?,
        );

        if let Some(player) = self.player_mut(slot) {
            player.open_chest = id;
        }
        Ok(())
    }

    /// Packet 130: an NPC pulled out of the water on a fishing line.
    ///
    /// Fishing is how three town slimes and a handful of enemies arrive, and the Red Slime is a
    /// permanent unlock — catching one for the first time means the world can spawn them from
    /// then on. Placing the NPC is the server's, since only it owns the roster.
    fn on_fished_out_npc(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.u16()?), i32::from(r.u16()?));
        let npc_type = r.i16()?;
        let Ok(npc_type) = u16::try_from(npc_type) else {
            return Ok(());
        };
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        // Only what a rod can actually bring up. Without this the packet is a free spawn of
        // anything in the game, Moon Lord included.
        if !is_fishable(npc_type) {
            debug!(slot, npc_type, "that is not something you can fish out");
            return Ok(());
        }

        let at = (
            x as f32 * crate::game::npc::TILE,
            y as f32 * crate::game::npc::TILE,
        );
        if let Some(index) = self.npcs.spawn(npc_type, at) {
            self.broadcast_npc(index);
            debug!(slot, npc_type, "fished out an npc");
        }
        Ok(())
    }

    /// Packet 140: the two town slimes that are made rather than found.
    ///
    /// A Copper Slime and an Old Slime are each transformed from another slime, once per world,
    /// and the transformation is a permanent unlock. Both have to be the server's: the unlock is
    /// world state and the transformation is a roster change.
    fn on_misc_event_value(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let what = r.u8()?;
        let value = r.i32()?;
        let Ok(index) = u8::try_from(value) else {
            return Ok(());
        };

        let (wanted, into) = match what {
            TRANSFORM_COPPER_SLIME => (None, COPPER_SLIME),
            TRANSFORM_ELDER_SLIME => (Some(OLD_SLIME_SOURCE), OLD_SLIME),
            // Case 0 is the credits roll's clock, which only ever goes the other way.
            _ => return Ok(()),
        };
        let Some(npc) = self.npcs.get(index) else {
            return Ok(());
        };
        if let Some(wanted) = wanted
            && npc.npc_type != wanted
        {
            return Ok(());
        }
        if let Some(npc) = self.npcs.get_mut(index) {
            npc.become_type(into);
            npc.dirty = true;
        }
        self.broadcast_npc(index);
        Ok(())
    }

    /// Packet 141: Lucy the Axe having something to say.
    ///
    /// Pure flavour, and relayed rather than modelled — but a talking axe only its owner can hear
    /// is a talking axe nobody believes in.
    fn on_lucy_popup(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let frame = packets::verbatim(id::REQUEST_LUCY_POPUP, payload)?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// Packet 85: quick stack — emptying an armful of loot into the chests it belongs in.
    ///
    /// The client offers a list of its own slots and the server decides where each goes, because
    /// only the server knows what is in chests nobody has opened and only the server can stop two
    /// players both being told the same slot was free.
    ///
    /// The client's word is taken for *which of its slots are eligible* — favourited items and
    /// coins are excluded, and that is the client's own bookkeeping — but never for where
    /// anything lands.
    fn on_quick_stack(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use crate::world::quick_stack::{self, Destination};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let count = r.i32()?;
        if !(0..=MAX_QUICK_STACK_SLOTS).contains(&count) {
            debug!(slot, count, "refusing an implausible quick stack");
            return Ok(());
        }
        let mut offered = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let which = r.i16()?;
            let Ok(which) = u16::try_from(which) else {
                continue;
            };
            // What the slot holds is the server's own record, not the client's claim.
            if let Some(held) = self
                .player(slot)
                .and_then(|p| p.inventory.get(&which))
                .map(|e| e.item)
                .filter(|i| !i.is_empty())
            {
                offered.push((which, held));
            }
        }
        let smart = r.bool().unwrap_or(false);
        let _ = smart; // the sorting mode; the plain rule is the same either way here

        let Some(from) = self.player(slot).map(|p| p.position) else {
            return Ok(());
        };
        // A chest somebody has open is off limits, which is what stops a quick stack landing in
        // the middle of somebody else's rummaging.
        let open: Vec<i16> = self
            .players
            .iter()
            .flatten()
            .filter(|p| p.slot != slot)
            .map(|p| p.open_chest)
            .filter(|id| *id >= 0)
            .collect();
        let mut destinations: Vec<Destination> = self
            .world
            .chests
            .iter()
            .enumerate()
            .filter_map(|(id, c)| c.as_ref().map(|c| (id as i16, c)))
            .map(|(id, c)| Destination {
                id,
                position: (
                    f32::from(c.x) * crate::game::npc::TILE + crate::game::npc::TILE,
                    f32::from(c.y) * crate::game::npc::TILE + crate::game::npc::TILE,
                ),
                items: c.items.clone(),
                blocked: open.contains(&id),
            })
            .collect();

        let outcome = quick_stack::run(from, &offered, &mut destinations);
        if outcome.moves.is_empty() && outcome.blocked.is_empty() {
            return Ok(());
        }

        // Write the results back and tell everybody: the chests to everyone, since anyone may
        // have one open, and the player's own slots to the player.
        for movement in &outcome.moves {
            if let Some(Some(chest)) = self
                .world
                .chests
                .get_mut(usize::try_from(movement.chest).unwrap_or(usize::MAX))
                && let Some(cell) = chest.items.get_mut(movement.chest_slot)
            {
                *cell = movement.chest_now;
            }
            let frame = SyncChestItem {
                chest: movement.chest,
                slot: movement.chest_slot as u8,
                item: movement.chest_now,
            }
            .encode()?;
            self.broadcast(frame, None);

            if let Some(player) = self.player_mut(slot)
                && let Some(held) = player.inventory.get_mut(&movement.from_slot)
            {
                held.item = movement.left_behind;
            }
        }
        // One equipment packet per slot that changed, rather than one per move: a stack split
        // across three chests changed once from the player's point of view.
        let mut told = std::collections::HashSet::new();
        for movement in &outcome.moves {
            if !told.insert(movement.from_slot) {
                continue;
            }
            if let Some(equip) = self
                .player(slot)
                .and_then(|p| p.inventory.get(&movement.from_slot))
                .copied()
                && let Ok(frame) = equip.encode()
            {
                self.broadcast(frame, None);
            }
        }

        // Which chests refused, so the client can mark them.
        if !outcome.blocked.is_empty() {
            let mut w = terrustia_proto::PacketWriter::new(id::QUICK_STACK_CHESTS);
            w.i32(outcome.blocked.len() as i32);
            for chest in &outcome.blocked {
                w.u16(*chest as u16);
            }
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }
        debug!(
            slot,
            moved = outcome.moves.len(),
            blocked = outcome.blocked.len(),
            "quick stack"
        );
        Ok(())
    }

    /// Packet 39: a player giving up their claim on an item.
    ///
    /// A dropped item is reserved for whoever is nearest so two players cannot both grab it. This
    /// is the other half of that: a player whose inventory is full, or who simply walked past,
    /// releases the claim so somebody else can have it. Without it a full player standing over a
    /// pile locks all of it for as long as they stand there.
    fn on_release_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let _force_to_server = r.bool()?;

        let Some(item) = self.items.get_mut(index) else {
            return Ok(());
        };
        // Only the holder may release it, or one client could free another's claim from under it.
        if item.owner != slot {
            return Ok(());
        }
        item.owner = items::NO_OWNER;
        item.reservation = 0;
        let position = item.position;
        // Told to everyone: the next tick will offer it to whoever is nearest, and until then no
        // client should believe it is spoken for.
        if let Ok(frame) = ItemOwner::reserve(index, items::NO_OWNER, position).encode() {
            self.broadcast(frame, None);
        }
        Ok(())
    }

    /// Packet 95: closing somebody else's portal.
    ///
    /// The Portal Gun's two ends are projectiles. Firing a third replaces the oldest, and the
    /// client that owns them says which one to close — because it is the one that knows which of
    /// its own pair is which.
    fn on_close_portal(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let owner = r.u16()?;
        let which = f32::from(r.u8()?);
        // The owner named has to be the sender, or one player could close another's portals.
        if usize::from(owner) != usize::from(slot) {
            return Ok(());
        }

        let found = self
            .projectiles
            .iter()
            .find(|(_, p)| {
                p.projectile_type == PORTAL_PROJECTILE && p.key.owner == slot && p.ai[1] == which
            })
            .map(|(index, p)| (index, p.key, p.position));
        let Some((index, key, position)) = found else {
            return Ok(());
        };
        self.projectiles.remove(index);
        let kill = terrustia_proto::projectile::KillProjectile { key, position };
        if let Ok(frame) = kill.encode() {
            self.broadcast(frame, None);
        }
        Ok(())
    }

    /// Packet 96: a player stepping through a portal.
    ///
    /// The client works out where it comes out — it knows where both ends are and how it entered
    /// — and the server's job is to agree and tell everybody else. Refusing would desync the one
    /// client that has already moved.
    fn on_portal_teleport(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let colour = r.i16()?;
        let (x, y) = r.vec2()?;
        let velocity = r.vec2()?;
        if !x.is_finite() || !y.is_finite() || !velocity.0.is_finite() || !velocity.1.is_finite() {
            return Ok(());
        }
        // A portal only reaches as far as its other end, which the server can bound even without
        // knowing where that is: nothing on the map is further than the map.
        if !self.world.in_bounds(
            (x / crate::game::npc::TILE) as i32,
            (y / crate::game::npc::TILE) as i32,
        ) {
            return Ok(());
        }
        if let Some(player) = self.player_mut(slot) {
            player.position = (x, y);
            player.velocity = velocity;
        }
        let mut w = terrustia_proto::PacketWriter::new(id::TELEPORT_PLAYER_THROUGH_PORTAL);
        w.u8(slot)
            .i16(colour)
            .f32(x)
            .f32(y)
            .f32(velocity.0)
            .f32(velocity.1);
        let frame = w.finish()?;
        self.broadcast(frame, Some(slot));
        Ok(())
    }

    /// Packet 102: a Nebula armour booster being picked up.
    ///
    /// Purely a relay — the effect is each client's own — but without it nobody else sees the
    /// burst, and a booster picked up in a group looks to everyone else like nothing happened.
    fn on_nebula_booster(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let _claimed = r.u8()?;
        let kind = r.u16()?;
        let at = r.vec2()?;
        let mut w = terrustia_proto::PacketWriter::new(id::NEBULA_LEVELUP_REQUEST);
        w.u8(slot).u16(kind).f32(at.0).f32(at.1);
        let frame = w.finish()?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 92: coins an NPC is carrying beyond its own worth.
    ///
    /// This is the Coin Loss revenge system: money dropped on death is remembered against
    /// whatever killed you, and killing that back gives it up. The server accumulates rather than
    /// overwrites, because two players can both feed the same enemy.
    fn on_extra_value(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let extra = r.i32()?;
        let at = r.vec2()?;
        let Ok(index) = u8::try_from(index) else {
            return Ok(());
        };
        let Some(npc) = self.npcs.get_mut(index) else {
            return Ok(());
        };
        npc.extra_value = npc.extra_value.saturating_add(extra);
        let total = npc.extra_value;
        let mut w = terrustia_proto::PacketWriter::new(id::SYNC_EXTRA_VALUE);
        w.i16(i16::from(index)).i32(total).f32(at.0).f32(at.1);
        let frame = w.finish()?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 143: a player asking the Old One's Army to send the next wave early.
    ///
    /// The gap between waves is generous on purpose, and skipping it is how a group that is
    /// ready gets on with it. Refused unless the event is actually waiting.
    fn on_skip_army_wait(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        if let Some(left) = self.army.skip_wait() {
            self.broadcast_army_wait(left);
            debug!(slot, "the next army wave was called early");
        }
        Ok(())
    }

    /// Tell clients how long is left before the next wave comes through the gates.
    ///
    /// The countdown on screen is this and nothing else. Without it the gap between waves is a
    /// blank pause of unknown length, which is exactly the part of the event a group needs to
    /// plan around.
    pub(super) fn broadcast_army_wait(&mut self, ticks: i32) {
        let mut w = terrustia_proto::PacketWriter::new(id::CRYSTAL_INVASION_SEND_WAIT_TIME);
        w.i32(ticks);
        if let Ok(frame) = w.finish() {
            self.broadcast(frame, None);
        }
    }

    /// Packet 144: the Dryad's little animation when a quest is handed in.
    ///
    /// Nothing but a flourish, and relayed rather than modelled — but a flourish only one client
    /// can see is worse than none.
    fn on_quest_effect(&mut self, slot: u8) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let frame = packets::empty(id::REQUEST_QUEST_EFFECT)?;
        self.broadcast(frame, None);
        Ok(())
    }

    /// Packet 109: the Grand Design, run along a path.
    ///
    /// Every wiring tool past the first works this way, and it has to be the server's job: the
    /// client does not know how much wire the player has left, and a run that stops halfway has
    /// to stop at the same tile for everybody or two players see different circuits.
    ///
    /// The reply is packet 110 — how much was actually spent — which is what stops a client
    /// believing it still has wire the server has already used.
    fn on_mass_wire(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        use crate::world::mass_wire::{self, Supplies, ToolMode};
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let from = (i32::from(r.i16()?), i32::from(r.i16()?));
        let to = (i32::from(r.i16()?), i32::from(r.i16()?));
        let mode = ToolMode(r.u8()?);
        if !mode.does_anything() {
            return Ok(());
        }

        // A drag across the whole world would be a denial of service dressed as a wiring tool.
        let span = (to.0 - from.0).abs().max((to.1 - from.1).abs());
        if span > MAX_WIRE_DRAG {
            debug!(slot, span, "refusing an implausibly long wire drag");
            return Ok(());
        }

        let Some(player) = self.player(slot) else {
            return Ok(());
        };
        let supplies = Supplies {
            wire: count_held(player, WIRE_ITEM),
            actuators: count_held(player, ACTUATOR_ITEM),
        };
        let facing_right = player.facing_right;

        let outcome = mass_wire::run(&mut self.world, from, to, mode, supplies, facing_right);
        for change in &outcome.changes {
            let edit = TileManipulation {
                action: change.action,
                x: change.x as i16,
                y: change.y as i16,
                arg: 0,
                style: 0,
            };
            if let Ok(frame) = edit.encode() {
                self.broadcast(frame, None);
            }
        }

        self.spawn_wire_drops(&outcome.drops);

        // Tell the player what it cost. Both are sent even when zero, as the game sends both.
        for (item, spent) in [
            (WIRE_ITEM, outcome.wire_spent),
            (ACTUATOR_ITEM, outcome.actuators_spent),
        ] {
            let mut w = terrustia_proto::PacketWriter::new(id::MASS_WIRE_OPERATION_PAY);
            w.i16(item).i16(spent as i16).u8(slot);
            if let Ok(frame) = w.finish() {
                self.send(slot, frame);
            }
        }
        debug!(
            slot,
            wire = outcome.wire_spent,
            actuators = outcome.actuators_spent,
            tiles = outcome.changes.len(),
            "mass wire operation"
        );
        Ok(())
    }

    /// Packet 69: a client asking what a chest is called.
    ///
    /// Sent for the map, which shows a chest's name without opening it. Answered to the asker
    /// alone, since another client that has not looked at the chest has no use for it.
    fn on_chest_name_request(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let claimed = r.i16()?;
        let (x, y) = (r.i16()?, r.i16()?);

        // A client may name the chest by id or ask the server to find it by position.
        let found = if claimed == -1 {
            self.world.chest_at(x, y)
        } else {
            self.world
                .chests
                .get(usize::try_from(claimed).unwrap_or(usize::MAX))
                .and_then(|c| c.as_ref())
                .filter(|c| c.x == x && c.y == y)
                .map(|c| (claimed, c))
        };
        let Some((id, chest)) = found else {
            return Ok(());
        };
        let mut w = terrustia_proto::PacketWriter::new(id::CHEST_NAME);
        w.i16(id).i16(x).i16(y).string(&chest.name);
        let frame = w.finish()?;
        self.send(slot, frame);
        Ok(())
    }

    /// Packet 105: locking or unlocking a gem lock.
    ///
    /// A tile toggle rather than a wiring one, but the effect is a circuit's: a locked gem lock
    /// is what a Chlorophyte Extractinator run is wired to.
    fn on_gem_lock(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut r = PacketReader::new(payload);
        let (x, y) = (i32::from(r.i16()?), i32::from(r.i16()?));
        let lock = r.bool()?;
        if !self.world.in_bounds(x, y) {
            return Ok(());
        }
        let tile = self.world.tile(x, y);
        if !tile.is_active() || tile.block != GEM_LOCK {
            return Ok(());
        }
        // A gem lock is a three-by-three object whose locked state lives in the frame's Y band:
        // the lower half of the sprite sheet is the locked form.
        let origin_y = tile.frame_y % GEM_LOCK_FRAME_HEIGHT;
        let wanted = if lock {
            origin_y + GEM_LOCK_FRAME_HEIGHT
        } else {
            origin_y
        };
        if tile.frame_y == wanted {
            return Ok(());
        }
        let (ox, oy) = (
            x - i32::from(tile.frame_x % 54) / 18,
            y - i32::from(origin_y) / 18,
        );
        for dx in 0..3 {
            for dy in 0..3 {
                let (tx, ty) = (ox + dx, oy + dy);
                let mut cell = self.world.tile(tx, ty);
                if !cell.is_active() || cell.block != GEM_LOCK {
                    continue;
                }
                cell.frame_y = if lock {
                    cell.frame_y % GEM_LOCK_FRAME_HEIGHT + GEM_LOCK_FRAME_HEIGHT
                } else {
                    cell.frame_y % GEM_LOCK_FRAME_HEIGHT
                };
                self.world.set_tile(tx, ty, cell);
            }
        }
        // Two tiles of reach covers the whole three-by-three from its centre.
        self.push_region(ox + 1, oy + 1, 2);
        Ok(())
    }

    /// Packet 32: a client changing one chest slot.
    fn on_chest_item(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let sync = SyncChestItem::decode(payload)?;

        // Only the player who has the chest open may change it.
        if self.player(slot).map(|p| p.open_chest) != Some(sync.chest) {
            debug!(
                slot,
                chest = sync.chest,
                "rejecting edit to a chest that is not open"
            );
            return Ok(());
        }

        let Some(chest) = self.world.chest_mut(sync.chest) else {
            return Ok(());
        };
        let Some(cell) = chest.items.get_mut(usize::from(sync.slot)) else {
            return Ok(());
        };
        *cell = sync.item;

        self.broadcast(sync.encode()?, Some(slot));
        Ok(())
    }

    /// Packet 33: a client reporting which chest it has open, including closing one.
    fn on_player_chest(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        let sync = SyncPlayerChest::decode(payload)?;
        if let Some(player) = self.player_mut(slot) {
            player.open_chest = sync.chest;
        }
        Ok(())
    }

    /// Packet 46: a client asking to read a sign.
    fn on_sign_request(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let request = RequestSign::decode(payload)?;
        let Some((id, sign)) = self.world.sign_at(request.x, request.y) else {
            return Ok(());
        };
        let frame = SignText {
            sign: id,
            x: sign.x,
            y: sign.y,
            text: sign.text.clone(),
            player: slot,
            editing: 0,
        }
        .encode()?;
        self.send(slot, frame);
        Ok(())
    }

    /// Packet 47: a client writing a sign.
    fn on_sign_write(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        if !self.player(slot).is_some_and(Player::is_playing) {
            return Ok(());
        }
        let mut update = SignText::decode(payload)?;
        if update.text.len() > MAX_SIGN_TEXT {
            debug!(slot, len = update.text.len(), "sign text too long");
            return Ok(());
        }

        let id = match self.world.sign_at(update.x, update.y) {
            Some((id, _)) => {
                if let Some(sign) = self.world.sign_mut(id) {
                    sign.text = update.text.clone();
                }
                id
            }
            None => {
                let sign = Sign {
                    x: update.x,
                    y: update.y,
                    text: update.text.clone(),
                };
                match self.world.add_sign(sign) {
                    Some(id) => id,
                    None => return Ok(()),
                }
            }
        };

        update.sign = id;
        update.player = slot;
        update.editing = 0;
        self.broadcast(update.encode()?, Some(slot));
        Ok(())
    }

    fn on_net_module(&mut self, slot: u8, payload: &[u8]) -> terrustia_proto::Result<()> {
        // Module 8: "take me to that pylon". Checked before chat because `IncomingChat::decode`
        // returns `None` for it and the request would otherwise be dropped on the floor.
        if let Some((message, pylon)) = net_module::decode_pylon_message(payload)?
            && message == net_module::PylonMessage::RequestTeleport
        {
            return self.on_pylon_teleport(slot, pylon);
        }

        // Module 4: Journey mode powers. Same reason as module 8 above — neither is chat, and
        // `IncomingChat::decode` would return `None` for it too, so it has to be checked first.
        if let Some(message) = net_module::decode_creative_power(payload)? {
            return self.on_creative_power(slot, message);
        }

        let Some(chat) = IncomingChat::decode(payload)? else {
            return Ok(());
        };
        if !self.player(slot).is_some_and(Player::is_playing) || !chat.is_say() {
            return Ok(());
        }
        if net_module::validate_chat(&chat.text, self.config.max_chat_len).is_err() {
            debug!(
                slot,
                len = chat.text.len(),
                "dropping out-of-range chat line"
            );
            return Ok(());
        }

        if let Some(command) = chat.text.strip_prefix('/') {
            return self.run_command(slot, command);
        }

        let name = self
            .player(slot)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        // Tagged so the web panel's live feed can tell an in-game chat line apart from an
        // operational one — both are `info!`, and only the target says which is which.
        info!(target: crate::term::CHAT_TARGET, "<{name}> {}", chat.text);

        // The text goes out bare, with the author's slot beside it. The client adds the name
        // itself — `ChatHelper.DisplayMessage` prefixes `Main.player[author].name` whenever the
        // author is a real slot — so a server that helpfully prefixes it too has every line
        // rendered with the speaker's name twice, and puts the tag inside the speech bubble over
        // their head as well. Found by asking a real server to relay a line and comparing: it
        // sends `"provoke: hello"` where this sent `"<provoke-actor> provoke: hello"`.
        //
        // The console line above keeps its own `<name>` because nothing is going to add one there.
        let frame = net_module::chat_broadcast(
            slot,
            &NetworkText::literal(chat.text.clone()),
            [255, 255, 255],
        )?;
        self.broadcast(frame, None);
        Ok(())
    }
}

/// Compare two byte strings without leaking their contents through timing.
///
/// A game password is hardly a high-value secret, but a length-independent compare costs nothing
/// and avoids having to argue about it.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Borrow helper: `introduce` needs the slot list detached from `self`.
fn other_slots(slots: &[u8]) -> Vec<u8> {
    slots.to_vec()
}

/// A join's own tile stream is spread across ticks (`drain_section_streams`) rather than sent in
/// one synchronous loop inside `on_spawn_tile_data`'s own packet handler — see
/// `SECTION_STREAM_BUDGET`'s own doc comment for the measured cost this bounds.
#[cfg(test)]
mod section_streaming {
    use super::*;
    use crate::config::Config;

    /// A real generated world, not an empty one: the measured section-encode costs
    /// `SECTION_STREAM_BUDGET` was set from (`examples/sectioncost.rs`, up to ~1,322µs on a
    /// comparable size) come from real terrain, and an empty world's sections would encode to
    /// almost nothing, proving nothing about pacing.
    fn real_world() -> crate::world::World {
        crate::world::worldgen::build(2400, 900, "section stream probe", 1234).0
    }

    /// A large outbound channel: this drains section frames without a live client reading them,
    /// and `send_bytes` drops (removes) a player whose channel fills up — sized well past
    /// anything one small world's own section count could produce.
    fn with_one_player(mut server: GameServer) -> (GameServer, mpsc::Receiver<Bytes>) {
        let (out_tx, out_rx) = mpsc::channel(100_000);
        let mut player = Player::new(0, "127.0.0.1:1".parse().unwrap(), out_tx);
        player.state = ConnState::WorldSent;
        server.players[0] = Some(player);
        (server, out_rx)
    }

    fn all_sections(server: &GameServer) -> VecDeque<(i32, i32)> {
        (0..server.world.sections_x())
            .flat_map(|sx| (0..server.world.sections_y()).map(move |sy| (sx, sy)))
            .collect()
    }

    /// The core architectural change: `SpawnTileData` used to stream every wanted section
    /// synchronously inside its own packet handler, reaching `TilesSent` in the same call. Now it
    /// only queues them — the state advance is deferred to whichever tick actually empties the
    /// queue.
    #[test]
    fn a_join_does_not_reach_tiles_sent_until_its_whole_section_queue_has_drained() {
        let (mut server, _rx) = with_one_player(GameServer::new(Config::default(), real_world()));

        // What a real client's handshake sends: `x = -1, y = -1` (no extra requested position),
        // `team = 0`.
        let mut payload = Vec::new();
        payload.extend_from_slice(&(-1i32).to_le_bytes());
        payload.extend_from_slice(&(-1i32).to_le_bytes());
        payload.push(0);

        server.on_spawn_tile_data(0, &payload).unwrap();

        assert!(
            !server.player(0).unwrap().pending_sections.is_empty(),
            "a real world's own spawn block should always want at least one section"
        );
        assert_eq!(
            server.player(0).unwrap().state,
            ConnState::WorldSent,
            "must not reach TilesSent before every queued section has actually gone out"
        );

        for _ in 0..10_000 {
            if server.player(0).unwrap().pending_sections.is_empty() {
                break;
            }
            server.drain_section_streams();
        }

        assert!(
            server.player(0).unwrap().pending_sections.is_empty(),
            "the queue never actually finished draining"
        );
        assert_eq!(
            server.player(0).unwrap().state,
            ConnState::TilesSent,
            "should reach TilesSent once the queue actually empties"
        );
    }

    /// The other half of the same change: draining is bounded per call, not "as many as fit."
    /// Queuing every section a real world has and draining once must leave some behind —
    /// regardless of how cheap any individual section happens to be, since there is always a
    /// number of them large enough to blow a four-millisecond budget.
    #[test]
    fn one_drain_call_never_empties_a_large_enough_queue() {
        let (mut server, _rx) = with_one_player(GameServer::new(Config::default(), real_world()));
        let all = all_sections(&server);
        let total = all.len();
        assert!(total > 8, "world too small to prove anything about pacing");
        server.player_mut(0).unwrap().pending_sections = all;

        server.drain_section_streams();

        let remaining = server.player(0).unwrap().pending_sections.len();
        assert!(
            remaining > 0,
            "all {total} sections drained in a single call — the budget bounded nothing"
        );
    }

    /// The budget is shared across every player streaming at once, not given to each one
    /// separately: two simultaneous joiners must not be able to drain roughly twice what one
    /// alone would in the same call — that would let a burst of simultaneous joins reproduce the
    /// exact stall this whole mechanism exists to prevent, just triggered by many joiners instead
    /// of one, exactly the scaling bug an earlier draft of this fix actually had (a `began`
    /// per player rather than per call).
    #[test]
    fn the_drain_budget_is_shared_across_players_not_given_to_each_one() {
        let (mut solo, _rx) = with_one_player(GameServer::new(Config::default(), real_world()));
        let queued = all_sections(&solo).len();
        solo.player_mut(0).unwrap().pending_sections = all_sections(&solo);
        solo.drain_section_streams();
        let solo_sent = queued - solo.player(0).unwrap().pending_sections.len();

        let (mut paired, _rx_a) = with_one_player(GameServer::new(Config::default(), real_world()));
        let (out_tx_b, _rx_b) = mpsc::channel(100_000);
        let mut player_b = Player::new(1, "127.0.0.1:2".parse().unwrap(), out_tx_b);
        player_b.state = ConnState::WorldSent;
        paired.players[1] = Some(player_b);
        paired.player_mut(0).unwrap().pending_sections = all_sections(&paired);
        paired.player_mut(1).unwrap().pending_sections = all_sections(&paired);

        paired.drain_section_streams();

        let paired_sent = (queued - paired.player(0).unwrap().pending_sections.len())
            + (queued - paired.player(1).unwrap().pending_sections.len());
        assert!(
            paired_sent <= solo_sent * 3 / 2,
            "two simultaneous joiners together drained {paired_sent} sections in one call, \
             vs {solo_sent} for one alone — the budget is being given to each player \
             separately instead of shared across the whole call"
        );
    }
}

/// Packet 55 (`AddPlayerBuffPvP`): a PvP-flagged player's own hit spreads one of `Main.pvpBuff`'s
/// twenty — now generated as `terrustia_proto::buffs::PVP_BUFF` — real buffs onto another
/// PvP-flagged player. Real vanilla's own server relays this to exactly the named target, not to
/// everyone, and gates it on both players being hostile-flagged and the buff itself being one of
/// the whitelisted twenty.
#[cfg(test)]
mod pvp_buff_spread {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "pvp buff spread probe")
    }

    /// Three players: the attacker, the real target, and a bystander who must never see this —
    /// proof that this is a targeted relay, not `relay_player_packet`'s usual broadcast.
    fn with_three_players(
        mut server: GameServer,
    ) -> (
        GameServer,
        mpsc::Receiver<Bytes>,
        mpsc::Receiver<Bytes>,
        mpsc::Receiver<Bytes>,
    ) {
        let (attacker_tx, attacker_rx) = mpsc::channel(16);
        let (target_tx, target_rx) = mpsc::channel(16);
        let (bystander_tx, bystander_rx) = mpsc::channel(16);
        for (slot, tx) in [(0, attacker_tx), (1, target_tx), (2, bystander_tx)] {
            let mut player = Player::new(slot, "127.0.0.1:1".parse().unwrap(), tx);
            player.state = ConnState::Playing;
            server.players[slot as usize] = Some(player);
        }
        (server, attacker_rx, target_rx, bystander_rx)
    }

    /// Poisoned (20) — one of the real twenty, transcribed directly from `Main.pvpBuff`, not
    /// guessed.
    const REAL_PVP_BUFF: u16 = 20;
    /// An ordinary, non-PvP-spreadable debuff — real vanilla's own `Main.debuff[21]`
    /// (PotionSickness), never added to `pvpBuff`.
    const ORDINARY_DEBUFF: u16 = 21;

    /// Just the payload `on_pvp_buff_spread` reads — no length prefix or message id, which is
    /// what a real dispatch has already stripped by the time a handler ever sees `payload`
    /// (`PacketWriter::finish` builds the whole framed packet instead, one layer too many here).
    fn packet_55(target: u8, buff: u16, duration: i32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(7);
        payload.push(target);
        payload.extend_from_slice(&buff.to_le_bytes());
        payload.extend_from_slice(&duration.to_le_bytes());
        payload
    }

    #[test]
    fn both_hostile_and_a_real_pvp_buff_reaches_only_the_named_target() {
        let (mut server, _attacker_rx, mut target_rx, mut bystander_rx) =
            with_three_players(GameServer::new(Config::default(), tiny_world()));
        server.player_mut(0).unwrap().pvp = true;
        server.player_mut(1).unwrap().pvp = true;

        let payload = packet_55(1, REAL_PVP_BUFF, 300);
        server.on_pvp_buff_spread(0, &payload).unwrap();

        assert!(
            target_rx.try_recv().is_ok(),
            "the named target should have received the relayed buff"
        );
        assert!(
            bystander_rx.try_recv().is_err(),
            "nobody else should ever see this — it is a targeted relay, not a broadcast"
        );
    }

    #[test]
    fn neither_player_needs_to_be_hostile_for_nothing_to_happen() {
        let (mut server, _attacker_rx, mut target_rx, _bystander_rx) =
            with_three_players(GameServer::new(Config::default(), tiny_world()));
        // Neither player.pvp is set — the ordinary, ungated case.
        let payload = packet_55(1, REAL_PVP_BUFF, 300);
        server.on_pvp_buff_spread(0, &payload).unwrap();

        assert!(
            target_rx.try_recv().is_err(),
            "a non-hostile attacker must not be able to spread a buff at all"
        );
    }

    #[test]
    fn a_buff_outside_the_real_whitelist_is_refused_even_with_both_hostile() {
        let (mut server, _attacker_rx, mut target_rx, _bystander_rx) =
            with_three_players(GameServer::new(Config::default(), tiny_world()));
        server.player_mut(0).unwrap().pvp = true;
        server.player_mut(1).unwrap().pvp = true;

        let payload = packet_55(1, ORDINARY_DEBUFF, 300);
        server.on_pvp_buff_spread(0, &payload).unwrap();

        assert!(
            target_rx.try_recv().is_err(),
            "only the real twenty `Main.pvpBuff` ids may be spread this way"
        );
    }
}

/// Does a client hammering tile edits get stopped, the way vanilla stops one?
///
/// Vanilla has this and we did not, which makes it a regression *from* the game rather than a
/// place where we are merely as trusting as it is. `RemoteClient` keeps a counter per kind, bumps
/// it per edit packet, decays it each tick and boots past a ceiling. The numbers are transcribed
/// rather than chosen, so a client vanilla tolerates is tolerated here and vice versa.
#[cfg(test)]
mod tile_spam {
    use super::*;

    /// Placing is the tight one: 100, recovering 0.3 a tick.
    #[test]
    fn the_ceilings_and_decay_match_the_game() {
        assert_eq!(SPAM_PLACE_MAX, 100.0);
        assert_eq!(SPAM_PLACE_DECAY, 0.3);
        assert_eq!(SPAM_BREAK_MAX, 500.0);
        assert_eq!(SPAM_BREAK_DECAY, 5.0);
        assert_eq!(SPAM_LIQUID_MAX, 50.0);
        assert_eq!(SPAM_LIQUID_DECAY, 0.2);
    }

    /// Sustained placing above the decay rate eventually trips; ordinary building does not.
    ///
    /// At 60 ticks a second, 0.3 a tick is eighteen placements a second recovered. A player
    /// building fast is well under that; a script is not.
    #[test]
    fn a_realistic_building_rate_never_trips_the_limit() {
        // Ten placements a second for a solid minute, decaying each tick.
        let mut budget = 0.0f32;
        let mut worst = 0.0f32;
        for tick in 0..3600 {
            if tick % 6 == 0 {
                budget += 1.0;
            }
            budget = (budget - SPAM_PLACE_DECAY).max(0.0);
            worst = worst.max(budget);
        }
        assert!(
            worst < SPAM_PLACE_MAX,
            "ten placements a second reached {worst}, which would boot a player who is just \
             building quickly"
        );
    }

    /// And a client placing as fast as it can trips within a few seconds.
    #[test]
    fn a_flood_of_placements_trips_the_limit() {
        let mut budget = 0.0f32;
        let mut tripped = None;
        for tick in 0..600 {
            // Twenty a tick — a script, not a person.
            budget += 20.0;
            budget = (budget - SPAM_PLACE_DECAY).max(0.0);
            if budget > SPAM_PLACE_MAX && tripped.is_none() {
                tripped = Some(tick);
            }
        }
        let tripped = tripped.expect("a flood has to trip the limit");
        assert!(
            tripped < 60,
            "a flood took {tripped} ticks to be noticed; that is a second of free vandalism"
        );
    }

    /// Breaking is deliberately looser, because mining legitimately produces packets very fast.
    ///
    /// A `const` block, so swapping the two by accident fails the build rather than a test run.
    const _: () = assert!(SPAM_BREAK_MAX > SPAM_PLACE_MAX);
    const _: () = assert!(SPAM_BREAK_DECAY > SPAM_PLACE_DECAY);
}
