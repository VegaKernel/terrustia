//! Terminal presentation: colour, the startup banner, and the log format.
//!
//! A server spends its life printing to somebody's terminal, so the printing is worth doing well.
//! This is a small ANSI layer rather than a dependency: the rules it follows are the usual ones —
//! colour only when the output is a terminal, `NO_COLOR` turns it off, `CLICOLOR_FORCE` turns it
//! back on, and `TERM=dumb` means plain text no matter what.

use std::{
    fmt::{self, Write as _},
    io::{IsTerminal, Write},
    time::Duration,
};

use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

/// SGR escapes, named for what they are used for rather than for their colour.
pub mod sgr {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const ITALIC: &str = "\x1b[3m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const BRIGHT_BLUE: &str = "\x1b[94m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
}

/// Whether this process should emit colour, decided once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    enabled: bool,
}

impl Palette {
    /// Work out from the environment whether colour is wanted.
    ///
    /// The precedence is the one every other tool uses: an explicit force wins, then an explicit
    /// disable, then whether there is actually a terminal on the other end.
    pub fn detect() -> Self {
        Self::decide(
            std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0"),
            std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            std::env::var("TERM").is_ok_and(|t| t == "dumb"),
            std::io::stdout().is_terminal(),
        )
    }

    /// The decision itself, separated from the environment so it can be tested.
    pub fn decide(forced: bool, no_color: bool, dumb: bool, is_tty: bool) -> Self {
        let enabled = if forced {
            true
        } else {
            !no_color && !dumb && is_tty
        };
        Self { enabled }
    }

    /// A palette that never emits escapes, for tests and for piped output.
    pub const PLAIN: Self = Self { enabled: false };

    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    /// Wrap `text` in `style`, or return it untouched when colour is off.
    pub fn paint(self, style: &str, text: &str) -> String {
        if self.enabled {
            format!("{style}{text}{}", sgr::RESET)
        } else {
            text.to_string()
        }
    }

    /// Emit a style escape, or nothing.
    fn on(self, style: &str) -> &str {
        if self.enabled { style } else { "" }
    }

    fn off(self) -> &'static str {
        if self.enabled { sgr::RESET } else { "" }
    }
}

/// How each level is shown: a fixed-width tag so messages line up in a column.
fn level_style(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::ERROR => ("ERROR", sgr::BRIGHT_RED),
        Level::WARN => ("WARN ", sgr::BRIGHT_YELLOW),
        Level::INFO => ("INFO ", sgr::BRIGHT_GREEN),
        Level::DEBUG => ("DEBUG", sgr::BRIGHT_BLUE),
        Level::TRACE => ("TRACE", sgr::MAGENTA),
    }
}

/// Collects an event's message and its other fields separately, because the message is prose and
/// the fields are data and they should not look alike.
#[derive(Default)]
struct Parts {
    message: String,
    fields: Vec<(&'static str, String)>,
}

impl Visit for Parts {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let mut rendered = String::new();
        let _ = write!(rendered, "{value:?}");
        let rendered = unquote(&rendered).to_string();
        if field.name() == "message" {
            self.message = rendered;
        } else {
            self.fields.push((field.name(), rendered));
        }
    }
}

/// Strip the quotes a string's `Debug` adds, leaving everything else alone.
///
/// Field values arrive already formatted, and a quoted world name in the middle of a log line is
/// noise. Only a value that both begins and ends with a quote is unwrapped, so a value that merely
/// contains one keeps it.
fn unquote(rendered: &str) -> &str {
    rendered
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(rendered)
}

/// The uptime-based clock shown on each line.
///
/// A wall clock in a log is for finding an event later; an uptime is for reading a running server.
/// Since this one is meant to be watched live, it shows how long the server has been up.
fn stamp(uptime: Duration, out: &mut String) {
    let secs = uptime.as_secs();
    let _ = write!(
        out,
        "{:02}:{:02}:{:02}.{:03}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        uptime.subsec_millis()
    );
}

/// Shorten `terrustia::game::server` to `game::server`: the crate name is on every line and never
/// tells anybody anything.
fn short_target(target: &str) -> &str {
    target.strip_prefix("terrustia::").unwrap_or(target)
}

/// A `tracing` layer that writes the lines this server prints.
pub struct TermLayer {
    palette: Palette,
    started: std::time::Instant,
}

impl TermLayer {
    pub fn new(palette: Palette) -> Self {
        Self {
            palette,
            started: std::time::Instant::now(),
        }
    }

    /// Render one event to a string. Split out from `on_event` so it can be tested without a
    /// subscriber.
    fn render(&self, level: Level, target: &str, parts: &Parts, uptime: Duration) -> String {
        let p = self.palette;
        let (tag, colour) = level_style(level);
        let mut line = String::with_capacity(128);

        line.push_str(p.on(sgr::DIM));
        stamp(uptime, &mut line);
        line.push_str(p.off());
        line.push(' ');

        line.push_str(p.on(colour));
        line.push_str(p.on(sgr::BOLD));
        line.push_str(tag);
        line.push_str(p.off());
        line.push(' ');

        line.push_str(p.on(sgr::DIM));
        let _ = write!(line, "{:<18}", short_target(target));
        line.push_str(p.off());
        line.push(' ');

        line.push_str(&parts.message);

        for (name, value) in &parts.fields {
            line.push(' ');
            line.push_str(p.on(sgr::DIM));
            line.push_str(name);
            line.push('=');
            line.push_str(p.off());
            line.push_str(p.on(sgr::CYAN));
            line.push_str(value);
            line.push_str(p.off());
        }
        line
    }
}

impl<S> Layer<S> for TermLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut parts = Parts::default();
        event.record(&mut parts);
        let meta = event.metadata();
        let line = self.render(*meta.level(), meta.target(), &parts, self.started.elapsed());
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
    }
}

/// The block printed before anything else happens.
pub fn banner(palette: Palette, version: &str, game: &str, protocol: u32) -> String {
    let p = palette;
    let art = [
        "█████ █████ ████  ████  █   █ █████ █████ █████ █████",
        "  █   █     █   █ █   █ █   █ █       █     █   █   █",
        "  █   ████  ████  ████  █   █ █████   █     █   █████",
        "  █   █     █  █  █  █  █   █     █   █     █   █   █",
        "  █   █████ █   █ █   █ █████ █████   █   █████ █   █",
    ];
    // Cyan fading into blue down the rows. 256-colour is thirty years old and universal enough
    // that a plain-colour fallback would only ever be a fallback nobody sees; `NO_COLOR` and a
    // pipe are the cases that actually matter, and those get no escapes at all.
    let ramp = [
        "\x1b[38;5;51m",
        "\x1b[38;5;45m",
        "\x1b[38;5;39m",
        "\x1b[38;5;33m",
        "\x1b[38;5;27m",
    ];

    let mut out = String::new();
    out.push('\n');
    for (line, colour) in art.iter().zip(ramp) {
        let _ = writeln!(out, "  {}{line}{}", p.on(colour), p.off());
    }
    let _ = writeln!(
        out,
        "  {}an async Terraria server{}   {}v{version}{}   {}Terraria {game} · protocol {protocol}{}\n",
        p.on(sgr::ITALIC),
        p.off(),
        p.on(sgr::BOLD),
        p.off(),
        p.on(sgr::DIM),
        p.off()
    );
    out
}

/// A titled box of `label: value` rows, used for the summaries a server prints once.
pub fn panel(palette: Palette, title: &str, rows: &[(&str, String)]) -> String {
    let p = palette;
    let widest_label = rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    let widest_value = rows
        .iter()
        .map(|(_, v)| v.chars().count())
        .max()
        .unwrap_or(0);
    // Two spaces of padding either side, plus the gap between the columns.
    let inner = (widest_label + widest_value + 3).max(title.chars().count() + 2);

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}╭─ {}{title}{}{} {}╮{}",
        p.on(sgr::DIM),
        p.off(),
        p.on(sgr::DIM),
        p.on(sgr::BOLD),
        "─".repeat(inner.saturating_sub(title.chars().count() + 2)),
        p.off()
    );
    for (label, value) in rows {
        let _ = writeln!(
            out,
            "{}│{} {}{label:<widest_label$}{}  {}{value:<widest_value$}{} {}│{}",
            p.on(sgr::DIM),
            p.off(),
            p.on(sgr::DIM),
            p.off(),
            p.on(sgr::BRIGHT_CYAN),
            p.off(),
            p.on(sgr::DIM),
            p.off(),
        );
    }
    let _ = writeln!(
        out,
        "{}╰{}╯{}",
        p.on(sgr::DIM),
        "─".repeat(inner + 1),
        p.off()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_follows_the_usual_environment_rules() {
        // A terminal with nothing set gets colour.
        assert!(Palette::decide(false, false, false, true).is_enabled());
        // A pipe does not.
        assert!(!Palette::decide(false, false, false, false).is_enabled());
        // NO_COLOR wins over a terminal.
        assert!(!Palette::decide(false, true, false, true).is_enabled());
        // A dumb terminal is not a terminal for this purpose.
        assert!(!Palette::decide(false, false, true, true).is_enabled());
        // CLICOLOR_FORCE wins over everything, which is how `| less -R` gets colour.
        assert!(Palette::decide(true, true, true, false).is_enabled());
    }

    #[test]
    fn a_plain_line_carries_no_escapes() {
        let layer = TermLayer::new(Palette::PLAIN);
        let parts = Parts {
            message: "world ready".into(),
            fields: vec![("width", "4200".into())],
        };
        let line = layer.render(
            Level::INFO,
            "terrustia::game::server",
            &parts,
            Duration::from_millis(65_432),
        );
        assert!(!line.contains('\x1b'), "got escapes in {line:?}");
        assert!(
            line.contains("00:01:05.432"),
            "uptime missing from {line:?}"
        );
        assert!(
            line.contains("game::server"),
            "target missing from {line:?}"
        );
        assert!(line.contains("world ready width=4200"), "got {line:?}");
    }

    #[test]
    fn a_coloured_line_resets_every_style_it_opens() {
        let layer = TermLayer::new(Palette::decide(true, false, false, false));
        let parts = Parts {
            message: "boom".into(),
            fields: vec![("slot", "3".into())],
        };
        let line = layer.render(Level::ERROR, "terrustia::net", &parts, Duration::ZERO);
        let opens = line.matches('\x1b').count();
        let resets = line.matches(sgr::RESET).count();
        // Every escape is either a style or its reset, and bold-over-colour opens two at once.
        assert!(opens > resets, "expected styles to be opened, got {line:?}");
        assert!(
            line.ends_with(sgr::RESET),
            "line must not leak style: {line:?}"
        );
        assert!(line.contains("ERROR"));
    }

    #[test]
    fn strings_lose_the_quotes_their_debug_adds() {
        assert_eq!(
            unquote(r#""The Successful Excrement""#),
            "The Successful Excrement"
        );
        // Numbers and everything else pass through.
        assert_eq!(unquote("4200"), "4200");
        // A quote in the middle is part of the value, not a wrapper.
        assert_eq!(unquote(r#"say "hi""#), r#"say "hi""#);
    }

    #[test]
    fn a_panel_is_rectangular() {
        let text = panel(
            Palette::PLAIN,
            "world",
            &[("name", "Test".into()), ("size", "4200 x 1200".into())],
        );
        let widths: Vec<usize> = text.lines().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged panel: {widths:?}\n{text}"
        );
    }
}
