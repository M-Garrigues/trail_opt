// Parcours 2 : annulation (bouton, Échap, Retour) et erreurs traduites, hors ligne.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr, plan1 } from './helpers';

test('annuler pendant le calcul (AC8)', async ({ page }) => {
  const t = T();
  await setup(page);
  let release: () => void = () => {};
  await mockPlan(page, async (route) => {
    await new Promise<void>((r) => (release = r));
    await route.fulfill({ contentType: 'application/json', body: plan1 }).catch(() => {});
    return true;
  });
  await page.goto('/');
  await placeStart(page);
  for (const how of ['button', 'escape', 'back'] as const) {
    await page.getByRole('button', { name: t.find }).click();
    await expect(page.getByRole('progressbar')).toBeVisible();
    if (how === 'button') await page.getByRole('button', { name: t.cancel }).click();
    if (how === 'escape') await page.keyboard.press('Escape');
    if (how === 'back') await page.goBack();
    await expect(page.getByRole('progressbar')).toHaveCount(0);
    await expect(page.getByRole('button', { name: t.find })).toBeEnabled();
    await expect(page.getByRole('alert')).toHaveCount(0);
    release();
  }
  await expect(page.getByTestId('headline')).toHaveCount(0);
});

test('codes d’erreur traduits (AC10)', async ({ page }) => {
  const t = T();
  await setup(page);
  const replies = [
    { status: 429, body: '' },
    { status: 422, body: JSON.stringify({ error: { code: 'outside_coverage', params: {} } }) },
    { status: 500, body: JSON.stringify({ error: { code: 'brand_new_code', params: {} } }) },
    { status: 400, body: JSON.stringify({ error: { code: 'distance_out_of_range', params: { min_km: 2, max_km: 100 } } }) },
  ];
  await mockPlan(page, async (route) => {
    const r = replies.shift()!;
    await route.fulfill({ status: r.status, contentType: 'application/json', body: r.body });
    return true;
  });
  await page.goto('/');
  await placeStart(page);
  const expected = isFr()
    ? ['Service en pause', 'Pas encore de données ici', 'Erreur interne (brand_new_code)', 'Distance entre 2 et 100 km.']
    : ['Service paused', 'No data here yet', 'Internal error (brand_new_code)', 'Distance between 2 and 100 km.'];
  for (const text of expected) {
    await page.getByRole('button', { name: t.find }).click();
    await expect(page.getByRole('alert')).toContainText(text);
    await page.getByRole('alert').getByRole('button').last().click();
  }
});

test('hors ligne (AC17)', async ({ page, context }) => {
  const t = T();
  await setup(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await context.setOffline(true);
  await expect(page.getByRole('status').filter({ hasText: isFr() ? 'Hors ligne' : 'Offline' })).toBeVisible();
  await expect(page.getByRole('button', { name: t.find })).toBeDisabled();
  await context.setOffline(false);
  await expect(page.getByRole('button', { name: t.find })).toBeEnabled();
});
