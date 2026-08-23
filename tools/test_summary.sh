#!/usr/bin/env bash
# Run the whole test suite and report honestly.
#
# `cargo test` stops at the first failing target, so a naive count of "N passed" lines both
# misses the failure and silently omits every target after it. That happened: a summary read
# 1,052 passing while a handshake test was red and the whole proto crate had never run.
#
# This reports the number of targets that ran, how many failed, and the total — and exits
# non-zero if anything failed or if a target was skipped.
set -uo pipefail

out=$(cargo test --no-fail-fast "$@" 2>&1)
status=$?

passed=$(echo "$out" | grep -oE 'test result: (ok|FAILED)\. [0-9]+ passed' \
    | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | paste -sd+ - | bc)
failed=$(echo "$out" | grep -oE '[0-9]+ failed' | grep -oE '[0-9]+' | paste -sd+ - | bc)
targets=$(echo "$out" | grep -cE '^test result:')

echo "targets  ${targets}"
echo "passing  ${passed:-0}"
echo "failing  ${failed:-0}"

if [ "${failed:-0}" -ne 0 ]; then
    echo
    echo "FAILURES:"
    echo "$out" | grep -E '^test .* FAILED$' | sed 's/^/  /'
    exit 1
fi
exit $status
