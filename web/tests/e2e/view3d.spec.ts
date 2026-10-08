// D40 : en 3D, clic sur une autre boucle (carte) et sur le profil fonctionnent ; couleur du ruban suit la sélection ; mobile.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr } from './helpers';

test('3D : sélection de boucle, profil, couleur', async ({ page, browserName }, info) => {
  test.slow(); // relief 3D sans carte graphique sur le runner de CI : rendu très lent
  test.setTimeout(180_000);
  const t = T();
  await setup(page);
  await mockPlan(page, async (route) => { await route.fulfill({ contentType: 'application/json', body: (await import('./helpers')).plan4 }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
  await page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' }).click();
  await expect(page.getByRole('button', { name: isFr() ? 'Vue 2D' : '2D view' })).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getPitch()), { timeout: 30_000 }).toBeGreaterThan(40); // CI sans carte graphique : rendu logiciel lent
  await expect.poll(() => page.evaluate(() => !!(window as any).tmap.map.getTerrain()), { timeout: 30_000 }).toBe(true);
  // la 3D est réellement active (mobile compris)
  expect(await page.evaluate(() => (window as any).tmap.map.getLayer('trail-3d') != null)).toBe(true);

  // clic sur une AUTRE boucle, sur la carte, en 3D
  const radios = page.getByRole('radio', { name: /^(Choisir la sortie|Choose run)/ });
  await expect(radios).toHaveCount(4);
  // caméra 3D arrivée (D57 : animation de cadrage) avant de viser un point à l'écran
  await page.waitForFunction(() => !(window as any).tmap.map.isMoving(), null, { timeout: 30_000 });
  // la caméra serre la sortie choisie (D57) : on cadre toutes les sorties au-dessus de la feuille pour viser une autre
  await page.evaluate(() => {
    const T = (window as any).tmap, m = T.map, cv = m.getCanvas().getBoundingClientRect();
    // haut de la feuille du bas (mobile) dans le repère de la carte ; ordinateur : 45 % de la hauteur
    const sheetTop = (r: DOMRect) => Math.min(r.height * 0.45, (document.querySelector('.sheet')?.getBoundingClientRect().top ?? 1e9) - r.y);
    (window as any).sheetTop = sheetTop;
    const xs = T.loops.flatMap((c: any) => c.lon), ys = T.loops.flatMap((c: any) => c.lat);
    m.fitBounds([[Math.min(...xs), Math.min(...ys)], [Math.max(...xs), Math.max(...ys)]],
      { padding: { top: 110, bottom: cv.height - sheetTop(cv) + 30, left: 30, right: 30 }, pitch: m.getPitch(), bearing: m.getBearing(), duration: 0 });
  });
  // rendu du nouveau cadrage avant le clic (la carte répond au clic sur la dernière image rendue)
  await page.waitForTimeout(1500);
  const pt = await page.evaluate(() => {
    const T = (window as any).tmap, m = T.map;
    for (let i = 1; i < T.loops.length; i++) {
      const c = T.loops[i];
      for (let k = 0; k < c.lat.length; k += 10) {
        const p = m.project([c.lon[k], c.lat[k]]), cv = m.getCanvas().getBoundingClientRect();
        const x = cv.x + p.x, y = cv.y + p.y;
        if (p.x > 20 && p.y > Math.max(100, cv.height * 0.15) && p.x < cv.width - 20 && p.y < (window as any).sheetTop(cv) - 20) { // sous la barre du haut, au-dessus de la feuille
          // pas de point d'une autre boucle à moins de 14 px
          const other = T.loops.some((o: any, j: number) => j !== i && o.lat.some((la: number, q: number) => q % 8 === 0 && (() => { const pp = m.project([o.lon[q], la]); return Math.hypot(pp.x - p.x, pp.y - p.y) < 14; })())) ;
          if (!other) return { i, x, y };
        }
      }
    }
    return null;
  });
  expect(pt, 'point cliquable d’une autre boucle').not.toBeNull();
  // WebKit : à 68° d'inclinaison (D57), `project` ne suit pas le relief rendu et le clic d'essai tombe à côté du
  // trait : on choisit par la liste (le clic sur la carte reste testé sous Chromium, mobile compris)
  if (browserName === 'webkit') await radios.nth(pt!.i).click();
  else if (info.project.use.hasTouch) await page.touchscreen.tap(pt!.x, pt!.y);
  else await page.mouse.click(pt!.x, pt!.y);
  await expect(radios.nth(pt!.i)).toHaveAttribute('aria-checked', 'true');
  await page.waitForFunction(() => !(window as any).tmap.map.isMoving(), null, { timeout: 30_000 }); // recadrage (D57)
  // une seule boucle active dans la source (mise en évidence 2D/3D), c'est la bonne
  // (la source GeoJSON se met à jour dans un worker : on attend)
  await expect.poll(() => page.evaluate(() => [...new Set((window as any).tmap.map.querySourceFeatures('loops', { filter: ['get', 'sel'] }).map((f: any) => f.properties.idx))]), { timeout: 40_000 }).toEqual([pt!.i]);
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

// D57 : la sortie choisie remplit l'écran (3D rapprochée et inclinée) ; choisir une autre variante la recadre, en 2D comme en 3D.
test('cadrage : variante choisie recadrée, 3D rapprochée', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-en');
  test.slow();
  test.setTimeout(180_000);
  const t = T();
  await setup(page);
  await mockPlan(page, async (route) => { await route.fulfill({ contentType: 'application/json', body: (await import('./helpers')).plan4 }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
  // part des points de la sortie i dans la carte, et largeur occupée (fraction de la largeur de la carte)
  const fill = (i: number) => page.evaluate((i) => {
    const T = (window as any).tmap, m = T.map, c = T.loops[i], cv = m.getCanvas().getBoundingClientRect();
    const ps = c.lat.map((la: number, k: number) => m.project([c.lon[k], la]));
    const inside = ps.filter((p: any) => p.x >= 0 && p.y >= 0 && p.x <= cv.width && p.y <= cv.height).length / ps.length;
    const xs = ps.map((p: any) => p.x), ys = ps.map((p: any) => p.y);
    // étendue : la plus grande des deux dimensions occupées (en 3D la sortie s'étire en profondeur)
    return { inside, width: Math.max((Math.max(...xs) - Math.min(...xs)) / cv.width, (Math.max(...ys) - Math.min(...ys)) / cv.height) };
  }, i);
  const settle = () => page.waitForFunction(() => !(window as any).tmap.map.isMoving(), null, { timeout: 30_000 });
  const radios = page.getByRole('radio', { name: /^(Choisir la sortie|Choose run)/ });
  await radios.nth(2).click();
  await settle();
  const f2 = await fill(2);
  expect(f2.inside).toBeGreaterThan(0.95);
  expect(f2.width).toBeGreaterThan(0.25);
  await page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getPitch()), { timeout: 30_000 }).toBeGreaterThan(60);
  await settle();
  await radios.nth(1).click();
  await settle();
  if (!process.env.CI) await page.screenshot({ path: test.info().outputPath('3d.png') }); // capture lente sans carte graphique
  const f1 = await fill(1);
  console.log('cadrage', JSON.stringify({ f2, f1 }));
  expect(f1.inside).toBeGreaterThan(0.9);
  expect(f1.width).toBeGreaterThan(0.35);
});
