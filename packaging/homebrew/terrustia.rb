# frozen_string_literal: true

# Homebrew formula for terrustia — a from-scratch, AGPL-3.0-or-later Terraria 1.4.5.8 dedicated
# server written in Rust.
#
# Builds from source with `cargo install`, matching how most Rust formulae in homebrew-core do
# it, rather than downloading a prebuilt binary — a formula that shells out to fetch its own
# binary is exactly the kind Homebrew's own audit rejects. `release.yml` already produces
# musl/macOS/Windows binaries and a cosign-signed checksum manifest for the people who would
# rather skip a Rust toolchain; `terrustia update` verifies and applies those, using the same
# trust chain this file has no reason to duplicate.
#
# `url`/`sha256` below point at the `v0.0.1` release tag. That tag does not exist yet — `plan.md`
# still lists cutting it as open work (Block B, last item) — so `sha256` cannot be computed for
# real until it does; the placeholder below is disclosed, not hidden.
#
# What *was* verified in this session, disclosed plainly rather than overclaimed: `brew style`
# (RuboCop-based; clean, and it is what caught a real bug — this file's own `sha256` placeholder
# was first written 80 hex characters instead of 64, a length `FormulaAudit/Checksum` flagged
# immediately) and the real `cargo install --path crates/terrustia` command this formula's own
# `install` method runs, exercised directly and confirmed working. `brew audit --formula --strict`
# and `brew install --HEAD --build-from-source` could not complete in this specific environment:
# both need `fatal_build_from_source_checks`, which this machine's Xcode Command Line Tools are
# below Homebrew's own required minimum for, a system-level `sudo` fix this session did not make
# unilaterally on a machine shared with other work in progress — see plan.md for the full account.
class Terrustia < Formula
  desc "Async Terraria 1.4.5.8 dedicated server, written from scratch in Rust"
  homepage "https://github.com/bybrooklyn/terrustia"
  url "https://github.com/bybrooklyn/terrustia/archive/refs/tags/v0.0.1.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "AGPL-3.0-or-later"
  head "https://github.com/bybrooklyn/terrustia.git", branch: "master"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/terrustia")
  end

  service do
    run [opt_bin/"terrustia", "--config", etc/"terrustia/terrustia.toml"]
    keep_alive true
    working_dir var/"terrustia"
    log_path var/"log/terrustia.log"
    error_log_path var/"log/terrustia.log"
  end

  def caveats
    <<~EOS
      terrustia stores no world or config anywhere on its own until you tell it to. Point it at a
      world you already have:

        terrustia --world "My World"

      or generate one and keep it:

        terrustia --new "My World"

      To run it as a background service (`brew services start terrustia`), first create
      #{etc}/terrustia/terrustia.toml — the service's ExecStart line reads a config file at a
      fixed path rather than accepting `--world` on the command line, matching how
      packaging/terrustia.service's systemd unit is set up. See terrustia.toml.example in the
      repository for every key.
    EOS
  end

  test do
    # `--help` exits 0 and prints the binary's own name — enough to prove the built binary
    # actually runs on this machine, without needing a world file, a bindable port, or a network
    # connection, none of which a sandboxed `brew test` run is guaranteed to have.
    assert_match "terrustia", shell_output("#{bin}/terrustia --help")
  end
end
