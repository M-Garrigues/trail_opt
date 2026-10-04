// Parcours 3 : autres boucles, historique, langue, dernier type, partage et lien /b/<id>.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr, plan1, plan4 } from './helpers';

test('autres boucles + historique (AC9, AC15)', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toHaveText('+424 m');
  await page.getByRole('button', { name: t.other, exact: true }).click();
  await expect(page.getByRole('radio', { name: /2$/ })).toBeVisible();
  expect(calls[1].searchParams.get('n_candidates')).toBe('4');
  expect(calls[1].searchParams.get('seed')).toBe(calls[0].searchParams.get('seed'));
  await expect(page.locator('.loops .loop')).toHaveCount(4); // n°1 non dupliquée
  await page.locator('.loops .loop').nth(1).click();
  await expect(page.getByTestId('headline')).toHaveText('+349 m');

  // historique : 1 + 3 boucles reçues
  await page.getByRole('button', { name: t.history }).first().click();
  await expect(page.getByTestId('history').locator('li')).toHaveCount(4);
  await expect(page.getByRole('button', { name: t.clear })).toBeInViewport();
  await page.getByTestId('history').locator('li').first().getByRole('button').last().click();
  await expect(page.getByTestId('history').locator('li')).toHaveCount(3);
  page.once('dialog', (d) => d.accept());
  await page.getByRole('button', { name: t.clear }).click();
  await expect(page.getByTestId('history')).toHaveCount(0);
});

test('réglage 3 boucles → « Autres boucles (2) » sans appel (AC9)', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page, async (route) => {
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...JSON.parse(plan1), candidates: JSON.parse((await import('./helpers')).plan4).candidates.slice(0, 3) }) });
    return true;
  });
  await page.goto('/');
  await page.getByRole('button', { name: t.menu }).click();
  await page.locator('#m-loops').selectOption('3');
  await page.goBack();
  await expect(page.getByRole('button', { name: t.find })).toBeVisible(); // menu refermé (feuille redescendue)
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByText(isFr() ? 'Autres boucles (2)' : 'Other loops (2)')).toBeVisible();
  expect(calls.length).toBe(1);
  expect(calls[0].searchParams.get('n_candidates')).toBe('3');
});

test('langue et dernier type gardés (AC20, AC22)', async ({ page }) => {
  await setup(page);
  await page.goto('/');
  const other = isFr() ? 'EN' : 'FR';
  await page.locator('.topbar').getByRole('radio', { name: other }).click();
  await page.getByRole('radio', { name: isFr() ? /^Target/ : /^Cible/ }).click();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('lang', other.toLowerCase());
  await expect(page.getByRole('radio', { name: isFr() ? /^Target/ : /^Cible/ })).toHaveAttribute('aria-checked', 'true');
});

test('partage : confirmation, lien copié, /b/<id> sans calcul (AC12, AC13)', async ({ page }) => {
  const t = T();
  await setup(page);
  // presse-papiers simulé (WebKit ne connaît pas les permissions clipboard-*)
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'share', { value: undefined });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (s: string) => { (window as unknown as { copied: string }).copied = s; } } });
  });
  const calls = await mockPlan(page);
  let posted: Record<string, unknown> | null = null;
  let sha = '';
  await page.route('**/api/loops', async (route) => {
    posted = route.request().postDataJSON();
    sha = route.request().headers()['x-amz-content-sha256'] ?? '';
    await route.fulfill({ status: 201, contentType: 'application/json', body: '{"id":"ZkD5HAVbAzLl"}' });
  });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await page.getByRole('button', { name: t.share, exact: true }).click();
  await expect(page.getByText(isFr() ? /montre ton point de départ/ : /shows your start/)).toBeVisible();
  expect(posted).toBeNull(); // confirmation d'abord
  await page.getByRole('button', { name: t.create }).click();
  await expect(page.getByRole('status').filter({ hasText: isFr() ? 'Lien copié' : 'Link copied' })).toBeVisible();
  expect(Object.keys(posted!).sort()).toEqual(['candidate', 'data_version', 'effective_start', 'request', 'solver_version', 'warnings', 'zone']);
  expect(sha).toMatch(/^[0-9a-f]{64}$/);
  // api.md v1.3 (M3) : candidate recopié tel quel, signature comprise
  const sig = (posted as unknown as { candidate: { sig: string } }).candidate.sig;
  expect([plan1, plan4].map((p) => JSON.parse(p).candidates[0].sig)).toContain(sig);
  expect(await page.evaluate(() => (window as unknown as { copied: string }).copied)).toMatch(/\/b\/ZkD5HAVbAzLl$/);

  // E10 : ouverture du lien sans /api/plan
  const stored = JSON.stringify(posted);
  await page.route('**/api/loops/ZkD5HAVbAzLl', (r) => r.fulfill({ contentType: 'application/json', body: stored }));
  await page.route('**/api/loops/nope000000', (r) => r.fulfill({ status: 404, contentType: 'application/json', body: '{"error":{"code":"loop_not_found","params":{}}}' }));
  const before = calls.length;
  await page.goto('/b/ZkD5HAVbAzLl');
  await expect(page.getByTestId('headline')).toHaveText('+424 m');
  await expect(page.getByText(isFr() ? 'Boucle partagée' : 'Shared loop')).toBeVisible();
  expect(calls.length).toBe(before);
  // repartage d'une boucle ouverte par lien : son URL, sans nouveau POST (GET ne rend pas sig)
  posted = null;
  await page.getByRole('button', { name: t.share, exact: true }).click();
  await page.getByRole('button', { name: t.create }).click();
  await expect(page.getByRole('status').filter({ hasText: isFr() ? 'Lien copié' : 'Link copied' })).toBeVisible();
  expect(posted).toBeNull();
  expect(await page.evaluate(() => (window as unknown as { copied: string }).copied)).toMatch(/\/b\/ZkD5HAVbAzLl$/);
  await page.goto('/b/nope000000');
  await expect(page.getByRole('alert')).toContainText(isFr() ? 'expiré' : 'expired');
});
