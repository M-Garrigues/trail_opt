<script lang="ts">
  // Graphique canvas maison (comme le profil) : courbes, ou barres empilées. `values` alignées sur `labels`.
  import { fmt } from './stats';

  type Series = { name: string; color: string; values: number[] };
  let { labels, series, kind = 'line', height = 200, digits = 0, title }: {
    labels: string[]; series: Series[]; kind?: 'line' | 'bar'; height?: number; digits?: number; title: string;
  } = $props();

  let canvas = $state<HTMLCanvasElement>();
  let width = $state(600);
  const empty = $derived(!labels.length || series.every((s) => s.values.every((v) => !v)));

  function draw() {
    if (!canvas || empty) return;
    const r = devicePixelRatio || 1, w = width, h = height;
    canvas.width = w * r;
    canvas.height = h * r;
    const c = canvas.getContext('2d')!;
    c.scale(r, r);
    const css = getComputedStyle(canvas);
    const muted = css.getPropertyValue('--muted').trim() || '#555', grid = css.getPropertyValue('--border').trim() || '#ddd';
    const L = 48, R = 8, T = 8, B = 22, n = labels.length;
    const tops = labels.map((_, i) => (kind === 'bar' ? series.reduce((a, s) => a + (s.values[i] ?? 0), 0) : Math.max(...series.map((s) => s.values[i] ?? 0))));
    // 4 intervalles « ronds » (entiers si digits = 0)
    const st0 = Math.max(...tops.filter(Number.isFinite), 1e-9) / 4, p = 10 ** Math.floor(Math.log10(st0));
    const max = 4 * ([1, 2, 2.5, 5, 10].map((k) => k * p).find((x) => x >= st0 && (digits > 0 || Number.isInteger(x))) ?? Math.ceil(st0));
    const step = (w - L - R) / n;
    const X = (i: number) => L + step * (i + 0.5), Y = (v: number) => T + (h - T - B) * (1 - v / max);
    c.font = '12px system-ui, sans-serif';
    c.fillStyle = muted;
    c.strokeStyle = grid;
    c.lineWidth = 1;
    c.textAlign = 'right';
    for (let k = 0; k <= 4; k++) {
      const v = (max * k) / 4, y = Math.round(Y(v)) + 0.5;
      c.beginPath(); c.moveTo(L, y); c.lineTo(w - R, y); c.stroke();
      c.fillText(fmt(v, digits), L - 6, y + 4);
    }
    c.textAlign = 'center';
    const every = Math.ceil(n / Math.max(1, Math.floor((w - L - R) / 70)));
    labels.forEach((l, i) => { if (i % every === 0) c.fillText(l, X(i), h - 6); });
    if (kind === 'bar') {
      const bw = Math.max(1, step * 0.7);
      labels.forEach((_, i) => {
        let base = 0;
        for (const s of series) {
          const v = s.values[i] ?? 0;
          c.fillStyle = s.color;
          c.fillRect(X(i) - bw / 2, Y(base + v), bw, Y(base) - Y(base + v));
          base += v;
        }
      });
    } else {
      for (const s of series) {
        c.strokeStyle = s.color;
        c.lineWidth = 2;
        c.beginPath();
        // valeur absente (NaN) : la courbe s'interrompt
        s.values.forEach((v, i) => (Number.isFinite(v) && i && Number.isFinite(s.values[i - 1]) ? c.lineTo(X(i), Y(v)) : c.moveTo(X(i), Y(v))));
        c.stroke();
        if (n < 60) { c.fillStyle = s.color; s.values.forEach((v, i) => { if (Number.isFinite(v)) { c.beginPath(); c.arc(X(i), Y(v), 2.5, 0, 7); c.fill(); } }); }
      }
    }
  }

  $effect(() => { void [labels, series, width, kind, canvas]; draw(); });
</script>

<figure>
  <figcaption>{title}</figcaption>
  {#if empty}
    <p class="none">Aucune donnée sur la période.</p>
  {:else}
    <div bind:clientWidth={width}>
      <canvas bind:this={canvas} style="width: 100%; height: {height}px" >{title}</canvas>
    </div>
    {#if series.length > 1}
      <ul class="legend">{#each series as s}<li><span style="background: {s.color}"></span>{s.name}</li>{/each}</ul>
    {/if}
  {/if}
</figure>

<style>
  figure { margin: 0; }
  figcaption { font-weight: 700; margin-bottom: 6px; }
  canvas { display: block; }
  .none { color: var(--muted); margin: 8px 0; }
  .legend { display: flex; gap: 14px; list-style: none; padding: 0; margin: 6px 0 0; color: var(--muted); font-size: 0.9rem; }
  .legend span { display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin-right: 6px; }
</style>
