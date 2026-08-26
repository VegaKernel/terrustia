# terrustia web panel

The frontend for the web admin panel — Svelte 5 + TypeScript, built with Vite, no UI kit. The
built output (`dist/`) is embedded into the `terrustia` binary via `rust-embed`; see
`crates/terrustia/src/panel/mod.rs`'s module doc for the bundling mechanism and its `../alchemist`
citation.

```sh
npm install
npm run dev      # local dev server against a running terrustia panel backend
npm run build    # writes dist/, which the Rust build embeds
npm run check    # svelte-check + tsc, no build
```

Design language: TUI-inspired, echoing the sticky console's own aesthetic rather than a generic
component-kit look — see `crates/terrustia/src/game/console.rs`/`term.rs` for the terminal this is
meant to feel like an extension of.
