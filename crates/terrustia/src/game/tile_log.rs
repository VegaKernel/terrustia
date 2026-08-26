//! A trailing log of player tile edits, for `/world undo <player> <duration>`.
//!
//! **Admin-only, time-windowed, in-memory.** This is an operator's grief-recovery tool — reverting
//! an accident or an active griefer — not a per-player self-service feature, matching this
//! project's existing security model (`SECURITY.md`: "whoever can read the console can control the
//! server"). It is bounded by age rather than count: entries older than [`RETENTION`] are dropped
//! as new ones arrive, so the log cannot grow without bound on a long-running server, and a server
//! nobody is editing costs nothing to keep this around.
//!
//! **Does not survive a restart, on purpose.** Persisting a rolling window of raw tile history to
//! disk is a different, larger feature (durable audit trail) than what this is — an operator's
//! short-lived "undo the last few minutes of damage" tool. Vanilla has nothing like this at all;
//! there is no parity bar to match. A restart clearing the window is the honest cost of keeping
//! this simple, not a silent gap: it is disclosed here and in `README.md`'s row for it.
//!
//! **Scope, also disclosed rather than silent**: only edits through [`super::server`]'s
//! `on_tile_manipulation` handler are logged — ordinary single-tile break/place/wall/wire/actuator/
//! slope edits, which is the overwhelming majority of what a player or a griefer actually does.
//! Bulk multi-tile edits from `on_tile_square` (the wire tool's drag-paint) are not logged, so
//! `world undo` cannot recover damage done that way. Framed multi-tile objects placed through
//! `place_object`-style machinery are also outside `on_tile_manipulation`'s own model already (see
//! that handler's own `frame_important` branch) and so are outside this log's reach too.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use terrustia_proto::Tile;

/// How long a tile edit stays undoable before it ages out of the log.
pub const RETENTION: Duration = Duration::from_secs(72 * 60 * 60);

/// One player edit, enough to put the tile back exactly as it was.
struct TileEdit {
    x: i32,
    y: i32,
    /// The tile's full state *before* this edit — not a diff, since a slope/wire/wall change can
    /// touch several flags at once and reconstructing "what it must have been" from the new state
    /// alone is exactly the kind of guess this project avoids.
    before: Tile,
    /// Lowercased once, at log time, so a lookup by name is a plain comparison rather than a
    /// case-fold on every entry in the window.
    player_lower: String,
    at: Instant,
}

/// The trailing edit log, owned by [`super::server::GameServer`].
#[derive(Default)]
pub struct TileLog {
    edits: VecDeque<TileEdit>,
}

impl TileLog {
    /// Record one edit. Prunes anything that has aged out of [`RETENTION`] first, so the log's
    /// size is always bounded by how much editing has actually happened in the window, not by
    /// every edit since the server started.
    pub fn record(&mut self, x: i32, y: i32, before: Tile, player: &str) {
        self.prune();
        self.edits.push_back(TileEdit {
            x,
            y,
            before,
            player_lower: player.to_ascii_lowercase(),
            at: Instant::now(),
        });
    }

    fn prune(&mut self) {
        let cutoff = Instant::now().checked_sub(RETENTION);
        while let Some(front) = self.edits.front() {
            if cutoff.is_some_and(|cutoff| front.at < cutoff) {
                self.edits.pop_front();
            } else {
                break;
            }
        }
    }

    /// Every edit by `player` within the trailing `within` duration, most-recent-first — so a
    /// caller reverting them in this order undoes the newest damage first, which matters if it
    /// stops partway through an ongoing attack rather than running to completion.
    ///
    /// Removes what it returns from the log: an edit that has already been undone should not be
    /// undoable a second time onto whatever the world looks like now.
    pub fn take_recent(&mut self, player: &str, within: Duration) -> Vec<(i32, i32, Tile)> {
        self.prune();
        let player_lower = player.to_ascii_lowercase();
        let cutoff = Instant::now().checked_sub(within);
        let mut taken = Vec::new();
        self.edits.retain(|edit| {
            let matches =
                edit.player_lower == player_lower && cutoff.is_none_or(|cutoff| edit.at >= cutoff);
            if matches {
                taken.push((edit.x, edit.y, edit.before));
            }
            !matches
        });
        taken.reverse(); // retain() walks front-to-back (oldest-first); undo wants newest-first.
        taken
    }
}

/// `10m`, `2h`, `1d`, `90` (bare seconds), `1h30m` — a small, forgiving parser for a chat command
/// argument. Rejects anything it cannot make sense of rather than guessing.
pub fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(secs) = text.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let mut total = 0u64;
    let mut number = String::new();
    let mut saw_any = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
            continue;
        }
        let scale = match ch.to_ascii_lowercase() {
            's' => 1,
            'm' => 60,
            'h' => 60 * 60,
            'd' => 24 * 60 * 60,
            _ => return None,
        };
        let n: u64 = number.parse().ok()?;
        number.clear();
        total = total.checked_add(n.checked_mul(scale)?)?;
        saw_any = true;
    }
    if !number.is_empty() {
        return None; // a trailing number with no unit — "10m5" — is ambiguous, not seconds.
    }
    saw_any.then(|| Duration::from_secs(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(block: u16) -> Tile {
        let mut t = Tile::AIR;
        t.block = block;
        t
    }

    #[test]
    fn a_recent_edit_by_the_right_player_is_returned() {
        let mut log = TileLog::default();
        log.record(5, 5, tile(1), "Steve");
        let taken = log.take_recent("steve", Duration::from_secs(3600));
        assert_eq!(taken, vec![(5, 5, tile(1))], "case-insensitive name match");
    }

    #[test]
    fn a_different_players_edit_is_left_alone() {
        let mut log = TileLog::default();
        log.record(5, 5, tile(1), "Steve");
        log.record(6, 6, tile(2), "Alex");
        let taken = log.take_recent("steve", Duration::from_secs(3600));
        assert_eq!(taken, vec![(5, 5, tile(1))]);
        // Alex's edit is still in the log — a lookup for Steve must not have consumed it too.
        let alex = log.take_recent("alex", Duration::from_secs(3600));
        assert_eq!(alex, vec![(6, 6, tile(2))]);
    }

    #[test]
    fn taken_edits_are_removed_so_undo_cannot_double_apply() {
        let mut log = TileLog::default();
        log.record(1, 1, tile(9), "Steve");
        let first = log.take_recent("steve", Duration::from_secs(3600));
        assert_eq!(first.len(), 1);
        let second = log.take_recent("steve", Duration::from_secs(3600));
        assert!(
            second.is_empty(),
            "an already-undone edit must not be returned twice"
        );
    }

    #[test]
    fn most_recent_edit_comes_back_first() {
        let mut log = TileLog::default();
        log.record(1, 1, tile(1), "Steve");
        log.record(2, 2, tile(2), "Steve");
        log.record(3, 3, tile(3), "Steve");
        let taken = log.take_recent("steve", Duration::from_secs(3600));
        assert_eq!(
            taken,
            vec![(3, 3, tile(3)), (2, 2, tile(2)), (1, 1, tile(1))],
            "newest-first, so a partial undo during an ongoing attack still helps"
        );
    }

    #[test]
    fn duration_parsing_understands_suffixes_and_bare_seconds() {
        assert_eq!(parse_duration("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("10m"), Some(Duration::from_secs(600)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("1d"), Some(Duration::from_secs(86400)));
        assert_eq!(
            parse_duration("1h30m"),
            Some(Duration::from_secs(5400)),
            "compound durations"
        );
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(
            parse_duration("10m5"),
            None,
            "a trailing bare number is ambiguous"
        );
    }
}
