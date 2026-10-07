// Page /admin (contracts/admin.md § 5) : ordinateur seulement, GET /api/admin/stats mocké par fixture.
import { readFileSync } from 'node:fs';
import { test, expect } from '@playwright/test';

const stats = readFileSync(new URL('../fixtures/admin-stats.json', import.meta.url), 'utf8');

test.beforeEach(({}, info) => test.skip(info.project.name !== 'desktop-en', 'page pensée pour ordinateur'));

test('clé, refus, rendu des vues, noindex', async ({ page }) => {
  const keys: (string | undefined)[] = [];
  await page.route('**/api/admin/stats**', async (route) => {
    const k = route.request().headers()['x-admin-key'];
    keys.push(k);
    if (k !== 'bonne') return route.fulfill({ status: 401, contentType: 'application/json', body: '{"error":{"code":"admin_denied","params":{}}}' });
    await route.fulfill({ contentType: 'application/json', body: stats });
  });
  let hits = 0;
  await page.route('**/api/hit', (route) => { hits++; return route.fulfill({ status: 204 }); });
  await page.goto('/admin');
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex');
  // mauvaise clé : refusée, champ de nouveau proposé
  await page.getByLabel('Clé d’administration').fill('mauvaise');
  await page.getByRole('button', { name: 'Entrer' }).click();
  await expect(page.getByRole('alert')).toHaveText('Clé refusée.');
  await page.getByLabel('Clé d’administration').fill('bonne');
  await page.getByRole('button', { name: 'Entrer' }).click();
  // tuiles
  const tiles = page.getByRole('region', { name: 'Chiffres clés' });
  await expect(tiles).toContainText('15'); // visiteurs
  await expect(tiles).toContainText('10,0 %'); // taux d'échec
  await expect(tiles).toContainText('4,1 / 9,6 s');
  // graphiques canvas, tableaux, carte
  await expect(page.locator('canvas:not(.maplibregl-canvas)')).toHaveCount(5);
  await expect(page.getByRole('cell', { name: 'timeout' })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'bot_check_failed' })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'Max D+' })).toBeVisible();
  await expect(page.getByRole('cell', { name: '—' }).first()).toBeVisible(); // type de voie absent (journal v1)
  await expect(page.getByRole('cell', { name: 'google.com' })).toBeVisible();
  await expect(page.locator('.maplibregl-canvas')).toBeVisible();
  // clé gardée pour la session : rechargement sans nouvelle saisie ; aucune mesure d'audience sur /admin
  await page.reload();
  await expect(page.getByRole('region', { name: 'Chiffres clés' })).toBeVisible();
  expect(keys.slice(-1)).toEqual(['bonne']);
  expect(hits).toBe(0);
});

test('trop d’essais, identifiants AWS expirés', async ({ page }) => {
  let status = 429;
  await page.route('**/api/admin/stats**', (route) => route.fulfill({
    status, contentType: 'application/json',
    body: status === 429 ? '{"error":{"code":"admin_locked","params":{}}}' : '{"error":{"code":"busy","params":{},"detail":"aws credentials expired"}}',
  }));
  await page.goto('/admin');
  await page.getByLabel('Clé d’administration').fill('x');
  await page.getByRole('button', { name: 'Entrer' }).click();
  await expect(page.getByRole('alert')).toHaveText('Trop d’essais, réessayer dans 15 min.');
  status = 503;
  await page.getByRole('button', { name: 'Actualiser' }).click();
  await expect(page.getByRole('alert')).toHaveText('Identifiants AWS expirés : aws login --profile optrail');
});
