// D40 : en 3D, clic sur une autre boucle (carte) et sur le profil fonctionnent ; couleur du ruban suit la sélection ; mobile.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr } from './helpers';

test('3D : sélection de boucle, profil, couleur', async ({ page }) => {
  test.setTimeout(120_000);
  const t = T();
  await setup(page);
  await mockPlan(page, async (route) => { await route.fulfill({ contentType: 'application/json', body: (await import('./helpers')).plan4 }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
  await page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' }).click();
  await expect(page.getByRole('button', { name: isFr() ? 'Vue 2D' : '2D view' })).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getPitch()), { timeout: 10_000 }).toBeGreaterThan(40);
  await expect.poll(() => page.evaluate(() => !!(window as any).tmap.map.getTerrain()), { timeout: 30_000 }).toBe(true);
  // la 3D est réellement active (mobile compris)
  expect(await page.evaluate(() => (window as any).tmap.map.getLayer('trail-3d') != null)).toBe(true);

  // clic sur une AUTRE boucle, sur la carte, en 3D
  const radios = page.getByRole('radio', { name: /^(Choisir la boucle|Choose loop)/ });
  await expect(radios).toHaveCount(4);
  const pt = await page.evaluate(() => {
    const T = (window as any).tmap, m = T.map;
    for (let i = 1; i < T.loops.length; i++) {
      const c = T.loops[i];
      for (let k = 0; k < c.lat.length; k += 10) {
        const p = m.project([c.lon[k], c.lat[k]]), cv = m.getCanvas().getBoundingClientRect();
        const x = cv.x + p.x, y = cv.y + p.y;
        if (p.x > 20 && p.y > 20 && p.x < cv.width - 20 && p.y < cv.height * 0.4) {
          // pas de point d'une autre boucle à moins de 14 px
          const other = T.loops.some((o: any, j: number) => j !== i && o.lat.some((la: number, q: number) => q % 8 === 0 && (() => { const pp = m.project([o.lon[q], la]); return Math.hypot(pp.x - p.x, pp.y - p.y) < 14; })())) ;
          if (!other) return { i, x, y };
        }
      }
    }
    return null;
  });
  expect(pt, 'point cliquable d’une autre boucle').not.toBeNull();
  await page.mouse.click(pt!.x, pt!.y);
  await expect(radios.nth(pt!.i)).toHaveAttribute('aria-checked', 'true');
  // une seule boucle active dans la source (mise en évidence 2D/3D), c'est la bonne
  // (la source GeoJSON se met à jour dans un worker : on attend)
  await expect.poll(() => page.evaluate(() => [...new Set((window as any).tmap.map.querySourceFeatures('loops', { filter: ['get', 'sel'] }).map((f: any) => f.properties.idx))]), { timeout: 10_000 }).toEqual([pt!.i]);
  // couleur du ruban 3D = couleur de la boucle choisie
  const color = await page.evaluate(() => (window as any).tmap.map.style._layers['trail-3d'].implementation.color);
  expect(color).toBe(['#a8441c', '#1565C0', '#6A1B9A', '#AD1457'][pt!.i]);

  // profil : un clic recentre la carte
  const before = await page.evaluate(() => (window as any).tmap.map.getCenter().toArray().join());
  await page.getByRole("slider").first().scrollIntoViewIfNeeded();
  const cv = (await page.getByRole("slider").first().boundingBox())!;
  await page.mouse.click(cv.x + cv.width * 0.8, cv.y + cv.height / 2);
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getCenter().toArray().join()), { timeout: 25_000 }).not.toBe(before) ;
});
