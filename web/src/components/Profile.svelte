<script lang="ts" module>
  import type { Candidate as C } from '../lib/types';
  import { idxAt } from '../lib/geo';
  export type Mark = { idx: number; label: string; kind: 'via' | 'col' | 'summit'; ele?: number | null };
  /** Repères du profil (D34) : points de passage numérotés (ordre de visite), cols et sommets. */
  export function profileMarks(c: C, via: number[]): Mark[] {
    return [...[...via].sort((a, b) => a - b).map((idx, k) => ({ idx, label: `P${k + 1}`, kind: 'via' as const })),
      ...(c.landmarks ?? []).map((l) => ({ idx: idxAt(c, l.dist_m), label: l.name, kind: l.kind, ele: l.ele_m }))];
  }

</script>

<script lang="ts">
  // Profil d'altitude en canvas, lié à la carte.
  import type { Candidate } from '../lib/types';
  import { grades, indexAtDist, nearestMark } from '../lib/geo';
  import { LM_PATH } from '../lib/icons';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num } from '../i18n/format';

  let { cand, cursor = $bindable(-1), height = 130, onpick, marks = [] }: { cand: Candidate; cursor?: number; height?: number; onpick?: (i: number) => void; marks?: Mark[] } = $props();

  const PAD = { r: 12, t: 24, b: 22 };
  // marge gauche à la largeur des altitudes (« 1 858 m » était coupé sur ordinateur)
  let padL = $state(46);
  const top = PAD.t;
  const H = $derived(height);
  // Dégradé selon la pente absolue : vert à plat, rouge sombre à 40 % et plus.
  const STOPS = [[0, 76, 175, 80], [5, 205, 220, 57], [10, 255, 193, 7], [18, 244, 81, 30], [28, 198, 40, 40], [40, 93, 15, 15]];
  function slopeColor(g: number) {
    g = Math.min(Math.abs(g), 40);
    for (let k = 1; k < STOPS.length; k++) {
      if (g <= STOPS[k][0]) {
        const a = STOPS[k - 1], b = STOPS[k], u = (g - a[0]) / (b[0] - a[0]);
        return `rgb(${[1, 2, 3].map((j) => Math.round(a[j] + u * (b[j] - a[j]))).join(',')})`;
      }
    }
    return 'rgb(93,15,15)';
  }

  let canvas: HTMLCanvasElement;
  let w = $state(300);
  const G = $derived(grades(cand.dist, cand.ele));
  const dmax = $derived(cand.dist[cand.dist.length - 1] || 1);
  const zr = $derived.by(() => {
    let zmin = Math.min(...cand.ele), zmax = Math.max(...cand.ele);
    if (zmax - zmin < 10) zmax = zmin + 10;
    return { zmin, zmax };
  });
  const X = (d: number) => padL + ((w - padL - PAD.r) * d) / dmax;
  const Y = (z: number) => top + (H - top - PAD.b) * (1 - (z - zr.zmin) / (zr.zmax - zr.zmin));
  const label = $derived(t().detail.profile({
    km: num(i18n.lang, dmax / 1000, 1), min: num(i18n.lang, zr.zmin), max: num(i18n.lang, zr.zmax),
  }));
  const bubble = (i: number) =>
    `km ${num(i18n.lang, cand.dist[i] / 1000, 1)} · ${num(i18n.lang, cand.ele[i])} m · ${num(i18n.lang, G[i])} %`;

  // cols et sommets : seulement leur symbole au-dessus du profil ; nom, altitude et km dans une bulle (survol,
  // clic, toucher, clavier) ; plus aucun nom écrit en permanence (retour du fondateur, 2026-10-07)
  const lms = $derived(marks.filter((m) => m.kind !== 'via' && m.idx >= 0 && m.idx < cand.dist.length));
  const lmx = $derived(lms.map((m) => X(cand.dist[m.idx])));
  let open = $state(-1), pinned = $state(false), strip: HTMLDivElement;
  const info = (m: Mark) =>
    `${m.label} · ≈ ${num(i18n.lang, m.ele != null ? Math.round(m.ele / 10) * 10 : cand.ele[m.idx])} m · km ${num(i18n.lang, cand.dist[m.idx] / 1000, 1)}`;
  function at(e: PointerEvent) {
    return nearestMark(lmx, e.clientX - strip.getBoundingClientRect().left, e.pointerType === 'mouse' ? 12 : 20);
  }
  function show(k: number, pin: boolean) {
    open = k; pinned = pin && k >= 0;
  }
  let bw = $state(0); // largeur de la bulle : gardée entière dans le profil

  $effect(() => {
    // dépendances : cand, cursor, w, langue, thème
    const n = cand.dist.length, r = devicePixelRatio || 1, D = cand, cur = cursor;
    void marks;
    void i18n.lang;
    canvas.width = w * r;
    canvas.height = H * r;
    const c = canvas.getContext('2d')!;
    const css = getComputedStyle(canvas);
    const text = css.getPropertyValue('--text').trim() || '#111', muted = css.getPropertyValue('--muted').trim() || '#555';
    const grid = css.getPropertyValue('--border').trim() || '#ddd', bg = css.getPropertyValue('--bg').trim() || '#fff';
    c.setTransform(r, 0, 0, r, 0, 0);
    c.clearRect(0, 0, w, H);
    c.font = '12px system-ui, sans-serif';
    c.lineWidth = 1;
    const zs = [0, 1, 2].map((i) => zr.zmin + ((zr.zmax - zr.zmin) * i) / 2);
    const need = Math.ceil(Math.max(...zs.map((z) => c.measureText(`${num(i18n.lang, z)} m`).width)) + 10);
    if (need !== padL) { padL = need; return; } // redessin avec la nouvelle marge
    for (let i = 0; i <= 2; i++) {
      const z = zr.zmin + ((zr.zmax - zr.zmin) * i) / 2, y = Y(z);
      c.strokeStyle = grid; c.beginPath(); c.moveTo(padL, y); c.lineTo(w - PAD.r, y); c.stroke();
      c.fillStyle = muted; c.textAlign = 'right'; c.fillText(`${num(i18n.lang, z)} m`, padL - 5, y + 4);
    }
    c.textAlign = 'center';
    const km = dmax / 1000, step = km > 40 ? 10 : km > 12 ? 5 : km > 5 ? 2 : 1;
    for (let k = 0; k <= km; k += step) c.fillText(`${k} km`, X(k * 1000), H - 6);
    const base = H - PAD.b;
    for (let i = 0; i < n - 1; i++) {
      const xa = X(D.dist[i]), xb = X(D.dist[i + 1]);
      c.fillStyle = slopeColor((G[i] + G[i + 1]) / 2);
      c.beginPath(); c.moveTo(xa, base); c.lineTo(xa, Y(D.ele[i])); c.lineTo(xb + 0.6, Y(D.ele[i + 1])); c.lineTo(xb + 0.6, base); c.closePath(); c.fill();
    }
    c.beginPath(); c.moveTo(X(D.dist[0]), Y(D.ele[0]));
    for (let i = 1; i < n; i++) c.lineTo(X(D.dist[i]), Y(D.ele[i]));
    c.strokeStyle = text; c.lineWidth = 1.5; c.stroke();
    // repères : trait pointillé + étiquette (points de passage en pastille, cols/sommets en triangle)
    c.font = 'bold 11px system-ui, sans-serif';
    // repères : trait pointillé jusqu'à la courbe ; points de passage en pastille numérotée, cols et sommets en symbole
    // (boutons au-dessus du canevas)
    c.font = 'bold 11px system-ui, sans-serif';
    for (const m of marks) {
      if (m.idx < 0 || m.idx >= n) continue;
      const x = X(D.dist[m.idx]), y = Y(D.ele[m.idx]);
      c.strokeStyle = text; c.lineWidth = 1; c.setLineDash([2, 3]);
      c.beginPath(); c.moveTo(x, top - 2); c.lineTo(x, y); c.stroke(); c.setLineDash([]);
      if (m.kind === 'via') {
        c.fillStyle = text; c.textAlign = 'center';
        c.beginPath(); c.arc(x, top - 12, 9, 0, 6.3); c.fill();
        c.fillStyle = bg; c.fillText(m.label, x, top - 8);
      }
    }
    if (cur >= 0 && cur < n) {
      const x = X(D.dist[cur]), left = x > w / 2;
      c.strokeStyle = text; c.lineWidth = 1;
      c.beginPath(); c.moveTo(x, top); c.lineTo(x, base); c.stroke();
      c.fillStyle = text; c.beginPath(); c.arc(x, Y(D.ele[cur]), 4, 0, 6.3); c.fill();
      c.font = 'bold 13px system-ui, sans-serif';
      const txt = bubble(cur), tw = c.measureText(txt).width + 8, tx = Math.max(0, left ? x - 8 - tw : x + 8);
      c.fillStyle = bg; c.globalAlpha = 0.9; c.fillRect(tx, 2, tw, 18); c.globalAlpha = 1;
      c.fillStyle = text; c.textAlign = 'left'; c.fillText(txt, tx + 4, 15);
    }
  });

  function pick(clientX: number) {
    const b = canvas.getBoundingClientRect();
    cursor = indexAtDist(cand.dist, ((clientX - b.left - padL) / (w - padL - PAD.r)) * dmax);
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && cursor >= 0) { onpick?.(cursor); return; }
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const d = cursor < 0 ? 0 : cand.dist[cursor] + (e.key === 'ArrowRight' ? 100 : -100);
    cursor = indexAtDist(cand.dist, Math.max(0, Math.min(dmax, d)));
  }
</script>

<div class="profile" bind:clientWidth={w}>
  <canvas
    bind:this={canvas}
    style:width="{w}px"
    style:height="{H}px"
    tabindex="0"
    role="slider"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={Math.round(dmax)}
    aria-valuenow={cursor >= 0 ? Math.round(cand.dist[cursor]) : 0}
    aria-valuetext={cursor >= 0 ? bubble(cursor) : label}
    onmousemove={(e) => pick(e.clientX)}
    onmouseleave={() => (cursor = -1)}
    onclick={(e) => { pick(e.clientX); onpick?.(cursor); }}
    ontouchend={() => { if (cursor >= 0) onpick?.(cursor); }}
    ontouchstart={(e) => { if (e.touches.length) { e.preventDefault(); pick(e.touches[0].clientX); } }}
    ontouchmove={(e) => { if (e.touches.length) { e.preventDefault(); pick(e.touches[0].clientX); } }}
    onkeydown={key}
    onblur={() => (cursor = -1)}
  ></canvas>
  <!-- symboles des cols et sommets : le pointeur prend le plus proche ; au clavier, Tab puis Entrée -->
  <div class="lms" role="group" aria-label={t().detail.landmarks} bind:this={strip} style:height="{PAD.t}px"
    onpointermove={(e) => { if (e.pointerType === 'mouse' && !pinned) show(at(e), false); }}
    onpointerleave={(e) => { if (e.pointerType === 'mouse' && !pinned) open = -1; }}
    onpointerdown={(e) => { const k = at(e); if (k >= 0) { e.preventDefault(); show(k, !(pinned && open === k)); if (!pinned) open = -1; } }}>
    {#each lms as m, k (k)}
      <button type="button" class="landmark {m.kind}" style:left="{lmx[k]}px" aria-label={info(m)} aria-expanded={open === k}
        onfocus={() => { if (!pinned) open = k; }} onblur={() => { if (!pinned && open === k) open = -1; }}
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); show(k, true); } else if (e.key === 'Escape') { open = -1; pinned = false; } }}>
        <svg viewBox="0 0 20 20" width="13" height="13" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={LM_PATH[m.kind as 'col' | 'summit']} /></svg>
      </button>
    {/each}
  </div>
  {#if open >= 0 && lms[open]}
    <div class="lm-bubble" role="status" bind:clientWidth={bw} style:left="{Math.min(Math.max(lmx[open], bw / 2 + 2), w - bw / 2 - 2)}px" style:top="{PAD.t + 4}px">{info(lms[open])}</div>
  {/if}
</div>
<svelte:document onpointerdown={(e) => { if (pinned && !strip?.contains(e.target as Node)) { open = -1; pinned = false; } }} />

<style>
  .profile { width: 100%; position: relative; }
  canvas { display: block; touch-action: none; border-radius: 8px; }
  canvas:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
  .lms { position: absolute; left: 0; right: 0; top: 0; touch-action: manipulation; }
  .lms .landmark { position: absolute; top: 1px; width: 20px; height: 20px; transform: translateX(-50%); padding: 0; box-shadow: none; pointer-events: none; }
  .lms .landmark:focus-visible { outline: 3px solid var(--accent); outline-offset: 1px; }
  .lm-bubble { position: absolute; transform: translateX(-50%); z-index: 2; width: max-content; max-width: calc(100% - 4px); padding: 4px 8px; border-radius: 6px;
    background: var(--bg); color: var(--text); border: 1px solid var(--border); box-shadow: 0 2px 8px rgb(0 0 0 / 0.25);
    font: 600 13px/1.3 system-ui, sans-serif; pointer-events: none; text-align: center; }
</style>
