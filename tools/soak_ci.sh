#!/usr/bin/env bash
#
# A minute of a real server with real clients, with assertions.
#
# The suite is 1,400 self-consistent tests and they are worth having, but they are structurally
# unable to notice a server that is *working* while doing something absurd. This one once sent
# 18,165 door packets in five minutes — 48% of everything it sent — behind a completely green
# suite, because a town NPC kept opening a door the server had not recorded as opened. Nothing
# failed. It was found months later by somebody reading a log.
#
# So: start a server, put clients on it, run for a minute, and then check the things a person
# would have noticed.
#
#   ./tools/soak_ci.sh [seconds]

set -euo pipefail

SECONDS_TO_RUN="${1:-60}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d)"
PORT=$((20000 + RANDOM % 20000))
trap 'rm -rf "$WORK"; [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null || true' EXIT

BIN="$ROOT/target/release/terrustia"
[[ -x "$BIN" ]] || { echo "build first: cargo build --release --workspace --bins --examples"; exit 1; }

cat > "$WORK/soak.toml" <<EOF
listen = "127.0.0.1:$PORT"
max_players = 8
motd = "soak"
world_name = "Soak"
world_width = 4200
world_height = 1200
seed = 12345
save_file = "$WORK/soak.wld"
autosave_secs = 20
EOF

echo "generating a world and starting a server on $PORT"
TERRUSTIA_LOG=debug "$BIN" -c "$WORK/soak.toml" > "$WORK/server.log" 2>&1 &
SERVER_PID=$!

# Wait for it to be listening rather than sleeping a guessed amount.
for _ in $(seq 1 120); do
    grep -q "accepting connections" "$WORK/server.log" && break
    sleep 1
done
grep -q "accepting connections" "$WORK/server.log" || { echo "server never started:"; cat "$WORK/server.log"; exit 1; }

echo "putting three players on it for ${SECONDS_TO_RUN}s"
CLIENT_PIDS=()
for i in 1 2 3; do
    # Unique name per client (the server kicks duplicates) and a different depth so they spread out.
    "$ROOT/target/release/examples/soak" "127.0.0.1:$PORT" "$SECONDS_TO_RUN" "$((i * 8))" "soak$i" \
        > "$WORK/client$i.log" 2>&1 &
    CLIENT_PIDS+=($!)
done

# Each client must actually exit 0. A client kicked at the door (duplicate name, server full) exits
# non-zero, which used to be swallowed by `wait ... || true` — turning a three-player soak into a
# one-player one with nothing red.
client_failures=0
for i in 1 2 3; do
    if ! wait "${CLIENT_PIDS[$((i - 1))]}"; then
        echo "FAIL: soak client $i did not exit cleanly:"
        cat "$WORK/client$i.log"
        client_failures=$((client_failures + 1))
    fi
    if ! grep -q "joined at" "$WORK/client$i.log"; then
        echo "FAIL: soak client $i never reported joining:"
        cat "$WORK/client$i.log"
        client_failures=$((client_failures + 1))
    fi
done

# Give the last tick a moment to land in the log, then stop cleanly so the shutdown save runs.
sleep 2
kill -TERM "$SERVER_PID" 2>/dev/null || true
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=

fail=0
note() { echo "  $1"; }
check() { if [[ $2 -eq 0 ]]; then note "ok    $1"; else note "FAIL  $1"; fail=1; fi; }

echo
echo "checks:"

# 0. All three clients had to connect and run. Two of three used to be kicked for a duplicate name
#    and exit non-zero silently, quietly turning a three-player soak into a one-player one.
[[ ${client_failures:-0} -eq 0 ]]
check "all three soak clients connected and ran" $?

# 1. It has to have survived. A panic now exits non-zero and says so.
! grep -qE "panicked|the game loop panicked|game task died" "$WORK/server.log"
check "no panic" $?

# 2. The world has to have reached disk.
[[ -f "$WORK/soak.wld" ]]
check "world saved" $?

# 3. No tick may blow its budget. The phase breakdown names the culprit when one does.
if grep -q "ticks are using a lot of their budget" "$WORK/server.log"; then
    note "FAIL  a tick went over half its budget:"
    grep "ticks are using a lot of their budget" "$WORK/server.log" | tail -3 | sed 's/^/        /'
    fail=1
else
    note "ok    every tick stayed inside its budget"
fi

# 4. The autosave must not be copying the whole world every time. On an idle world it should be
#    copying no sections at all; a number equal to every section means the incremental path has
#    silently stopped working, which is a 40 MB memcpy back on the game task.
if grep -q "sections_copied" "$WORK/server.log"; then
    worst=$(grep -o "sections_copied=[0-9]*" "$WORK/server.log" | cut -d= -f2 | sort -n | tail -1)
    [[ "${worst:-0}" -lt 200 ]]
    check "snapshots stayed incremental (worst ${worst:-0} sections)" $?
else
    note "ok    no incremental snapshot taken (too short a run to autosave twice)"
fi

# 5. Nothing may be shouting. A packet storm shows up here long before anybody measures bandwidth.
storm=$(grep -c "WARN" "$WORK/server.log" || true)
[[ "$storm" -lt 20 ]]
check "log is quiet ($storm warnings)" $?

echo
if [[ $fail -ne 0 ]]; then
    echo "soak failed; last of the server log:"
    tail -40 "$WORK/server.log"
    exit 1
fi
echo "soak passed"
