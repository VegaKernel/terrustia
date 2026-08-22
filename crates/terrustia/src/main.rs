use std::{path::PathBuf, process::ExitCode, time::Instant};

use terrustia::{
    config::Config,
    game::{GameServer, ServerEvent},
    net::listener,
    term::{self, Palette},
    world::{wld, worldgen},
};
use tokio::{net::TcpListener, signal, sync::mpsc};
use tracing::{error, info};
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

    let started = Instant::now();
    let world = match &config.world_file {
        Some(path) => {
            info!(path = %path.display(), "loading world file");
            wld::load(path)?
        }
        None => worldgen::generate(
            config.world_width,
            config.world_height,
            config.world_name.clone(),
            config.seed,
        ),
    };
    let ready = term::panel(
        palette,
        "world",
        &[
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
        ],
    );
    print!("{ready}");

    // Bind before starting the game task so a port clash fails fast.
    let listener = TcpListener::bind(config.listen).await?;
    print!(
        "{}",
        term::panel(
            palette,
            "server",
            &[
                ("listening", config.listen.to_string()),
                ("players", format!("up to {}", config.max_players)),
                (
                    "log filter",
                    std::env::var("TERRUSTIA_LOG").unwrap_or_else(|_| "info".into())
                ),
            ],
        )
    );
    println!();

    let (events_tx, events_rx) = mpsc::channel::<ServerEvent>(EVENT_QUEUE);
    let game = tokio::spawn(GameServer::new(config.clone(), world).run(events_rx));
    let accept = tokio::spawn(listener::run(listener, config, events_tx.clone()));

    // Dropping the last sender is what tells the game task to stop.
    tokio::select! {
        _ = signal::ctrl_c() => info!("shutting down"),
        _ = game => info!("game task ended"),
    }

    accept.abort();
    drop(events_tx);
    Ok(())
}

struct Args {
    config: PathBuf,
    listen: Option<std::net::SocketAddr>,
    seed: Option<u64>,
    world: Option<PathBuf>,
    help: bool,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut parsed = Self {
            config: PathBuf::from("terrustia.toml"),
            listen: None,
            seed: None,
            world: None,
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
                    parsed.world = Some(args.next().ok_or("--world needs a path")?.into());
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
        Ok(parsed)
    }
}

/// The version of the game this server speaks to.
const GAME_VERSION: &str = "1.4.5.7";

fn print_usage(palette: Palette) {
    let heading = |text: &str| palette.paint(term::sgr::BOLD, text);
    let flag = |text: &str| palette.paint(term::sgr::BRIGHT_CYAN, text);
    let note = |text: &str| palette.paint(term::sgr::DIM, text);
    let options = [
        ("-c, --config <PATH>", "Config file", "terrustia.toml"),
        ("-l, --listen <ADDR>", "Address to bind", "0.0.0.0:7777"),
        (
            "-w, --world <PATH>",
            "Serve an existing .wld instead of generating",
            "",
        ),
        ("-s, --seed <NUMBER>", "World generation seed", "random"),
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
}
