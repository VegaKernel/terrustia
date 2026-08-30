#!/usr/bin/env bash
#
# A scale soak: put N headless clients on one world in a single join burst, hold for a while, and
# then check the things a person would notice. This is the 255-player qualification run in a form
# that can be repeated; tools/soak_ci.sh is the small three-player version that runs per commit.
#
# Every client here connects from 127.0.0.1, so the server's per-address connection cap (a real
# anti-DoS limit, default 8) would refuse all but a handful. Real players arrive from distinct
# addresses, so for this local test we lift the per-address cap to the player count. Nothing else
# about the server is loosened.
#
#   ./tools/soak_scale.sh [players] [seconds]
#
# Defaults: 255 players, a 1800s (30 minute) hold. Exits non-zero if the server panics, dies, or
# saves nothing, or if fewer than 90% of the clients get on.

set -uo pipefail

PLAYERS="${1:-255}"
HOLD="${2:-1800}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d)"
BIN="$ROOT/target/release/terrustia"
SOAK="$ROOT/target/release/examples/soak"

trap 'kill "${SRV:-}" "${SAMPLER:-}" 2>/dev/null; rm -rf "$WORK"' EXIT

[ -x "$BIN" ]  || { echo "build first: cargo build --release -p terrustia --bin terrustia"; exit 1; }
[ -x "$SOAK" ] || { echo "build first: cargo build --release -p terrustia-client --example soak"; exit 1; }

PORT=$((20000 + RANDOM % 20000))
echo "=== server: max_players=$PLAYERS, world 4200x1200, port $PORT ==="
# Lift the per-address cap (all clients share 127.0.0.1) and give max_connections a little headroom
# over the player count. Every other limit is left at its default.
TERRUSTIA_LOG=info \
  TERRUSTIA_MAX_PLAYERS="$PLAYERS" \
  TERRUSTIA_MAX_CONNECTIONS="$((PLAYERS + 16))" \
  TERRUSTIA_MAX_CONNECTIONS_PER_ADDRESS="$PLAYERS" \
  "$BIN" --headless --new soakscale --listen "127.0.0.1:$PORT" --save "$WORK/soak.wld" \
  > "$WORK/server.log" 2>&1 &
SRV=$!

# Wait for the server to be listening by its own log line, not a guessed sleep.
ready=0
for _ in $(seq 1 120); do
  grep -q "accepting connections" "$WORK/server.log" 2>/dev/null && { ready=1; break; }
  kill -0 "$SRV" 2>/dev/null || { echo "FAIL: server exited during startup"; tail -20 "$WORK/server.log"; exit 1; }
  sleep 1
done
[ "$ready" = 1 ] || { echo "FAIL: server never became ready"; tail -20 "$WORK/server.log"; exit 1; }

rss_kib() { ps -o rss= -p "$1" 2>/dev/null | tr -d ' '; }
MEM_START=$(rss_kib "$SRV")
echo "server ready; RSS at start: $((MEM_START/1024)) MiB"

echo "=== launching $PLAYERS clients in one burst for ${HOLD}s ==="
CPIDS=()
for i in $(seq 1 "$PLAYERS"); do
  depth=$(( (i % 60) * 8 + 200 ))
  "$SOAK" "127.0.0.1:$PORT" "$HOLD" "$depth" "soak$i" > "$WORK/c$i.log" 2>&1 &
  CPIDS+=($!)
done
echo "all $PLAYERS clients launched"

# Sample the server's RSS every two minutes across the hold, so a leak shows as a rising curve
# rather than a single end number.
: > "$WORK/mem.log"
( t=0
  while kill -0 "$SRV" 2>/dev/null; do
    echo "t=$(printf '%5d' "$t")s  RSS=$(( $(rss_kib "$SRV")/1024 )) MiB"
    sleep 120; t=$((t+120))
  done ) >> "$WORK/mem.log" 2>&1 &
SAMPLER=$!

ok=0; kicked=0
for pid in "${CPIDS[@]}"; do
  if wait "$pid"; then ok=$((ok+1)); else kicked=$((kicked+1)); fi
done
kill "$SAMPLER" 2>/dev/null
MEM_END=$(rss_kib "$SRV")

echo ""
echo "=== RESULTS ==="
echo "clients exited clean: $ok / $PLAYERS   (non-zero: $kicked)"
peak_slot=$(grep -oE 'slot=[0-9]+' "$WORK/server.log" | grep -oE '[0-9]+' | sort -n | tail -1)
echo "peak slot the server assigned: ${peak_slot:-none}"
echo "server RSS: start $((MEM_START/1024)) MiB -> end $(( ${MEM_END:-0}/1024 )) MiB"
echo "--- RSS curve over the hold (plateau vs leak) ---"; cat "$WORK/mem.log" 2>/dev/null || echo "(no samples)"
echo "--- server tick cost: its own cpu vs any external stall ---"
grep -oE 'cpu_us=[0-9]+' "$WORK/server.log" | grep -oE '[0-9]+' | sort -n | tail -1 \
  | xargs -I{} echo "highest cpu_us sample: {} us (budget 16667 us/tick)"
grep -c "held off the processor" "$WORK/server.log" | xargs echo "external-stall warnings (test box, not the server):"

# Shut the server down and confirm it writes a world on the way out.
kill "$SRV" 2>/dev/null
for _ in $(seq 1 30); do kill -0 "$SRV" 2>/dev/null || break; sleep 1; done
saved_bytes=$(stat -f%z "$WORK/soak.wld" 2>/dev/null || stat -c%s "$WORK/soak.wld" 2>/dev/null || echo 0)

echo ""
echo "=== VERDICT ==="
fail=0
panics=$(grep -icE "panic|thread .* panicked" "$WORK/server.log")
[ "$panics" -eq 0 ] && echo "PASS  no panics" || { echo "FAIL  $panics panic line(s)"; fail=1; }
if [ "$ok" -ge $(( PLAYERS * 9 / 10 )) ]; then echo "PASS  $ok/$PLAYERS clients connected and held"; else echo "FAIL  only $ok/$PLAYERS clients held"; fail=1; fi
if [ "$saved_bytes" -gt 0 ]; then echo "PASS  world saved on shutdown ($saved_bytes bytes)"; else echo "FAIL  no world written on shutdown"; fail=1; fi
[ "$fail" -eq 0 ] && echo "=== soak PASSED ===" || echo "=== soak FAILED ==="
exit "$fail"
