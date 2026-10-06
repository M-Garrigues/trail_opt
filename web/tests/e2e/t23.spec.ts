// T23 : pente max (api.md v1.2), couverture, zone (51e sommet, édition), hors ligne, ?debug=1, « Oublier ma position ».
import { test, expect, type Page } from '@playwright/test';
import { setup, mockPlan, placeStart, openOptions, closeOptions, zoneClick, T, isFr } from './helpers';

const zoneBtn = (page: Page) => page.getByRole('button', { name: isFr() ? /^Zone/ : /^Area/ });
async function frac(page: Page) {
  const box = (await page.locator('.map').boundingBox())!;
  return (fx: number, fy: number) => [box.x + box.width * fx, box.y + box.height * fy] as const;
}

test('pente max : défaut 60 % non envoyé, « sans limite » = 0', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await openOptions(page);
  const sel = page.getByLabel(isFr() ? 'Pente max des chemins (sur 50 m)' : 'Max path grade (over 50 m)');
  await expect(sel).toHaveValue('60');
  const opts = await sel.locator('option').allTextContents();
  expect(opts[0]).toBe('5 %');
  expect(opts.at(-2)).toBe('60 %');
  expect(opts.at(-1)).toBe(isFr() ? 'sans limite' : 'no limit');
  await closeOptions(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[0].searchParams.has('max_grade_pct')).toBe(false); // absent = 60 (api.md v1.2)
  await page.keyboard.press('Escape'); // la feuille se replie
  await expect(page.getByTestId('headline')).toHaveCount(0);
  await openOptions(page);
  await sel.selectOption({ label: isFr() ? 'sans limite' : 'no limit' });
  await closeOptions(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[1].searchParams.get('max_grade_pct')).toBe('0');
});

test('hors couverture : grisé, erreur avant tout appel', async ({ page }) => {
  const t = T();
  await setup(page, { start: { lat: 44.84, lon: -0.58 } }); // Bordeaux
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  // couche grisée = monde troué par la couverture (2 polygones : IdF, Isère)
  await expect.poll(() => page.evaluate(() => {
    const src = (window as unknown as { tmap: { map: { getSource(id: string): { serialize(): { data: GeoJSON.Feature<GeoJSON.Polygon> } } } } })
      .tmap.map.getSource('outside');
    return src?.serialize().data.geometry?.coordinates.length ?? 0;
  })).toBe(3);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByRole('alert')).toContainText(isFr() ? 'Pas encore de données ici' : 'No data here yet');
  await expect(page.getByRole('button', { name: isFr() ? 'Voir la couverture' : 'See coverage' })).toBeVisible();
  expect(calls.length).toBe(0);
});

test('zone : le 51ᵉ sommet est refusé', async ({ page }) => {
  test.slow(); // 51 clics vérifiés un à un : > 1 min sur le runner de CI
  await setup(page);
  await page.goto('/');
  await placeStart(page);
  await zoneBtn(page).click();
  await expect(page.locator('.count')).toHaveText('0/50');
  const P = await frac(page);
  // terra-draw ferme le polygone si l'on touche (< 40 px) le 1er ou le dernier point : 1er point isolé en haut,
  // puis 49 points en zigzag entre deux rangées éloignées (loin du départ, au-dessus de la feuille)
  await zoneClick(page, ...P(0.5, 0.1), 1);
  for (let i = 0; i < 49; i++) await zoneClick(page, ...P(0.05 + (0.85 * i) / 48, i % 2 ? 0.45 : 0.2), i + 2);
  await expect(page.getByRole('alert')).toHaveText('50 points maximum.');
  await page.mouse.click(...P(0.95, 0.2)); // 51ᵉ
  await expect(page.locator('.count')).toHaveText('50/50');
  await page.mouse.click(...P(0.5, 0.1)); // fermeture : 50 sommets gardés
  await expect(page.getByTestId('zone-validate')).toBeEnabled();
  await expect(page.locator('.count')).toHaveText('50/50');
});

test('zone : fermée, un sommet se déplace ; rouverte, toujours éditable', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await zoneBtn(page).click();
  await expect(page.locator('.count')).toHaveText('0/50');
  const P = await frac(page);
  const tri = [P(0.15, 0.1), P(0.85, 0.1), P(0.5, 0.45)];
  for (const [i, p] of tri.entries()) await zoneClick(page, ...p, i + 1);
  await zoneClick(page, ...tri[0], 3); // fermeture → mode édition
  await expect(page.getByText(isFr() ? /Fais glisser un point/ : /Drag a point/)).toBeVisible();
  // on tire le sommet du bas un peu plus bas
  const [x, y] = tri[2];
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let k = 1; k <= 5; k++) await page.mouse.move(x, y + 6 * k);
  await page.mouse.up();
  await page.getByTestId('zone-validate').click();
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  const ring = calls[0].searchParams.get('polygon')!.split(';').map((p) => p.split(',').map(Number));
  expect(ring).toHaveLength(3);
  const startLat = Number(calls[0].searchParams.get('lat'));
  const lowest = Math.min(...ring.map(([, la]) => la));
  const topLat = Math.max(...ring.map(([, la]) => la));
  // départ à 30 % de la hauteur H, haut à 10 %, bas à 45 % : (départ − bas) / (haut − départ) = 0,75 sans glissement,
  // 0,75 + 30 / (0,2 H) après 30 px ; on exige au moins la moitié du glissement
  const H = (tri[2][1] - tri[0][1]) / 0.35;
  expect((startLat - lowest) / (topLat - startLat)).toBeGreaterThan(0.75 + 15 / (0.2 * H));
  await page.keyboard.press('Escape');

  // rouverte : directement en édition (3 sommets, aide d'édition)
  await zoneBtn(page).click();
  await expect(page.locator('.count')).toHaveText('3/50');
  await expect(page.getByText(isFr() ? /Fais glisser un point/ : /Drag a point/)).toBeVisible();
  await expect(page.getByTestId('zone-validate')).toBeEnabled();
});

test('hors ligne : Partager désactivé (E14)', async ({ page, context }) => {
  const t = T();
  await setup(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  const share = page.getByRole('button', { name: t.share, exact: true });
  await expect(share).toBeEnabled();
  await context.setOffline(true);
  await expect(share).toBeDisabled();
  await page.getByRole('button', { name: t.details }).click({ timeout: 3000 }).catch(() => {}); // bureau : détail déjà embarqué
  await expect(page.getByRole('button', { name: t.share, exact: true })).toBeDisabled();
  await context.setOffline(false);
  await expect(page.getByRole('button', { name: t.share, exact: true })).toBeEnabled();
});

test('?debug=1 : versions, durée, codes', async ({ page }) => {
  const t = T();
  await setup(page);
  let fail = false;
  await mockPlan(page, async (route) => {
    if (!fail) return false;
    await route.fulfill({ status: 422, contentType: 'application/json', body: '{"error":{"code":"no_loop_of_distance","params":{}}}' });
    return true;
  });
  await page.goto('/?debug=1');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  const dbg = page.getByTestId('debug');
  await expect(dbg).toContainText('bdtopo-wfs-2026-10');
  await expect(dbg).toContainText('solver 0.1.0');
  await expect(dbg).toContainText('1.86 s server');
  await expect(dbg).toContainText('access_round_trip');
  await page.keyboard.press('Escape');
  fail = true;
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('debug-error')).toHaveText('no_loop_of_distance · HTTP 422');
});

test('« Oublier ma position » efface la dernière position', async ({ page }) => {
  const t = T();
  await setup(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('optrail.lastStart'))).not.toBeNull();
  await page.getByRole('button', { name: t.menu }).click();
  await page.getByRole('button', { name: isFr() ? 'Oublier ma dernière position' : 'Forget my last position' }).click();
  await expect(page.getByRole('status').filter({ hasText: isFr() ? 'Position oubliée.' : 'Position forgotten.' })).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('optrail.lastStart'))).toBeNull();
});
