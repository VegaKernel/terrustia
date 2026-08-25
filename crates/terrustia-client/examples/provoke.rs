//! Check what this client *sends* against a server that did not write it.
//!
//! ```sh
//! cargo run --release -p terrustia-client --example provoke -- 127.0.0.1:7930   # real Terraria
//! cargo run --release -p terrustia-client --example provoke -- 127.0.0.1:7777   # ours
//! ```
//!
//! ## The half `conform` cannot check
//!
//! `conform` reads a real server's bytes and proves this project can decode them. That covers
//! everything the server says and nothing the client says, and the two are separate risks: a field
//! written at the wrong offset in an *outgoing* packet is invisible to any amount of decoding, and
//! shows up only as a real server quietly ignoring the request — which looks exactly like a server
//! that received nothing.
//!
//! So this asks the other question: **does a server that owes nothing to this code act on what we
//! send it?**
//!
//! ## Two clients, because a server does not echo to the sender
//!
//! Terraria relays a tile edit with `TrySendData(17, -1, whoAmI, ...)` — everyone *except* the
//! client that sent it, which has already made the change locally. A probe that acts and then
//! waits for its own edit to come back therefore reports "ignored" for a request the server
//! handled perfectly. (It does exactly that; the first version of this file made the mistake and
//! the real server was blamed for it.)
//!
//! So one client acts and a second watches. The watcher seeing the consequence is proof the server
//! understood the actor — which is the thing being tested.
//!
//! Running it against both servers and comparing is the point. A row that fails on **both** is a
//! shared misreading in `terrustia-proto`, which is the whole class of bug that testing our client
//! against our server cannot see.
//!
//! Deliberately destructive in a small way: it mines a tile and puts one back. Point it at a copy
//! of a world, never at one anybody cares about.

use std::{net::SocketAddr, process::ExitCode, time::Duration};

use terrustia_client::{Client, Event};
use terrustia_proto::id;

/// How long to wait for any one consequence before calling it absent.
const PATIENCE: Duration = Duration::from_secs(3);

struct Outcome {
    what: &'static str,
    happened: bool,
    detail: String,
}

fn main() -> ExitCode {
    let Some(addr) = std::env::args().nth(1) else {
        eprintln!("usage: provoke <host:port>");
        return ExitCode::FAILURE;
    };
    let Ok(addr) = addr.parse::<SocketAddr>() else {
        eprintln!("not a socket address: {addr}");
        return ExitCode::FAILURE;
    };

    let runtime = tokio::runtime::Runtime::new().expect("a tokio runtime");
    match runtime.block_on(run(addr)) {
        Ok(outcomes) => {
            println!("\n{:<32} {:<8} what the watcher saw", "action", "result");
            let failed = outcomes.iter().filter(|o| !o.happened).count();
            for outcome in &outcomes {
                println!(
                    "{:<32} {:<8} {}",
                    outcome.what,
                    if outcome.happened { "ok" } else { "IGNORED" },
                    outcome.detail
                );
            }
            println!(
                "\n{} of {} actions were acted on",
                outcomes.len() - failed,
                outcomes.len()
            );
            if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("provoke: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(addr: SocketAddr) -> Result<Vec<Outcome>, Box<dyn std::error::Error>> {
    let mut actor = Client::join(addr, "provoke-actor").await?;
    let mut watcher = Client::join(addr, "provoke-watcher").await?;
    actor.set_timeout(Duration::from_secs(10));
    watcher.set_timeout(Duration::from_secs(10));
    println!(
        "joined {:?} {}x{} spawn {},{}",
        actor.world().name,
        actor.world().width,
        actor.world().height,
        actor.world().spawn.0,
        actor.world().spawn.1,
    );

    let mut outcomes = Vec::new();
    let (spawn_x, spawn_y) = actor.world().spawn;

    // ---- asking for a section ------------------------------------------------------------------
    //
    // Asked for by number rather than by walking, because `walk_to_tile` only requests what it does
    // not already hold — and the sections around spawn arrive during the handshake, so a walk near
    // spawn sends nothing at all and would report "ignored" for a request never made.
    let far_section = (
        (i32::from(spawn_x) / 200 + 6).clamp(0, actor.world().width / 200 - 1) as u16,
        (i32::from(spawn_y) / 150 + 2).clamp(0, actor.world().height / 150 - 1) as u16,
    );
    actor.request_section(far_section.0, far_section.1).await?;
    let arrived = actor
        .try_wait_for(
            "a section",
            |event| matches!(event, Event::SectionLoaded { .. }),
            PATIENCE,
        )
        .await;
    outcomes.push(Outcome {
        what: "request a distant section",
        happened: arrived.is_some(),
        detail: match &arrived {
            Some(Event::SectionLoaded {
                section_x,
                section_y,
            }) => format!("section ({section_x}, {section_y})"),
            _ => format!("nothing came back for section {far_section:?}"),
        },
    });

    // ---- chat ---------------------------------------------------------------------------------
    actor.say("provoke: hello").await?;
    let heard = watcher
        .try_wait_for(
            "the line",
            |event| matches!(event, Event::Chat { text, .. } if text.contains("provoke: hello")),
            PATIENCE,
        )
        .await;
    outcomes.push(Outcome {
        what: "say something",
        happened: heard.is_some(),
        detail: match &heard {
            Some(Event::Chat { author, text }) => format!("from {author}: {text:?}"),
            _ => "the other client never heard it".into(),
        },
    });

    // ---- moving --------------------------------------------------------------------------------
    let to = (i32::from(spawn_x) + 20, i32::from(spawn_y));
    actor
        .move_to(to.0 as f32 * 16.0, to.1 as f32 * 16.0)
        .await?;
    let seen = watcher
        .try_wait_for(
            "the move",
            |event| matches!(event, Event::PlayerMoved { slot, .. } if *slot == 0 || *slot == 1),
            PATIENCE,
        )
        .await;
    outcomes.push(Outcome {
        what: "report a position",
        happened: seen.is_some(),
        detail: match &seen {
            Some(Event::PlayerMoved { slot, x, y }) => format!("slot {slot} at {x:.0},{y:.0}"),
            _ => "the other client never saw them move".into(),
        },
    });

    // ---- mining and building --------------------------------------------------------------------
    let dig = find_solid_ground(&actor, spawn_x, spawn_y);
    match dig {
        Some((x, y, block)) => {
            actor.break_tile(x, y).await?;
            let broke = watcher
                .try_wait_for(
                    "the edit",
                    |event| matches!(event, Event::TileChanged(edit) if edit.x == x && edit.y == y),
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "mine a tile",
                happened: broke.is_some(),
                detail: match &broke {
                    Some(Event::TileChanged(edit)) => {
                        format!("action {} at {},{}", edit.action, edit.x, edit.y)
                    }
                    _ => format!("no edit reached the watcher for {x},{y}"),
                },
            });

            // Put it back, which is both good manners and a second, different action.
            actor.place_tile(x, y, block).await?;
            let placed = watcher
                .try_wait_for(
                    "the placement",
                    |event| matches!(event, Event::TileChanged(edit) if edit.x == x && edit.y == y),
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "place a tile",
                happened: placed.is_some(),
                detail: match &placed {
                    Some(Event::TileChanged(edit)) => {
                        format!("action {} at {},{}", edit.action, edit.x, edit.y)
                    }
                    _ => "the block was not put back".into(),
                },
            });
        }
        None => {
            for what in ["mine a tile", "place a tile"] {
                outcomes.push(Outcome {
                    what,
                    happened: false,
                    detail: "no plain ground found near spawn to try it on".into(),
                });
            }
        }
    }

    // ---- walls ----------------------------------------------------------------------------------
    //
    // A separate action from a block, with its own two ids, and one that would go unnoticed if the
    // block path were the only thing checked.
    let wall_at = find_wall(&actor, spawn_x, spawn_y);
    match wall_at {
        Some((x, y, wall)) => {
            actor.break_wall(x, y).await?;
            let broke = watcher
                .try_wait_for(
                    "the wall edit",
                    |event| matches!(event, Event::TileChanged(edit) if edit.x == x && edit.y == y),
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "break a wall",
                happened: broke.is_some(),
                detail: match &broke {
                    Some(Event::TileChanged(edit)) => format!("action {}", edit.action),
                    _ => format!("no edit reached the watcher for {x},{y}"),
                },
            });

            actor.place_wall(x, y, wall).await?;
            let put_back = watcher
                .try_wait_for(
                    "the wall back",
                    |event| matches!(event, Event::TileChanged(edit) if edit.x == x && edit.y == y),
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "place a wall",
                happened: put_back.is_some(),
                detail: match &put_back {
                    Some(Event::TileChanged(edit)) => format!("action {}", edit.action),
                    _ => "the wall was not put back".into(),
                },
            });
        }
        None => {
            for what in ["break a wall", "place a wall"] {
                outcomes.push(Outcome {
                    what,
                    happened: false,
                    detail: "no walled tile found near spawn".into(),
                });
            }
        }
    }

    // ---- dropping an item -------------------------------------------------------------------------
    //
    // The server assigns the entity its index, so the watcher seeing *any* new item is the signal.
    let drop_at = (f32::from(spawn_x) * 16.0, (f32::from(spawn_y) - 4.0) * 16.0);
    actor
        .drop_item(terrustia_proto::ItemStack::new(3507, 1, 0), drop_at)
        .await?;
    let landed = watcher
        .try_wait_for(
            "the item",
            |event| matches!(event, Event::ItemSynced(item) if item.item.stack > 0),
            PATIENCE,
        )
        .await;
    outcomes.push(Outcome {
        what: "drop an item",
        happened: landed.is_some(),
        detail: match &landed {
            Some(Event::ItemSynced(item)) => {
                format!("index {} holding {}", item.index, item.item.id)
            }
            _ => "the other client never saw it land".into(),
        },
    });

    // ---- hitting an NPC ---------------------------------------------------------------------------
    //
    // Any live one will do; the signal is the server sending its state back with less life than it
    // had. Town NPCs included — the game lets a player hit them, and this is only one blow.
    //
    // Read from the client's roster rather than sampled from a window of events. The two servers
    // sync NPCs on different cadences — the real one keeps re-sending them, ours only when one
    // changes — so any fixed listening window reports a difference in cadence as a difference in
    // whether NPCs exist. The roster is what a client actually knows.
    let victim = actor
        .world()
        .npcs()
        .find(|npc| npc.life > 1)
        .map(|npc| (npc.index, npc.generation, npc.life));
    match victim {
        Some((index, generation, before)) => {
            actor.hit_npc(index, generation, 1, 0.0, 1).await?;
            let hurt = watcher
                .try_wait_for(
                    "the NPC's new state",
                    |event| {
                        matches!(event, Event::NpcSynced(npc)
                            if npc.index == index && npc.life < before)
                    },
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "hit an NPC",
                happened: hurt.is_some(),
                detail: match &hurt {
                    Some(Event::NpcSynced(npc)) => {
                        format!("NPC {index} went from {before} to {}", npc.life)
                    }
                    _ => format!(
                        "NPC {index} (gen {generation}) was on {before} life and never lost any"
                    ),
                },
            });
        }
        None => outcomes.push(Outcome {
            what: "hit an NPC",
            happened: false,
            detail: "no living NPC in view, so this was not tried".into(),
        }),
    }

    // ---- doors ------------------------------------------------------------------------------------
    let door = find_door(&actor);
    match door {
        Some((x, y)) => {
            actor.walk_to_tile(i32::from(x), i32::from(y)).await?;
            actor.toggle_door(0, x, y, 1).await?;
            let swung = watcher
                .try_wait_for(
                    "the door",
                    |event| {
                        matches!(event, Event::Other(frame)
                            if frame.id == id::TOGGLE_DOOR_STATE || frame.id == id::AREA_TILE_CHANGE)
                    },
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "open a door",
                happened: swung.is_some(),
                detail: match &swung {
                    Some(Event::Other(frame)) => {
                        format!("packet {} ({})", frame.id, id::name(frame.id))
                    }
                    _ => format!("the door at {x},{y} did not move"),
                },
            });
        }
        None => outcomes.push(Outcome {
            what: "open a door",
            happened: false,
            detail: "no door in the sections loaded, so this was not tried".into(),
        }),
    }

    // ---- signs ------------------------------------------------------------------------------------
    //
    // Answered to the asker alone, like a chest.
    let sign = actor.world().signs().map(|s| (s.x, s.y)).next();
    match sign {
        Some((x, y)) => {
            actor.walk_to_tile(i32::from(x), i32::from(y)).await?;
            actor.read_sign(x, y).await?;
            let read = actor
                .try_wait_for(
                    "the sign's text",
                    |event| {
                        matches!(event, Event::Other(frame) if frame.id == id::OPEN_SIGN_RESPONSE)
                    },
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "read a sign",
                happened: read.is_some(),
                detail: match &read {
                    Some(Event::Other(frame)) => format!("{} bytes back", frame.payload.len()),
                    _ => format!("no answer for the sign at {x},{y}"),
                },
            });
        }
        None => outcomes.push(Outcome {
            what: "read a sign",
            happened: false,
            detail: "no sign in the sections loaded, so this was not tried".into(),
        }),
    }

    // ---- chests ---------------------------------------------------------------------------------
    //
    // Answered to the asker alone, so this one is watched on the actor's own socket.
    let chest = actor.world().chests().map(|c| (c.x, c.y)).next();
    match chest {
        Some((x, y)) => {
            actor.walk_to_tile(i32::from(x), i32::from(y)).await?;
            actor.open_chest(x, y).await?;
            let opened = actor
                .try_wait_for(
                    "the chest",
                    |event| {
                        matches!(event, Event::Other(frame)
                            if frame.id == id::SYNC_CHEST_ITEM
                                || frame.id == id::SYNC_PLAYER_CHEST)
                    },
                    PATIENCE,
                )
                .await;
            outcomes.push(Outcome {
                what: "open a chest",
                happened: opened.is_some(),
                detail: match &opened {
                    Some(Event::Other(frame)) => {
                        format!("packet {} ({})", frame.id, id::name(frame.id))
                    }
                    _ => format!("no answer for the chest at {x},{y}"),
                },
            });
        }
        None => outcomes.push(Outcome {
            what: "open a chest",
            happened: false,
            detail: "no chest in the sections loaded, so this was not tried".into(),
        }),
    }

    Ok(outcomes)
}

/// Find a tile near spawn with a wall behind it, and say which wall.
fn find_wall(client: &Client, spawn_x: i16, spawn_y: i16) -> Option<(i16, i16, u16)> {
    for dy in 0..60i16 {
        for dx in [0i16, -2, 2, -5, 5] {
            let (x, y) = (spawn_x + dx, spawn_y + dy);
            if let Some(tile) = client.world().tile(i32::from(x), i32::from(y))
                && tile.wall != 0
            {
                return Some((x, y, tile.wall));
            }
        }
    }
    None
}

/// Find a closed door in whatever the client has loaded.
///
/// Tile 10 is a door; 11 is the same door already open, which would make "open it" a no-op and
/// report as ignored for a request the server was right to refuse.
fn find_door(client: &Client) -> Option<(i16, i16)> {
    client
        .world()
        .known_tiles()
        .find(|(_, _, tile)| tile.is_active() && tile.block == 10)
        .map(|(x, y, _)| (x as i16, y as i16))
}

/// Find a plain block near spawn worth mining, and say what it was.
///
/// Dirt, stone or grass only: something with no frame to get wrong and nothing attached that would
/// make the edit mean something other than "remove this block".
fn find_solid_ground(client: &Client, spawn_x: i16, spawn_y: i16) -> Option<(i16, i16, u16)> {
    for dy in 0..60i16 {
        for dx in [0i16, -1, 1, -3, 3] {
            let (x, y) = (spawn_x + dx, spawn_y + dy);
            if let Some(tile) = client.world().tile(i32::from(x), i32::from(y))
                && tile.is_active()
                && matches!(tile.block, 0..=2)
            {
                return Some((x, y, tile.block));
            }
        }
    }
    None
}
