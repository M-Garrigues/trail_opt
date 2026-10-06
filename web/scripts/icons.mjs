// Icônes de l'app installée, générées depuis le logo du site (public/icon.svg, seule source) :
//   node scripts/icons.mjs        (à relancer si le logo change ; les PNG sont versionnés)
// Rendu par le Chromium de Playwright (déjà installé pour les tests), aucune dépendance ajoutée.
import { readFileSync } from 'node:fs';
import { chromium } from '@playwright/test';

const pub = new URL('../public/', import.meta.url);
const svg = readFileSync(new URL('icon.svg', pub), 'utf8');
const bg = svg.match(/<rect[^>]*fill="(#[0-9a-f]{3,8})"/i)?.[1];
if (!bg) throw new Error('icon.svg : fond plein (<rect fill="#…">) attendu, iOS et les icônes masquables refusent la transparence');
const src = `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`;

// scale < 1 : logo réduit sur fond plein. Masquable : le dessin doit tenir dans le cercle de sécurité (rayon 40 %) ;
// le tracé du logo s'étend jusqu'à 31/32 du demi-côté → échelle ≤ 0,82, 0,75 retenu (marge).
const ICONS = [
  ['icon-192.png', 192, 1],
  ['icon-512.png', 512, 1],
  ['icon-maskable-512.png', 512, 0.75],
  ['apple-touch-icon.png', 180, 1],
];

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });
for (const [name, size, scale] of ICONS) {
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(`<body style="margin:0;height:100vh;display:grid;place-items:center;background:${bg}">
    <img src="${src}" style="width:${scale * 100}%;height:${scale * 100}%">`);
  await page.locator('img').evaluate((i) => i.decode());
  await page.screenshot({ path: new URL(name, pub).pathname });
  console.log(name);
}
await browser.close();
