<script lang="ts">
  // Bottom sheet 3 crans (replié ≈ 112 px, mi ≈ 50 %, plein ≈ 90 %) ; bureau : panneau latéral (pas de poignée).
  import type { Snippet } from 'svelte';
  import { app, level as uiLevel } from '../lib/app.svelte';
  import { nature, wave, wavePath } from '../lib/design';

  let { desktop, children, label }: { desktop: boolean; children: Snippet; label: string } = $props();

  // Niveaux (D27) : plus le panneau est déplié, plus il est « haut » (ombre + anneaux de courbes sur le bord).
  // Mobile : cran 0–2 ; bureau : profondeur des couches (réglages 1, résultat 2, détail… 3).
  const level = $derived(uiLevel(desktop));
  const edge = (len: number, dy: number) => wavePath(wave(len, 14, 7, 3, dy));
  const fill = (len: number) => `${edge(len, 0)}L${len},40L0,40Z`;
  // bord droit du panneau (bureau) : même vague, axes permutés, anneaux vers la carte
  const vedge = (dx: number) => wavePath(wave(1000, 14, 7, 5, dx).map(([a, b]) => [40 - b, a]));

  let vh = $state(innerHeight);
  const heights = () => [112, Math.round(vh * 0.5), Math.round(vh * 0.9)];
  let drag = $state<{ y0: number; h0: number; t0: number; y: number; moved: boolean } | null>(null);
  const height = $derived.by(() => {
    const h = heights()[app.snap];
    if (!drag) return h;
    return Math.min(heights()[2], Math.max(heights()[0], drag.h0 + drag.y0 - drag.y));
  });

  function down(e: PointerEvent) {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { y0: e.clientY, h0: heights()[app.snap], t0: performance.now(), y: e.clientY, moved: false };
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    drag.y = e.clientY;
    if (Math.abs(drag.y - drag.y0) > 6) drag.moved = true;
  }
  function up() {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (!d.moved) { app.snap = ((app.snap + 1) % 3) as 0 | 1 | 2; return; }
    const h = Math.min(heights()[2], Math.max(heights()[0], d.h0 + d.y0 - d.y));
    const v = (d.y0 - d.y) / Math.max(1, performance.now() - d.t0); // px/ms, > 0 vers le haut
    let k = heights().reduce((b, x, i, a) => (Math.abs(x - h) < Math.abs(a[b] - h) ? i : b), 0);
    if (Math.abs(v) > 0.5) k = Math.max(0, Math.min(2, (v > 0 ? Math.max(k, app.snap + 1) : Math.min(k, app.snap - 1))));
    app.snap = k as 0 | 1 | 2;
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowUp') { app.snap = Math.min(2, app.snap + 1) as 0 | 1 | 2; e.preventDefault(); }
    if (e.key === 'ArrowDown') { app.snap = Math.max(0, app.snap - 1) as 0 | 1 | 2; e.preventDefault(); }
  }
</script>

<svelte:window bind:innerHeight={vh} />

{#if desktop}
  <aside class="panel" class:nature aria-label={label} data-level={level}>
    {#if nature}
      <svg class="edge-v" viewBox="0 0 40 1000" preserveAspectRatio="none" aria-hidden="true">
        {#each [3, 2, 1] as k}<path class="ring" class:on={k <= level} d={vedge(6 * k)} />{/each}
        <path class="fill" d="{vedge(0)}L0,1000L0,0Z" />
      </svg>
      <div class="topo head-topo"></div>
    {/if}
    <div class="scroll">{@render children()}</div>
  </aside>
{:else}
  <section class="sheet" class:nature class:dragging={!!drag} style:height="{height}px" aria-label={label} data-snap={app.snap} data-level={level}>
    {#if nature}
      <svg class="edge-h" viewBox="0 0 1000 40" preserveAspectRatio="none" aria-hidden="true">
        {#each [3, 2, 1] as k}<path class="ring" class:on={k <= level} d={edge(1000, 6 * k)} />{/each}
        <path class="fill" d={fill(1000)} />
      </svg>
      <div class="topo head-topo"></div>
    {/if}
    <button
      class="handle"
      aria-label="{label} ({['1/3', '2/3', '3/3'][app.snap]})"
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={() => (drag = null)}
      onkeydown={key}
    ><span></span></button>
    <div class="content">
      {@render children()}
    </div>
  </section>
{/if}

<style>
  .panel {
    position: absolute;
    inset: 0 auto 0 0;
    width: 400px;
    background: var(--bg);
    color: var(--text);
    border-right: 1px solid var(--border);
    z-index: 5;
    box-sizing: border-box;
  }
  .scroll { position: relative; height: 100%; overflow-y: auto; padding: 12px 16px 24px; box-sizing: border-box; }
  .sheet {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    background: var(--bg);
    color: var(--text);
    border-radius: 16px 16px 0 0;
    box-shadow: 0 -2px 12px rgb(0 0 0 / 0.25);
    z-index: 5;
    display: flex;
    flex-direction: column;
    transition: height 0.22s ease;
    padding-bottom: env(safe-area-inset-bottom);
  }
  .sheet.dragging { transition: none; }
  @media (prefers-reduced-motion: reduce) { .sheet { transition: none; } }
  .handle {
    flex: none;
    height: 44px;
    min-height: 44px;
    width: 100%;
    border: 0;
    background: transparent;
    cursor: grab;
    touch-action: none;
    display: grid;
    place-items: center;
    padding: 0;
  }
  .handle span { width: 44px; height: 5px; border-radius: 3px; background: var(--muted); }
  .content { overflow-y: auto; padding: 0 16px; flex: 1; overscroll-behavior: contain; }

  /* ---- directions nature ---- */
  .sheet.nature, .panel.nature { background: var(--lvl-2); transition: height 0.32s var(--ease), box-shadow 0.4s var(--ease), background-color 0.4s; }
  .sheet.nature { border-radius: 0; box-shadow: none; }
  .sheet.nature.dragging { transition: none; }
  .sheet.nature[data-level='1'] { --sheet-bg: var(--lvl-1); background: var(--lvl-1); filter: drop-shadow(0 -1px 3px rgb(0 0 0 / 0.18)); }
  .sheet.nature[data-level='2'] { --sheet-bg: var(--lvl-2); background: var(--lvl-2); filter: drop-shadow(0 -3px 8px rgb(0 0 0 / 0.22)); }
  .sheet.nature[data-level='3'] { --sheet-bg: var(--lvl-3); background: var(--lvl-3); filter: drop-shadow(0 -6px 16px rgb(0 0 0 / 0.28)); }
  .edge-h { position: absolute; left: 0; right: 0; bottom: calc(100% - 14px); width: 100%; height: 40px; pointer-events: none; overflow: visible; }
  .edge-h .fill { fill: var(--lvl-2); transition: fill 0.4s; }
  .sheet[data-level='1'] .edge-h .fill { fill: var(--lvl-1); }
  .sheet[data-level='3'] .edge-h .fill { fill: var(--lvl-3); }
  .ring { fill: none; stroke: var(--contour); stroke-width: 1.3; vector-effect: non-scaling-stroke; opacity: 0; transition: opacity 0.5s var(--ease); }
  .ring.on { opacity: 0.7; }
  .head-topo { inset: 0 0 auto 0; height: 120px; z-index: -1;
    -webkit-mask-image: var(--topo), linear-gradient(#000, transparent); mask-image: var(--topo), linear-gradient(#000, transparent);
    -webkit-mask-composite: source-in; mask-composite: intersect; -webkit-mask-size: 480px, 100% 100%; mask-size: 480px, 100% 100%; }
  .sheet.nature .handle, .sheet.nature .content { position: relative; }
  .panel.nature { border-right: 0; }
  .panel.nature[data-level='1'] { --sheet-bg: var(--lvl-1); background: var(--lvl-1); box-shadow: 2px 0 4px rgb(0 0 0 / 0.12); }
  .panel.nature[data-level='2'] { --sheet-bg: var(--lvl-2); background: var(--lvl-2); box-shadow: 4px 0 12px rgb(0 0 0 / 0.18); }
  .panel.nature[data-level='3'] { --sheet-bg: var(--lvl-3); background: var(--lvl-3); box-shadow: 8px 0 24px rgb(0 0 0 / 0.24); }
  .edge-v { position: absolute; top: 0; bottom: var(--dock-h, 0px); left: calc(100% - 1px); width: 40px; height: calc(100% - var(--dock-h, 0px)); pointer-events: none; overflow: visible; }
  .edge-v .fill { fill: var(--lvl-2); }
  .panel[data-level='1'] .edge-v .fill { fill: var(--lvl-1); }
  .panel[data-level='3'] .edge-v .fill { fill: var(--lvl-3); }
  @media (prefers-reduced-motion: reduce) { .sheet.nature, .panel.nature, .ring, .edge-h .fill { transition: none; } }
</style>
