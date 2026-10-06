// T36 (D34) : grille d'altitudes RÉELLE pour le décor en courbes de niveau (src/assets/contours.svg).
// MNT Mapterhorn (terrarium, = RGE ALTI® IGN en France), décodé par Chromium (webp). Hors build, à la main :
//   node scripts/dem-fetch.mjs chartreuse 45.31 5.85 13 2   → chartreuse.json (1024², ≈ 6,9 km de côté)
//   npm i --no-save d3-contour@4 && node scripts/contours.mjs chartreuse 20 4 1 src/assets/contours.svg
import { chromium } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const [,, name, lat, lon, z = '13', n = '2'] = process.argv;
const Z = +z, N = +n;
const xt = Math.floor(((+lon + 180) / 360) * 2 ** Z);
const yt = Math.floor((1 - Math.log(Math.tan((+lat * Math.PI) / 180) + 1 / Math.cos((+lat * Math.PI) / 180)) / Math.PI) / 2 * 2 ** Z);
const b = await chromium.launch(); const p = await b.newPage(); await p.goto('https://tiles.mapterhorn.com/');
const out = await p.evaluate(async ({ Z, N, xt, yt }) => {
  const S = 512, W = S * N, c = new OffscreenCanvas(W, W), g = c.getContext('2d');
  for (let i = 0; i < N; i++) for (let j = 0; j < N; j++) {
    const im = new Image(); im.crossOrigin = 'anonymous'; im.src = `https://tiles.mapterhorn.com/${Z}/${xt + i}/${yt + j}.webp`;
    await im.decode(); g.drawImage(im, i * S, j * S);
  }
  const d = g.getImageData(0, 0, W, W).data, e = new Array(W * W);
  for (let k = 0; k < W * W; k++) e[k] = Math.round((d[4 * k] * 256 + d[4 * k + 1] + d[4 * k + 2] / 256 - 32768) * 10) / 10;
  return { W, e };
}, { Z, N, xt, yt });
await b.close();
writeFileSync(`${name}.json`, JSON.stringify(out));
console.log(name, out.W, out.e.reduce((a,b)=>Math.min(a,b)), out.e.reduce((a,b)=>Math.max(a,b)));
