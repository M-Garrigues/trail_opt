// T21 (D27) : directions « nature » (?design=a|b|c) — sélecteur sans cartes, profil → carte, bascule 3D, axe.
import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setup, mockPlan, placeStart, T, isFr } from './helpers';

async function axe(page: Page) {
  const r = await new AxeBuilder({ page }).exclude('.map').analyze();
  return r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' | ')}`);
}
const center = (page: Page) => page.evaluate(() => (window as unknown as { tmap: { map: { getCenter(): { lng: number; lat: number } } } }).tmap.map.getCenter());

for (const [d, pick] of [['a', 'ridge'], ['b', 'list']] as const) {
  for (const scheme of ['light', 'dark'] as const) {
    test(`design ${d} (${pick}, ${scheme}) : sélecteur, profil → carte, 3D, axe`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      const t = T();
      await setup(page);
      await mockPlan(page);
      await page.goto(`/?design=${d}`);
      await placeStart(page);
      await expect(page.locator('.cards')).toHaveCount(0);
      const radios = page.getByRole('radiogroup', { name: isFr() ? 'Type d’entraînement' : 'Training type' }).getByRole('radio');
      await expect(radios).toHaveCount(3);
      await radios.first().focus();
      await page.keyboard.press('ArrowRight');
      await expect(radios.nth(1)).toHaveAttribute('aria-checked', 'true');
      await expect(page.locator('#f-dplus_m')).toBeVisible();
      // les flèches dans un champ ne changent pas de type
      await page.locator('#f-dplus_m').focus();
      await page.keyboard.press('ArrowUp');
      await expect(radios.nth(1)).toHaveAttribute('aria-checked', 'true');
      await radios.nth(1).focus();
      await page.keyboard.press('ArrowLeft');
      await expect(page.locator('#f-dplus_m')).toHaveCount(0);
      expect(await axe(page), 'axe réglages').toEqual([]);

      await page.getByRole('button', { name: t.find }).click();
      await expect(page.getByTestId('headline')).toBeVisible();
      expect(await axe(page), 'axe résultat').toEqual([]);
      // bureau : profil dans la bande du bas
      if (!isFr()) await expect(page.locator('.dock [role=slider]')).toBeVisible();
      const before = await center(page);
      const prof = page.getByRole('slider').first();
      const b = (await prof.boundingBox())!;
      await prof.click({ position: { x: 46 + (b.width - 58) * 0.9, y: b.height / 2 } });
      await expect.poll(async () => Math.abs((await center(page)).lng - before.lng) + Math.abs((await center(page)).lat - before.lat)).toBeGreaterThan(0.001);

      const v3 = page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' });
      await v3.click();
      await expect(page.getByRole('button', { name: isFr() ? 'Vue 2D' : '2D view' })).toHaveAttribute('aria-pressed', 'true');
    });
  }
}

test('?design mémorisé, ?design=v1 revient à l’actuel', async ({ page }) => {
  await setup(page);
  await page.goto('/?design=c');
  await expect(page.locator('html')).toHaveAttribute('data-design', 'c');
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-design', 'c');
  await page.goto('/?design=v1');
  await expect(page.locator('html')).toHaveAttribute('data-design', 'v1');
  await expect(page.getByRole('button', { name: /^(Vue 3D|3D view)$/ })).toHaveCount(0);
});
