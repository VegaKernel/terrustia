# Contributing

Contributions are welcome: bug reports, fixes, worldgen and AI parity work, docs, packaging, tests.
Open an issue if you want to discuss something first, or send a pull request directly for a
self-contained change.

## The bar for a change

Every change is held to the same standard the rest of the project follows:

1. It matches vanilla where it transcribes vanilla. This is a reimplementation of the 1.4.5.8
   dedicated server, so behaviour is checked against the decompiled game, not against a guess.
2. It has a test that fails on the unfixed code and passes on the fixed code, where a test can
   express it.
3. It builds clean: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` with no
   warnings, and `cargo test --workspace` green.
4. It is honest about its own limits. A partial implementation says what it does not do, in the code
   and in the README row, rather than implying completeness.

`docs/` and `plan.md` describe how the project is put together and how it verifies itself against
the real game. Reading the relevant part before a large change saves a round trip.

## AI-assisted contributions

AI-assisted contributions are welcome. Much of this project was written that way. The condition is
that a human is accountable for every line:

- You have read and understood the change you are submitting. You can explain why it is correct and
  answer questions about it in review.
- It meets the bar above, including a real test and a clean build. AI output that was not run,
  tested, or checked against the decompiled source is not acceptable, whether a person or a model
  produced it.
- You do not paste decompiled game source, game assets, or game text into the repository. The data
  tables are generated from a local decompiled tree that never ships; see `docs/generated-tables.md`.

A change that clears the bar is judged on the change, not on how it was written. A change that does
not clear it is declined, for the same reason.

## Contributor license terms

By submitting a contribution (a pull request, a patch, or any other work) to this project, you agree
to the following. Read them, because they are broader than an ordinary inbound license.

1. **You have the right to submit it.** The contribution is your own work, or you have the right to
   submit it under these terms, and submitting it does not knowingly violate anyone else's rights.

2. **You keep your copyright.** You are not assigning ownership of your contribution to anyone.

3. **You grant a broad, relicensable license.** You grant the project maintainer (the owner of the
   `github.com/bybrooklyn/terrustia` repository) a perpetual, irrevocable, worldwide, royalty-free,
   sublicensable, and transferable license to use, reproduce, modify, prepare derivative works of,
   publicly display, distribute, and **relicense** your contribution, in whole or in part, under any
   license terms, including licenses different from the project's current one and including
   proprietary terms.

This grant is what lets the project change its license later (for example, to dual-license or to
move to a different open-source license) without tracking down every past contributor. Your
contribution stays available to everyone under the project's public license at the time it was
made; the grant is in addition to that, not instead of it.

If you cannot agree to these terms for a particular contribution, say so in the pull request rather
than submitting it, and we can work out another way to get the change in.
