#!/usr/bin/env bash
#
# Reclaim build space across the repo and every agent worktree.
#
# A worktree carries its own `target/`, and a handful of them at 3 to 8 GiB each is how this
# checkout has twice run a disk to nearly nothing. This cleans all of them, plus the main tree.
#
#   ./tools/clean_all.sh                  clean everything
#   ./tools/clean_all.sh --keep NAME...   clean everything except worktrees whose path matches NAME
#   ./tools/clean_all.sh --dry-run        report sizes and do nothing
#
# Cleaning a worktree an agent is actively building in destroys its build state and wastes the
# rebuild, so pass --keep for anything still running.
#
# DO NOT point several worktrees at one CARGO_TARGET_DIR to save space. It looks like the obvious
# fix and it silently produces wrong builds. Cargo's unit hash for a workspace-local crate does not
# include the worktree path, so every worktree's `terrustia-proto` writes the same artefact filename
# and the last writer wins: the next worktree's `terrustia` then links against a sibling's proto
# crate. The lock cargo takes on a target directory makes concurrent builds serialise, which is a
# different guarantee from making them correct, and it is easy to mistake one for the other.
#
# The failure is quiet and misleading rather than loud. It surfaced here as a reproducible
# `unresolved import terrustia_proto::happiness` in a worktree whose source did define it, with
# `cargo test -p terrustia-proto` passing while `cargo test --workspace` failed. Identical artefact
# hashes were confirmed across both directories. If several trees must build at once, give each its
# own target directory and clean it afterwards, which is what this script is for.

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DRY=0
KEEP=()
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1; shift ;;
    --keep)    shift; while [ $# -gt 0 ] && [[ "$1" != --* ]]; do KEEP+=("$1"); shift; done ;;
    *)         echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

kept() {
  local path="$1"
  for k in "${KEEP[@]+"${KEEP[@]}"}"; do
    [[ "$path" == *"$k"* ]] && return 0
  done
  return 1
}

before=$(df -g "$ROOT" | awk 'NR==2 {print $4}')
freed=0

# The main tree first, then each worktree. `cargo clean` is used rather than `rm -rf` so a
# workspace with a custom target directory is still honoured.
for manifest in "$ROOT/Cargo.toml" "$ROOT"/.claude/worktrees/*/Cargo.toml; do
  [ -f "$manifest" ] || continue
  dir="$(dirname "$manifest")"
  [ -d "$dir/target" ] || continue

  size=$(du -sm "$dir/target" 2>/dev/null | cut -f1)
  name="${dir#"$ROOT"/}"
  [ "$dir" = "$ROOT" ] && name="(main tree)"

  if kept "$dir"; then
    echo "keep   ${size} MiB  $name"
    continue
  fi
  if [ "$DRY" = 1 ]; then
    echo "would  ${size} MiB  $name"
    continue
  fi

  cargo clean --manifest-path "$manifest" >/dev/null 2>&1
  echo "clean  ${size} MiB  $name"
  freed=$((freed + size))
done

after=$(df -g "$ROOT" | awk 'NR==2 {print $4}')
if [ "$DRY" = 1 ]; then
  echo "dry run, nothing removed. ${before} GiB free."
else
  echo "freed about $((freed / 1024)) GiB. ${before} -> ${after} GiB free."
fi
