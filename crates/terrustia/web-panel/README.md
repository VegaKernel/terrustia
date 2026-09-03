# terrustia web panel

The frontend for the web admin panel — Svelte 5 + TypeScript, built with Vite, no UI kit. The
built output (`dist/`) is embedded into the `terrustia` binary via `rust-embed`; see
`crates/terrustia/src/panel/mod.rs`'s module doc for the bundling mechanism and its `../alchemist`
citation.

```sh
bun install
bun run dev      # local dev server against a running terrustia panel backend
bun run build    # writes dist/, which the Rust build embeds
bun run check    # svelte-check + tsc, no build
```

Design language: TUI-inspired, echoing the sticky console's own aesthetic rather than a generic
component-kit look — see `crates/terrustia/src/game/console.rs`/`term.rs` for the terminal this is
meant to feel like an extension of. Monospace-only: no web fonts, no icon fonts, nothing the panel
ever fetches over the network to render correctly — `src/app.css`'s top comment says so directly.
Labels read in the app's own lowercase, TUI-style case ("world", "saves to", "listening"), never
shouted in caps.

## Design tokens

Every colour, spacing and radius value in the panel is one of the tokens below, defined once in
`src/app.css`'s `:root` and read everywhere else through `var(--x)` — including the two places that
draw with a real `<canvas>` 2D context rather than CSS (`Metrics.svelte`'s sparklines,
`WorldView.svelte`'s avatar/UI colours, not its tile palette — see below), which read the same
custom properties at draw time via `getComputedStyle` instead of keeping a second hardcoded copy.
Adding a new literal colour, radius or spacing value outside this table anywhere in the panel is
very likely the wrong move; extend the table first.

### Colour

| token | value | used for |
|---|---|---|
| `--bg` | `#0a0d10` | page background, the darkest surface |
| `--bg-raised` | `#10151a` | cards, panels, dialogs, table headers — one step up from `--bg` |
| `--border` | `#1f2a30` | every 1px border and table rule |
| `--text` | `#d7dde1` | primary text |
| `--text-dim` | `#7c8791` | secondary text, labels, captions |
| `--accent` | `#59d499` | the panel's one accent colour: links, active state, healthy values |
| `--accent-dim` | `#2c5b45` | accent-toned borders/fills that need to sit quietly (e.g. an active tab's background) |
| `--warn` | `#e0b34d` | warnings, the players-online chart |
| `--danger` | `#e0645a` | errors, destructive actions, PvP indicators, the tick-budget threshold line |

`WorldView.svelte`'s `TILE_COLORS` map (the world render's terrain palette) is deliberately outside
this table: it is its own small, documented design system — a stylized, procedural render, never a
transcription of Terraria's own art — not applications of the app-chrome tokens above.

### Spacing

A 4px grid, six steps. Prefer the token over a bare `rem`/`px` value in any new or edited rule.

| token | value | used for |
|---|---|---|
| `--sp-1` | 4px | tight inline gaps |
| `--sp-2` | 8px | a control's own padding, a row's internal gap |
| `--sp-3` | 12px | gap between cards/stats in a grid |
| `--sp-4` | 16px | a panel's inner padding, a section's top margin |
| `--sp-5` | 24px | a view's outer padding |
| `--sp-6` | 32px | between major sections |

### Radius

Three steps, mapped to component kind — never a fourth value:

| token | value | used for |
|---|---|---|
| `--radius-sm` | 2px | buttons, inputs, table-row chips |
| `--radius-md` | 3px | small badges, hover labels, hint chips |
| `--radius-lg` | 4px | cards, dialogs, chart/stat panels |

A circular element (the live/dead status dot, a player's avatar swatch) uses `border-radius: 50%`
directly — that is a shape, not a step on this scale.
