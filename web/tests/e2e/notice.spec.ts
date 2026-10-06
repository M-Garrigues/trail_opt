// D40 : « message important » modal quand la demande n'est pas atteinte ; action « Inclure les routes ? » change le réglage et relance.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, openSheet, plan1, T, isFr } from './helpers';

test('demande non atteinte : popup + inclure les routes', async ({ page }) => {
  const t = T();
  await setup(page);
  const body = JSON.stringify({ ...JSON.parse(plan1), warnings: [{ code: 'target_not_reached', params: { dplus_m: 900, best_dplus_m: 600, km: null, best_km: null } }] });
  const calls = await mockPlan(page, async (route, url) => {
    // D46 : la réponse du plan n'a plus `suggest` ; il vient d'un second appel diagnose=1
    await route.fulfill({ contentType: 'application/json', body: url.searchParams.has('diagnose') ? JSON.stringify({ suggest: { roads: 'all' }, checked: true }) : body });
    return true;
  });
  await page.goto('/');
  await placeStart(page);
  await openSheet(page);
  await page.locator('details.more summary').click();
  await page.locator('#roads').selectOption('unpaved');
  await page.getByRole('button', { name: t.find }).click();
  const dlg = page.getByRole('alertdialog');
  await expect(dlg).toBeVisible({ timeout: 20_000 });
  await expect(dlg).toContainText(isFr() ? '+600 m obtenus pour +900 m demandés' : '+600 m found for +900 m requested');
  // le diagnostic arrive en arrière-plan : le bouton apparaît ensuite
  await dlg.getByRole('button', { name: isFr() ? 'Inclure toutes les routes ?' : 'Include all roads?' }).click();
  expect(calls[1].searchParams.get('diagnose')).toBe('1');
  await expect.poll(() => calls.length).toBeGreaterThanOrEqual(3); // la réponse simulée manque encore la cible : un nouveau diagnostic peut suivre
  expect(calls[2].searchParams.get('roads')).toBe('all');
  expect(calls[2].searchParams.has('diagnose')).toBe(false);
  await expect(dlg).toBeVisible();
  await dlg.getByRole('button', { name: isFr() ? 'Garder cette boucle' : 'Keep this loop' }).click();
  await expect(dlg).toHaveCount(0);
});

test('aucune boucle : erreur tout de suite, puis suggestion « routes » (diagnose)', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page, async (route, url) => {
    if (url.searchParams.has('diagnose')) await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ suggest: { roads: 'all' }, checked: true }) });
    else if (url.searchParams.get('roads') === 'all') await route.fulfill({ contentType: 'application/json', body: plan1 });
    else await route.fulfill({ status: 422, contentType: 'application/json', body: JSON.stringify({ error: { code: 'no_loop_of_distance', params: {} } }) });
    return true;
  });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  const alert = page.getByRole('alert');
  await expect(alert).toContainText(isFr() ? 'Pas de boucle de cette longueur' : 'No loop');
  await alert.getByRole('button', { name: isFr() ? 'Inclure toutes les routes ?' : 'Include all roads?' }).click();
  await expect.poll(() => calls.length).toBeGreaterThanOrEqual(3); // la réponse simulée manque encore la cible : un nouveau diagnostic peut suivre
  expect(calls[2].searchParams.get('roads')).toBe('all');
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
});

test('fewer_loops seul : aucun appel diagnose (inutile, 15 s de Lambda + un jeton)', async ({ page }) => {
  const t = T();
  await setup(page);
  const body = JSON.stringify({ ...JSON.parse(plan1), warnings: [{ code: 'fewer_loops', params: { asked: 3, got: 1 } }] });
  const calls = await mockPlan(page, async (route) => { await route.fulfill({ contentType: 'application/json', body }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByRole('alertdialog')).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(1500); // le diagnostic partirait tout de suite
  expect(calls).toHaveLength(1);
  expect(calls[0].searchParams.has('diagnose')).toBe(false);
});
