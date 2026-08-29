//! Terminal presentation: colour, the startup banner, and the log format.
//!
//! A server spends its life printing to somebody's terminal, so the printing is worth doing well.
//! This is a small ANSI layer rather than a dependency: the rules it follows are the usual ones —
//! colour only when the output is a terminal, `NO_COLOR` turns it off, `CLICOLOR_FORCE` turns it
//! back on, and `TERM=dumb` means plain text no matter what.

use std::{
    fmt::{self, Write as _},
    io::{IsTerminal, Write},
    sync::{Mutex, OnceLock},
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

    /// Render a player's chat line: an uptime and a coloured `CHAT` tag, then the message — no
    /// level and no target column, so a conversation reads as a conversation rather than as a
    /// column of identical `INFO … chat` operational events.
    fn render_chat(&self, parts: &Parts, uptime: Duration) -> String {
        let p = self.palette;
        let mut line = String::with_capacity(64);
        line.push_str(p.on(sgr::DIM));
        stamp(uptime, &mut line);
        line.push_str(p.off());
        line.push(' ');
        line.push_str(p.on(sgr::BRIGHT_CYAN));
        line.push_str(p.on(sgr::BOLD));
        line.push_str("CHAT ");
        line.push_str(p.off());
        line.push(' ');
        line.push_str(&parts.message);
        line
    }

    /// Render a console command's own reply: no timestamp, no level tag, no target column — the
    /// furniture that makes sense for a stream of log events reads as noise around the one line
    /// somebody just asked for by typing a command. A REPL prints its own output plainly; this is
    /// that output, tagged by `run_console` with `target: "console_reply"` rather than routed
    /// through a second, parallel print path.
    fn render_reply(&self, parts: &Parts) -> String {
        let p = self.palette;
        let mut line = String::with_capacity(64);
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

/// The tag `run_console` gives its own replies, so they can be told apart from ordinary log
/// events. Public so `game::server` can reuse the exact string rather than retyping it.
pub const CONSOLE_REPLY_TARGET: &str = "console_reply";

/// The tag a player's ordinary chat line is logged under, so it can be told apart from an
/// operational log line carrying the same level. Public for the same reason as
/// [`CONSOLE_REPLY_TARGET`]: `game::server` sets it, this module reads it back.
pub const CHAT_TARGET: &str = "chat";

/// One line for the web panel's live feed: the terminal's own output, mirrored off the same
/// `tracing` events, without the ANSI a browser has no use for.
///
/// This exists so the panel does not need its own parallel logging path — every console reply,
/// every ordinary log line and every line of in-game chat already flows through
/// [`TermLayer::on_event`] once; this just also hands a plain copy to anyone listening.
#[derive(Debug, Clone)]
pub struct ConsoleLine {
    pub kind: ConsoleLineKind,
    /// `"INFO"`, `"WARN"`, `"ERROR"` and so on; empty for a console reply or a chat line, which
    /// have no level of their own.
    pub level: &'static str,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLineKind {
    Log,
    Reply,
    Chat,
}

/// A ring-bounded broadcast of every line this process has printed since the panel first asked.
///
/// `tokio::sync::broadcast` with no subscribers is cheap to send into — the point of a `OnceLock`
/// here rather than plumbing a sender through every call site is that most runs of this server
/// never start the panel at all, and this must cost nothing when nobody is listening. 500 lines is
/// a few minutes of an active server's chat and console traffic, which is enough for a panel that
/// only ever attaches live rather than asking for history.
static CONSOLE_FEED: OnceLock<tokio::sync::broadcast::Sender<ConsoleLine>> = OnceLock::new();

pub fn console_feed() -> &'static tokio::sync::broadcast::Sender<ConsoleLine> {
    CONSOLE_FEED.get_or_init(|| tokio::sync::broadcast::channel(500).0)
}

/// The live furniture the terminal keeps at the bottom of the screen beneath the scrolling log: an
/// optional one-line status footer (`status`) and the console's own prompt (`prompt`), plus the
/// cursor's offset into the prompt (`cursor_back`) so a status refresh can put it back where the
/// user was typing.
///
/// Both writers — the console's keystrokes (`console.rs`) and `on_event`'s log lines, which can run
/// on whichever thread `tracing`'s dispatch happens to use — go through this one lock before
/// touching stdout, so neither can land in the middle of the other. An empty `prompt` means nothing
/// interactive is shown, the state a non-interactive console (piped stdin, no TTY) never leaves — so
/// log lines there fall straight through to a plain `writeln!`, exactly today's behaviour, and a
/// status set in that state is stored but never drawn.
#[derive(Default)]
struct Footer {
    status: String,
    prompt: String,
    cursor_back: usize,
}

static FOOTER: OnceLock<Mutex<Footer>> = OnceLock::new();

fn footer_lock() -> &'static Mutex<Footer> {
    FOOTER.get_or_init(|| Mutex::new(Footer::default()))
}

/// Tell the shared state what the console currently has drawn. Called by `console.rs` on exit to
/// clear the prompt; the drawing on every keystroke is done by [`redraw_prompt`].
pub fn set_prompt_drawn(current: &str) {
    let mut guard = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    guard.prompt.clear();
    guard.prompt.push_str(current);
}

/// Erase one row and return the cursor to column 0. Pulled out as a constant so the composition
/// below is one string operation, not several.
const ERASE: &str = "\r\x1b[2K";

/// The bytes that erase the whole footer from the screen, leaving the cursor at column 0 of its
/// topmost row: nothing when no prompt is shown, one erased row for a bare prompt, two when a
/// status line sits above it.
fn footer_erase(f: &Footer) -> String {
    if f.prompt.is_empty() {
        String::new()
    } else if f.status.is_empty() {
        ERASE.to_string()
    } else {
        format!("{ERASE}\x1b[1A{ERASE}")
    }
}

/// The bytes that draw the footer, assuming the cursor starts at column 0 of where its top row
/// goes, and leaving it at the prompt's own edit point (`cursor_back` columns from the end).
fn footer_draw(f: &Footer) -> String {
    if f.prompt.is_empty() {
        return String::new();
    }
    let mut s = if f.status.is_empty() {
        f.prompt.clone()
    } else {
        format!("{}\r\n{}", f.status, f.prompt)
    };
    if f.cursor_back > 0 {
        let _ = write!(s, "\x1b[{}D", f.cursor_back);
    }
    s
}

/// Build the exact bytes a log write should emit: erase the footer, write the line, redraw the
/// footer beneath it. Split from the stdout write so it can be checked as a pure function — this is
/// the whole concurrency contract. The no-status paths are byte-for-byte what they always were.
fn compose_log_write(footer: &Footer, line: &str) -> String {
    if footer.prompt.is_empty() {
        format!("{line}\n")
    } else if footer.status.is_empty() {
        format!("{ERASE}{line}\r\n{}", footer.prompt)
    } else {
        format!(
            "{ERASE}\x1b[1A{ERASE}{line}\r\n{}\r\n{}",
            footer.status, footer.prompt
        )
    }
}

/// Write one already-rendered line, erasing and redrawing the footer around it under one lock so a
/// second writer — another log line, or the console's own next keystroke — can never interleave.
fn write_line_coordinated(line: &str) {
    let guard = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    let composed = compose_log_write(&guard, line);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(composed.as_bytes());
    let _ = out.flush();
}

/// Print a line under the same coordination as a log write, for output unrelated to `tracing` —
/// Tab completion's candidate list, for one. Public so `console.rs` can print directly.
pub fn print_notice(line: &str) {
    write_line_coordinated(line);
}

/// Set the one-line status footer shown above the prompt (an empty string clears it). Called by the
/// game task as players join, the tick cost moves, and so on. Redrawn in place under the shared
/// lock, with the cursor restored to the prompt's edit point, so it never disturbs a half-typed
/// command. While no prompt is shown (a non-interactive console) it is stored but not drawn.
pub fn set_status(status: &str) {
    let mut guard = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    if guard.status == status {
        return;
    }
    if guard.prompt.is_empty() {
        guard.status.clear();
        guard.status.push_str(status);
        return;
    }
    let before = Footer {
        status: guard.status.clone(),
        prompt: guard.prompt.clone(),
        cursor_back: guard.cursor_back,
    };
    guard.status.clear();
    guard.status.push_str(status);
    let after = Footer {
        status: guard.status.clone(),
        prompt: guard.prompt.clone(),
        cursor_back: guard.cursor_back,
    };
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{}{}", footer_erase(&before), footer_draw(&after));
    let _ = out.flush();
}

/// Redraw the console's own prompt line under the shared lock, storing the cursor offset so a status
/// refresh can restore it. Only the prompt row is touched — a status line above it stays put.
///
/// `cursor_back` moves the terminal cursor left that many columns after drawing, for when the edit
/// point is not at the end of the line (arrowed left, then typed). Done under the same write as the
/// redraw, so no log line can land between the text landing and the cursor settling on it.
pub fn redraw_prompt(current: &str, cursor_back: usize) {
    let mut guard = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    guard.prompt.clear();
    guard.prompt.push_str(current);
    guard.cursor_back = cursor_back;
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "{ERASE}{current}");
    if cursor_back > 0 {
        let _ = write!(out, "\x1b[{cursor_back}D");
    }
    let _ = out.flush();
}

impl<S> Layer<S> for TermLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut parts = Parts::default();
        event.record(&mut parts);
        let meta = event.metadata();
        let is_reply = meta.target() == CONSOLE_REPLY_TARGET;
        let is_chat = meta.target() == CHAT_TARGET;
        let line = if is_reply {
            self.render_reply(&parts)
        } else if is_chat {
            self.render_chat(&parts, self.started.elapsed())
        } else {
            self.render(*meta.level(), meta.target(), &parts, self.started.elapsed())
        };
        write_line_coordinated(&line);

        // A second, ANSI-free rendering for the web panel's live feed. `broadcast::Sender::send`
        // on a channel nobody is subscribed to just returns an error immediately, so this costs
        // nothing on every run that never starts the panel.
        let kind = if is_reply {
            ConsoleLineKind::Reply
        } else if is_chat {
            ConsoleLineKind::Chat
        } else {
            ConsoleLineKind::Log
        };
        let plain = TermLayer {
            palette: Palette::PLAIN,
            started: self.started,
        };
        let text = if is_reply {
            plain.render_reply(&parts)
        } else if is_chat {
            plain.render_chat(&parts, self.started.elapsed())
        } else {
            plain.render(*meta.level(), meta.target(), &parts, self.started.elapsed())
        };
        let _ = console_feed().send(ConsoleLine {
            kind,
            level: meta.level().as_str(),
            text,
        });
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

/// Braille spinner frames for the boot stages.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// One step of the boot sequence, shown live: a spinner while it runs, a green ✓ when it finishes.
///
/// Falls back to a single plain line when stdout is not a terminal (piped, or a log file), so a
/// captured boot log carries no cursor games. The spinner rides the same prompt-redraw coordination
/// the sticky console uses ([`redraw_prompt`]/[`write_line_coordinated`]), so a log line arriving
/// mid-stage floats it rather than smearing it.
pub struct Stage {
    label: String,
    palette: Palette,
    tty: bool,
    stop: Option<(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        std::thread::JoinHandle<()>,
    )>,
}

impl Stage {
    /// Begin a stage. On a terminal this starts the spinner animating on its own thread; piped, it
    /// draws nothing until [`Stage::finish`].
    pub fn begin(palette: Palette, label: &str) -> Self {
        use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
        let tty = std::io::stdout().is_terminal();
        let stop = if tty {
            let flag = Arc::new(AtomicBool::new(false));
            let f = flag.clone();
            let label = label.to_string();
            let handle = std::thread::spawn(move || {
                let mut i = 0usize;
                while !f.load(Ordering::Relaxed) {
                    let frame = SPINNER[i % SPINNER.len()];
                    let line = format!(
                        "  {}{frame}{} {label}{} …{}",
                        palette.on(sgr::BRIGHT_CYAN),
                        palette.off(),
                        palette.on(sgr::DIM),
                        palette.off(),
                    );
                    redraw_prompt(&line, 0);
                    i += 1;
                    std::thread::sleep(Duration::from_millis(80));
                }
            });
            Some((flag, handle))
        } else {
            None
        };
        Self {
            label: label.to_string(),
            palette,
            tty,
            stop,
        }
    }

    /// Finish a stage: stop the spinner and leave a green ✓ in its place, with an optional trailing
    /// detail (dimmed). `detail` may be empty for a bare tick.
    pub fn finish(mut self, detail: &str) {
        use std::sync::atomic::Ordering;
        if let Some((flag, handle)) = self.stop.take() {
            flag.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
        let done = done_line(self.palette, &self.label, detail);
        if self.tty {
            // Replace the spinner in place and clear the shared prompt, so a later log line does not
            // try to redraw a stage that has already ended.
            let mut guard = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
            guard.prompt.clear();
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{ERASE}{done}");
            let _ = out.flush();
        } else {
            write_line_coordinated(&done);
        }
    }
}

impl Drop for Stage {
    /// If a stage is dropped without [`Stage::finish`] — an error propagated out from under it —
    /// stop the spinner thread so it cannot keep redrawing over the error on its way out.
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        if let Some((flag, handle)) = self.stop.take() {
            flag.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
    }
}

/// An instant ✓ line, for a step that does no work worth a spinner — a disabled panel, say.
pub fn tick(palette: Palette, label: &str, detail: &str) {
    write_line_coordinated(&done_line(palette, label, detail));
}

/// The shared shape of a finished stage line: a green ✓, the label, and an optional dimmed detail.
fn done_line(palette: Palette, label: &str, detail: &str) -> String {
    let p = palette;
    if detail.is_empty() {
        format!("  {}✓{} {label}", p.on(sgr::BRIGHT_GREEN), p.off())
    } else {
        format!(
            "  {}✓{} {label}   {}{detail}{}",
            p.on(sgr::BRIGHT_GREEN),
            p.off(),
            p.on(sgr::DIM),
            p.off(),
        )
    }
}

/// The line that closes the boot: a bold, green "ready in Xs".
pub fn ready_line(palette: Palette, elapsed: Duration) -> String {
    let p = palette;
    format!(
        "\n  {}✓{} {}ready{} in {}{:.2}s{}\n",
        p.on(sgr::BRIGHT_GREEN),
        p.off(),
        p.on(sgr::BOLD),
        p.off(),
        p.on(sgr::BRIGHT_CYAN),
        elapsed.as_secs_f64(),
        p.off(),
    )
}

/// The width `panel` would draw these rows at on their own, before alignment with a sibling
/// panel. A caller with two panels to print side by side in spirit (if not in fact, since they
/// print one above the other) computes this for both and passes the larger back in as `panel`'s
/// `min_inner`, so neither panel decides its width without knowing about the other.
pub fn panel_width(title: &str, rows: &[(&str, String)]) -> usize {
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
    (widest_label + widest_value + 3).max(title.chars().count() + 2)
}

/// A titled box of `label: value` rows, used for the summaries a server prints once.
///
/// `min_inner` widens the panel beyond what its own rows need, so a caller can line up two panels
/// to the same width — pass `0` for a panel with no sibling to match.
pub fn panel(palette: Palette, title: &str, rows: &[(&str, String)], min_inner: usize) -> String {
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
    let natural = panel_width(title, rows);
    let inner = natural.max(min_inner);
    // Any width forced on top of what the rows need goes entirely to the value column, so a
    // panel widened to match a sibling stays rectangular instead of opening a gap before its
    // right border.
    let value_width = widest_value + (inner - natural);

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
            "{}│{} {}{label:<widest_label$}{}  {}{value:<value_width$}{} {}│{}",
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
            0,
        );
        let widths: Vec<usize> = text.lines().map(|l| l.chars().count()).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged panel: {widths:?}\n{text}"
        );
    }

    /// The mechanism `main.rs` relies on to make the world and server panels the same width: ask
    /// each its natural width, pass the larger to both as `min_inner`, and the narrower one still
    /// comes out rectangular — not just as wide, but without a ragged gap before its border.
    #[test]
    fn a_panel_widened_to_match_a_sibling_stays_rectangular() {
        let narrow_rows = [("name", "Test".into())];
        let wide_rows = [
            ("listening", "0.0.0.0:7777".into()),
            (
                "save destination",
                "/home/brooklyn/.local/share/terrustia/worlds/my great big world name.wld".into(),
            ),
        ];
        let width = panel_width("world", &narrow_rows).max(panel_width("server", &wide_rows));

        let narrow = panel(Palette::PLAIN, "world", &narrow_rows, width);
        let wide = panel(Palette::PLAIN, "server", &wide_rows, width);

        let narrow_widths: Vec<usize> = narrow.lines().map(|l| l.chars().count()).collect();
        assert!(
            narrow_widths.windows(2).all(|w| w[0] == w[1]),
            "ragged narrow panel: {narrow_widths:?}\n{narrow}"
        );
        assert_eq!(
            narrow_widths[0],
            wide.lines().next().unwrap().chars().count(),
            "panels do not match: {narrow_widths:?} vs {wide}"
        );
    }

    /// The whole concurrency contract, as a pure function: what should a log write actually send
    /// to the terminal, given what the console currently has drawn? This is the piece that matters
    /// most and the one a real terminal cannot easily prove — get this string right and the actual
    /// `write_all` call underneath it is not where a bug could hide.
    #[test]
    fn a_log_line_with_no_prompt_showing_is_written_plainly() {
        assert_eq!(
            compose_log_write(&Footer::default(), "world ready"),
            "world ready\n"
        );
    }

    #[test]
    fn a_log_line_erases_and_redraws_a_shown_prompt() {
        let footer = Footer {
            status: String::new(),
            prompt: "> kick bri".into(),
            cursor_back: 0,
        };
        let composed = compose_log_write(&footer, "a player joined");
        // Erased first, or the old prompt bleeds into the new line.
        assert!(
            composed.starts_with(ERASE),
            "must erase before writing: {composed:?}"
        );
        // The log line itself is in there, terminated so the redrawn prompt starts a fresh line.
        assert!(composed.contains("a player joined\r\n"));
        // And the prompt is redrawn afterward, verbatim — nothing typed should be lost.
        assert!(
            composed.ends_with("> kick bri"),
            "the typed line must survive the write: {composed:?}"
        );
    }

    /// With a status footer shown, a log line erases both rows, lands above them, and both the
    /// status and the prompt are redrawn beneath it — nothing on either line is lost.
    #[test]
    fn a_status_footer_floats_a_log_line_above_both_of_its_rows() {
        let footer = Footer {
            status: "● 3 online".into(),
            prompt: "❯ ".into(),
            cursor_back: 0,
        };
        let composed = compose_log_write(&footer, "a player joined");
        // Two rows erased: the prompt, then up one and the status.
        assert!(
            composed.starts_with("\r\x1b[2K\x1b[1A\r\x1b[2K"),
            "must erase both footer rows: {composed:?}"
        );
        // The log, then the status and prompt redrawn under it.
        assert!(
            composed.contains("a player joined\r\n● 3 online\r\n❯ "),
            "status and prompt must survive beneath the log: {composed:?}"
        );
    }

    /// `set_prompt_drawn` and `write_line_coordinated` share one lock; a log write must pick up
    /// whatever the console most recently drew, not a stale value from before it typed.
    #[test]
    fn a_log_write_sees_the_most_recently_drawn_prompt() {
        set_prompt_drawn("> first");
        set_prompt_drawn("> first draft");
        let guard = footer_lock().lock().unwrap();
        assert_eq!(
            compose_log_write(&guard, "x"),
            "\r\x1b[2Kx\r\n> first draft"
        );
        drop(guard);
        // Leave the shared state clean for any test that runs after this one.
        set_prompt_drawn("");
    }

    /// A chat line gets its own `CHAT` tag and its message, but none of the level/target furniture
    /// an operational log line carries — so a room full of chatter does not read as a wall of
    /// identical `INFO … chat` events.
    #[test]
    fn a_chat_line_reads_as_chat_not_as_a_log_event() {
        let layer = TermLayer::new(Palette::PLAIN);
        let parts = Parts {
            message: "<bri> hello everyone".into(),
            fields: vec![],
        };
        let line = layer.render_chat(&parts, Duration::from_millis(65_432));
        assert!(line.contains("CHAT"), "missing the chat tag: {line:?}");
        assert!(line.contains("<bri> hello everyone"), "lost the message: {line:?}");
        assert!(
            !line.contains("INFO"),
            "chat must not look like an INFO log: {line:?}"
        );
        assert!(line.contains("00:01:05.432"), "uptime missing: {line:?}");
    }

    /// A `console_reply`-tagged event is rendered without the timestamp/level/target furniture
    /// ordinary log lines carry — it is meant to look like a REPL printing its own output.
    #[test]
    fn a_console_reply_carries_no_log_furniture() {
        let layer = TermLayer::new(Palette::PLAIN);
        let parts = Parts {
            message: "1 player connected.".into(),
            fields: vec![("online", "1".into())],
        };
        let line = layer.render_reply(&parts);
        assert_eq!(line, "1 player connected. online=1");
        assert!(
            !line.contains("INFO"),
            "a reply must not look like a log line: {line:?}"
        );
    }
}
