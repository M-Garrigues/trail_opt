// D40 : repères cols/sommets (api.md v1.5) : libellé sans doublon ni NaN, clic → centre la carte, boutons GPX/Partager avant le détail.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, plan1, T, isFr } from './helpers';

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

// 2026-10-07 : sur le profil, seulement le symbole du col ; nom, altitude et km dans une bulle (survol, toucher, clavier).
test('profil : symbole du repère, bulle au survol / au toucher / au clavier', async ({ page }, info) => {
  await setup(page);
  const p = JSON.parse(plan1);
  const c = p.candidates[0], i = Math.floor(c.lat.length * 0.6);
  c.landmarks = [{ kind: 'col', name: 'Col du Granier', ele_m: 1327, dist_m: c.dist[i], lat: c.lat[i], lon: c.lon[i] }];
  p.warnings = [];
  await mockPlan(page, async (r) => { await r.fulfill({ contentType: 'application/json', body: JSON.stringify(p) }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: T().find }).click();
  const sym = page.locator('.lms .landmark').first();
  await expect(sym).toHaveAttribute('aria-label', /Col du Granier · ≈ 1.330 m · km /);
  await sym.scrollIntoViewIfNeeded();
  const bubble = page.locator('.lm-bubble');
  await expect(bubble).toHaveCount(0); // aucun nom écrit en permanence
  const b = (await sym.boundingBox())!;
  if (info.project.use.hasTouch) {
    await page.touchscreen.tap(b.x + b.width / 2, b.y + b.height / 2);
    await expect(bubble).toContainText('Col du Granier');
    await page.getByTestId('headline').tap(); // toucher ailleurs ferme
    await expect(bubble).toHaveCount(0);
  } else {
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    await expect(bubble).toContainText('Col du Granier');
    await page.mouse.move(b.x + b.width / 2, b.y + 200);
    await expect(bubble).toHaveCount(0);
    await sym.focus(); // clavier : Tab arrive sur le symbole, la bulle s'ouvre
    await expect(bubble).toContainText('Col du Granier');
  }
});

// 2026-10-07 : un simple départ déplacé n'est plus une popup « demande non atteinte », mais une ligne d'information.
test('départ déplacé : ligne d’information, pas de popup', async ({ page }) => {
  await setup(page);
  const p = JSON.parse(plan1);
  p.warnings = [{ code: 'start_moved', params: { distance_m: 404 } }];
  await mockPlan(page, async (r) => { await r.fulfill({ contentType: 'application/json', body: JSON.stringify(p) }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: T().find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  await expect(page.getByText(isFr() ? 'Départ déplacé de 404 m vers le chemin le plus proche.' : 'Start moved 404 m to the nearest path.').first()).toBeVisible();
  await expect(page.getByRole('alertdialog')).toHaveCount(0);
});
