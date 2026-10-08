// Parcours 3 : autres boucles, historique, langue, dernier type, partage et lien /b/<id>.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, openOptions, closeOptions, T, isFr, plan1, plan4 } from './helpers';

test('autres boucles + historique (AC9, AC15)', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toHaveText('+424 m');
  await page.getByRole('button', { name: t.other, exact: true }).click();
  await expect(page.getByRole('radio', { name: /2$/ })).toBeVisible();
  expect(calls[1].searchParams.get('n_candidates')).toBe('4');
  expect(calls[1].searchParams.get('seed')).toBe(calls[0].searchParams.get('seed'));
  await expect(page.locator('.loops .loop')).toHaveCount(4); // n°1 non dupliquée
  await page.locator('.loops .loop').nth(1).click();
  await expect(page.getByTestId('headline')).toHaveText('+349 m');

  // historique : 1 + 3 boucles reçues
  await page.getByRole('button', { name: t.history }).first().click();
  await expect(page.getByTestId('history').locator('li')).toHaveCount(4);
  await expect(page.getByRole('button', { name: t.clear })).toBeInViewport();
  await page.getByTestId('history').locator('li').first().getByRole('button').last().click();
  await expect(page.getByTestId('history').locator('li')).toHaveCount(3);
  page.once('dialog', (d) => d.accept());
  await page.getByRole('button', { name: t.clear }).click();
  await expect(page.getByTestId('history')).toHaveCount(0);
});

test('réglage 3 boucles → « Autres boucles (2) » sans appel (AC9)', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page, async (route) => {
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ ...JSON.parse(plan1), candidates: JSON.parse((await import('./helpers')).plan4).candidates.slice(0, 3) }) });
    return true;
  });
  await page.goto('/');
  await page.getByRole('button', { name: t.menu }).click();
  await expect(page.locator('a[href="https://github.com/M-Garrigues/trail_opt"]')).toHaveAttribute('target', '_blank');
  await page.locator('#m-loops').selectOption('3');
  await page.goBack();
  await expect(page.getByRole('button', { name: t.find })).toBeVisible(); // menu refermé (feuille redescendue)
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByText(isFr() ? 'Autres sorties (2)' : 'Other runs (2)')).toBeVisible();
  expect(calls.length).toBe(1);
  expect(calls[0].searchParams.get('n_candidates')).toBe('3');
});

test('première visite : Cible présélectionné (10 km, +300 m), en tête des types', async ({ page }) => {
  const t = T();
  await setup(page, { type: null });
  const calls = await mockPlan(page);
  await page.goto('/');
  const types = page.locator('[aria-labelledby="type-l"] [role=radio]');
  await expect(types.first()).toHaveAttribute('id', 'type-target');
  await expect(types.first()).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('.estimate strong')).toHaveText('~1 h 18'); // (10 km + 300 m / 100) × 6 min
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toContainText('+424 m');
  const q = calls[0].searchParams;
  expect([q.get('goal'), q.get('distance_km'), q.get('dplus_m')]).toEqual(['target', '10', '300']);
});

test('type de voie : trois choix, part de chemin affichée, avertissement sous 50 %', async ({ page }) => {
  const t = T();
  await setup(page);
  // « Chemins » : trois parts (api.md v1.8) ; « Route » : ancienne réponse sans `surface_share` (deux parts)
  const body = (f: number, parts?: number[]) => { const p = JSON.parse(plan1); p.candidates[0].trail_frac = f; if (parts) p.candidates[0].surface_share = parts; p.warnings = [{ code: 'low_surface_share', params: { pct: 38 } }]; return JSON.stringify(p); };
  const calls = await mockPlan(page, async (route, url) => {
    const road = url.searchParams.get('surface') === 'road';
    await route.fulfill({ contentType: 'application/json', body: road ? body(0.2) : body(0.38, [0.3, 0.16, 0.54]) });
    return true;
  });
  await page.goto('/');
  await placeStart(page);
  await openOptions(page);
  const seg = page.locator('#surface');
  await expect(seg.locator('label')).toHaveText(isFr() ? ['Chemins', 'Tout', 'Route'] : ['Trails', 'Any', 'Roads']);
  await expect(seg.locator('label.on')).toHaveText(isFr() ? 'Chemins' : 'Trails');
  await closeOptions(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[0].searchParams.get('surface')).toBe('trail');
  expect(calls[0].searchParams.has('roads')).toBe(false);
  const low = isFr() ? 'Seulement 38 % de chemins ici : peu de sentiers autour de ce départ.' : 'Only 38% trails here: few paths around this start.';
  const share = isFr() ? '30 % chemin · 16 % aménagé · 54 % route' : '30% trail · 16% mixed · 54% road';
  await expect(page.getByText(share).first()).toBeVisible();
  await expect(page.locator('.warnings').first()).toHaveText(low); // une seule ligne : celle de la sortie affichée
  await expect(page.getByRole('alertdialog')).toHaveCount(0); // discret : pas de popup
  // « Route » : 80 % de route, jamais d'avertissement (D53 : revêtu par construction)
  await page.goBack();
  await openOptions(page);
  await seg.locator('label').nth(2).click();
  await closeOptions(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByText(isFr() ? '20 % chemin · 80 % route' : '20% trail · 80% road').first()).toBeVisible();
  expect(calls[1].searchParams.get('surface')).toBe('road');
  await expect(page.locator('.warnings')).toHaveCount(0);
});

test('langue et dernier type gardés (AC20, AC22)', async ({ page }) => {
  await setup(page);
  await page.goto('/');
  const other = isFr() ? 'EN' : 'FR';
  await page.locator('.topbar').getByRole('radio', { name: other }).click();
  await page.getByRole('radio', { name: isFr() ? /^Target/ : /^Cible/ }).click();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('lang', other.toLowerCase());
  await expect(page.getByRole('radio', { name: isFr() ? /^Target/ : /^Cible/ })).toHaveAttribute('aria-checked', 'true');
});

test('partage : confirmation, lien copié, /b/<id> sans calcul (AC12, AC13)', async ({ page }) => {
  const t = T();
  await setup(page);
  // presse-papiers simulé (WebKit ne connaît pas les permissions clipboard-*)
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'share', { value: undefined });
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (s: string) => { (window as unknown as { copied: string }).copied = s; } } });
  });
  const calls = await mockPlan(page);
  let posted: Record<string, unknown> | null = null;
  let sha = '';
  await page.route('**/api/loops', async (route) => {
    posted = route.request().postDataJSON();
    sha = route.request().headers()['x-amz-content-sha256'] ?? '';
    await route.fulfill({ status: 201, contentType: 'application/json', body: '{"id":"ZkD5HAVbAzLl"}' });
  });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await page.getByRole('button', { name: t.share, exact: true }).click();
  await expect(page.getByText(isFr() ? /montre ton point de départ/ : /shows your start/)).toBeVisible();
  expect(posted).toBeNull(); // confirmation d'abord
  await page.getByRole('button', { name: t.create }).click();
  await expect(page.getByRole('status').filter({ hasText: isFr() ? 'Lien copié' : 'Link copied' })).toBeVisible();
  expect(Object.keys(posted!).sort()).toEqual(['candidate', 'data_version', 'effective_start', 'request', 'solver_version', 'warnings', 'zone']);
  expect(sha).toMatch(/^[0-9a-f]{64}$/);
  // api.md v1.3 (M3) : candidate recopié tel quel, signature comprise
  const sig = (posted as unknown as { candidate: { sig: string } }).candidate.sig;
  expect([plan1, plan4].map((p) => JSON.parse(p).candidates[0].sig)).toContain(sig);
  expect(await page.evaluate(() => (window as unknown as { copied: string }).copied)).toMatch(/\/b\/ZkD5HAVbAzLl$/);

  // E10 : ouverture du lien sans /api/plan
  const stored = JSON.stringify(posted);
  await page.route('**/api/loops/ZkD5HAVbAzLl', (r) => r.fulfill({ contentType: 'application/json', body: stored }));
  await page.route('**/api/loops/nope000000', (r) => r.fulfill({ status: 404, contentType: 'application/json', body: '{"error":{"code":"loop_not_found","params":{}}}' }));
  const before = calls.length;
  await page.goto('/b/ZkD5HAVbAzLl');
  await expect(page.getByTestId('headline')).toHaveText('+424 m');
  await expect(page.getByText(isFr() ? 'Sortie partagée' : 'Shared run')).toBeVisible();
  expect(calls.length).toBe(before);
  // repartage d'une boucle ouverte par lien : son URL, sans nouveau POST (GET ne rend pas sig)
  posted = null;
  await page.getByRole('button', { name: t.share, exact: true }).click();
  await page.getByRole('button', { name: t.create }).click();
  await expect(page.getByRole('status').filter({ hasText: isFr() ? 'Lien copié' : 'Link copied' })).toBeVisible();
  expect(posted).toBeNull();
  expect(await page.evaluate(() => (window as unknown as { copied: string }).copied)).toMatch(/\/b\/ZkD5HAVbAzLl$/);
  await page.goto('/b/nope000000');
  await expect(page.getByRole('alert')).toContainText(isFr() ? 'expiré' : 'expired');
});

// D62 : « Sorties plus fluides » en bas des Options : rien d'envoyé tant qu'on n'y touche pas ; forcé, envoyé et mémorisé.
test('option « Sorties plus fluides » : défaut du mode, puis forcée et mémorisée', async ({ page }) => {
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: T().find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[0].searchParams.has('smooth')).toBe(false);
  await page.goBack();
  await openOptions(page);
  const sel = page.locator('#smooth');
  await expect(sel).toHaveValue('auto');
  await expect(page.locator('#smooth-help')).toHaveText(isFr() ? 'Moins de lacets inutiles, explore plus de reliefs ; un peu plus lent.' : 'Fewer pointless zigzags, explores more terrain; a little slower.');
  await sel.selectOption('on');
  await closeOptions(page);
  if (await page.locator('.sheet').count()) await expect(page.locator('.bar .btn.ico .dot')).toBeVisible(); // pastille du bouton Options
  await page.getByRole('button', { name: T().find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[1].searchParams.get('smooth')).toBe('true');
  await page.reload();
  // la sortie affichée revient au rechargement (2026-10-08) : on la ferme pour retrouver les réglages
  await page.getByRole('button', { name: T().back }).first().click();
  await openOptions(page);
  await expect(page.locator('#smooth')).toHaveValue('on'); // mémorisé
});
