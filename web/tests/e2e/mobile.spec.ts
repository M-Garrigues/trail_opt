// Mobile v3 : aucun clavier du système dans le parcours courant. Type en rangée de puces (extensible), valeurs au pavé
// numérique intégré à la feuille (même hauteur, rien ne bouge), feuille repliable par chevron, recherche en plein écran.
import { test, expect, type Page } from '@playwright/test';
import { setup, mockPlan, placeStart, T } from './helpers';

test.beforeEach(({}, info) => test.skip(info.project.name === 'desktop-en'));

const sheetH = async (page: Page) => Math.round((await page.locator('.sheet').boundingBox())!.height);
const key = (page: Page, k: string) => page.locator(`.pad [data-k="${k}"]`).click();
const OK = (page: Page) => page.getByRole('button', { name: 'OK', exact: true });

test('réglages : type choisi lisible et extensible, cibles ≥ 44 px, rien ne saute', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await expect(page.locator('.maplibregl-ctrl-zoom-in')).toHaveCount(0);
  // aucun champ de saisie dans la feuille : le clavier du système ne peut pas s'y ouvrir
  await expect(page.locator('.sheet input:not([type=radio]), .sheet textarea, .sheet [contenteditable]')).toHaveCount(0);
  const h0 = await sheetH(page);

  // type : groupe intitulé, puce choisie pleine (fond accent) et cochée, ses valeurs juste dessous
  await expect(page.locator('#type-l')).toBeVisible();
  const target = page.locator('#type-target'), max = page.locator('#type-max_dplus');
  await target.click();
  await expect(target).toHaveAttribute('aria-checked', 'true');
  await expect(target.locator('.check')).toBeVisible();
  await expect(max.locator('.check')).toHaveCount(0);
  const bg = (l: typeof target) => l.evaluate((e) => getComputedStyle(e).backgroundColor);
  expect(await bg(target)).not.toBe(await bg(max));
  await expect(page.locator('#f-dplus_m')).toBeVisible();
  expect(await sheetH(page)).toBe(h0); // changer de type ne déplace pas la feuille

  // toutes les commandes de la feuille font au moins 44 px
  for (const b of await page.locator('.sheet button').all()) {
    const r = (await b.boundingBox())!;
    expect(Math.min(r.width, r.height), await b.evaluate((e) => e.outerHTML.slice(0, 80))).toBeGreaterThanOrEqual(44);
  }

  // 7 types (4 puces fictives ajoutées au DOM) : même hauteur, la rangée défile, la dernière est atteignable
  const row = page.locator('[aria-labelledby="type-l"]');
  await row.evaluate((el) => { for (const n of ['Côtes', 'Fractionné', 'Sortie longue', 'Récup']) { const b = el.lastElementChild!.cloneNode(true) as HTMLElement; b.id = `fake-${n}`; b.lastChild!.textContent = ' ' + n; el.append(b); } });
  expect(await sheetH(page)).toBe(h0);
  expect(await row.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);
  await row.locator('button').last().scrollIntoViewIfNeeded();
  await expect(row.locator('button').last()).toBeInViewport({ ratio: 1 });
  await row.evaluate((el) => { el.querySelectorAll('[id^=fake-]').forEach((b) => b.remove()); });

  // point de passage : état actif visible
  const via = page.locator('.actions .btn[aria-pressed]').nth(1);
  await via.click();
  await expect(via).toHaveAttribute('aria-pressed', 'true');
  await expect(via.locator('.dot')).toBeVisible();
  await via.click();

  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[0].searchParams.get('goal')).toBe('target');
  // résultat : les actions sont entièrement visibles sans défiler
  await expect(page.getByRole('button', { name: t.download })).toBeInViewport({ ratio: 1 });
  await expect(page.getByRole('button', { name: t.details })).toBeInViewport({ ratio: 1 });
});

test('pavé numérique intégré : saisie, effacement, bornes, puce, OK / Annuler, feuille immobile', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.locator('#type-target').click();
  const km = page.locator('#f-distance_km'), dp = page.locator('#f-dplus_m'), value = page.locator('#pad-value'), pad = page.locator('.pad');
  const h0 = await sheetH(page);

  // ouverture : même hauteur de feuille, aucun champ (donc pas de clavier du système), la valeur est annoncée
  await km.click();
  await expect(pad).toBeVisible();
  expect(Math.abs((await sheetH(page)) - h0)).toBeLessThanOrEqual(1);
  await expect(page.locator('input:not([type=radio]):visible, textarea:visible')).toHaveCount(0);
  await expect(value).toHaveAttribute('aria-live', 'polite');
  await expect(value).toContainText('10');
  for (const b of await pad.locator('button').all()) {
    const r = (await b.boundingBox())!;
    expect(Math.min(r.width, r.height)).toBeGreaterThanOrEqual(44);
  }
  // le premier chiffre remplace la valeur ; décimale ; OK referme, « Trouver » aussitôt atteignable, feuille au même endroit
  await key(page, '1'); await key(page, '2'); await key(page, ','); await key(page, '5');
  await expect(value).toContainText('12,5');
  await OK(page).click();
  await expect(pad).toHaveCount(0);
  await expect(km.locator('strong')).toHaveText('12,5');
  await expect(page.getByRole('button', { name: t.find })).toBeInViewport({ ratio: 1 });
  expect(await sheetH(page)).toBe(h0);

  // hors bornes ou vide : OK désactivé, plage en rouge ; effacer ; Annuler ne change rien
  await km.click();
  await key(page, '1');
  await expect(OK(page)).toBeDisabled();
  await expect(page.locator('#pad-range')).toHaveClass(/bad/);
  await key(page, 'back');
  await expect(OK(page)).toBeDisabled();
  await key(page, '3'); await key(page, '0');
  await expect(OK(page)).toBeEnabled();
  await page.getByRole('button', { name: t.cancel }).click();
  await expect(pad).toHaveCount(0);
  await expect(km.locator('strong')).toHaveText('12,5');

  // une puce règle et referme ; bouton Retour du téléphone = annuler ; clavier physique
  await km.click();
  await pad.getByRole('button', { name: '21 km', exact: true }).click();
  await expect(km.locator('strong')).toHaveText('21');
  await km.click();
  await key(page, '5');
  await page.goBack();
  await expect(pad).toHaveCount(0);
  await expect(km.locator('strong')).toHaveText('21');
  await km.click();
  await page.keyboard.type('15');
  await page.keyboard.press('Enter');
  await expect(km.locator('strong')).toHaveText('15');

  // D+ : pas de virgule, touche 00 ; − / + en ajustement fin
  await dp.click();
  await expect(pad.locator('[data-k=","]')).toHaveCount(0);
  await key(page, '4'); await key(page, '00');
  await page.getByRole('button', { name: 'Augmenter : D+' }).click();
  await expect(value).toContainText('450');
  await OK(page).click();
  await expect(dp.locator('strong')).toHaveText('450');

  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  expect(calls[0].searchParams.get('distance_km')).toBe('15');
  expect(calls[0].searchParams.get('dplus_m')).toBe('450');
});

test('feuille repliable par le chevron (réglages et résultat) ; recherche en plein écran', async ({ page }) => {
  const t = T();
  await setup(page); await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  const fold = page.locator('.fold'), find = page.getByRole('button', { name: t.find });
  const h0 = await sheetH(page);
  // dépliée à l'arrivée ; chevron ≥ 44 px
  await expect(fold).toHaveAttribute('aria-expanded', 'true');
  const fb = (await fold.boundingBox())!;
  expect(Math.min(fb.width, fb.height)).toBeGreaterThanOrEqual(44);
  // repliée : une ligne de résumé, plus rien d'autre
  await fold.click();
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expect.poll(() => sheetH(page)).toBeLessThanOrEqual(72);
  await expect(fold).toContainText('Max D+ · 10 km');
  for (const l of [page.locator('.search-open'), page.locator('#f-distance_km'), find]) await expect(l).toBeHidden();
  // redépliée : tout revient, à la même hauteur
  await fold.click();
  await expect(find).toBeInViewport({ ratio: 1 });
  await expect.poll(() => sheetH(page)).toBe(h0);

  // recherche : plein écran, champ et Retour en haut, donc au-dessus du clavier (hauteur visible réduite à 55 %)
  await page.locator('.search-open').click();
  const box = page.getByRole('combobox');
  await expect(box).toBeFocused();
  const vp = page.viewportSize()!, h = Math.round(vp.height * 0.55);
  await page.setViewportSize({ width: vp.width, height: h });
  for (const s of ['dialog[open] input', 'dialog[open] .icon-btn']) {
    const b = (await page.locator(s).boundingBox())!;
    expect(b.y, s).toBeGreaterThanOrEqual(0);
    expect(b.y + b.height, s).toBeLessThanOrEqual(h);
  }
  await page.setViewportSize(vp);
  await box.fill('48.75, 2.28');
  await page.getByRole('option').first().click();
  await expect(box).toHaveCount(0);
  await expect(page.locator('.hint')).toContainText('48,75');

  // résultat : même chevron
  await find.click();
  const head = page.getByTestId('headline');
  await expect(head).toBeVisible();
  await fold.click();
  await expect.poll(() => sheetH(page)).toBeLessThanOrEqual(72);
  await expect(head).toBeHidden();
  await expect(fold).toContainText('+424 m');
  await fold.click();
  await expect(head).toBeVisible();
  await expect(page.getByRole('button', { name: t.download })).toBeInViewport({ ratio: 1 });
});
