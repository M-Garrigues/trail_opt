// Parcours réel SANS mock de l'API (2026-10-08, partage cassé en prod) : calcul local → partager → ouvrir le lien → GPX,
// avec les événements de mesure d'audience. Sauté en CI ou sans API locale.
import { test, expect } from '@playwright/test';
import { setup, placeStart, T, isFr } from './helpers';

for (const type of ['max_dplus', null] as const) test(`réel (${type ?? 'Cible'}) : calcul → partage → lien → GPX, événements acceptés`, async ({ page, request }, info) => {
  test.skip(!!process.env.CI, 'pas d’API en CI');
  test.skip(info.project.name !== 'desktop-en' && info.project.name !== 'iphone-fr', 'deux projets suffisent');
  const alive = await request.get('/api/plan?lat=abc').then((r) => r.status() === 400).catch(() => false);
  test.skip(!alive, 'API locale absente');
  test.setTimeout(120_000);
  const t = T();
  await setup(page, { start: { lat: 48.7309, lon: 2.2713 }, type }); // null : première visite, Cible (défaut)
  await page.addInitScript(() => {
    (window as unknown as { optrailHitTest: boolean }).optrailHitTest = true; // mesure d'audience forcée en local
    Object.defineProperty(navigator, 'share', { value: undefined });
    Object.defineProperty(navigator, 'canShare', { value: undefined });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (s: string) => { (window as unknown as { copied: string }).copied = s; } } });
  });
  const events: { body: Record<string, unknown>; status: number }[] = [];
  page.on('response', async (r) => {
    if (r.url().endsWith('/api/hit') && r.request().method() === 'POST') {
      const body = r.request().postDataJSON() as Record<string, unknown>;
      if (body.event) events.push({ body, status: r.status() });
    }
  });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 60_000 });
  // partage réel : signature posée par /api/plan, vérifiée par POST /api/loops
  const posted = page.waitForResponse((r) => r.url().endsWith('/api/loops') && r.request().method() === 'POST');
  await page.getByRole('button', { name: t.share, exact: true }).click();
  await page.getByRole('button', { name: t.create }).click();
  const res = await posted;
  expect(res.status(), await res.text()).toBe(201);
  const url = await expect.poll(() => page.evaluate(() => (window as unknown as { copied?: string }).copied)).toMatch(/\/b\/[A-Za-z0-9]+$/).then(() => page.evaluate(() => (window as unknown as { copied: string }).copied));
  // ouverture du lien, puis GPX depuis la sortie partagée
  await page.goto(new URL(url).pathname);
  await expect(page.getByText(isFr() ? 'Sortie partagée' : 'Shared run')).toBeVisible({ timeout: 20_000 });
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: t.download }).first().click();
  expect((await dl).suggestedFilename()).toMatch(/^optrail-.*\.gpx$/);
  // événements : partage (clic, lien créé), ouverture du lien, GPX, tous acceptés (204)
  await expect.poll(() => events.map((e) => e.body.event).sort()).toEqual(['gpx', 'share_click', 'share_created', 'shared_open']);
  for (const e of events) expect(e.status, JSON.stringify(e.body)).toBe(204);
});
