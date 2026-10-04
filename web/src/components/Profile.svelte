<script lang="ts">
  // Profil d'altitude en canvas, lié à la carte (port de ProfileLink, trailopt/maplayers.py).
  import type { Candidate } from '../lib/types';
  import { grades, indexAtDist } from '../lib/geo';
  import { i18n, t } from '../i18n/i18n.svelte';
  import { num } from '../i18n/format';

  let { cand, cursor = $bindable(-1), height = 130, onpick }: { cand: Candidate; cursor?: number; height?: number; onpick?: (i: number) => void } = $props();

  const PAD = { l: 46, r: 12, t: 22, b: 22 };
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
  const X = (d: number) => PAD.l + ((w - PAD.l - PAD.r) * d) / dmax;
  const Y = (z: number) => PAD.t + (height - PAD.t - PAD.b) * (1 - (z - zr.zmin) / (zr.zmax - zr.zmin));
  const label = $derived(t().detail.profile({
    km: num(i18n.lang, dmax / 1000, 1), min: num(i18n.lang, zr.zmin), max: num(i18n.lang, zr.zmax),
  }));
  const bubble = (i: number) =>
    `km ${num(i18n.lang, cand.dist[i] / 1000, 1)} · ${num(i18n.lang, cand.ele[i])} m · ${num(i18n.lang, G[i])} %`;

  $effect(() => {
    // dépendances : cand, cursor, w, langue, thème
    const n = cand.dist.length, r = devicePixelRatio || 1, D = cand, cur = cursor;
    void i18n.lang;
    canvas.width = w * r;
    canvas.height = height * r;
    const c = canvas.getContext('2d')!;
    const css = getComputedStyle(canvas);
    const text = css.getPropertyValue('--text').trim() || '#111', muted = css.getPropertyValue('--muted').trim() || '#555';
    const grid = css.getPropertyValue('--border').trim() || '#ddd', bg = css.getPropertyValue('--bg').trim() || '#fff';
    c.setTransform(r, 0, 0, r, 0, 0);
    c.clearRect(0, 0, w, height);
    c.font = '12px system-ui, sans-serif';
    c.lineWidth = 1;
    for (let i = 0; i <= 2; i++) {
      const z = zr.zmin + ((zr.zmax - zr.zmin) * i) / 2, y = Y(z);
      c.strokeStyle = grid; c.beginPath(); c.moveTo(PAD.l, y); c.lineTo(w - PAD.r, y); c.stroke();
      c.fillStyle = muted; c.textAlign = 'right'; c.fillText(`${num(i18n.lang, z)} m`, PAD.l - 5, y + 4);
    }
    c.textAlign = 'center';
    const km = dmax / 1000, step = km > 40 ? 10 : km > 12 ? 5 : km > 5 ? 2 : 1;
    for (let k = 0; k <= km; k += step) c.fillText(`${k} km`, X(k * 1000), height - 6);
    const base = height - PAD.b;
    for (let i = 0; i < n - 1; i++) {
      const xa = X(D.dist[i]), xb = X(D.dist[i + 1]);
      c.fillStyle = slopeColor((G[i] + G[i + 1]) / 2);
      c.beginPath(); c.moveTo(xa, base); c.lineTo(xa, Y(D.ele[i])); c.lineTo(xb + 0.6, Y(D.ele[i + 1])); c.lineTo(xb + 0.6, base); c.closePath(); c.fill();
    }
    c.beginPath(); c.moveTo(X(D.dist[0]), Y(D.ele[0]));
    for (let i = 1; i < n; i++) c.lineTo(X(D.dist[i]), Y(D.ele[i]));
    c.strokeStyle = text; c.lineWidth = 1.5; c.stroke();
    if (cur >= 0 && cur < n) {
      const x = X(D.dist[cur]), left = x > w / 2;
      c.strokeStyle = text; c.lineWidth = 1;
      c.beginPath(); c.moveTo(x, PAD.t); c.lineTo(x, base); c.stroke();
      c.fillStyle = text; c.beginPath(); c.arc(x, Y(D.ele[cur]), 4, 0, 6.3); c.fill();
      c.font = 'bold 13px system-ui, sans-serif';
      const txt = bubble(cur), tw = c.measureText(txt).width + 8, tx = Math.max(0, left ? x - 8 - tw : x + 8);
      c.fillStyle = bg; c.globalAlpha = 0.9; c.fillRect(tx, 2, tw, 18); c.globalAlpha = 1;
      c.fillStyle = text; c.textAlign = 'left'; c.fillText(txt, tx + 4, 15);
    }
  });

  function pick(clientX: number) {
    const b = canvas.getBoundingClientRect();
    cursor = indexAtDist(cand.dist, ((clientX - b.left - PAD.l) / (w - PAD.l - PAD.r)) * dmax);
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
    style:height="{height}px"
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
</div>

<style>
  .profile { width: 100%; position: relative; }
  canvas { display: block; touch-action: none; border-radius: 8px; }
  canvas:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
</style>
