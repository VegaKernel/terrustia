//! The sticky console prompt.
//!
//! Before this, stdin was a bare `BufReader::lines()` loop: no prompt, no history, no completion,
//! and every log line the server printed landed in the middle of whatever you were typing. That
//! is not "just a TUI" — it is barely a console at all. This is the tiny REPL-ish thing instead:
//! a prompt that survives the log stream, arrow-key history, and Tab completion for command names
//! and, where it makes sense, their arguments.
//!
//! Raw mode is the only thing here that actually needs a terminal library — line editing, history
//! and completion are hand-rolled on top of it, the same "take a dependency only for what cannot
//! reasonably be written here" rule the rest of this workspace follows (see `argon2` for the one
//! other exception).
//!
//! The one real risk in a feature like this is a log line landing mid-redraw and corrupting what
//! is on screen. `term.rs` owns the coordination lock both sides go through — see
//! `term::redraw_prompt` and `term::write_line_coordinated` — so this module only ever needs to
//! call `term::redraw_prompt` and never touches stdout directly.

use std::{io::IsTerminal, time::Duration};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};
use tokio::sync::{mpsc, oneshot};

use crate::{game::ServerEvent, term};

/// The prompt text itself. Not `: `, not `terrustia> ` for parity with anything — ours, by
/// decision: this is a product, not a re-enactment of vanilla's console.
const PROMPT: &str = "❯ ";

/// How many past commands are remembered. Generous for a console nobody scripts against, and
/// small enough that it costs nothing.
const HISTORY_CAP: usize = 500;

/// The known command names, for completing the first word. Still written out rather than derived
/// from `help` at runtime, because completion wants only the bare names and `help` is prose; the
/// drift the old note here accepted as the price of that is now a test instead (see
/// `every_command_help_lists_can_be_completed`). It had already happened: `mute`, `unmute` and
/// `audit` were all real commands, all listed by `help`, and none of them completed.
const COMMANDS: &[&str] = &[
    "help",
    "say",
    "players",
    "save",
    "backups",
    "rollback",
    "whitelist",
    "claim",
    "kick",
    "ban",
    "unban",
    "mute",
    "unmute",
    "group",
    "world",
    "audit",
    "panel",
    "stop",
];

/// Commands whose second word is worth completing against the connected player roster. `mute`
/// takes a connected player's name exactly as `kick` and `ban` do; `unmute` does not, because like
/// `unban` it lifts an entry by value against somebody who need not be online.
const PLAYER_ARG_COMMANDS: &[&str] = &["kick", "ban", "unban", "mute"];
/// Commands whose second word is worth completing against the known groups.
const GROUP_ARG_COMMANDS: &[&str] = &["group"];

/// Start the console. Sticky raw-mode editing when both ends of stdio are a real terminal and the
/// operator did not ask for headless; otherwise the plain line reader, so a service started with no
/// terminal, a piped `echo stop | terrustia`, or an explicit `--headless` keeps working unchanged.
pub fn spawn(events: mpsc::Sender<ServerEvent>, headless: bool) -> tokio::task::JoinHandle<()> {
    if !headless && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        // `event::read()` blocks the OS thread it runs on waiting for a keypress; a tokio worker
        // thread must never be parked like that, which is exactly what `spawn_blocking` is for.
        // Sending the resulting `ServerEvent`s back onto the async side uses the blocking
        // variants of `mpsc`/`oneshot` built for calling from outside the runtime.
        tokio::task::spawn_blocking(move || run_sticky(&events))
    } else {
        tokio::spawn(run_plain(events))
    }
}

/// Today's behaviour, unchanged: a bare line reader for when there is no terminal to be sticky
/// about.
async fn run_plain(events: mpsc::Sender<ServerEvent>) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                if events.send(ServerEvent::Console { line }).await.is_err() {
                    return; // the game task has gone
                }
            }
            // End of input: a service started without a terminal. Not an error, and not a reason
            // to stop the server — just nothing more to read.
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, "console input closed");
                return;
            }
        }
    }
}

/// A line being typed: the characters, where the cursor sits among them, and how far back through
/// history the up-arrow has scrolled.
struct Editor {
    buf: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// Position within `history` while scrolling with the arrow keys; `None` means the line being
    /// edited is a fresh one, not a history entry pulled back up.
    browsing: Option<usize>,
    /// What was being typed before the up-arrow was first pressed, so pressing down enough times
    /// returns to it rather than to an empty line.
    stash: String,
}

impl Editor {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            browsing: None,
            stash: String::new(),
        }
    }

    fn line(&self) -> String {
        self.buf.iter().collect()
    }

    /// The exact text the prompt should show right now.
    fn rendered(&self) -> String {
        format!("{PROMPT}{}", self.line())
    }

    /// How far back from the end of the *rendered* line the cursor sits, for positioning it after
    /// a redraw.
    fn cursor_back(&self) -> usize {
        self.buf.len() - self.cursor
    }

    fn insert(&mut self, c: char) {
        self.buf.insert(self.cursor, c);
        self.cursor += 1;
        self.browsing = None;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buf.remove(self.cursor);
        }
    }

    fn delete_forward(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
        }
    }

    fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.buf.len());
    }

    fn home(&mut self) {
        self.cursor = 0;
    }

    fn end(&mut self) {
        self.cursor = self.buf.len();
    }

    /// Clear the line back to empty without touching history — what Ctrl-C does on a non-empty
    /// line.
    fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.browsing = None;
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.browsing {
            None => {
                self.stash = self.line();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.browsing = Some(next);
        self.buf = self.history[next].chars().collect();
        self.cursor = self.buf.len();
    }

    fn history_down(&mut self) {
        let Some(i) = self.browsing else { return };
        if i + 1 >= self.history.len() {
            self.browsing = None;
            self.buf = self.stash.chars().collect();
        } else {
            self.browsing = Some(i + 1);
            self.buf = self.history[i + 1].chars().collect();
        }
        self.cursor = self.buf.len();
    }

    /// Finish the line: record it in history (skipping blanks and exact repeats of the last
    /// entry, so scrollback is not a wall of the same command), reset editing state, and hand
    /// back what was typed.
    fn submit(&mut self) -> String {
        let line = self.line();
        let trimmed = line.trim();
        if !trimmed.is_empty() && self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_string());
            if self.history.len() > HISTORY_CAP {
                self.history.remove(0);
            }
        }
        self.clear();
        line
    }
}

/// Ask the game task who is connected and what the groups are called, for Tab completion.
/// Bounded to a short timeout — a busy tick must never stall a keypress — and falls back to no
/// suggestions rather than blocking, since a completion popup that occasionally offers nothing is
/// far less surprising than a console that occasionally freezes.
fn ask_completion_context(events: &mpsc::Sender<ServerEvent>) -> (Vec<String>, Vec<String>) {
    let (reply, rx) = oneshot::channel();
    if events
        .blocking_send(ServerEvent::ConsoleContext { reply })
        .is_err()
    {
        return (Vec::new(), Vec::new());
    }
    // `blocking_recv` has no timeout of its own; the game task answers within a tick or two under
    // any real load, so a fixed short wait on a second thread is enough to bound it without
    // pulling in a timer-aware channel for a feature this small.
    let (tx, rx2) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(rx.blocking_recv());
    });
    match rx2.recv_timeout(Duration::from_millis(200)) {
        Ok(Ok(ctx)) => (ctx.players, ctx.groups),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Complete the word under the cursor, using command names for the first word and, for a known
/// second-argument-taking command, the roster or the group list.
///
/// Returns the completed line when exactly one candidate matches. More than one candidate is
/// printed as a plain list — through `term::print_notice`, not a round trip to the game task,
/// since the console already has everything it needs to say — rather than silently doing nothing:
/// a Tab that appears to do nothing is indistinguishable from a Tab that was not pressed.
fn complete(events: &mpsc::Sender<ServerEvent>, editor: &Editor) -> Option<String> {
    let line = editor.line();
    let before_cursor = &line[..line
        .char_indices()
        .nth(editor.cursor)
        .map_or(line.len(), |(i, _)| i)];
    let mut words: Vec<&str> = before_cursor.split(' ').collect();
    let partial = words.pop().unwrap_or("");

    let candidates: Vec<String> = if words.is_empty() || (words.len() == 1 && words[0].is_empty()) {
        COMMANDS
            .iter()
            .filter(|c| c.starts_with(partial))
            .map(|c| (*c).to_string())
            .collect()
    } else {
        let command = words[0];
        // Only asked when the command actually takes an argument worth completing — no point
        // paying for a round trip to the game task otherwise, and only once regardless of which
        // of the two lists this command wants.
        if !PLAYER_ARG_COMMANDS.contains(&command) && !GROUP_ARG_COMMANDS.contains(&command) {
            return None;
        }
        let (players, groups) = ask_completion_context(events);
        let pool = if PLAYER_ARG_COMMANDS.contains(&command) {
            players
        } else {
            groups
        };
        pool.into_iter()
            .filter(|n| n.starts_with(partial))
            .collect()
    };

    match candidates.as_slice() {
        [] => None,
        [one] => {
            let completed_word = one.clone();
            let prefix_len = before_cursor.len() - partial.len();
            let mut new_line = String::new();
            new_line.push_str(&line[..prefix_len]);
            new_line.push_str(&completed_word);
            new_line.push_str(&line[before_cursor.len()..]);
            Some(new_line)
        }
        many => {
            term::print_notice(&many.join("  "));
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
            None
        }
    }
}

/// The raw-mode loop. Runs on a dedicated blocking thread — see `spawn` — and never returns while
/// the process is meant to keep running.
fn run_sticky(events: &mpsc::Sender<ServerEvent>) {
    if terminal::enable_raw_mode().is_err() {
        // Could not get raw mode despite looking like a terminal (rare — a terminal that lies
        // about being one, or one already in a mode this can't negotiate). Fall back rather than
        // spin: a console that reads nothing is worse than a console that reads plainly.
        tracing::warn!("could not enter raw mode; falling back to plain console input");
        // No async runtime is entered here — `run_plain` is `async` because it uses tokio's
        // stdin, which needs one. Route back through a fresh current-thread runtime rather than
        // duplicate the read loop.
        let rt = match tokio::runtime::Builder::new_current_thread().build() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(run_plain(events.clone()));
        return;
    }
    // The terminal is now in raw mode, so its own `\n` -> `\r\n` translation is off; tell `term` so
    // every line it prints carries its own carriage return until we leave raw mode below.
    term::set_raw_mode(true);

    let mut editor = Editor::new();
    term::redraw_prompt(&editor.rendered(), editor.cursor_back());

    let result = (|| -> std::io::Result<()> {
        loop {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            // Unix reports only presses; Windows also reports releases and, with the enhanced
            // keyboard protocol, repeats. Only releases are worth ignoring — a repeat is a real
            // keystroke and should act like one.
            if key.kind == KeyEventKind::Release {
                continue;
            }

            match dispatch(events, &mut editor, key) {
                Dispatch::Continue => {}
                Dispatch::Stop => {
                    let _ = events.blocking_send(ServerEvent::Console {
                        line: "stop".to_string(),
                    });
                    return Ok(());
                }
                // A `stop` command was typed and already sent; leave the loop so the footer comes
                // down and the shutdown log prints on a clean, cooked-mode terminal below.
                Dispatch::Exit => return Ok(()),
                Dispatch::Closed => return Ok(()),
            }
        }
    })();

    term::clear_footer();
    let _ = terminal::disable_raw_mode();
    term::set_raw_mode(false);
    if let Err(e) = result {
        tracing::warn!(error = %e, "console input closed");
    }
}

enum Dispatch {
    Continue,
    Stop,
    /// A `stop` command was typed and its event already sent; the loop should end and tear the
    /// console down, so the server's shutdown log prints on a clean terminal rather than under a
    /// prompt this task keeps redrawing.
    Exit,
    /// The channel to the game task has closed — it is gone, nothing left to do here.
    Closed,
}

/// Handle one key event: update the editor, redraw if anything changed, and say what the caller
/// should do next.
fn dispatch(events: &mpsc::Sender<ServerEvent>, editor: &mut Editor, key: KeyEvent) -> Dispatch {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter => {
            // Commit what was typed into the scrollback above the fresh prompt, the way a shell
            // leaves your command on screen, so a reply below it is not an answer to a question the
            // log no longer shows. Blank lines are not echoed.
            let committed = editor.rendered();
            let echo = !editor.line().trim().is_empty();
            let line = editor.submit();
            let is_stop = line.trim() == "stop";
            term::redraw_prompt(&editor.rendered(), 0);
            if echo {
                term::print_notice(&committed);
            }
            if events.blocking_send(ServerEvent::Console { line }).is_err() {
                return Dispatch::Closed;
            }
            if is_stop {
                // The game task will save and shut down now. Bow out so the footer is cleared and
                // its goodbye prints cleanly, rather than parking here redrawing a prompt over it.
                return Dispatch::Exit;
            }
        }
        KeyCode::Char('c') if ctrl => {
            if editor.buf.is_empty() {
                return Dispatch::Stop;
            }
            editor.clear();
            term::redraw_prompt(&editor.rendered(), 0);
        }
        KeyCode::Char('d') if ctrl => {
            // Ctrl-D exits only on an empty line, the way a shell does. On a half-typed command it
            // clears the line instead, so an adjacent-key typo cannot silently discard the text and
            // take the whole server down with it.
            if editor.buf.is_empty() {
                return Dispatch::Stop;
            }
            editor.clear();
            term::redraw_prompt(&editor.rendered(), 0);
        }
        KeyCode::Char(c) if !ctrl => {
            editor.insert(c);
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Backspace => {
            editor.backspace();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Delete => {
            editor.delete_forward();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Left => {
            editor.left();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Right => {
            editor.right();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Home => {
            editor.home();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::End => {
            editor.end();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Up => {
            editor.history_up();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Down => {
            editor.history_down();
            term::redraw_prompt(&editor.rendered(), editor.cursor_back());
        }
        KeyCode::Tab => {
            if let Some(completed) = complete(events, editor) {
                editor.buf = completed.chars().collect();
                editor.cursor = editor.buf.len();
                term::redraw_prompt(&editor.rendered(), editor.cursor_back());
            }
        }
        _ => {}
    }
    Dispatch::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_inserts_at_the_cursor_not_always_at_the_end() {
        let mut e = Editor::new();
        for c in "helo".chars() {
            e.insert(c);
        }
        e.left();
        e.left();
        e.insert('l');
        assert_eq!(e.line(), "hello");
    }

    #[test]
    fn backspace_and_delete_remove_on_the_correct_side_of_the_cursor() {
        let mut e = Editor::new();
        for c in "hello".chars() {
            e.insert(c);
        }
        e.left();
        e.left();
        // Cursor is now between the two 'l's: "hel|lo"
        e.backspace();
        assert_eq!(
            e.line(),
            "helo",
            "backspace removes the first 'l', on the left"
        );
        // The cursor is now between the (remaining) 'l' and 'o': "he|lo"
        e.delete_forward();
        assert_eq!(
            e.line(),
            "heo",
            "delete_forward removes the char to the right of the cursor"
        );
    }

    #[test]
    fn home_and_end_move_to_the_edges() {
        let mut e = Editor::new();
        for c in "abc".chars() {
            e.insert(c);
        }
        e.home();
        assert_eq!(e.cursor, 0);
        e.end();
        assert_eq!(e.cursor, 3);
    }

    #[test]
    fn history_scrolls_up_and_back_down_to_the_line_being_typed() {
        let mut e = Editor::new();
        for c in "first".chars() {
            e.insert(c);
        }
        e.submit();
        for c in "second".chars() {
            e.insert(c);
        }
        e.submit();
        for c in "not yet sent".chars() {
            e.insert(c);
        }

        e.history_up();
        assert_eq!(e.line(), "second");
        e.history_up();
        assert_eq!(e.line(), "first");
        // Past the oldest entry, staying put rather than wrapping or going blank.
        e.history_up();
        assert_eq!(e.line(), "first");

        e.history_down();
        assert_eq!(e.line(), "second");
        e.history_down();
        assert_eq!(
            e.line(),
            "not yet sent",
            "should return to what was being typed"
        );
    }

    #[test]
    fn a_blank_line_and_an_exact_repeat_are_not_recorded() {
        let mut e = Editor::new();
        e.submit(); // blank
        assert!(e.history.is_empty());

        for c in "save".chars() {
            e.insert(c);
        }
        e.submit();
        for c in "save".chars() {
            e.insert(c);
        }
        e.submit();
        assert_eq!(
            e.history,
            vec!["save".to_string()],
            "an exact repeat should not double up"
        );
    }

    #[test]
    fn ctrl_c_clears_a_line_rather_than_ending_it() {
        // Exercised through `dispatch` would need a real channel; the editor-level behaviour is
        // what actually matters and is what `dispatch` calls into, so it is tested directly here.
        let mut e = Editor::new();
        for c in "half typed".chars() {
            e.insert(c);
        }
        e.clear();
        assert_eq!(e.line(), "");
    }

    #[test]
    fn command_names_complete_from_the_start_of_the_line() {
        // "ba" is ambiguous on purpose — "backups" and "ban" both match, in command-list order.
        let names: Vec<&&str> = COMMANDS.iter().filter(|c| c.starts_with("ba")).collect();
        assert_eq!(names, vec![&"backups", &"ban"]);
        // "ban" itself is unambiguous.
        let names: Vec<&&str> = COMMANDS.iter().filter(|c| c.starts_with("ban")).collect();
        assert_eq!(names, vec![&"ban"]);
        let names: Vec<&&str> = COMMANDS.iter().filter(|c| c.starts_with("s")).collect();
        assert!(names.contains(&&"say"));
        assert!(names.contains(&&"save"));
        assert!(names.contains(&&"stop"));
    }

    /// `help` is the list of record. Anything it tells an operator to type must also complete,
    /// or the console quietly offers a smaller server than the one it is running. This had already
    /// drifted by three commands.
    #[test]
    fn every_command_help_lists_can_be_completed() {
        let missing: Vec<&str> = crate::game::server::console_help_commands()
            .into_iter()
            .filter(|name| !COMMANDS.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "help lists these commands but tab completion does not know them: {missing:?}"
        );
    }

    /// And the other direction, so completion cannot offer something that does not exist. `help`
    /// does not list itself, which is the one allowed difference.
    #[test]
    fn completion_offers_nothing_help_does_not_list() {
        let listed = crate::game::server::console_help_commands();
        let extra: Vec<&&str> = COMMANDS
            .iter()
            .filter(|name| **name != "help" && !listed.contains(name))
            .collect();
        assert!(
            extra.is_empty(),
            "tab completion offers these, but help does not list them: {extra:?}"
        );
    }

    /// The guard against both checks passing by reading an empty list, the failure mode this
    /// project has now hit in several of its own checkers.
    #[test]
    fn the_help_command_list_is_actually_parsed() {
        let listed = crate::game::server::console_help_commands();
        assert!(
            listed.len() >= 15,
            "expected help to list at least fifteen commands, parsed {}: {listed:?}",
            listed.len()
        );
        assert!(listed.contains(&"mute"), "the parse dropped a real command");
    }
}
