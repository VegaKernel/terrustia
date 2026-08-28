<script lang="ts">
  import { onDestroy } from "svelte";
  import { watchWorld, type Player, type WorldTiles, type TileColorName } from "./api";

  let { session }: { session: string } = $props();

  // Stylized, procedural colours — not sprite-accurate, on purpose. See `panel/mod.rs`'s module
  // doc: terrustia has never bundled or shipped Terraria's copyrighted art, and this view doesn't
  // either. Every colour here is a deliberate design choice, not a transcription of anything.
  const TILE_COLORS: Record<TileColorName, string> = {
    empty: "#0d1117",
    dirt: "#8a5a34",
    stone: "#6b6b73",
    grass: "#4caf6b",
    corruption: "#5b4a9a",
    crimson: "#9a2a44",
    sand: "#d8c27a",
    snow: "#e8eef2",
    ice: "#9fd6e8",
    jungle: "#2f8f4a",
    ore: "#c9a34a",
    gem: "#4ac9c9",
    water: "#2f6fa8",
    lava: "#c8501f",
    honey: "#e0a015",
    ash: "#3a2e2e",
    other: "#4a4a54",
  };

  let tiles = $state<WorldTiles | null>(null);
  let players = $state<Player[]>([]);
  let hoverName = $state<string | null>(null);
  let canvasEl = $state<HTMLCanvasElement | undefined>(undefined);

  const CELL = 10; // backing-store pixels per tile sample; the canvas is then scaled to fit

  // svelte-ignore state_referenced_locally
  const stop = watchWorld(
    session,
    (p) => (players = p),
    (t) => (tiles = t),
  );
  onDestroy(stop);

  function hashColor(id: number): string {
    // A stable, distinct accent per equipped item id — not a rarity table (this project has none
    // and building one just for a colour accent was not worth transcribing thousands of items),
    // but real gear, coloured consistently rather than invented per render.
    const hue = Math.abs((id * 2654435761) % 360);
    return `hsl(${hue}, 68%, 58%)`;
  }

  function draw() {
    if (!canvasEl || !tiles) return;
    const w = tiles.sample_cols * CELL;
    const h = tiles.sample_rows * CELL;
    if (canvasEl.width !== w) canvasEl.width = w;
    if (canvasEl.height !== h) canvasEl.height = h;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;

    for (let row = 0; row < tiles.sample_rows; row++) {
      for (let col = 0; col < tiles.sample_cols; col++) {
        const cell = tiles.tiles[row * tiles.sample_cols + col];
        ctx.fillStyle = TILE_COLORS[cell] ?? TILE_COLORS.other;
        // A one-pixel overlap keeps the solid blocks seam-free once the canvas is scaled to fit.
        ctx.fillRect(col * CELL, row * CELL, CELL + 1, CELL + 1);
      }
    }

    // A player exactly at — or, for a client that overshoots, just past — the world edge should
    // still be drawn pinned to the edge rather than vanishing off the canvas. Clamp with a small
    // margin so the whole avatar stays visible; an in-bounds player is unaffected.
    const margin = 14;
    for (const p of players) {
      const fx = Math.max(margin, Math.min(w - margin, (p.x / 16 / tiles.world_width) * w));
      const fy = Math.max(margin, Math.min(h - margin, (p.y / 16 / tiles.world_height) * h));
      drawAvatar(ctx, fx, fy, p);
    }
  }

  function drawAvatar(ctx: CanvasRenderingContext2D, x: number, y: number, p: Player) {
    const a = p.appearance;
    const skin = a ? `rgb(${a.skin_color.join(",")})` : "#c8a878";
    const hair = a ? `rgb(${a.hair_color.join(",")})` : "#5a3a2a";
    const shirt = a ? `rgb(${a.shirt_color.join(",")})` : "#557799";
    const eye = a ? `rgb(${a.eye_color.join(",")})` : "#222";
    const r = 9;

    // A soft shadow disc so the avatar reads against any tile colour behind it.
    ctx.beginPath();
    ctx.arc(x, y, r + 2.5, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(0,0,0,0.45)";
    ctx.fill();

    // Body.
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fillStyle = skin;
    ctx.fill();

    // Hair, as a cap on the upper half.
    ctx.beginPath();
    ctx.arc(x, y - 1, r - 1, Math.PI, Math.PI * 2);
    ctx.fillStyle = hair;
    ctx.fill();

    // Shirt collar along the lower third.
    ctx.beginPath();
    ctx.arc(x, y + r * 0.4, r * 0.85, 0.15 * Math.PI, 0.85 * Math.PI);
    ctx.fillStyle = shirt;
    ctx.fill();

    // Eye.
    ctx.beginPath();
    ctx.arc(x + 2.5, y - 0.5, 1.7, 0, Math.PI * 2);
    ctx.fillStyle = eye;
    ctx.fill();

    // A crisp outline — red for a PvP-enabled player, otherwise a light ring.
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = p.pvp ? "#e0645a" : "rgba(215,221,225,0.85)";
    ctx.stroke();

    // Equipped-gear accents: small dots arced beneath the avatar, one per worn item.
    const shown = p.equipped.slice(0, 6);
    shown.forEach((id, i) => {
      const angle = Math.PI * 0.25 + (i / Math.max(shown.length - 1, 1)) * Math.PI * 0.5;
      const ax = x + Math.cos(angle) * (r + 5);
      const ay = y + Math.sin(angle) * (r + 5) * 0.6 + r * 0.6;
      ctx.beginPath();
      ctx.arc(ax, ay, 2.1, 0, Math.PI * 2);
      ctx.fillStyle = hashColor(id);
      ctx.fill();
    });

    // Name, on a small dark pill so it stays legible over noisy terrain, and clamped so a player
    // near an edge does not get their label clipped off the canvas.
    ctx.font = "600 13px ui-monospace, monospace";
    const label = p.name;
    const tw = ctx.measureText(label).width;
    const padX = 4;
    const half = tw / 2 + padX;
    let lx = x;
    if (lx - half < 1) lx = half + 1;
    if (lx + half > ctx.canvas.width - 1) lx = ctx.canvas.width - half - 1;
    const ly = y - r - 9;
    ctx.fillStyle = "rgba(10,13,16,0.72)";
    ctx.fillRect(lx - half, ly - 11, tw + padX * 2, 16);
    ctx.fillStyle = p.pvp ? "#e0645a" : "#d7dde1";
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    ctx.fillText(label, lx, ly);
  }

  $effect(() => {
    // Tracked dependencies: redraw whenever a new tile sample or player snapshot arrives, or once
    // the canvas element itself first becomes available.
    void tiles;
    void players;
    void canvasEl;
    draw();
  });

  function pointerMove(e: MouseEvent) {
    if (!canvasEl || !tiles) {
      hoverName = null;
      return;
    }
    const rect = canvasEl.getBoundingClientRect();
    const scaleX = canvasEl.width / rect.width;
    const scaleY = canvasEl.height / rect.height;
    const px = (e.clientX - rect.left) * scaleX;
    const py = (e.clientY - rect.top) * scaleY;
    const w = tiles.sample_cols * CELL;
    const h = tiles.sample_rows * CELL;
    const margin = 14;
    const hit = players.find((p) => {
      const fx = Math.max(margin, Math.min(w - margin, (p.x / 16 / tiles!.world_width) * w));
      const fy = Math.max(margin, Math.min(h - margin, (p.y / 16 / tiles!.world_height) * h));
      return Math.hypot(fx - px, fy - py) < 18;
    });
    hoverName = hit ? `${hit.name} — ${hit.life}/${hit.life_max} HP` : null;
  }
</script>

<div class="world">
  {#if !tiles}
    <p class="dim pad">waiting for the world…</p>
  {:else}
    <div class="canvas-wrap">
      <canvas bind:this={canvasEl} onmousemove={pointerMove} onmouseleave={() => (hoverName = null)}
      ></canvas>
      {#if hoverName}<div class="hover-label">{hoverName}</div>{/if}
    </div>
    <p class="dim caption">
      stylized, procedural render from real position and appearance data — colours come from each
      player's actual skin/hair/gear, tiles are coloured by type. no Terraria art is used or
      shipped.
    </p>
  {/if}
</div>

<style>
  .world {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    padding: 1rem 1.5rem;
    gap: 0.5rem;
  }

  .pad {
    padding: 1rem 0;
  }

  .canvas-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #000;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
  }

  canvas {
    max-width: 100%;
    max-height: 100%;
    cursor: crosshair;
  }

  .hover-label {
    position: absolute;
    top: 0.5rem;
    left: 0.5rem;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0.3rem 0.55rem;
    font-size: 0.78rem;
    pointer-events: none;
  }

  .caption {
    font-size: 0.78rem;
    max-width: 60em;
  }
</style>
