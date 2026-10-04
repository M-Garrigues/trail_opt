// T21 (D27) : directions visuelles « nature » sélectionnables par ?design=a|b|c (v1 = actuel, par défaut),
// ?pick=ridge|list (sélecteur d'entraînement). Mémorisé en localStorage pour garder le choix sur le téléphone.
import { load, save } from './store';

export type Design = 'v1' | 'a' | 'b' | 'c';
export type Pick = 'ridge' | 'list';

const q = new URLSearchParams(location.search);
const qd = q.get('design'), qp = q.get('pick');
export const design: Design = (['v1', 'a', 'b', 'c'] as const).find((d) => d === (qd ?? load('design', 'v1'))) ?? 'v1';
export const pick: Pick = qp === 'list' || qp === 'ridge' ? qp : load<Pick | null>('pick', null) ?? (design === 'b' ? 'list' : 'ridge');
if (qd) save('design', design);
if (qp) save('pick', pick);
/** Nouvelle interface (niveaux, profil en bas, sélecteur sans cartes, 3D). */
export const nature = design !== 'v1';

// Bruit déterministe : somme de sinus à phases tirées (lisse, sans dépendance).
function rng(seed: number) {
  return () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 2 ** 32);
}
function noise(seed: number) {
  const r = rng(seed), w = [1, 2.3, 4.1].map((f) => ({ f, p: r() * 6.283, a: r() * 0.5 + 0.5 }));
  return (t: number) => w.reduce((s, k) => s + (k.a * Math.sin(k.f * t + k.p)) / k.f, 0) / 1.6;
}
const pts = (p: [number, number][], d = 1) => p.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(d)},${y.toFixed(d)}`).join('');

/** Bord organique : polyligne y(x) sur [0, len], base à `base`, amplitude `amp`, décalée de `dy`. */
export function wave(len: number, base: number, amp: number, seed: number, dy = 0): [number, number][] {
  const n = noise(seed);
  return Array.from({ length: 41 }, (_, i) => [(len * i) / 40, base - dy + amp * n((i / 40) * 6.283)] as [number, number]);
}
export const wavePath = (p: [number, number][]) => pts(p);

/** Motif de courbes de niveau (tuile raccordable) : collines aux anneaux déformés, 1 maîtresse sur 5. */
export function topoTile(size = 480, seed = 7): string {
  const r = rng(seed), out: string[] = [];
  for (let h = 0; h < 5; h++) {
    const cx = r() * size, cy = r() * size, R = size * (0.18 + r() * 0.22), n = noise(seed + h * 31), K = 6 + Math.floor(r() * 6);
    for (let k = 1; k <= K; k++) {
      const rk = (R * k) / K;
      const ring = Array.from({ length: 37 }, (_, i) => {
        const a = (i / 36) * 6.283, d = rk * (1 + 0.28 * n(a) * (0.6 + k / K));
        return [cx + d * Math.cos(a), cy + d * Math.sin(a)] as [number, number];
      });
      const w = k % 5 === 0 ? 1.6 : 0.8;
      // copies décalées pour raccorder les bords de la tuile
      for (const ox of [-size, 0, size]) for (const oy of [-size, 0, size]) {
        const e = rk * 1.6; // la copie touche-t-elle la tuile ?
        if (cx + ox + e < 0 || cx + ox - e > size || cy + oy + e < 0 || cy + oy - e > size) continue;
        out.push(`<path d="${pts(ring.map(([x, y]) => [x + ox, y + oy]), 0)}" stroke-width="${w}"/>`);
      }
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}"><g fill="none" stroke="#000">${out.join('')}</g></svg>`;
}

document.documentElement.dataset.design = design;
if (nature) document.documentElement.style.setProperty('--topo', `url("data:image/svg+xml,${encodeURIComponent(topoTile())}")`);
