// Page /admin (contracts/admin.md § 5, v2 D77) : GET /api/admin/stats mocké par fixture.
import { readFileSync } from 'node:fs';
import { test, expect, type Page } from '@playwright/test';

const stats = readFileSync(new URL('../fixtures/admin-stats.json', import.meta.url), 'utf8');
// réponse réelle de /api/admin/usage (sources AWS en lecture, 2026-10-08)
const usage = readFileSync(new URL('../fixtures/admin-usage.json', import.meta.url), 'utf8');

const desktopOnly = (info: { project: { name: string } }) => test.skip(info.project.name !== 'desktop-en', 'page pensée pour ordinateur');

/** Mocke stats (renvoie `filter` comme le serveur, d'après la query) et usage ; compte les appels. */
async function mock(page: Page) {
  const calls = { stats: [] as string[], usage: 0, keys: [] as (string | undefined)[], hits: 0 };
  await page.route('**/api/admin/stats**', async (route) => {
    const k = route.request().headers()['x-admin-key'];
    calls.keys.push(k);
    if (k !== 'bonne') return route.fulfill({ status: 401, contentType: 'application/json', body: '{"error":{"code":"admin_denied","params":{}}}' });
    const u = new URL(route.request().url());
    calls.stats.push(u.search);
    const filter = Object.fromEntries(['sport', 'surface', 'goal'].filter((f) => u.searchParams.has(f)).map((f) => [f, u.searchParams.get(f)]));
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...JSON.parse(stats), filter }) });
  });
  await page.route('**/api/admin/usage', (route) => { calls.usage++; return route.fulfill({ contentType: 'application/json', body: usage }); });
  await page.route('**/api/hit', (route) => { calls.hits++; return route.fulfill({ status: 204 }); });
  return calls;
}

async function login(page: Page) {
  await page.goto('/admin');
  await page.getByLabel('Clé d’administration').fill('bonne');
  await page.getByRole('button', { name: 'Entrer' }).click();
}

test('clé, refus, menu Visu, noindex', async ({ page }, info) => {
  desktopOnly(info);
  const calls = await mock(page);
  await page.goto('/admin');
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex');
  // mauvaise clé : refusée, champ de nouveau proposé
  await page.getByLabel('Clé d’administration').fill('mauvaise');
  await page.getByRole('button', { name: 'Entrer' }).click();
  await expect(page.getByRole('alert')).toHaveText('Clé refusée.');
  await page.getByLabel('Clé d’administration').fill('bonne');
  await page.getByRole('button', { name: 'Entrer' }).click();
  // Visu par défaut
  await expect(page.getByRole('link', { name: 'Visu' })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('link', { name: 'Surveillance' })).not.toHaveAttribute('aria-current', 'page');
  const tiles = page.getByRole('region', { name: 'Chiffres clés' });
  await expect(tiles).toHaveCount(1);
  await expect(tiles).toContainText('15'); // visiteurs
  await expect(tiles).toContainText('Sorties demandées');
  await expect(tiles).toContainText('20'); // somme calcs_by_day.n
  // liens : comptes du serveur (3 créés, 5 ouvertures) même sans événement mesuré ; événements en petit
  await expect(tiles).toContainText('Liens partagés3créés · 5 ouvertures (serveur)');
  await expect(tiles).toContainText('1 ouverts');
  await expect(tiles).not.toContainText('Taux d’échec'); // = Surveillance
  // graphiques canvas (visiteurs, sorties, actions, distance, D+), parts, carte
  await expect(page.locator('canvas:not(.maplibregl-canvas)')).toHaveCount(5);
  const sport = page.locator('.card', { has: page.getByRole('heading', { name: 'Sport', exact: true }) });
  await expect(sport.getByRole('row', { name: /Vélo/ })).toHaveAttribute('data-sport', 'bike');
  const voie = page.locator('.card', { has: page.getByRole('heading', { name: 'Sport · voie' }) });
  await expect(voie.getByRole('cell', { name: 'Trail · Route' })).toBeVisible();
  await expect(voie.getByRole('cell', { name: 'Vélo · Route' })).toBeVisible();
  await expect(voie.getByRole('cell', { name: 'Trail · —' })).toBeVisible(); // voie absente (journal v1)
  await expect(page.getByRole('cell', { name: 'Max D+' }).first()).toBeVisible();
  // taux d'action : un seul tableau, dimension au choix
  const dim = page.getByLabel('Dimension');
  await expect(page.locator('.card', { hasText: 'Taux d’action' }).locator('table')).toHaveCount(1);
  const rates = page.locator('.card', { hasText: 'Taux d’action' });
  await dim.selectOption('rank');
  await expect(rates.getByRole('row', { name: /n° 1/ })).toContainText('33,3 %');
  await dim.selectOption('km');
  await expect(rates.getByRole('cell', { name: '10–15 km' })).toBeVisible();
  await dim.selectOption('surface');
  await expect(rates.getByRole('row', { name: /Vélo · Route/ })).toContainText('100,0 %'); // gpx_rate 1
  const combos = page.locator('.card', { hasText: 'Combinaisons qui amènent' }).getByRole('row');
  await expect(combos).toHaveCount(4);
  await expect(combos.nth(1)).toContainText('Trail · Chemins');
  await expect(combos.nth(2)).toContainText('Vélo · Route');
  // audience : régions avec pays (plus de tableau Pays), appareils, sites ; dite tous sports
  await expect(page.getByText('Audience : tous sports')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Pays', exact: true })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Régions' })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'google.com' })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'Mobile' })).toBeVisible();
  await expect(page.locator('.maplibregl-canvas')).toBeVisible();
  // départs de la fixture (2 cellules + 1 hors couverture) posés sur la carte
  await expect(page.getByRole('img', { name: 'Carte de densité des départs' })).toHaveAttribute('data-points', '3');
  // rien de Surveillance, aucun appel AWS tant que le menu n'est pas ouvert
  await expect(page.getByRole('region', { name: 'Consommation AWS' })).toHaveCount(0);
  expect(calls.usage).toBe(0);
  // clé gardée pour la session : rechargement sans nouvelle saisie ; aucune mesure d'audience sur /admin
  await page.reload();
  await expect(page.getByRole('region', { name: 'Chiffres clés' })).toBeVisible();
  expect(calls.keys.slice(-1)).toEqual(['bonne']);
  expect(calls.hits).toBe(0);
});

test('menu Surveillance : par clic, par #surveillance', async ({ page }, info) => {
  desktopOnly(info);
  const calls = await mock(page);
  await login(page);
  await expect(page.getByRole('region', { name: 'Chiffres clés' })).toContainText('Visiteurs');
  await page.getByRole('link', { name: 'Surveillance' }).click();
  await expect(page).toHaveURL(/#surveillance$/);
  await expect(page.getByRole('link', { name: 'Surveillance' })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('link', { name: 'Visu' })).not.toHaveAttribute('aria-current', 'page');
  const tiles = page.getByRole('region', { name: 'Chiffres clés' });
  await expect(tiles).toHaveCount(1);
  await expect(tiles).toContainText('10,0 %'); // taux d'échec
  await expect(tiles).toContainText('5,0 % des calculs'); // timeouts : 1 sur 20 (vue modes)
  await expect(tiles).toContainText('4,1 / 9,6 s');
  await expect(tiles).toContainText('Rejetés avant calcul');
  await expect(tiles).not.toContainText('Visiteurs');
  await expect(page.locator('canvas:not(.maplibregl-canvas)')).toHaveCount(2); // calculs/jour, p50-p95/jour
  await expect(page.getByRole('cell', { name: 'timeout', exact: true })).toBeVisible();
  await expect(page.getByRole('cell', { name: 'bot_check_failed' })).toBeVisible();
  // par mode
  const modes = page.locator('.card', { has: page.getByRole('heading', { name: 'Par mode' }) });
  await expect(modes.getByRole('row')).toHaveCount(4);
  await expect(modes.getByRole('row', { name: /Vélo · Route/ })).toContainText('0,00024 $');
  await expect(modes.getByRole('row', { name: /Vélo · Route/ })).toContainText('14,2');
  await expect(modes.getByRole('row', { name: /Trail · Mixte/ })).toContainText('Le plus court');
  await expect(modes.getByRole('row', { name: /Trail · Mixte/ }).getByRole('cell', { name: '—' })).toHaveCount(2); // p50 et p95 sans calcul réussi
  // consommation AWS, chargée à l'ouverture : postes, seuil gratuit, projection, dépense (Budgets), crédits, reste non ventilé
  const aws = page.getByRole('region', { name: 'Consommation AWS' });
  await expect(aws.getByRole('row', { name: /Calcul facturé/ })).toContainText('400 000 Go·s');
  await expect(aws).toContainText('0,021 $');
  await expect(aws).toContainText('0,088 $');
  await expect(aws).toContainText('139,98 $');
  await expect(aws.getByRole('row', { name: /Non ventilé/ })).toContainText('0,020 $');
  await expect(aws.getByRole('row', { name: /Stockage \(dalles/ })).toContainText('aucun');
  await expect(tiles).toContainText('0,021 $'); // dépense du mois, une fois chargée
  const method = aws.locator('details');
  await expect(method).not.toHaveAttribute('open', '');
  await method.getByText('Sources et méthode').click();
  await expect(method).toContainText('Cost Explorer');
  expect(calls.usage).toBe(1);
  // retour à Visu puis nouvelle ouverture : pas de second chargement
  await page.getByRole('link', { name: 'Visu' }).click();
  await expect(page.getByRole('region', { name: 'Chiffres clés' })).toContainText('Visiteurs');
  await page.getByRole('link', { name: 'Surveillance' }).click();
  await expect(aws).toBeVisible();
  expect(calls.usage).toBe(1);
  // ouverture directe par le hash
  await page.goto('/admin#surveillance');
  await page.reload();
  await expect(page.getByRole('link', { name: 'Surveillance' })).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('region', { name: 'Consommation AWS' })).toContainText('139,98 $');
});

test('filtres sport / voie et type de sortie passés à l’API', async ({ page }, info) => {
  desktopOnly(info);
  const calls = await mock(page);
  await login(page);
  await expect(page.getByRole('link', { name: 'Visu' })).toHaveAttribute('aria-current', 'page');
  expect(calls.stats[0]).not.toMatch(/sport|surface|goal/); // aucun filtre au départ
  const req = page.waitForRequest((r) => r.url().includes('sport=bike&surface=road'));
  await page.locator('header').getByLabel('Sport et voie').selectOption({ label: 'Vélo · Route' });
  const url = new URL((await req).url());
  expect([url.searchParams.get('sport'), url.searchParams.get('surface'), url.searchParams.has('goal')]).toEqual(['bike', 'road', false]);
  await expect(page.getByText('Filtre : Vélo · Route')).toBeVisible();
  const req2 = page.waitForRequest((r) => r.url().includes('goal=target'));
  await page.locator('header').getByLabel('Type de sortie').selectOption({ label: 'Cible' });
  expect(new URL((await req2).url()).searchParams.get('sport')).toBe('bike');
  await expect(page.getByText('Filtre : Vélo · Route, Cible')).toBeVisible();
  // retour à « Tous » : plus de paramètre
  const req3 = page.waitForRequest((r) => r.url().includes('/api/admin/stats') && !r.url().includes('sport='));
  await page.locator('header').getByLabel('Sport et voie').selectOption({ label: 'Tous' });
  expect(new URL((await req3).url()).searchParams.get('goal')).toBe('target');
});

test('trop d’essais, identifiants AWS expirés', async ({ page }, info) => {
  desktopOnly(info);
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

test('mobile : onglets visibles, pas de défilement horizontal', async ({ page }, info) => {
  test.skip(info.project.name === 'desktop-en', 'projets mobiles');
  await mock(page);
  await login(page);
  await expect(page.getByRole('link', { name: 'Visu' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Surveillance' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Chiffres clés' })).toContainText('Visiteurs');
  const wide = () => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);
  expect(await wide()).toBe(true);
  await page.getByRole('link', { name: 'Surveillance' }).click();
  await expect(page.getByRole('region', { name: 'Consommation AWS' })).toContainText('139,98 $');
  await expect(page.getByRole('cell', { name: 'Vélo · Route' })).toBeVisible();
  expect(await wide()).toBe(true);
});
