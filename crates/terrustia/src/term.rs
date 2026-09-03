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
    pub const WHITE: &str = "\x1b[37m";
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const BRIGHT_BLUE: &str = "\x1b[94m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
    pub const BRIGHT_MAGENTA: &str = "\x1b[95m";

    /// The banner's cyan-into-blue gradient, one 256-colour escape per row of the ASCII art. Named
    /// here rather than left as bare literals at the call site, so every colour this module ever
    /// prints has one name and one place it is defined.
    pub const BANNER_RAMP: [&str; 5] = [
        "\x1b[38;5;51m",
        "\x1b[38;5;45m",
        "\x1b[38;5;39m",
        "\x1b[38;5;33m",
        "\x1b[38;5;27m",
    ];
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
        Level::TRACE => ("TRACE", sgr::BRIGHT_MAGENTA),
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
        let rendered = sanitise_control(unquote(&rendered));
        if field.name() == "message" {
            self.message = rendered;
        } else {
            self.fields.push((field.name(), rendered));
        }
    }
}

/// Strip control code points that could reinterpret this line's meaning once it lands on a real
/// terminal or in the panel's live feed, rather than merely reading as text: `ESC`-led sequences
/// (clear the screen, move the cursor, retitle the window, spoof a prompt), a raw `CR`/`LF` (which
/// could forge what looks like a second, unrelated log line), the rest of the C0 range, `DEL`, and
/// the C1 control range (`U+0080..=U+009F`: some terminals still act on these). Every event's
/// message and field values pass through here, in [`Parts::record_debug`], before `TermLayer` ever
/// renders them for the TTY or hands a copy to [`console_feed`]: untrusted text from anywhere
/// (unauthenticated chat, above all; see `game/server/dispatch.rs`'s `on_net_module`) therefore
/// cannot smuggle terminal control codes into either destination, including a future call site
/// nobody has written yet. Tab survives: it carries no cursor or line hazard and is occasionally
/// legitimate content. Everything else (punctuation, accents, CJK, emoji, any ordinary Unicode
/// text) passes through unchanged; only control code points are touched.
fn sanitise_control(text: &str) -> String {
    text.chars()
        .filter(|&c| {
            let code = c as u32;
            match code {
                0x09 => true,
                0x00..=0x1f => false,
                0x7f => false,
                0x80..=0x9f => false,
                _ => true,
            }
        })
        .collect()
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

        // A one-column severity bar, reserved for every level so the tag column always starts at
        // the same place, but only ever coloured in for WARN/ERROR — a scrolling log's problems
        // should catch the eye at a glance, not require reading the tag text.
        if matches!(level, Level::WARN | Level::ERROR) {
            line.push_str(p.on(colour));
            line.push('▎');
            line.push_str(p.off());
        } else {
            line.push(' ');
        }
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
            line.push_str(p.on(sgr::BRIGHT_CYAN));
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
            line.push_str(p.on(sgr::BRIGHT_CYAN));
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
    /// Physical rows the footer occupied after its last draw, so the next redraw erases exactly
    /// that many. A prompt or status wider than the terminal wraps onto several rows, and erasing
    /// only one left the rest on screen, stacking a fresh copy below on every keystroke; tracking
    /// the real count is what stops that.
    drawn_rows: usize,
}

static FOOTER: OnceLock<Mutex<Footer>> = OnceLock::new();

fn footer_lock() -> &'static Mutex<Footer> {
    FOOTER.get_or_init(|| Mutex::new(Footer::default()))
}

/// The terminal width in columns, for working out how many physical rows a line wraps into. Falls
/// back to 80 when there is no terminal to ask; a footer is only ever drawn when one exists.
fn terminal_cols() -> usize {
    crossterm::terminal::size()
        .map(|(c, _)| c as usize)
        .unwrap_or(80)
        .max(1)
}

/// Whether the sticky console currently has the terminal in raw mode. Raw mode turns off the
/// terminal's own `\n` -> `\r\n` translation, so a plain `\n` moves the cursor down without
/// returning it to column 0, and a line printed after one lands mid-column. Every line this module
/// prints while this is set has to carry its own carriage return. `console.rs` sets and clears it
/// around `enable_raw_mode`/`disable_raw_mode`; while it is false (piped output, a service with no
/// terminal, or before the console starts) a bare `\n` is correct and a `\r` would be litter.
static RAW_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Tell this module whether the terminal is in raw mode; see [`RAW_MODE`]. Called by the console.
pub fn set_raw_mode(on: bool) {
    RAW_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The line ending to use right now: `\r\n` while the raw-mode console holds the terminal, a bare
/// `\n` otherwise. One place decides it, so every writer here agrees.
fn newline() -> &'static str {
    if RAW_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        "\r\n"
    } else {
        "\n"
    }
}

/// The visible column an ordinary log line's message begins at: the width of the timestamp,
/// severity bar, level tag and target columns [`TermLayer::render`] lays down before it
/// (12 + 1 + 1 + 1 + 5 + 1 + 18 + 1). A wrapped log line indents its continuation rows to here so
/// the message reads as one block rather than sprawling back to column 0. Tied to `render`'s layout
/// by `the_message_column_matches_render`.
const MESSAGE_COL: usize = 40;

/// The same, for a chat line, which carries only a timestamp and a `CHAT` tag (12 + 1 + 5 + 1).
const CHAT_COL: usize = 19;

/// Re-flow `line` to `cols` columns, indenting every row after the first to `indent`, so a log line
/// wider than the terminal wraps into an aligned block instead of spilling back to column 0. ANSI
/// colour and cursor escapes are copied through without counting toward the width, and characters
/// (not words) are the break unit, matching what the terminal's own wrap would have done. A line
/// that fits is returned unchanged.
fn wrap_ansi(line: &str, cols: usize, indent: usize) -> String {
    // Nothing sane to do if the terminal is narrower than the indent itself; leave it to wrap on
    // its own rather than emit rows that are all indent and no room.
    if cols <= indent || visible_len(line) <= cols {
        return line.to_string();
    }
    let pad = " ".repeat(indent);
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len() + line.len() / cols.max(1) + indent);
    let mut col = 0usize;
    let mut i = 0usize;
    // Where the current row's text starts in `out`, and the last point in it we may break without
    // splitting a word. Breaking anywhere is what produced `<pa` / `ssword>` in a real log line.
    let mut row_start = 0usize;
    let mut last_space: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // Copy a CSI escape verbatim; it takes no visible columns.
            let start = i;
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            out.push_str(&line[start..i]);
            continue;
        }
        // The start of a UTF-8 character. Wrap before it if the row is full, at the last space on
        // the row where there is one, so a word carries over whole.
        if col >= cols {
            let broke_at_space = match last_space {
                // Everything after the break moves down a row. It holds no space of its own, being
                // the tail after the last one, so it needs no further break opportunity, and it
                // has to fit on the row it is moving to with a column still to spare. Strictly
                // less than `cols`, not at most: the character being wrapped is appended to that
                // same row immediately below, so a tail that exactly fills the row overflows it by
                // one. That is what put a 47-column row in a 46-column terminal. When the tail
                // does not fit there is nothing to gain by carrying it, so fall through to the
                // hard cut, which is bounded by construction.
                Some(at)
                    if at > row_start
                        && indent + visible_len(out[at..].trim_start_matches(' ')) < cols =>
                {
                    let carried = out.split_off(at);
                    let carried = carried.trim_start_matches(' ');
                    out.push_str(newline());
                    out.push_str(&pad);
                    row_start = out.len();
                    col = indent + visible_len(carried);
                    out.push_str(carried);
                    true
                }
                // No space to break at: a single word longer than the row. Cut it, which is what
                // any terminal does, rather than run off the edge.
                _ => false,
            };
            if !broke_at_space {
                out.push_str(newline());
                out.push_str(&pad);
                row_start = out.len();
                col = indent;
            }
            last_space = None;
        }
        if bytes[i] == b' ' {
            last_space = Some(out.len());
        }
        let char_len = utf8_len(bytes[i]);
        let end = (i + char_len).min(bytes.len());
        // The same column arithmetic `visible_len` uses, so a wrapped row and a measured row agree
        // about their width. Counting every character as one made a CJK line wrap at twice the
        // terminal's real width.
        col += line[i..end].chars().next().map_or(1, char_cols);
        out.push_str(&line[i..end]);
        i = end;
    }
    out
}

/// Byte length of a UTF-8 character from its lead byte.
fn utf8_len(lead: u8) -> usize {
    match lead {
        b if b >= 0xF0 => 4,
        b if b >= 0xE0 => 3,
        b if b >= 0xC0 => 2,
        _ => 1,
    }
}

/// Columns one character occupies on a terminal: 0, 1 or 2.
///
/// Not a full UAX #11 table, which is thousands of ranges and a dependency this workspace has no
/// reason to take. It is the set that actually reaches a Terraria server's console: chat in
/// Chinese, Japanese or Korean, emoji, and accented Latin typed as a base letter plus a combining
/// mark. Everything outside these ranges is one column, which is what the old code assumed for
/// everything.
///
/// The ranges are the wide and fullwidth blocks of UAX #11 (Hangul Jamo, the CJK blocks, Hiragana
/// and Katakana, Yi, Hangul syllables, the compatibility and fullwidth forms, and the emoji and
/// CJK extension planes), plus the zero-width ones: combining diacriticals, the zero-width
/// space/joiner group, and variation selectors.
fn char_cols(c: char) -> usize {
    let c = c as u32;
    match c {
        0x0300..=0x036F | 0x200B..=0x200F | 0xFE00..=0xFE0F => 0,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// How many visible columns a rendered line occupies, skipping ANSI CSI escapes (colour and cursor
/// moves).
///
/// This used to count every character as one column, which is right for ASCII and wrong for every
/// script that is not. A CJK or emoji character is two columns wide, so a line carrying them
/// occupied more physical rows than this reported, [`Footer::rows`] under-counted, and `erase_seq`
/// then moved the cursor up too few rows and left stale prompt rows on screen. That is exactly the
/// corruption `drawn_rows` was added to stop, reachable with one `say` in Chinese.
fn visible_len(line: &str) -> usize {
    let mut cols = 0;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // A CSI escape: `ESC [` then parameters, ending at the first alphabetic byte. Anything
            // else after ESC is a one-character sequence and is skipped with it.
            if chars.next() == Some('[') {
                for p in chars.by_ref() {
                    if p.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            cols += char_cols(c);
        }
    }
    cols
}

/// Physical rows one logical line takes at this width.
fn line_rows(line: &str, cols: usize) -> usize {
    visible_len(line).div_ceil(cols).max(1)
}

/// A dim horizontal rule, `cols` wide, separating the scrolled log above from the live status and
/// prompt below — the one part of the screen that never moves. Always exactly one row.
fn status_rule(cols: usize) -> String {
    Palette::detect().paint(sgr::DIM, &"─".repeat(cols))
}

/// Physical rows the whole footer (the status line, if any, above the prompt) takes at this width.
fn footer_rows(f: &Footer, cols: usize) -> usize {
    if f.prompt.is_empty() {
        return 0;
    }
    let mut rows = line_rows(&f.prompt, cols);
    if !f.status.is_empty() {
        // +1 for the separating rule, always exactly one row regardless of width.
        rows += line_rows(&f.status, cols) + 1;
    }
    rows
}

/// The bytes that erase a footer of `drawn_rows` physical rows, given the cursor sits somewhere on
/// its bottom row: return to column 0, move up to the top row, and clear from there to the end of
/// the screen. Clearing to end of screen rather than one row at a time is what keeps a wrapped
/// prompt from leaving stranded copies above the cursor.
fn erase_seq(drawn_rows: usize) -> String {
    if drawn_rows == 0 {
        return String::new();
    }
    let mut s = String::from("\r");
    if drawn_rows > 1 {
        let _ = write!(s, "\x1b[{}A", drawn_rows - 1);
    }
    s.push_str("\x1b[0J");
    s
}

/// The bytes that draw the footer from column 0 of its top row, leaving the cursor at the prompt's
/// edit point. `cursor_back` is honoured only while the prompt fits on one row, since positioning
/// the cursor back across a wrap is not worth the arithmetic for a rare case.
fn draw_seq(f: &Footer, cols: usize) -> String {
    if f.prompt.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    if !f.status.is_empty() {
        s.push_str(&status_rule(cols));
        s.push_str("\r\n");
        s.push_str(&f.status);
        s.push_str("\r\n");
    }
    s.push_str(&f.prompt);
    if f.cursor_back > 0 && line_rows(&f.prompt, cols) == 1 {
        let _ = write!(s, "\x1b[{}D", f.cursor_back);
    }
    s
}

/// Redraw the footer in place after its state changed, emitting `middle` (a log line and its line
/// break, or nothing) between erasing the rows it drew last and drawing the new ones. Every writer
/// funnels through here, so the erase always matches what was last drawn, and the new physical row
/// count is remembered for next time.
fn redraw_footer(f: &mut Footer, middle: &str) {
    let cols = terminal_cols();
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(erase_seq(f.drawn_rows).as_bytes());
    if !middle.is_empty() {
        let _ = out.write_all(middle.as_bytes());
    }
    let _ = out.write_all(draw_seq(f, cols).as_bytes());
    let _ = out.flush();
    f.drawn_rows = footer_rows(f, cols);
}

/// Erase the drawn footer and forget it, for the console's exit path so it does not leave a stale
/// prompt row glued onto whatever prints next.
pub fn clear_footer() {
    let mut f = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    if f.drawn_rows > 0 {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(erase_seq(f.drawn_rows).as_bytes());
        let _ = out.flush();
    }
    *f = Footer::default();
}

/// Write one already-rendered line, floating the footer above it under one lock so a second writer
/// (another log line, or the console's next keystroke) can never interleave. `indent` is the column
/// a wrapped line's continuation rows align to (0 to leave wrapping to the terminal). With no prompt
/// shown (a non-interactive console) it is a plain write, with the line ending [`newline`] chose.
fn write_line_coordinated(line: &str, indent: usize) {
    let mut f = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    if f.prompt.is_empty() {
        // No sticky footer: piped output, or a service console. Wrapping would bake hard breaks and
        // carriage returns into a captured log, so print the line untouched with the right ending.
        let mut out = std::io::stdout().lock();
        let _ = write!(out, "{line}{}", newline());
        let _ = out.flush();
        return;
    }
    // A sticky console is up. Re-flow a too-wide line so its continuation rows sit under the message
    // column rather than sprawling back to column 0, and let `redraw_footer` handle the coordination.
    let body = if indent > 0 {
        wrap_ansi(line, terminal_cols(), indent)
    } else {
        line.to_string()
    };
    redraw_footer(&mut f, &format!("{body}\r\n"));
}

/// Print a line under the same coordination as a log write, for output unrelated to `tracing` —
/// Tab completion's candidate list and the console's own command echo, for two. Public so
/// `console.rs` can print directly.
pub fn print_notice(line: &str) {
    write_line_coordinated(line, 0);
}

/// Set the one-line status footer shown above the prompt (an empty string clears it). Called by the
/// game task as players join, the tick cost moves, and so on. Redrawn in place under the shared
/// lock, with the cursor restored to the prompt's edit point, so it never disturbs a half-typed
/// command. While no prompt is shown (a non-interactive console) it is stored but not drawn.
pub fn set_status(status: &str) {
    let mut f = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    if f.status == status {
        return;
    }
    f.status.clear();
    f.status.push_str(status);
    if f.prompt.is_empty() {
        return;
    }
    redraw_footer(&mut f, "");
}

/// Redraw the console's own prompt line under the shared lock, storing the cursor offset so a status
/// refresh can restore it. Only the prompt row is touched — a status line above it stays put.
///
/// `cursor_back` moves the terminal cursor left that many columns after drawing, for when the edit
/// point is not at the end of the line (arrowed left, then typed). Done under the same write as the
/// redraw, so no log line can land between the text landing and the cursor settling on it.
pub fn redraw_prompt(current: &str, cursor_back: usize) {
    let mut f = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
    f.prompt.clear();
    f.prompt.push_str(current);
    f.cursor_back = cursor_back;
    redraw_footer(&mut f, "");
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
        // A console reply is meant to read like a REPL's own output, so it wraps at column 0 like
        // anything typed; a chat and an ordinary log line indent their continuation rows under where
        // their message began.
        let indent = if is_reply {
            0
        } else if is_chat {
            CHAT_COL
        } else {
            MESSAGE_COL
        };
        write_line_coordinated(&line, indent);

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
    let mut out = String::new();
    out.push('\n');
    for (line, colour) in art.iter().zip(sgr::BANNER_RAMP) {
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
            // Erase the spinner (however many rows it wrapped into) and print the finished line in
            // its place, then forget the prompt so a later log line does not redraw a stage that
            // has already ended.
            let mut f = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(erase_seq(f.drawn_rows).as_bytes());
            let _ = writeln!(out, "{done}");
            let _ = out.flush();
            f.prompt.clear();
            f.drawn_rows = 0;
        } else {
            write_line_coordinated(&done, 0);
        }
    }

    /// Finish a stage by erasing its spinner and leaving nothing in its place, for when what
    /// replaces it is drawn separately — the boot card printed below it. Stops the spinner thread
    /// and forgets the prompt exactly the way [`Stage::finish`] does, minus the ✓ line. Piped,
    /// where the spinner never drew anything, this is a no-op and the card alone stands in for it.
    pub fn clear(mut self) {
        use std::sync::atomic::Ordering;
        if let Some((flag, handle)) = self.stop.take() {
            flag.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
        if self.tty {
            let mut f = footer_lock().lock().unwrap_or_else(|e| e.into_inner());
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(erase_seq(f.drawn_rows).as_bytes());
            let _ = out.flush();
            f.prompt.clear();
            f.drawn_rows = 0;
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
    write_line_coordinated(&done_line(palette, label, detail), 0);
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

/// An aligned block of `key   value` lines for the summary a server prints once at boot: a dim key
/// padded to a shared width, then the value. One left margin, no boxes, so the block sits in the
/// same rhythm as the ✓ stage lines above it rather than each row hugging its own width the way the
/// old two boxes did.
pub fn info_block(palette: Palette, rows: &[(&str, String)]) -> String {
    info_block_at(palette, rows, terminal_cols())
}

/// The card, laid out for a terminal `cols` wide. Split from [`info_block`] so the wrapping can be
/// tested at a width, rather than only at whatever the machine running the tests happens to have.
///
/// The card's rows are a fixed four-space margin, a key column, three spaces, then the value. Every
/// other line this module prints (a stage spinner, a ✓ line, the status footer) is a bare glyph
/// with a two-space margin instead; the extra two spaces here are what a key column needs to sit
/// clear of that glyph column, not a second, unrelated convention. On a
/// terminal narrower than that plus the value (measured: the art is 55 columns, the tagline 69, and
/// the `saves to` row 74 with a realistic world name) the value wrapped back to column 0 and the
/// alignment the card exists for was gone. `wrap_ansi` already knows how to carry a value down
/// under a hanging indent without breaking it mid-word, so the fix is to use it at the indent the
/// value already starts at, not to invent a second layout.
fn info_block_at(palette: Palette, rows: &[(&str, String)], cols: usize) -> String {
    let p = palette;
    let key_width = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    // 4 for the left margin, the key column, then the 3 that separate key from value.
    let indent = 4 + key_width + 3;
    let mut out = String::new();
    for (key, value) in rows {
        let line = format!(
            "    {}{key:<key_width$}{}   {}",
            p.on(sgr::DIM),
            p.off(),
            value
        );
        let _ = writeln!(out, "{}", wrap_ansi(&line, cols, indent));
    }
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

    /// The boot summary block pads its keys to a shared width, so every value starts in the same
    /// column and the block reads as one aligned unit rather than ragged rows.
    #[test]
    fn an_info_block_aligns_every_value_to_the_same_column() {
        let text = info_block(
            Palette::PLAIN,
            &[("world", "Alpha".into()), ("saves to", "Beta".into())],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines.iter().all(|l| l.starts_with("    ")),
            "every row shares the four-space margin: {text}"
        );
        assert_eq!(
            lines[0].find("Alpha"),
            lines[1].find("Beta"),
            "values must start at the same column:\n{text}"
        );
    }

    /// The wrap arithmetic is the whole concurrency contract, and the one a real terminal cannot
    /// easily prove. `visible_len` counts the columns a line occupies, ignoring the ANSI colour and
    /// cursor escapes that take no space.
    #[test]
    fn visible_len_ignores_ansi_escapes() {
        assert_eq!(visible_len("hello"), 5);
        assert_eq!(visible_len("\x1b[92m✓\x1b[0m done"), 6);
        assert_eq!(visible_len("\x1b[2m00:00:01\x1b[0m"), 8);
        assert_eq!(visible_len(""), 0);
    }

    /// A CJK or emoji character takes two columns, not one. Counting them as one made the footer
    /// think a line fitted in fewer rows than it did, so the erase that follows moved up too few
    /// rows and left stale prompt rows on screen.
    /// The boot card keeps its alignment on a narrow terminal instead of falling back to column 0.
    #[test]
    fn the_boot_card_wraps_a_long_value_under_its_own_column() {
        let rows = [
            (
                "world",
                "A Rather Long World Name · 8400 x 2400 · crimson".to_string(),
            ),
            (
                "saves to",
                "worlds/A_Rather_Long_World_Name.wld".to_string(),
            ),
        ];
        // 8 is the widest key, so values start at 4 + 8 + 3 = 15.
        let card = info_block_at(Palette::PLAIN, &rows, 46);
        let lines: Vec<&str> = card.lines().collect();
        assert!(
            lines.len() > rows.len(),
            "a value too wide for the terminal should have wrapped: {lines:?}"
        );
        assert!(
            lines.iter().all(|l| visible_len(l) <= 46),
            "no row may run past the terminal: {lines:?}"
        );
        for line in &lines {
            // A continuation row is the one that does not begin with the four-space margin and a
            // non-space, and it has to be indented to the value column rather than to 0.
            if !line.starts_with("    world") && !line.starts_with("    saves to") {
                assert!(
                    line.starts_with(&" ".repeat(15)),
                    "a continuation row must line up under the value column: {line:?}"
                );
            }
        }
    }

    /// And a wide terminal is left exactly as it was, so nothing changed for the ordinary case.
    #[test]
    fn the_boot_card_is_unchanged_when_it_fits() {
        let rows = [("world", "Terrustia · 4200 x 1200 · corruption".to_string())];
        let card = info_block_at(Palette::PLAIN, &rows, 120);
        assert_eq!(card, "    world   Terrustia · 4200 x 1200 · corruption\n");
    }

    #[test]
    fn a_wide_character_takes_two_columns() {
        assert_eq!(
            visible_len("你好"),
            4,
            "two CJK characters are four columns"
        );
        assert_eq!(visible_len("こんにちは"), 10);
        assert_eq!(visible_len("한글"), 4);
        assert_eq!(visible_len("🙂"), 2);
        assert_eq!(visible_len("ab你好cd"), 8, "mixed with ASCII");
        // Still one column each, so nothing ordinary regressed.
        assert_eq!(visible_len("café"), 4);
        assert_eq!(visible_len("❯ "), 2);
        assert_eq!(visible_len("✓ ready"), 7);
    }

    /// A combining mark occupies no column of its own: it draws on the character before it.
    #[test]
    fn a_combining_mark_takes_no_column() {
        assert_eq!(visible_len("e\u{0301}"), 1, "e plus combining acute is one");
        assert_eq!(visible_len("a\u{200b}b"), 2, "zero-width space");
    }

    /// The reason any of this matters: the footer has to know how many physical rows it drew, or
    /// it cannot erase them. A chat line in Chinese wraps twice as early as its character count
    /// suggests.
    #[test]
    fn a_wide_line_wraps_at_half_the_characters() {
        // 40 CJK characters are 80 columns, which is exactly two rows at 40 wide, where 40 ASCII
        // characters would have been one.
        let wide: String = std::iter::repeat_n('你', 40).collect();
        assert_eq!(line_rows(&wide, 40), 2);
        assert_eq!(line_rows(&"x".repeat(40), 40), 1);
    }

    /// A line wider than the terminal wraps onto more than one physical row, and the footer counts
    /// every one of them.
    #[test]
    fn a_line_wider_than_the_terminal_takes_more_than_one_row() {
        assert_eq!(line_rows("short", 80), 1);
        assert_eq!(line_rows(&"x".repeat(80), 40), 2);
        assert_eq!(line_rows(&"x".repeat(81), 40), 3);
        let f = Footer {
            status: "x".repeat(100),
            prompt: "❯ ".into(),
            cursor_back: 0,
            drawn_rows: 0,
        };
        assert_eq!(
            footer_rows(&f, 40),
            5,
            "status wraps to 3 rows, plus the separating rule, plus a prompt that fits on 1"
        );
    }

    /// The erase clears exactly as many rows as were drawn, by moving to the top of the footer and
    /// clearing to the end of the screen. This is the fix for the wrapped-prompt storm: a three-row
    /// footer is fully erased, not just its bottom row.
    #[test]
    fn erase_clears_the_whole_footer_however_many_rows_it_wrapped_into() {
        assert_eq!(erase_seq(0), "");
        assert_eq!(erase_seq(1), "\r\x1b[0J");
        assert_eq!(erase_seq(3), "\r\x1b[2A\x1b[0J");
    }

    /// Drawing a footer lays the status above the prompt and restores the cursor into the prompt,
    /// but only while the prompt fits on one row; positioning across a wrap is skipped on purpose.
    #[test]
    fn draw_lays_status_above_the_prompt_and_restores_the_cursor() {
        let f = Footer {
            status: "● 3 online".into(),
            prompt: "❯ kick bri".into(),
            cursor_back: 3,
            drawn_rows: 0,
        };
        assert_eq!(
            draw_seq(&f, 80),
            format!("{}\r\n● 3 online\r\n❯ kick bri\x1b[3D", "─".repeat(80))
        );
        let bare = Footer {
            status: String::new(),
            prompt: "❯ ".into(),
            cursor_back: 0,
            drawn_rows: 0,
        };
        assert_eq!(draw_seq(&bare, 80), "❯ ");
        let wrapped = Footer {
            status: String::new(),
            prompt: format!("❯ {}", "x".repeat(100)),
            cursor_back: 5,
            drawn_rows: 0,
        };
        assert!(
            !draw_seq(&wrapped, 40).contains("\x1b[5D"),
            "a wrapped prompt should not get the one-row cursor-back move"
        );
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
        assert!(
            line.contains("<bri> hello everyone"),
            "lost the message: {line:?}"
        );
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

    /// The hanging-indent constants are only right if they match the columns `render` actually lays
    /// down before the message. Pin them to the real output so a change to the prefix layout that
    /// forgets to update them fails here rather than misaligning every wrapped line at runtime.
    #[test]
    fn the_message_column_matches_render() {
        let layer = TermLayer::new(Palette::PLAIN);
        let parts = Parts {
            message: "MARKER".into(),
            fields: vec![],
        };
        let log = layer.render(
            Level::INFO,
            "terrustia::game::server",
            &parts,
            Duration::ZERO,
        );
        assert_eq!(
            log.find("MARKER"),
            Some(MESSAGE_COL),
            "a log line's message must begin at MESSAGE_COL: {log:?}"
        );
        let chat = layer.render_chat(&parts, Duration::ZERO);
        assert_eq!(
            chat.find("MARKER"),
            Some(CHAT_COL),
            "a chat line's message must begin at CHAT_COL: {chat:?}"
        );
    }

    /// A line wider than the terminal is re-flowed so its continuation rows begin at the indent
    /// column, the escapes ride along without counting toward the width, and a line that fits is
    /// left exactly as it was.
    #[test]
    fn wrap_ansi_reflows_with_a_hanging_indent() {
        // 20 visible columns, indent 4, at a width of 10: rows are "aaaaaaaaaa", then
        // "    aaaaaa", "    aaaa" (indent 4 + 6 message cols per continuation).
        let wrapped = wrap_ansi(&"a".repeat(20), 10, 4);
        let rows: Vec<&str> = wrapped.split(newline()).collect();
        assert_eq!(rows[0], "a".repeat(10), "the first row is full width");
        assert!(
            rows[1..].iter().all(|r| r.starts_with("    ")),
            "every continuation row is indented to column 4: {rows:?}"
        );
        assert!(
            rows[1..].iter().all(|r| visible_len(r) <= 10),
            "no row is wider than the terminal: {rows:?}"
        );
        let msg_chars: usize = rows
            .iter()
            .enumerate()
            .map(|(k, r)| {
                if k == 0 {
                    visible_len(r)
                } else {
                    visible_len(r) - 4
                }
            })
            .sum();
        assert_eq!(
            msg_chars, 20,
            "no visible characters are lost or duplicated: {rows:?}"
        );

        // Colour escapes do not count toward the width, so this fits on one row untouched.
        let coloured = format!("\x1b[92m{}\x1b[0m", "x".repeat(8));
        assert_eq!(
            wrap_ansi(&coloured, 10, 4),
            coloured,
            "8 visible chars fit in 10 columns even wrapped in escapes"
        );
    }

    /// A row is broken between words, not through one.
    ///
    /// The claim-token warning is the line that showed this up in a real terminal: it wrapped
    /// mid-word and printed `<pa` at the end of one row and `ssword>` at the start of the next,
    /// which is unreadable exactly where a reader is being asked to copy a command.
    #[test]
    fn wrap_ansi_breaks_between_words_not_through_them() {
        let line = "claim it with: /register <name> <password> w6y2c8kwqcdr";
        let wrapped = wrap_ansi(line, 30, 4);
        let rows: Vec<&str> = wrapped.split(newline()).collect();

        assert!(rows.len() > 1, "it should have wrapped at all: {rows:?}");
        assert!(
            rows.iter().all(|r| visible_len(r) <= 30),
            "no row is wider than the terminal: {rows:?}"
        );

        // Every word of the original survives whole on some row. Splitting one is the bug.
        for word in line.split_whitespace() {
            assert!(
                rows.iter().any(|r| r.contains(word)),
                "{word:?} was split across rows: {rows:?}"
            );
        }

        // A word longer than the row still has to be cut, or it would run off the edge.
        let long = wrap_ansi(&"z".repeat(40), 10, 4);
        assert!(
            long.split(newline()).all(|r| visible_len(r) <= 10),
            "an unbreakable word is still cut to the width: {long:?}"
        );

        // A short plain line is returned unchanged.
        assert_eq!(wrap_ansi("short", 80, 38), "short");

        // The row break goes through `newline()` like every other writer here, rather than being
        // a hardcoded `\r\n`. Outside raw mode, which is where a piped log and the boot card both
        // live, a bare `\n` is correct and the `\r` was litter in the captured file.
        assert!(
            !wrap_ansi(&"z".repeat(40), 10, 4).contains('\r'),
            "a wrapped line must not carry carriage returns when nothing is in raw mode"
        );
    }

    /// The pure filter behind L6-02's fix: C0 controls other than tab, `DEL`, and the C1 range are
    /// stripped; tab and every other Unicode character (accents, CJK, emoji) survive untouched.
    #[test]
    fn sanitise_control_strips_c0_del_c1_but_keeps_tab_and_text() {
        assert_eq!(sanitise_control("hello\tworld"), "hello\tworld");
        // Only the ESC lead byte is stripped: the printable `[2J` that follows is inert text once
        // the byte that would have turned it into a cursor command is gone, so it is left alone.
        assert_eq!(sanitise_control("clear\x1b[2Jscreen"), "clear[2Jscreen");
        assert_eq!(sanitise_control("a\r\nb"), "ab");
        assert_eq!(sanitise_control("bell\x07"), "bell");
        assert_eq!(sanitise_control("del\x7fete"), "delete");
        assert_eq!(sanitise_control("c1\u{0085}end"), "c1end");
        assert_eq!(
            sanitise_control("caf\u{e9} \u{1f600} \u{4e2d}\u{6587}"),
            "caf\u{e9} \u{1f600} \u{4e2d}\u{6587}",
            "ordinary Unicode must pass through untouched"
        );
    }

    /// Fail-then-pass for L6-02: unauthenticated chat text reaches `TermLayer` exactly the way
    /// `on_net_module`'s `<name> {text}` chat line does, and nothing on the packet path validates
    /// its content beyond length (`net_module::validate_chat`). Before `Parts::record_debug` ran the
    /// value through `sanitise_control`, an embedded `ESC` byte rode straight through into both the
    /// rendered line and the copy handed to the panel's live feed; this fires a real event through a
    /// real subscriber and checks the feed's copy, which is built from the same sanitised `Parts` the
    /// TTY line is.
    #[test]
    fn a_chat_line_with_an_embedded_escape_reaches_the_feed_sanitised() {
        use tracing_subscriber::layer::SubscriberExt;

        let mut feed = console_feed().subscribe();
        let subscriber =
            tracing_subscriber::Registry::default().with(TermLayer::new(Palette::PLAIN));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: CHAT_TARGET, "<attacker> hi\x1b[2J\x1b[Hpwned-marker");
        });

        let line = feed
            .try_recv()
            .expect("the event should have reached the feed");
        assert!(
            !line.text.contains('\x1b'),
            "an ESC byte reached the panel feed unsanitised: {:?}",
            line.text
        );
        assert!(
            line.text.contains("pwned-marker"),
            "the rest of the message must still be there: {:?}",
            line.text
        );
    }
}
