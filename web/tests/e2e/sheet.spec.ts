// Feuille du bas (mobile) : barre de progression visible pendant le calcul, menu défilé en haut, réglages à hauteur de contenu (repli : mobile.spec).
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T } from './helpers';

test('calcul : la barre de progression est visible et avance', async ({ page }, info) => {
  const t = T();
  await setup(page);
  let release!: () => void;
  const gate = new Promise<void>((ok) => (release = ok));
  await mockPlan(page, async (route) => { await gate; await route.fulfill({ contentType: 'application/json', body: (await import('./helpers')).plan1 }); return true; });
  await page.goto('/');
  await placeStart(page);
  const bar = page.getByRole('progressbar');
  await page.getByRole('button', { name: t.find }).click();
  await expect(bar).toBeInViewport();
  const v0 = Number(await bar.getAttribute('aria-valuenow'));
  await expect.poll(async () => Number(await bar.getAttribute('aria-valuenow'))).toBeGreaterThan(v0);
  if (info.project.name !== 'desktop-en') { // la feuille ne change pas de place : Annuler reste sous le pouce
    await expect(page.getByRole('button', { name: t.cancel })).toBeInViewport({ ratio: 1 });
  }
  release();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
});

test('menu : défilé tout en haut ; retour aux réglages : feuille à hauteur de contenu', async ({ page }, info) => {
  test.skip(info.project.name === 'desktop-en');
  const t = T();
  await setup(page); await mockPlan(page);
  await page.goto('/');
  await page.getByRole('button', { name: t.menu }).click();
  const content = page.locator('.sheet .content');
  await expect.poll(() => content.evaluate((e) => e.scrollTop)).toBe(0);
  await expect(page.getByRole('heading', { name: 'Menu' })).toBeInViewport();
  await expect(page.locator('.fold')).toHaveCount(0);
  await page.goBack();
  // réglages : tout est visible d'emblée (recherche, type, valeurs, barre), la carte reste visible au-dessus ; menu non repliable
  await expect(page.locator('.search-open')).toBeInViewport({ ratio: 1 });
  await expect(page.locator('.sheet .bar .btn.primary')).toBeInViewport({ ratio: 1 });
  await expect.poll(async () => (await page.locator('.sheet').boundingBox())!.height).toBeLessThan(page.viewportSize()!.height * 0.6);
});
