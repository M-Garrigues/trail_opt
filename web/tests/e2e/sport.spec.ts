// W1 (D70, D74) : sélecteur de sport Trail / Vélo, accent vert, réglages par sport, lien partagé vélo, en-tête à 360 px, axe.
import { test, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setup, mockPlan, placeStart, plan1, T, isFr, openOptions, closeOptions } from './helpers';

async function axe(page: Page) {
  const r = await new AxeBuilder({ page }).exclude('.map').analyze();
  return r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' | ')}`);
}
const group = (page: Page) => page.getByRole('radiogroup', { name: 'Sport' });
const trailBtn = (page: Page) => group(page).getByRole('radio', { name: 'Trail', exact: true });
const bikeBtn = (page: Page) => group(page).getByRole('radio', { name: isFr() ? 'Vélo' : 'Bike', exact: true });
const token = (page: Page, name: string) => page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim(), name);
const surfaces = (page: Page) => page.locator('#surface label');
// EN : « Find my run » (trail) / « Find my ride » (vélo)
const find = () => (isFr() ? 'Trouver ma sortie' : /Find my (run|ride)/);
const GREEN = 'rgb(1, 125, 56)';

test('bascule de sport : accent vert, voies et réglages du vélo, sport mémorisé', async ({ page }) => {
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'trail');
  await expect(trailBtn(page)).toBeChecked();
  expect(await token(page, '--accent')).toBe('#a8441c');
  await expect(page.locator('.start-marker')).toHaveCSS('background-color', 'rgb(168, 68, 28)');

  await bikeBtn(page).click();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
  expect([await token(page, '--accent'), await token(page, '--accent-text'), await token(page, '--accent-soft'), await token(page, '--map-accent')])
    .toEqual(['#017d38', '#03692e', '#d7ebda', '#017d38']);
  await expect(page.locator('.start-marker')).toHaveCSS('background-color', GREEN); // le départ suit le sport
  await expect(surfaces(page)).toHaveText(isFr() ? ['VTT', 'Gravel', 'Route'] : ['MTB', 'Gravel', 'Roads']);
  await expect(page.locator('#surface label.on')).toHaveText(isFr() ? 'VTT' : 'MTB');
  await expect(page.getByRole('button', { name: find() })).toBeVisible();

  // VTT : 30 km, pente de la montée par défaut (25 %) non envoyée, sport=bike
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  let q = calls[0].searchParams;
  expect([q.get('sport'), q.get('surface'), q.get('distance_km'), q.has('max_grade_pct')]).toEqual(['bike', 'mtb', '30', false]);
  await page.goBack();
  // Gravel : 50 km ; le défaut suit la voie tant qu'on n'a rien choisi
  await surfaces(page).nth(1).click();
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  q = calls[1].searchParams;
  expect([q.get('surface'), q.get('distance_km')]).toEqual(['gravel', '50']);

  // le sport est mémorisé : rechargement = vélo (la sortie revient, session d'onglet)
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
  expect(await page.evaluate(() => localStorage.getItem('optrail.sport'))).toBe('"bike"');
  expect(await page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith('optrail.settings')).sort())).toEqual(['optrail.settings', 'optrail.settings.bike']);

  // retour au trail : ses réglages (clé d'origine) et plus de sport dans la requête
  await trailBtn(page).click();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'trail');
  await expect(surfaces(page)).toHaveText(isFr() ? ['Chemins', 'Mixte', 'Route'] : ['Trails', 'Mixed', 'Roads']);
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  q = calls[2].searchParams;
  expect([q.has('sport'), q.get('surface'), q.get('distance_km')]).toEqual([false, 'trail', '10']);
});

test('changer de sport ferme la sortie sans recalcul, le départ est gardé', async ({ page }) => {
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  await bikeBtn(page).click();
  await expect(page.getByTestId('headline')).toHaveCount(0);
  await expect(page.getByRole('button', { name: find() })).toBeEnabled(); // départ gardé
  await expect(page.locator('.start-marker')).toBeVisible();
  expect(calls.length).toBe(1); // aucun recalcul
});

test('pente max vélo : « sans limite » par défaut en Route seulement (D78)', async ({ page }) => {
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/?sport=bike');
  await placeStart(page);
  const none = isFr() ? 'sans limite' : 'no limit';
  await openOptions(page);
  await expect(page.locator('#max-grade option', { hasText: none })).toHaveCount(0); // VTT : 25 %, pas de « sans limite »
  await closeOptions(page);
  await surfaces(page).nth(2).click();
  await openOptions(page);
  await expect(page.locator('#max-grade option:checked')).toHaveText(none);
  await closeOptions(page);
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[0].searchParams.get('surface')).toBe('road');
  expect(calls[0].searchParams.has('max_grade_pct')).toBe(false); // défaut du profil : rien d'envoyé
});

test('bascule désactivée pendant un calcul', async ({ page }) => {
  await setup(page);
  await mockPlan(page, async () => { await new Promise((r) => setTimeout(r, 1500)); });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: find() }).click();
  await expect(bikeBtn(page)).toBeDisabled();
  await expect(trailBtn(page)).toBeDisabled();
  await expect(page.getByTestId('headline')).toBeVisible();
  await expect(bikeBtn(page)).toBeEnabled();
});

test('?sport= : lu, mémorisé puis retiré de l’URL', async ({ page }) => {
  await setup(page);
  await page.goto('/?sport=bike&lang=fr');
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
  expect(new URL(page.url()).search).toBe('?lang=fr'); // sport retiré, le reste gardé
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
  await trailBtn(page).click();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'trail'); // une bascule n'est pas défaite par le rechargement
  await page.goto('/?sport=ski'); // valeur inconnue ignorée
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'trail');
});

test('lien partagé vélo : sport, accent et réglages de la sortie', async ({ page }) => {
  await setup(page);
  const calls = await mockPlan(page);
  const p = JSON.parse(plan1);
  const body = JSON.stringify({
    request: { lat: '48.73', lon: '2.27', sport: 'bike', goal: 'target', distance_km: '80', dplus_m: '900', surface: 'gravel', max_grade_pct: '25', climbs: 'balanced', n_candidates: '1', seed: '0' },
    data_version: p.data_version, solver_version: p.solver_version, effective_start: p.effective_start, zone: null, candidate: p.candidates[0], warnings: [],
  });
  await page.route('**/api/loops/BikeLink0001', (r) => r.fulfill({ contentType: 'application/json', body }));
  await page.goto('/b/BikeLink0001');
  await expect(page.getByTestId('headline')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
  expect(await token(page, '--accent')).toBe('#017d38');
  await expect(page.getByText(isFr() ? 'Sortie partagée' : 'Shared ride')).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('optrail.sport'))).toBe('"bike"');
  expect(calls.length).toBe(0);
  // « Recalculer depuis ici » : réglages vélo préremplis
  await (isFr() ? page.getByRole('button', { name: 'Retour' }).first() : page.getByRole('button', { name: 'Back' }).first()).click();
  await expect(page.locator('#surface label.on')).toHaveText('Gravel');
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  const q = calls[0].searchParams;
  expect([q.get('sport'), q.get('surface'), q.get('distance_km'), q.get('dplus_m'), q.get('max_grade_pct')]).toEqual(['bike', 'gravel', '80', '900', '25']);
});

test('historique : pictogramme du sport par ligne, ouvrir une entrée bascule le sport', async ({ page }) => {
  const t = T();
  await setup(page);
  await mockPlan(page);
  await page.goto('/?sport=bike');
  await placeStart(page);
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  await trailBtn(page).click();
  await page.getByRole('button', { name: find() }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  await bikeBtn(page).click(); // ferme la sortie trail
  await page.getByRole('button', { name: t.history }).first().click();
  const rows = page.getByTestId('history').locator('li');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).locator('svg.sp')).toHaveAttribute('aria-label', 'Trail'); // la plus récente : trail
  await expect(rows.nth(1).locator('svg.sp')).toHaveAttribute('aria-label', isFr() ? 'Vélo' : 'Bike');
  await rows.nth(0).locator('.open').click();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'trail');
  await expect(page.getByTestId('headline')).toBeVisible();
});

test('en-tête : sélecteur à droite du logo, sans débordement à 360 px', async ({ page }, info) => {
  test.skip(info.project.name === 'desktop-en');
  await setup(page);
  await page.setViewportSize({ width: 360, height: 740 });
  await page.goto('/');
  await expect(group(page)).toBeVisible();
  await expect(page.locator('.topbar .lang')).toHaveCount(0); // FR·EN quitte l'en-tête mobile
  const box = async (s: string) => (await page.locator(s).first().boundingBox())!;
  const [brand, sports, hist, menu] = [await box('.topbar .brand'), await box('.topbar .sports'), await box('.topbar .icon-btn:nth-of-type(1)'), await box('.topbar .icon-btn:last-child')];
  expect(brand.x + brand.width).toBeLessThanOrEqual(sports.x + 0.5);
  expect(sports.x + sports.width).toBeLessThanOrEqual(hist.x + 0.5);
  expect(menu.x + menu.width).toBeLessThanOrEqual(360);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360);
  for (const r of await group(page).getByRole('radio').all()) expect((await r.boundingBox())!.width).toBeGreaterThanOrEqual(44);
  // pictogramme seul sur mobile
  expect((await trailBtn(page).innerText()).trim()).toBe('');
});

test('bureau : pictogramme et nom, FR·EN garde sa place', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-en');
  await setup(page);
  await page.goto('/');
  await expect(trailBtn(page)).toContainText('Trail');
  await expect(bikeBtn(page)).toContainText('Bike');
  await expect(page.locator('.topbar .lang')).toBeVisible();
  // flèches ←/→ dans le radiogroup
  await trailBtn(page).focus();
  await page.keyboard.press('ArrowRight');
  await expect(bikeBtn(page)).toBeChecked();
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
});

test('accueil : deux boutons Trail / Vélo, choisir son sport = entrer', async ({ page }) => {
  await setup(page, { intro: true });
  await page.goto('/');
  const go = page.locator('.go .btn');
  await expect(go).toHaveText(isFr() ? ['Trail', 'Vélo'] : ['Trail', 'Bike']);
  await expect(go.nth(1)).toHaveCSS('background-color', GREEN); // chaque bouton porte l'accent de son sport
  await go.nth(1).click();
  await expect(page.getByTestId('safety')).toHaveCount(0);
  await expect(page.locator('html')).toHaveAttribute('data-sport', 'bike');
});

for (const scheme of ['light', 'dark'] as const) {
  for (const sport of ['trail', 'bike'] as const) {
    test(`axe : accueil, réglages et sortie, ${sport}, thème ${scheme}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await setup(page, { intro: true });
      const calls = await mockPlan(page);
      await page.goto(`/?sport=${sport}`);
      expect(await axe(page), 'axe E1').toEqual([]);
      await page.locator('.go .btn').nth(sport === 'bike' ? 1 : 0).click();
      await placeStart(page);
      expect(await axe(page), 'axe E3').toEqual([]);
      await page.getByRole('button', { name: find() }).click();
      await expect(page.getByTestId('headline')).toBeVisible();
      expect(calls.length).toBe(1);
      expect(await axe(page), 'axe E6').toEqual([]);
    });
  }
}
