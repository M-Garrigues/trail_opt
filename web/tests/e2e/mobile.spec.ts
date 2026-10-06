// Retours mobile : pas de boutons zoom, barre en pictogrammes (états actifs), champ numérique visible clavier ouvert (viewport réduit).
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, openSheet, T } from './helpers';

test('mobile : barre en pictogrammes, sans zoom, champ au-dessus du clavier', async ({ page }, info) => {
  test.skip(info.project.name === 'desktop-en');
  const t = T();
  await setup(page); await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await expect(page.locator('.maplibregl-ctrl-zoom-in')).toHaveCount(0);
  const find = page.getByRole('button', { name: t.find });
  for (const b of [find, page.locator('.actions .btn').first()]) {
    const r = (await b.boundingBox())!;
    expect(r.width).toBeGreaterThanOrEqual(44); expect(r.height).toBeGreaterThanOrEqual(44);
  }
  await page.screenshot({ path: `../.team/handoffs/img/t36-mobile-bar-${info.project.name}.png` });
  const via = page.locator('.actions .btn[aria-pressed]').nth(1);
  await via.click();
  await expect(via).toHaveAttribute('aria-pressed', 'true');
  await expect(via.locator('.dot')).toBeVisible();
  await via.click();
  // clavier simulé : feuille dépliée (le champ n'est pas atteignable replié), viewport réduit puis focus du champ
  await openSheet(page);
  const vp = page.viewportSize()!;
  await page.setViewportSize({ width: vp.width, height: Math.round(vp.height * 0.55) });
  await page.locator('#f-distance_km').focus();
  await page.waitForTimeout(700);
  const f = (await page.locator('#f-distance_km').boundingBox())!;
  expect(f.y).toBeGreaterThanOrEqual(0);
  expect(f.y + f.height).toBeLessThanOrEqual(Math.round(vp.height * 0.55));
  await page.screenshot({ path: `../.team/handoffs/img/t36-mobile-kb-${info.project.name}.png` });
});
