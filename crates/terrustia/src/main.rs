use std::{path::PathBuf, process::ExitCode, time::Instant};

use terrustia::{
    config::Config,
    console,
    game::{GameServer, ServerEvent, Stopped},
    net::listener,
    term::{self, Palette},
    world::{wld, worldgen},
};
use tokio::{net::TcpListener, signal, sync::mpsc};
use tracing::{error, info, warn};
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

/// Events queued from all connections before the game task applies backpressure.
const EVENT_QUEUE: usize = 4096;

#[tokio::main]
async fn main() -> ExitCode {
    // `Targets` understands the same `terrustia=debug,info` syntax as `EnvFilter` but is a plain
    // prefix matcher, so it costs no regex engine.
    let filter = std::env::var("TERRUSTIA_LOG")
        .ok()
        .and_then(|spec| spec.parse::<Targets>().ok())
        .unwrap_or_else(|| Targets::new().with_default(tracing::Level::INFO));
    let palette = Palette::detect();
    tracing_subscriber::registry()
        .with(term::TermLayer::new(palette))
        .with(filter)
        .init();

    match run(palette).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(palette: Palette) -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse(std::env::args().skip(1))?;
    if args.help {
        print_usage(palette);
        return Ok(());
    }
    if args.list_worlds {
        print_worlds();
        return Ok(());
    }

    print!(
        "{}",
        term::banner(
            palette,
            env!("CARGO_PKG_VERSION"),
            GAME_VERSION,
            terrustia_proto::id::CUR_RELEASE
        )
    );

    let mut config = Config::load(&args.config)?;
    if let Some(listen) = args.listen {
        config.listen = listen;
    }
    if let Some(seed) = args.seed {
        config.seed = seed;
    }
    if let Some(world_file) = args.world {
        config.world_file = Some(world_file);
    }
    if let Some(name) = args.new_world {
        let destination = terrustia::worlds::new_world_path(&name)?;
        if destination.exists() {
            return Err(format!(
                "a world named \"{name}\" already exists at {} — pick another name, or serve it \
                 with --world {name}",
                destination.display()
            )
            .into());
        }
        // The world directory itself may not exist yet — nothing has ever saved there, which on
        // a fresh machine (nobody has run Terraria itself, or this is a fresh headless install)
        // is the ordinary case, not an error to stop on.
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "could not create the world directory {}: {e}",
                    parent.display()
                )
            })?;
        }
        config.world_name = name;
        config.save_file = Some(destination);
    }
    if let Some(save_file) = args.save {
        config.save_file = Some(save_file);
    }

    let started = Instant::now();
    let loaded_from = config.world_file.clone();
    let world = match &config.world_file {
        Some(path) => wld::load(path)?,
        None => worldgen::generate(
            config.world_width,
            config.world_height,
            config.world_name.clone(),
            config.seed,
        ),
    };
    let world_rows = [
        ("name", world.name.clone()),
        ("size", format!("{} x {}", world.width(), world.height())),
        ("spawn", format!("{}, {}", world.spawn_x, world.spawn_y)),
        (
            "evil",
            if world.crimson {
                "crimson"
            } else {
                "corruption"
            }
            .to_string(),
        ),
        ("chests", world.chests.len().to_string()),
        ("loaded in", format!("{} ms", started.elapsed().as_millis())),
    ];

    // Bind before starting the game task so a port clash fails fast.
    let listener = TcpListener::bind(config.listen).await?;
    let save_destination = config.save_target().map_or_else(
        || "none — this world will not be saved".to_string(),
        |p| p.display().to_string(),
    );
    let autosave_interval = if config.save_target().is_none() {
        "disabled (no save destination)".to_string()
    } else if config.autosave_secs == 0 {
        "disabled".to_string()
    } else {
        format!("every {}s", config.autosave_secs)
    };
    let server_rows = [
        ("listening", config.listen.to_string()),
        ("players", format!("up to {}", config.max_players)),
        (
            "log filter",
            std::env::var("TERRUSTIA_LOG").unwrap_or_else(|_| "info".into()),
        ),
        ("save destination", save_destination),
        ("autosave interval", autosave_interval),
    ];

    // Both panels are sized off the wider of the two, so they line up to the same gutter the log
    // lines below them use rather than each hugging its own content.
    let width =
        term::panel_width("world", &world_rows).max(term::panel_width("server", &server_rows));
    print!("{}", term::panel(palette, "world", &world_rows, width));
    if let Some(path) = &loaded_from {
        info!(path = %path.display(), "loading world file");
    }
    print!("{}", term::panel(palette, "server", &server_rows, width));
    println!();

    let recorder = match &args.record {
        Some(path) => Some(terrustia::net::record::Recorder::create(path)?),
        None => None,
    };

    let (events_tx, events_rx) = mpsc::channel::<ServerEvent>(EVENT_QUEUE);
    let mut game = tokio::spawn(GameServer::new(config.clone(), world).run(events_rx));

    // Foundation only — see `panel/mod.rs`'s module doc. Opt-in, so a bind failure here is a
    // configuration mistake worth failing loudly on rather than silently running without it.
    let _panel = if config.panel_enabled {
        Some(terrustia::panel::run(config.clone(), events_tx.clone()).await?)
    } else {
        None
    };

    let accept = tokio::spawn(listener::run(listener, config, events_tx.clone(), recorder));

    // Whoever has the terminal already has the world file, so the console is not gated. Reading
    // stdin has to be its own task: a blocking read would otherwise hold up the accept loop, and a
    // closed stdin (a service with no terminal) simply ends the task rather than the server.
    let console = console::spawn(events_tx.clone());

    // Dropping the last sender is what tells the game task to stop. The handle is borrowed rather
    // than moved so it is still here afterwards to be waited on.
    // A crash and a clean stop used to be indistinguishable from out here, so a server that had
    // panicked still exited 0 and no supervisor restarted it.
    let mut crashed = false;
    // `ended = &mut game` already resolves `game`'s `JoinHandle` when the game task stops on its
    // own (a console `stop`, among other things) — awaiting it again below would poll a
    // `JoinHandle` a second time after it already completed, which panics. This flag is `true`
    // only when the signal branch fired instead, which is the one case where `game` is still
    // pending and genuinely needs waiting on.
    let still_running = tokio::select! {
        reason = stop_signal() => {
            info!(reason, "shutting down");
            true
        }
        ended = &mut game => {
            match ended {
                Ok(Stopped::Cleanly) => info!("game task ended"),
                Ok(Stopped::Panicked) => {
                    error!("the game loop stopped because something panicked");
                    crashed = true;
                }
                Err(e) if e.is_cancelled() => info!("game task cancelled"),
                Err(e) => {
                    error!(error = %e, "the game task died");
                    crashed = true;
                }
            }
            false
        }
    };

    accept.abort();
    console.abort();
    drop(events_tx);
    // Wait for the game task to finish. It saves the world on its way out, and returning here
    // without waiting would drop the runtime mid-write — which is a shutdown that quietly loses
    // everything since the last autosave.
    if still_running {
        match game.await {
            Ok(Stopped::Panicked) => crashed = true,
            Ok(Stopped::Cleanly) => {}
            Err(e) if e.is_cancelled() => {}
            Err(e) => {
                error!(error = %e, "the game task did not shut down cleanly");
                crashed = true;
            }
        }
    }
    if crashed {
        // Non-zero, so `Restart=on-failure` and container restart policies actually fire.
        return Err("the server stopped because of a crash".into());
    }
    Ok(())
}

/// List the worlds Terraria has on this machine.
///
/// Enough to pick one by name without opening a file manager: the size the header claims, and how
/// recently it was played. Reading each header is a few hundred bytes and worth it — a list of
/// bare filenames does not tell you which of three saves is the one you want.
fn print_worlds() {
    let Some(dir) = terrustia::worlds::directory() else {
        println!("no world directory on this platform, or no home directory set");
        return;
    };
    let worlds = terrustia::worlds::list();
    if worlds.is_empty() {
        println!("no worlds in {}", dir.display());
        return;
    }
    println!("{}\n", dir.display());
    for path in worlds {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        let size = std::fs::metadata(&path).map_or(0, |m| m.len());
        // The dimensions come from the header, which is cheap to read and the only way to tell a
        // small world from a large one without opening it in the game.
        let dims = match wld::load(&path) {
            Ok(w) => format!("{} x {}", w.width(), w.height()),
            Err(_) => "unreadable".to_string(),
        };
        println!("  {name:<32} {dims:>12}   {:>6} MB", size / 1_048_576);
    }
    println!("\nserve one with:  terrustia --world <name>");
}

/// Wait for whichever signal asks the server to stop, and say which it was.
///
/// A process manager sends `SIGTERM`, not `SIGINT`: systemd, Docker and Kubernetes all stop a
/// service that way. Handling only Ctrl-C means every managed shutdown kills the server outright
/// and the world is lost back to its last autosave.
async fn stop_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal as unix_signal};
        let mut term = match unix_signal(SignalKind::terminate()) {
            Ok(s) => s,
            // Without a handler, Ctrl-C alone is better than refusing to start.
            Err(e) => {
                warn!(error = %e, "cannot listen for SIGTERM; only Ctrl-C will stop cleanly");
                let _ = signal::ctrl_c().await;
                return "ctrl-c";
            }
        };
        tokio::select! {
            _ = signal::ctrl_c() => "ctrl-c",
            _ = term.recv() => "SIGTERM",
        }
    }
    // Windows has no signals. It has three separate console control events, and a service or a
    // container stop sends one of the two that are *not* Ctrl-C — so listening for Ctrl-C alone
    // meant a managed shutdown skipped the save entirely and lost everything since the last
    // autosave. `ctrl_close` is the console window closing; `ctrl_shutdown` is the machine going
    // down. Both are worth catching, and both give only a short grace period, which is why the
    // shutdown save has to already be quick.
    #[cfg(windows)]
    {
        use tokio::signal::windows;

        let mut close = match windows::ctrl_close() {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "cannot listen for the close event; only Ctrl-C stops cleanly");
                let _ = signal::ctrl_c().await;
                return "ctrl-c";
            }
        };
        let mut shutdown = match windows::ctrl_shutdown() {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "cannot listen for the shutdown event");
                tokio::select! {
                    _ = signal::ctrl_c() => return "ctrl-c",
                    _ = close.recv() => return "console closing",
                }
            }
        };
        tokio::select! {
            _ = signal::ctrl_c() => "ctrl-c",
            _ = close.recv() => "console closing",
            _ = shutdown.recv() => "system shutting down",
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = signal::ctrl_c().await;
        "ctrl-c"
    }
}

struct Args {
    config: PathBuf,
    listen: Option<std::net::SocketAddr>,
    seed: Option<u64>,
    world: Option<PathBuf>,
    /// Generate a fresh world under this name, written into the Terraria world directory itself —
    /// so it shows up beside every other world, in the actual game, without anyone touching a
    /// file path at all.
    new_world: Option<String>,
    /// Where to write the world, for a generated one that has nowhere else to go.
    save: Option<PathBuf>,
    /// Where to record every byte of every connection, for checking against a real client.
    record: Option<PathBuf>,
    /// List the worlds Terraria has on this machine, and stop.
    list_worlds: bool,
    help: bool,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut parsed = Self {
            config: PathBuf::from("terrustia.toml"),
            listen: None,
            seed: None,
            world: None,
            new_world: None,
            save: None,
            record: None,
            list_worlds: false,
            help: false,
        };
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => parsed.help = true,
                "-c" | "--config" => {
                    parsed.config = args.next().ok_or("--config needs a path")?.into();
                }
                "-l" | "--listen" => {
                    let value = args.next().ok_or("--listen needs an address")?;
                    parsed.listen = Some(
                        value
                            .parse()
                            .map_err(|_| format!("not a socket address: {value}"))?,
                    );
                }
                "-w" | "--world" => {
                    let given = args.next().ok_or("--world needs a name or a path")?;
                    parsed.world = Some(terrustia::worlds::resolve(&given));
                }
                "-n" | "--new" => {
                    parsed.new_world = Some(args.next().ok_or("--new needs a world name")?);
                }
                "--worlds" => parsed.list_worlds = true,
                "--save" => {
                    parsed.save = Some(args.next().ok_or("--save needs a path")?.into());
                }
                "--record" => {
                    parsed.record = Some(args.next().ok_or("--record needs a path")?.into());
                }
                "-s" | "--seed" => {
                    let value = args.next().ok_or("--seed needs a number")?;
                    parsed.seed = Some(
                        value
                            .parse()
                            .map_err(|_| format!("not a number: {value}"))?,
                    );
                }
                other => return Err(format!("unrecognised argument: {other}")),
            }
        }
        if parsed.world.is_some() && parsed.new_world.is_some() {
            return Err("--world and --new cannot both be given — pick one world to serve".into());
        }
        Ok(parsed)
    }
}

/// The version of the game this server speaks to.
///
/// Both releases, in fact: 1.4.5.7 and 1.4.5.8 differ on the wire only in the number they announce
/// and in four bytes at the end of packet 7, and refusing the older one would strand anybody who
/// has not updated for no reason at all. See `id::SUPPORTED_RELEASES`.
const GAME_VERSION: &str = "1.4.5.8";

fn print_usage(palette: Palette) {
    let heading = |text: &str| palette.paint(term::sgr::BOLD, text);
    let flag = |text: &str| palette.paint(term::sgr::BRIGHT_CYAN, text);
    let note = |text: &str| palette.paint(term::sgr::DIM, text);
    let options = [
        ("-c, --config <PATH>", "Config file", "terrustia.toml"),
        ("-l, --listen <ADDR>", "Address to bind", "0.0.0.0:7777"),
        (
            "-w, --world <NAME|PATH>",
            "Serve an existing world, by name or by path",
            "",
        ),
        (
            "-n, --new <NAME>",
            "Generate a fresh world, saved into the Terraria world directory",
            "",
        ),
        (
            "    --worlds",
            "List the worlds Terraria has on this machine",
            "",
        ),
        (
            "    --save <PATH>",
            "Where to write the world; a loaded one saves back over itself",
            "",
        ),
        ("-s, --seed <NUMBER>", "World generation seed", "random"),
        (
            "    --record <PATH>",
            "Record every connection's bytes, for checking against a real client",
            "",
        ),
        ("-h, --help", "Show this message", ""),
    ];

    println!(
        "{} {}\n",
        heading("terrustia"),
        note(&format!("an async Terraria {GAME_VERSION} server"))
    );
    println!("{}\n    terrustia [OPTIONS]\n", heading("USAGE"));
    println!("{}", heading("OPTIONS"));
    for (name, what, default) in options {
        let tail = if default.is_empty() {
            String::new()
        } else {
            note(&format!(" [default: {default}]"))
        };
        println!("    {} {what}{tail}", flag(&format!("{name:<32}")));
    }
    println!("\n{}", heading("ENVIRONMENT"));
    println!(
        "    {} Log filter, e.g. debug or terrustia=debug",
        flag(&format!("{:<32}", "TERRUSTIA_LOG"))
    );
    println!(
        "    {} Turn colour off, or force it on through a pipe",
        flag(&format!("{:<32}", "NO_COLOR / CLICOLOR_FORCE"))
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_no_arguments_are_given() {
        let args = Args::parse(std::iter::empty()).unwrap();
        assert_eq!(args.config, PathBuf::from("terrustia.toml"));
        assert!(args.listen.is_none());
        assert!(!args.help);
    }

    #[test]
    fn flags_are_parsed() {
        let args = Args::parse(
            ["--listen", "127.0.0.1:1234", "--seed", "9", "-c", "x.toml"]
                .into_iter()
                .map(String::from),
        )
        .unwrap();
        assert_eq!(args.listen.unwrap().port(), 1234);
        assert_eq!(args.seed, Some(9));
        assert_eq!(args.config, PathBuf::from("x.toml"));
    }

    #[test]
    fn bad_input_is_reported_rather_than_ignored() {
        for bad in [
            vec!["--listen"],
            vec!["--listen", "not-an-address"],
            vec!["--seed", "abc"],
            vec!["--nonsense"],
        ] {
            assert!(
                Args::parse(bad.iter().map(|s| s.to_string())).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn new_world_name_is_parsed() {
        let args = Args::parse(["--new", "My Fork World"].into_iter().map(String::from)).unwrap();
        assert_eq!(args.new_world.as_deref(), Some("My Fork World"));
        assert!(args.world.is_none());
    }

    #[test]
    fn new_and_world_together_are_rejected() {
        assert!(Args::parse(["--new", "A", "--world", "B"].into_iter().map(String::from)).is_err());
    }

    #[test]
    fn new_needs_a_name() {
        assert!(Args::parse(["--new"].into_iter().map(String::from)).is_err());
    }
}
