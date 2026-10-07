// Actions suivies (D59) : un POST /api/hit part au clic GPX, au clic partager et à la création du lien,
// avec réglages et stats de la sortie, sans départ ni identifiant. Exclusion du local levée par
// `window.optrailHitTest` (tests seulement) ; aucun envoi si la case Réglages est décochée.
import { test, expect, type Page } from '@playwright/test';
import { setup, mockPlan, placeStart, T } from './helpers';

async function events(page: Page) {
  const sent: Record<string, unknown>[] = [];
  await page.addInitScript(() => {
    (window as unknown as { optrailHitTest: boolean }).optrailHitTest = true;
    Object.defineProperty(navigator, 'canShare', { value: undefined }); // GPX : branche téléchargement
    Object.defineProperty(navigator, 'share', { value: undefined });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async () => {} } });
  });
  await page.route('**/api/hit', async (route) => {
    sent.push(route.request().postDataJSON());
    await route.fulfill({ status: 204 });
  });
  await page.route('**/api/loops', (route) => route.fulfill({ status: 201, contentType: 'application/json', body: '{"id":"ZkD5HAVbAzLl"}' }));
  return sent;
}

test('événements GPX et partage', async ({ page }) => {
  const t = T();
  await setup(page);
  const sent = await events(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect.poll(() => sent.filter((b) => b.page).length).toBe(1); // visite
  await page.getByRole('button', { name: t.download }).click();
  await expect.poll(() => sent.filter((b) => b.event === 'gpx').length).toBe(1);
  const gpx = sent.find((b) => b.event === 'gpx')!;
  expect(gpx).toMatchObject({ goal: 'max_dplus', surface: 'trail', rank: 1, max_grade_pct: 60, zone: false, via_n: 0 });
  expect([gpx.got_km, gpx.got_dplus_m]).toEqual([10, 250]); // tranches 5 km / 250 m (revue M4)
  for (const k of ['lat', 'lon', 'polygon', 'via', 'seed', 'id', 'sig']) expect(gpx, k).not.toHaveProperty(k);
  await page.getByRole('button', { name: t.share, exact: true }).click();
  await page.getByRole('button', { name: t.create }).click();
  await expect.poll(() => sent.map((b) => b.event).filter(Boolean)).toEqual(['gpx', 'share_click', 'share_created']);
  // tout ce qui part est dans la liste blanche du serveur (engine/src/hit.rs)
  const allowed = ['event', 'lang', 'goal', 'km', 'dplus_m', 'surface', 'climbs', 'max_grade_pct', 'zone', 'via_n', 'no_repeat', 'rank',
    'got_km', 'got_dplus_m', 'trail_pct', 'mixed_pct', 'road_pct', 'compute_s'];
  for (const b of sent.filter((x) => x.event)) for (const k of Object.keys(b)) expect(allowed, k).toContain(k);
});

test('case « Mesure d’audience » décochée : rien ne part', async ({ page }) => {
  const t = T();
  await setup(page);
  await page.addInitScript(() => localStorage.setItem('optrail.nostats', 'true'));
  const sent = await events(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await page.getByRole('button', { name: t.download }).click();
  await page.waitForTimeout(500);
  expect(sent).toEqual([]);
});
