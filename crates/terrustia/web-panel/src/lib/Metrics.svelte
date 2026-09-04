<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { fetchMetrics, type Metrics } from "./api";

  let { session }: { session: string } = $props();

  const HISTORY = 120; // samples kept for the rolling charts (~2 min at 1s)
  const POLL_MS = 1000;

  let m = $state<Metrics | null>(null);
  let error = $state("");
  let cpuHist = $state<number[]>([]);
  let wallHist = $state<number[]>([]);
  let playerHist = $state<number[]>([]);
  let memHist = $state<(number | null)[]>([]);

  let cpuCanvas = $state<HTMLCanvasElement | undefined>(undefined);
  let wallCanvas = $state<HTMLCanvasElement | undefined>(undefined);
  let playerCanvas = $state<HTMLCanvasElement | undefined>(undefined);
  let memCanvas = $state<HTMLCanvasElement | undefined>(undefined);

  // A canvas 2D context needs a real colour string, not a `var(--x)` reference, so these read the
  // same tokens `app.css` defines rather than duplicating their hex values here  -  a colour picked
  // once in one place, not two copies that can drift.
  function cssVar(name: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }
  const COL = {
    get accent() {
      return cssVar("--accent");
    },
    get accentDim() {
      return cssVar("--accent-dim");
    },
    get warn() {
      return cssVar("--warn");
    },
    get danger() {
      return cssVar("--danger");
    },
    get dim() {
      return cssVar("--text-dim");
    },
    get grid() {
      return cssVar("--border");
    },
  };

  function push<T>(arr: T[], v: T): T[] {
    const next = [...arr, v];
    if (next.length > HISTORY) next.splice(0, next.length - HISTORY);
    return next;
  }

  async function poll() {
    try {
      const data = await fetchMetrics(session);
      m = data;
      cpuHist = push(cpuHist, data.cpu_us);
      wallHist = push(wallHist, data.wall_us);
      playerHist = push(playerHist, data.player_count);
      memHist = push(memHist, data.memory_bytes == null ? null : data.memory_bytes / 1_048_576);
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  onMount(() => {
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  });

  function fmtUs(us: number): string {
    return us >= 1000 ? `${(us / 1000).toFixed(2)} ms` : `${Math.round(us)} µs`;
  }

  function fmtMem(bytes: number | null): string {
    if (bytes == null) return "n/a";
    return `${(bytes / 1_048_576).toFixed(1)} MB`;
  }

  function budgetPct(): number {
    if (!m || m.budget_us === 0) return 0;
    return (m.cpu_us / m.budget_us) * 100;
  }

  // Draw a filled sparkline into `canvas`, scaled to `[0, max]`. `threshold` draws a reference
  // line (the tick budget); `nullable` data leaves gaps where a sample is missing.
  function spark(
    canvas: HTMLCanvasElement | undefined,
    data: (number | null)[],
    color: string,
    opts: { max?: number; threshold?: number } = {},
  ) {
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const cssW = canvas.clientWidth || 600;
    const cssH = canvas.clientHeight || 120;
    const w = Math.round(cssW * dpr);
    const h = Math.round(cssH * dpr);
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, w, h);

    const values = data.filter((v): v is number => v != null);
    const dataMax = values.length ? Math.max(...values) : 1;
    // Scale to the data (with headroom), not to the threshold: a healthy tick is a tiny fraction
    // of the 16.67 ms budget, so including the budget in the scale would flatten every real tick
    // to a line on the floor. The budget line is still drawn — it simply sits off the top until a
    // tick actually approaches it, which is exactly when it becomes worth seeing.
    const max = Math.max(opts.max ?? 0, dataMax, 1) * 1.15;
    const pad = 4 * dpr;
    const plotH = h - pad * 2;
    const x = (i: number) => (data.length <= 1 ? w : (i / (data.length - 1)) * w);
    const y = (v: number) => pad + plotH - (v / max) * plotH;

    // Baseline grid.
    ctx.strokeStyle = COL.grid;
    ctx.lineWidth = 1 * dpr;
    ctx.beginPath();
    ctx.moveTo(0, h - pad);
    ctx.lineTo(w, h - pad);
    ctx.stroke();

    // Threshold (budget) line.
    if (opts.threshold != null) {
      ctx.strokeStyle = COL.danger;
      ctx.setLineDash([4 * dpr, 4 * dpr]);
      ctx.beginPath();
      ctx.moveTo(0, y(opts.threshold));
      ctx.lineTo(w, y(opts.threshold));
      ctx.stroke();
      ctx.setLineDash([]);
    }

    // Area + line.
    ctx.beginPath();
    let started = false;
    for (let i = 0; i < data.length; i++) {
      const v = data[i];
      if (v == null) {
        started = false;
        continue;
      }
      if (!started) {
        ctx.moveTo(x(i), y(v));
        started = true;
      } else {
        ctx.lineTo(x(i), y(v));
      }
    }
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5 * dpr;
    ctx.stroke();

    // Soft fill under the line (only when the series is contiguous at the end).
    if (values.length > 1 && data[data.length - 1] != null) {
      ctx.lineTo(x(data.length - 1), h - pad);
      ctx.lineTo(x(0), h - pad);
      ctx.closePath();
      ctx.fillStyle = color + "22";
      ctx.fill();
    }
  }

  $effect(() => {
    void cpuHist;
    void cpuCanvas;
    spark(cpuCanvas, cpuHist, COL.accent, { threshold: m?.budget_us });
  });
  $effect(() => {
    void wallHist;
    void wallCanvas;
    spark(wallCanvas, wallHist, COL.accentDim);
  });
  $effect(() => {
    void playerHist;
    void playerCanvas;
    spark(playerCanvas, playerHist, COL.warn, { max: 1 });
  });
  $effect(() => {
    void memHist;
    void memCanvas;
    spark(memCanvas, memHist, COL.dim);
  });

  // The last tick's phase breakdown, sorted so the costliest phase is first.
  const phases = $derived(
    m ? [...m.phases].filter((p) => p.us > 0).sort((a, b) => b.us - a.us) : [],
  );
  const phaseMax = $derived(phases.length ? Math.max(...phases.map((p) => p.us)) : 1);
</script>

<h2>metrics <span class="dim">· live, rolling — not persisted</span></h2>

{#if error}<p class="danger">{error}</p>{/if}

{#if m}
  <div class="stats">
    <div class="stat">
      <span class="label">tick cpu</span>
      <span class="value" class:over={budgetPct() > 100} class:warnv={budgetPct() > 50}>
        {fmtUs(m.cpu_us)}
      </span>
      <span class="sub">{budgetPct().toFixed(0)}% of {fmtUs(m.budget_us)} budget</span>
    </div>
    <!-- "this window" named a window the page neither controls nor draws: `worst_cpu_us` is reset
         by the server every TICK_REPORT_EVERY = 600 ticks, which is the log reporter's own 10s
         cycle, not the 120-sample chart directly below. The number visibly sawtoothed on a cadence
         nothing here explained. Say which window it is. -->
    <div class="stat">
      <span class="label">worst cpu</span>
      <span class="value">{fmtUs(m.worst_cpu_us)}</span>
      <span class="sub">server's last 10s</span>
    </div>
    <div class="stat">
      <span class="label">players</span>
      <span class="value">{m.player_count}</span>
    </div>
    <div class="stat">
      <span class="label">memory</span>
      <span class="value">{fmtMem(m.memory_bytes)}</span>
      <span class="sub">resident</span>
    </div>
    <div class="stat">
      <span class="label">npcs</span>
      <span class="value">{m.npc_count}</span>
    </div>
    <div class="stat">
      <span class="label">projectiles</span>
      <span class="value">{m.projectile_count}</span>
    </div>
    <div class="stat">
      <span class="label">items</span>
      <span class="value">{m.item_count}</span>
    </div>
    <div class="stat">
      <span class="label">ticks</span>
      <span class="value">{m.ticks.toLocaleString()}</span>
    </div>
  </div>

  <div class="charts">
    <div class="chart">
      <div class="chart-head">
        <span>tick cpu time</span>
        <span class="dim">µs — <span class="budget-key">dashed = budget</span></span>
      </div>
      <canvas bind:this={cpuCanvas}></canvas>
    </div>
    <div class="chart">
      <div class="chart-head">
        <span>tick wall time</span>
        <span class="dim">µs  -  wall clock, not just cpu</span>
      </div>
      <canvas bind:this={wallCanvas}></canvas>
    </div>
    <div class="chart">
      <div class="chart-head"><span>players online</span></div>
      <canvas bind:this={playerCanvas}></canvas>
    </div>
    <div class="chart">
      <div class="chart-head">
        <span>memory</span>
        <span class="dim">MB resident</span>
      </div>
      <canvas bind:this={memCanvas}></canvas>
    </div>
  </div>

  <div class="phases">
    <div class="chart-head"><span>last tick, by phase</span><span class="dim">µs of cpu</span></div>
    {#if phases.length === 0}
      <p class="dim">the last tick did no measurable work in any phase.</p>
    {:else}
      <div class="phase-rows">
        {#each phases as p (p.name)}
          <div class="phase-row">
            <span class="phase-name">{p.name}</span>
            <div class="phase-bar-track">
              <div class="phase-bar" style:width="{(p.us / phaseMax) * 100}%"></div>
            </div>
            <span class="phase-us">{fmtUs(p.us)}</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{:else}
  <p class="dim">waiting for the first sample…</p>
{/if}

<style>
  h2 {
    margin: 0 0 1rem;
    font-size: 1rem;
    font-weight: 600;
  }

  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 0.6rem;
    margin-bottom: 1.25rem;
  }

  .stat {
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: var(--radius-lg);
    padding: 0.7rem 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .label {
    font-size: 0.68rem;
    letter-spacing: 0.08em;
    color: var(--text-dim);
  }

  .value {
    font-size: 1.15rem;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }

  .value.warnv {
    color: var(--warn);
  }

  .value.over {
    color: var(--danger);
  }

  .sub {
    font-size: 0.68rem;
    color: var(--text-dim);
  }

  .charts {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 0.75rem;
    margin-bottom: 1.25rem;
  }

  .chart {
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: var(--radius-lg);
    padding: 0.6rem 0.75rem 0.4rem;
  }

  .chart-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--sp-2);
    font-size: 0.75rem;
    margin-bottom: 0.4rem;
    color: var(--text);
  }

  /* The title must never be the one that shrinks to make room for a longer caption next to it  - 
     that is what let "tick wall time" collapse into three stacked lines. */
  .chart-head > span:first-child {
    flex-shrink: 0;
  }

  .chart-head .dim {
    text-align: right;
  }

  .budget-key {
    color: var(--danger);
  }

  canvas {
    width: 100%;
    height: 110px;
    display: block;
  }

  .phases {
    border: 1px solid var(--border);
    background: var(--bg-raised);
    border-radius: var(--radius-lg);
    padding: 0.6rem 0.85rem 0.85rem;
    max-width: 720px;
  }

  .phase-rows {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .phase-row {
    display: grid;
    grid-template-columns: 84px 1fr 72px;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.8rem;
  }

  .phase-name {
    color: var(--text-dim);
  }

  .phase-bar-track {
    background: var(--bg);
    border-radius: var(--radius-sm);
    height: 12px;
    overflow: hidden;
  }

  .phase-bar {
    height: 100%;
    background: var(--accent-dim);
    border-right: 2px solid var(--accent);
    min-width: 2px;
  }

  .phase-us {
    text-align: right;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }
</style>
