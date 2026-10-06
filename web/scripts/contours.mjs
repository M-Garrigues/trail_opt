// T36 : isolignes d'un vrai relief (voir dem-fetch.mjs). Isolignes (grille Mapterhorn) → SVG : équidistance E, maîtresse toutes les 5, coordonnées entières.
import { contours } from 'd3-contour';
import { readFileSync, writeFileSync } from 'node:fs';
const [,, name, E = '20', f = '4', blur = '1', out = name + '.svg', OUTW = '1000'] = process.argv;
const { W, e } = JSON.parse(readFileSync(`${name}.json`));
const F = +f, n = Math.floor(W / F);
let g = new Float64Array(n * n);
for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
  let s = 0; for (let j = 0; j < F; j++) for (let i = 0; i < F; i++) s += e[(y * F + j) * W + x * F + i];
  g[y * n + x] = s / (F * F);
}
for (let b = 0; b < +blur; b++) { // flou 3×3 : courbes lisses comme une carte (généralisation)
  const h = new Float64Array(n * n);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    let s = 0, k = 0;
    for (let j = -1; j <= 1; j++) for (let i = -1; i <= 1; i++) { const X = x + i, Y = y + j; if (X >= 0 && Y >= 0 && X < n && Y < n) { s += g[Y * n + X]; k++; } }
    h[y * n + x] = s / k;
  }
  g = h;
}
let lo = Infinity, hi = -Infinity; for (const v of g) { lo = Math.min(lo, v); hi = Math.max(hi, v); }
const step = +E, th = []; for (let z = Math.ceil(lo / step) * step; z <= hi; z += step) th.push(z);
const S = +OUTW / n;
const paths = { m: [], i: [] };
for (const c of contours().size([n, n]).smooth(true).thresholds(th)(Array.from(g))) {
  const master = Math.round(c.value / step) % 5 === 0;
  for (const poly of c.coordinates) for (const ring of poly) {
    // retire le cadre de la grille (bords de l'extrait) : on ne garde que les segments intérieurs
    let d = '', pen = false, last = null;
    for (const [x, y] of ring) {
      const edge = x <= 0.5 || y <= 0.5 || x >= n - 0.5 || y >= n - 0.5;
      if (edge) { pen = false; continue; }
      const X = Math.round(x * S), Y = Math.round(y * S);
      if (last && Math.abs(X - last[0]) + Math.abs(Y - last[1]) < 3) continue;
      d += (pen ? `l${X - last[0]} ${Y - last[1]}` : `M${X} ${Y}`); pen = true; last = [X, Y];
    }
    if (d.includes('l')) (master ? paths.m : paths.i).push(d);
  }
}
const O = +OUTW;
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${O} ${O}" width="${O}" height="${O}" fill="none" stroke="#000" stroke-linejoin="round"><path stroke-width="1" d="${paths.i.join('')}"/><path stroke-width="2.2" d="${paths.m.join('')}"/></svg>`;
writeFileSync(out, svg);
console.log(out, th.length, 'niveaux', (svg.length / 1024).toFixed(0), 'Ko');
