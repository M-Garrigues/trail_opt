// D69 : fond de carte (Plan IGN, Plan IGN HD, Satellite) : sélecteur au clavier, mémorisé au rechargement, 2D et 3D.
// SHOTS=<dossier> : captures Plan / Satellite / HD avec une sortie affichée (contrôle visuel).
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr } from './helpers';

const visible = (id: string) => (window as unknown as { tmap: { map: { getLayoutProperty: (l: string, p: string) => string } } })
  .tmap.map.getLayoutProperty(id, 'visibility');

test('fond de carte : choix, clavier, mémorisation, 3D', async ({ page }, info) => {
  test.slow();
  const t = T(), fr = isFr();
  const title = fr ? 'Fond de carte' : 'Base map';
  const sat = 'Satellite';
  await setup(page);
  await mockPlan(page);
  // tuiles WMTS refusées (ex. 2026-10-15 : style du HD renommé par l'IGN → 400)
  const refused: string[] = [];
  page.on('response', (r) => { if (r.url().includes('data.geopf.fr/wmts') && r.status() >= 400) refused.push(`${r.status()} ${r.url()}`); });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });

  const btn = page.getByRole('button', { name: title });
  await btn.click();
  await expect(btn).toHaveAttribute('aria-expanded', 'true');
  const group = page.getByRole('group', { name: title });
  await expect(group.getByRole('radio')).toHaveCount(4);
  // D75 : défaut Plan IGN + HD à 50 % + estompage, sans ombrage LiDAR
  await expect(group.getByRole('radio', { name: fr ? 'Plan IGN + HD' : 'IGN map + HD', exact: true })).toBeChecked();
  expect(await page.evaluate(visible, 'hd-mix')).toBe('visible');
  expect(await page.evaluate(visible, 'shade')).toBe('visible');
  expect(await page.evaluate(visible, 'shadow')).toBe('none');
  expect(await page.evaluate(visible, 'loops-halo')).toBe('none');
  const shot = async (name: string) => {
    if (!process.env.SHOTS) return;
    await page.waitForTimeout(2500); // tuiles
    await page.screenshot({ path: `${process.env.SHOTS}/${info.project.name}-${name}.png` });
  };
  await shot('menu');
  await btn.click();
  await shot('mix');
  await btn.click();

  await group.getByRole('radio', { name: sat }).check();
  await expect.poll(() => page.evaluate(visible, 'ortho')).toBe('visible');
  expect(await page.evaluate(visible, 'shadow')).toBe('none');
  expect(await page.evaluate(visible, 'loops-halo')).toBe('visible'); // halo sombre du tracé sur photo (T82)
  // Échap ferme le sélecteur sans fermer la sortie affichée, focus rendu au bouton
  await page.keyboard.press('Escape');
  await expect(group).toBeHidden();
  await expect(btn).toBeFocused();
  await expect(page.getByTestId('headline')).toBeVisible();
  await shot('satellite');

  // clavier : Entrée ouvre, Espace choisit le radio focalisé
  await btn.press('Enter');
  await group.getByRole('radio', { name: fr ? 'Plan IGN HD (bêta)' : 'IGN HD map (beta)' }).focus();
  await page.keyboard.press('Space');
  await expect.poll(() => page.evaluate(visible, 'hd')).toBe('visible');
  await expect.poll(() => page.evaluate(() => (window as unknown as { tmap: { map: { isSourceLoaded: (s: string) => boolean } } }).tmap.map.isSourceLoaded('hd')), { timeout: 20_000 }).toBe(true);
  expect(refused).toEqual([]);
  expect(await page.evaluate(visible, 'ortho')).toBe('none');
  await btn.click();
  await shot('hd');
  // Plan IGN seul : ombrage LiDAR HD, ni HD ni estompage
  await btn.click();
  await group.getByRole('radio', { name: fr ? 'Plan IGN' : 'IGN map', exact: true }).check();
  await expect.poll(() => page.evaluate(visible, 'shadow')).toBe('visible');
  expect(await page.evaluate(visible, 'hd-mix')).toBe('none');
  await btn.click();
  await shot('plan');

  // mémorisé au rechargement (la sortie revient sans recalcul), et la 3D garde le fond
  await btn.click();
  await group.getByRole('radio', { name: sat }).check();
  await page.reload();
  await page.locator('.map[data-ready]').waitFor();
  await expect.poll(() => page.evaluate(visible, 'ortho')).toBe('visible');
  await page.getByRole('button', { name: fr ? 'Vue 3D' : '3D view' }).click();
  await expect.poll(() => page.evaluate(() => !!(window as unknown as { tmap: { map: { getTerrain: () => unknown } } }).tmap.map.getTerrain()), { timeout: 30_000 }).toBe(true);
  expect(await page.evaluate(visible, 'ortho')).toBe('visible');
  await shot('satellite-3d');
});
