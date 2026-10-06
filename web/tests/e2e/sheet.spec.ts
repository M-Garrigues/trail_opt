// Feuille du bas (mobile) : barre de progression visible pendant le calcul (repliée ou dépliée), menu défilé en haut, repli complet.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T } from './helpers';

test('calcul : la barre de progression est visible et avance, feuille repliée ou dépliée', async ({ page }, info) => {
  const t = T();
  await setup(page);
  let release!: () => void;
  const gate = new Promise<void>((ok) => (release = ok));
  await mockPlan(page, async (route) => { await gate; await route.fulfill({ contentType: 'application/json', body: (await import('./helpers')).plan1 }); return true; });
  await page.goto('/');
  await placeStart(page);
  const bar = page.getByRole('progressbar');
  await page.getByRole('button', { name: t.find }).click(); // mobile : feuille repliée au départ
  await expect(bar).toBeInViewport();
  const v0 = Number(await bar.getAttribute('aria-valuenow'));
  await expect.poll(async () => Number(await bar.getAttribute('aria-valuenow'))).toBeGreaterThan(v0);
  if (info.project.name !== 'desktop-en') { // puis dépliée
    await page.locator('.handle').click();
    await expect(page.locator('.sheet')).toHaveAttribute('data-snap', '2');
    await expect(bar).toBeInViewport();
  }
  release();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
});

test('menu : défilé tout en haut au dépliage ; feuille réellement repliée', async ({ page }, info) => {
  test.skip(info.project.name === 'desktop-en');
  const t = T();
  await setup(page); await mockPlan(page);
  await page.goto('/');
  await page.getByRole('button', { name: t.menu }).click();
  const content = page.locator('.sheet .content');
  await expect.poll(() => content.evaluate((e) => e.scrollTop)).toBe(0);
  await expect(page.getByRole('heading', { name: 'Menu' })).toBeInViewport();
  // replié : poignée + barre minimale, le champ de recherche ne dépasse pas
  await page.goBack(); // retour aux réglages : feuille repliée
  await expect(page.locator('.sheet')).toHaveAttribute('data-snap', '0');
  await expect.poll(async () => (await page.locator('.sheet').boundingBox())!.height).toBeLessThan(200);
  await expect(page.locator('.start-row input').first()).not.toBeInViewport();
  await expect(page.locator('.sheet .bar')).toBeInViewport({ ratio: 1 });
  // déplié puis replié à la main : toujours rien du champ
  await page.locator('.handle').click();
  await expect(page.locator('.start-row input').first()).toBeInViewport();
  await page.locator('.handle').click();
  await expect.poll(async () => (await page.locator('.sheet').boundingBox())!.height).toBeLessThan(200);
  await expect(page.locator('.start-row input').first()).not.toBeInViewport();
});
