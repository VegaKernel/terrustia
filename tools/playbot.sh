#!/usr/bin/env bash
#
# Play the game toward a player's goals and report the ones that could not be reached.
#
#   ./tools/playbot.sh
#
# Two thousand unit tests were green while Moon Lord could not be killed, chests did nothing
# server-side, doors reverted on reload, and Gel dropped from nothing. Every one of those was
# found by a person playing for an hour, because every one of them is an *absence*: nothing is
# ever in an illegal state, the thing simply never happens. This walks the goals instead.
#
# It owns the server's lifecycle because two of the goals are about the save: place a chest, put
# something in it, open a door, write the world to disk, restart on the same file, and go back to
# look. That needs a real reload of a real world file, not a re-read of memory.
#
# Exits non-zero if any goal was missed. Missed goals are the point.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d)"
PORT=$((20000 + RANDOM % 20000))
SERVER_PID=
trap 'rm -rf "$WORK"; [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null; true' EXIT

BIN="$ROOT/target/release/terrustia"
BOT="$ROOT/target/release/examples/playbot"
for exe in "$BIN" "$BOT"; do
    [[ -x "$exe" ]] || {
        echo "build first: cargo build --release -p terrustia -p terrustia-client --bins --examples"
        exit 1
    }
done

cat > "$WORK/playbot.toml" <<EOF
listen = "127.0.0.1:$PORT"
max_players = 4
motd = ""
world_name = "Playbot"
world_width = 4200
world_height = 1200
seed = 4242
save_file = "$WORK/playbot.wld"
autosave_secs = 0
EOF

# Start the server and wait for it to be listening, rather than sleeping a guessed amount.
#
# A log per run, because the restart is watched for the same line the first start already wrote:
# appending to one file makes the second `grep` match the *first* server's line and hand the bot a
# port nothing is listening on yet.
start_server() {
    RUN_LOG="$WORK/server-$1.log"
    shift
    TERRUSTIA_LOG="${TERRUSTIA_LOG:-info}" "$BIN" -c "$WORK/playbot.toml" "$@" > "$RUN_LOG" 2>&1 &
    SERVER_PID=$!
    for _ in $(seq 1 240); do
        grep -q "accepting connections" "$RUN_LOG" && return 0
        kill -0 "$SERVER_PID" 2>/dev/null || break
        sleep 1
    done
    echo "the server never started:"
    tail -40 "$RUN_LOG"
    return 1
}

stop_server() {
    # SIGTERM so the shutdown save runs, which is the thing the second half reads back.
    kill -TERM "$SERVER_PID" 2>/dev/null
    wait "$SERVER_PID" 2>/dev/null
    SERVER_PID=
}

echo "generating a 4200x1200 world and starting a server on $PORT"
start_server first || exit 1

echo
echo "=== playing ==="
"$BOT" "127.0.0.1:$PORT" build "$WORK/playbot.state"
build_status=$?

echo
echo "saving and restarting the server on the same world file"
stop_server
[[ -f "$WORK/playbot.wld" ]] || { echo "FAIL: the world never reached disk"; exit 1; }
# `--world`, not the config's `save_file`. `save_file` says only where to *write*: a server given
# one and no `world_file` generates a brand-new world on every boot and saves over the file it was
# handed. Without this the second half plays a fresh world that has never seen the chest, and
# reports the save as broken when it was the restart that was wrong.
start_server second --world "$WORK/playbot.wld" || exit 1

echo
echo "=== playing again, on the reloaded world ==="
"$BOT" "127.0.0.1:$PORT" verify "$WORK/playbot.state"
verify_status=$?

stop_server

echo
if grep -qE "panicked|the game loop panicked|game task died" "$WORK"/server-*.log; then
    echo "the server panicked while being played:"
    grep -hE "panicked|game task died" "$WORK"/server-*.log | head -5
    exit 1
fi

if [[ $build_status -eq 0 && $verify_status -eq 0 ]]; then
    echo "every goal was reached: this server can be played."
    exit 0
fi
echo "goals were missed; see the two reports above."
exit 1
