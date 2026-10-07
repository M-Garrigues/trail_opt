// D40 : repères cols/sommets (api.md v1.5) : libellé sans doublon ni NaN, clic → centre la carte, boutons GPX/Partager avant le détail.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, plan1, T } from './helpers';

test('repères : liste, clic centre la carte', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-en');
  const t = T();
  await setup(page);
  const p = JSON.parse(plan1);
  const c = p.candidates[0], i = Math.floor(c.lat.length * 0.6);
  c.landmarks = [{ kind: 'col', name: 'Col du Granier', ele_m: 1327, dist_m: c.dist[i], lat: c.lat[i], lon: c.lon[i] }];
  const body = JSON.stringify(p);
  await mockPlan(page, async (r) => { await r.fulfill({ contentType: 'application/json', body }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  const row = page.locator('.lm').first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  const txt = (await row.innerText()).replace(/\s+/g, ' ');
  expect(txt).not.toMatch(/NaN/);
  expect(txt).not.toMatch(/Col\s+Col/i);
  expect(txt).toContain('≈ 1,330 m');
  // GPX / Partager au-dessus du détail
  const gpx = await page.getByRole('button', { name: /GPX/ }).first().boundingBox();
  const grid = await page.locator('dl.grid').boundingBox();
  expect(gpx!.y).toBeLessThan(grid!.y);
  // clic → la carte se centre sur le repère
  const before = await page.evaluate(() => (window as any).tmap.map.getCenter().toArray().join());
  await row.click();
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getCenter().toArray().join()), { timeout: 15_000 }).not.toBe(before);
  // le repère sur la carte (couche sous les libellés du fond, demande 5) est cliquable aussi
  await page.evaluate(([lo, la]) => (window as any).tmap.map.jumpTo({ center: [lo + 0.01, la] }), [c.lon[i], c.lat[i]]);
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.queryRenderedFeatures({ layers: ['landmarks'] }).length)).toBe(1);
  const xy = await page.evaluate(([lo, la]) => (window as any).tmap.map.project([lo, la]), [c.lon[i], c.lat[i]]);
  const box = (await page.locator('.maplibregl-canvas').boundingBox())!;
  await page.mouse.click(box.x + xy.x, box.y + xy.y);
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getCenter().lng), { timeout: 15_000 }).toBeCloseTo(c.lon[i], 2);
});
