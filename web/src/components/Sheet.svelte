<script lang="ts">
  // Bottom sheet 2 états (replié = 0, déplié ≈ 90 % = 2 ; plus de mi-hauteur) ; bureau : panneau latéral (pas de poignée).
  // Niveaux (D27, D34) : plus le panneau est déplié, plus il « monte » au-dessus de la carte : ombre en couches plus large,
  // teinte plus claire. Bords nets (D34 : pas de bord « organique » sans modèle crédible).
  import type { Snippet } from 'svelte';
  import { app, level as uiLevel } from '../lib/app.svelte';

  let { desktop, children, label, peek = 0, fit = false, full = 0 }: { desktop: boolean; children: Snippet; label: string; peek?: number; fit?: boolean; full?: number } = $props();

  const level = $derived(uiLevel(desktop));

  let vh = $state(window.visualViewport?.height ?? innerHeight);
  // clavier virtuel : hauteur visible (visualViewport) ; la feuille se cale au-dessus du clavier
  let kb = $state(0);
  $effect(() => {
    const v = window.visualViewport;
    if (!v) return;
    const f = () => { vh = v.height; kb = Math.max(0, Math.round(innerHeight - v.height - v.offsetTop)); if (kb > 80) app.snap = 2; };
    v.addEventListener('resize', f); v.addEventListener('scroll', f);
    return () => { v.removeEventListener('resize', f); v.removeEventListener('scroll', f); };
  });
  // replié (peek) : poignée + barre collante du contenu (réglages : résumé + boutons) restent visibles
  // replié : peek > 0 = hauteur imposée ; 0 = poignée + barre collante mesurée (réglages), sans rien du champ de recherche
  let bar = $state(0);
  const heights = () => [peek || 32 + bar, 0, full || Math.round(vh * 0.9)];
  let content: HTMLElement | undefined = $state();
  // hauteur de la barre collante (suit ses changements : progression, annulation…)
  $effect(() => {
    const b = content?.querySelector<HTMLElement>('.bar');
    void peek; void app.layers.length;
    if (!content || !b) { bar = 0; return; }
    const pad = parseFloat(getComputedStyle(content.parentElement!).paddingBottom || '0');
    const ro = new ResizeObserver(() => { bar = b.offsetHeight + pad; });
    ro.observe(b);
    return () => ro.disconnect();
  });
  // à chaque changement d'état/de couche : replié = voir le CSS (rien ne défile) ; déplié = tout en haut (sauf champ focalisé)
  $effect(() => {
    void app.snap; void app.layers.length;
    if (!content) return;
    if (app.snap === 0) { if (!content.querySelector('.bar')) content.scrollTop = 0; }
    else if (!content.contains(document.activeElement) || document.activeElement === document.body) content.scrollTop = 0;
  });
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
    if (!d.moved) { app.snap = app.snap === 2 ? 0 : 2; return; }
    const h = Math.min(heights()[2], Math.max(heights()[0], d.h0 + d.y0 - d.y));
    const v = (d.y0 - d.y) / Math.max(1, performance.now() - d.t0); // px/ms, > 0 vers le haut
    const [lo, , hi] = heights();
    app.snap = Math.abs(v) > 0.5 ? (v > 0 ? 2 : 0) : Math.abs(hi - h) < Math.abs(h - lo) ? 2 : 0;
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowUp') { app.snap = 2; e.preventDefault(); }
    if (e.key === 'ArrowDown') { app.snap = 0; e.preventDefault(); }
  }
</script>



{#if desktop}
  <aside class="panel" aria-label={label} data-level={level}>
    <div class="topo head-topo"></div>
    <div class="scroll">{@render children()}</div>
  </aside>
{:else}
  <section class="sheet" class:dragging={!!drag} style:height={fit ? 'auto' : `${kb > 80 ? Math.min(height, vh - 8) : height}px`} style:max-height="{Math.round(vh * 0.9)}px" style:bottom="{kb > 80 ? kb : 0}px" aria-label={label} data-snap={app.snap} data-level={level}>
    <div class="topo head-topo"></div>
    <button
      class="handle"
      aria-label="{label} ({['1/3', '2/3', '3/3'][app.snap]})"
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={() => (drag = null)}
      onkeydown={key}
    ><span></span></button>
    <div class="content" bind:this={content}>
      {@render children()}
    </div>
  </section>
  {#if kb > 80}<button class="kb-ok" style:bottom="{kb + 8}px" onpointerdown={(e) => e.preventDefault()} onclick={() => (document.activeElement as HTMLElement | null)?.blur()}>OK</button>{/if}
{/if}

<style>
  /* clavier ouvert : toujours de quoi le fermer */
  .kb-ok { position: absolute; right: 12px; z-index: 7; min-height: 44px; min-width: 64px; border-radius: 22px; border: 0; font-weight: 800; background: var(--accent); color: var(--on-accent); box-shadow: 0 2px 8px rgb(0 0 0 / 0.35); }
  .panel, .sheet { position: absolute; z-index: 5; color: var(--text); background: var(--lvl-2); box-sizing: border-box; display: flex; flex-direction: column;
    transition: box-shadow 0.4s var(--ease), background-color 0.4s; }
  .panel { inset: 0 auto 0 0; width: 400px; }
  .scroll { position: relative; flex: 1; overflow-y: auto; padding: 12px 16px 0; display: flex; flex-direction: column; }
  .scroll > :global(*) { flex: none; }
  .sheet { left: 0; right: 0; bottom: 0; border-radius: 14px 14px 0 0; padding-bottom: env(safe-area-inset-bottom);
    transition: height 0.32s var(--ease), box-shadow 0.4s var(--ease), background-color 0.4s; }
  .sheet.dragging { transition: none; }
  .handle { position: relative; flex: none; height: 32px; width: 100%; border: 0; background: transparent; cursor: grab; touch-action: none;
    display: grid; place-items: center; padding: 0; }
  .handle span { width: 44px; height: 5px; border-radius: 3px; background: var(--muted); }
  .content { position: relative; overflow-y: auto; padding: 0 16px; flex: 1; overscroll-behavior: contain; display: flex; flex-direction: column; }
  .content > :global(*) { flex: none; }
  /* replié : rien ne défile ; avec la barre des réglages, seul le bas du contenu (la barre) reste visible, le champ de recherche est rogné */
  .sheet[data-snap='0'] .content { overflow: hidden; }
  .sheet[data-snap='0'] .content:has(:global(.bar)) { justify-content: flex-end; }
  .head-topo { inset: 0 0 auto 0; height: 140px; border-radius: inherit;
    -webkit-mask-image: var(--topo), linear-gradient(#000, transparent); mask-image: var(--topo), linear-gradient(#000, transparent);
    -webkit-mask-composite: source-in; mask-composite: intersect; -webkit-mask-size: 1000px, 100% 100%; mask-size: 1000px, 100% 100%; }
  /* élévation : ombre vers la carte (haut pour la feuille, droite pour le panneau) */
  .sheet[data-level='1'] { --here: var(--lvl-1); background: var(--lvl-1); box-shadow: 0 -1px 1px rgb(var(--sh-rgb) / 0.10), 0 -2px 6px rgb(var(--sh-rgb) / 0.10); }
  .sheet[data-level='2'] { --here: var(--lvl-2); background: var(--lvl-2); box-shadow: 0 -1px 2px rgb(var(--sh-rgb) / 0.12), 0 -4px 10px rgb(var(--sh-rgb) / 0.12), 0 -12px 28px rgb(var(--sh-rgb) / 0.12); }
  .sheet[data-level='3'] { --here: var(--lvl-3); background: var(--lvl-3); box-shadow: 0 -1px 2px rgb(var(--sh-rgb) / 0.14), 0 -6px 14px rgb(var(--sh-rgb) / 0.14), 0 -20px 44px rgb(var(--sh-rgb) / 0.16); }
  .panel[data-level='1'] { --here: var(--lvl-1); background: var(--lvl-1); box-shadow: 1px 0 1px rgb(var(--sh-rgb) / 0.10), 2px 0 6px rgb(var(--sh-rgb) / 0.10); }
  .panel[data-level='2'] { --here: var(--lvl-2); background: var(--lvl-2); box-shadow: 1px 0 2px rgb(var(--sh-rgb) / 0.12), 4px 0 10px rgb(var(--sh-rgb) / 0.12), 12px 0 28px rgb(var(--sh-rgb) / 0.12); }
  .panel[data-level='3'] { --here: var(--lvl-3); background: var(--lvl-3); box-shadow: 1px 0 2px rgb(var(--sh-rgb) / 0.14), 6px 0 14px rgb(var(--sh-rgb) / 0.14), 20px 0 44px rgb(var(--sh-rgb) / 0.16); }
  @media (prefers-reduced-motion: reduce) { .sheet, .panel { transition: none; } }
</style>
