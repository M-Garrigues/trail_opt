// Parcours 1 : première visite → départ → réglages → calcul → résultat, profil, détail, GPX.
import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setup, mockPlan, placeStart, zoneClick, openOptions, closeOptions, T, isFr } from './helpers';

async function axe(page: import('@playwright/test').Page) {
  const r = await new AxeBuilder({ page }).exclude('.map').analyze();
  return r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(' | ')}`);
}

test('parcours principal', async ({ page }) => {
  const t = T();
  await setup(page, { intro: true });
  const calls = await mockPlan(page);
  await page.goto('/');

  // AC1 intro + sécurité, puis absentes au rechargement
  await expect(page.getByTestId('safety')).toBeVisible();
  expect(await axe(page), 'axe E1').toEqual([]);
  await page.getByRole('button', { name: t.go }).click();
  await page.reload();
  await expect(page.getByTestId('safety')).toHaveCount(0);
  await expect(page.getByRole('button', { name: t.find })).toBeDisabled();

  // AC2 aucune géolocalisation à froid ; AC3 toucher la carte active Calculer
  expect(await page.evaluate(() => (window as unknown as { geoCalls: number }).geoCalls)).toBe(0);
  await placeStart(page);
  await expect(page.getByRole('button', { name: t.find })).toBeEnabled();

  // AC4 radiogroup, flèches
  const radios = page.getByRole('radiogroup', { name: isFr() ? 'Type de sortie' : 'Run type' }).getByRole('radio');
  await expect(radios).toHaveCount(3);
  // ordre : Cible, Max D+ (mémorisé ici, voir `setup`), Le plus court
  await radios.nth(1).focus();
  await page.keyboard.press('ArrowLeft');
  await expect(radios.first()).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('#f-dplus_m')).toBeVisible();
  await page.keyboard.press('ArrowRight');
  await expect(radios.nth(1)).toHaveAttribute('aria-checked', 'true');
  await expect(page.locator('#f-dplus_m')).toHaveCount(0);

  // AC5 1 km : bureau → ramené à 2 + message ; mobile → refusé par le pavé
  const mobile = (await page.locator('.sheet').count()) > 0;
  if (mobile) { // pavé numérique intégré : hors bornes = OK désactivé et plage signalée
    const ok = page.getByRole('button', { name: 'OK', exact: true }), k = (d: string) => page.locator(`.pad [data-k="${d}"]`).click();
    await page.locator('#f-distance_km').click();
    await k('1');
    await expect(ok).toBeDisabled();
    await expect(page.locator('#pad-range')).toHaveClass(/bad/);
    expect(await axe(page), 'axe pavé').toEqual([]);
    await k('back'); await k('2');
    await ok.click();
    await expect(page.locator('#f-distance_km strong')).toHaveText('2');
    await page.locator('#f-distance_km').click();
    await k('1'); await k('0');
    await ok.click();
    await expect(page.locator('#f-distance_km strong')).toHaveText('10');
  } else {
    await page.locator('#f-distance_km').fill('1');
    await page.locator('#f-distance_km').blur();
    await expect(page.locator('#f-distance_km')).toHaveValue('2');
    await expect(page.locator('#m-distance_km')).not.toBeEmpty();
    await page.locator('#f-distance_km').fill('10');
    await page.locator('#f-distance_km').blur();
  }

  // AC6 note « courtes » et durée ~1 h 00
  if (mobile) await openOptions(page);
  await page.locator('label', { hasText: isFr() ? 'Courtes et raides' : 'Short & steep' }).click();
  await expect(page.getByText(isFr() ? /moins de D\+ au total/ : /less total climb/)).toBeVisible();
  if (mobile) { expect(await axe(page), 'axe options').toEqual([]); await closeOptions(page); }
  await expect(page.locator('.estimate strong')).toHaveText('~1 h 00');
  expect(await axe(page), 'axe E3').toEqual([]);

  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toHaveText('+424 m');
  const q = calls[0].searchParams;
  expect([...q.keys()].every((k) => ['lat', 'lon', 'goal', 'distance_km', 'dplus_m', 'max_distance_km', 'climbs', 'max_grade_pct', 'surface', 'no_repeat_junction', 'smooth', 'n_candidates', 'polygon', 'seed'].includes(k))).toBe(true);
  expect(q.get('n_candidates')).toBe('1');
  expect(q.get('climbs')).toBe('short');
  expect(await axe(page), 'axe E6').toEqual([]);

  // AC11 survol du profil à 50 % → curseur ≈ length/2
  const prof = page.getByRole('slider').first();
  const b = (await prof.boundingBox())!;
  await page.mouse.move(b.x + 46 + (b.width - 58) / 2, b.y + b.height / 2);
  await expect(prof).toHaveAttribute('aria-valuetext', /km 5[,.][12]/);
  const now = Number(await prof.getAttribute('aria-valuenow'));
  expect(Math.abs(now - 10495 / 2)).toBeLessThan(300);

  // E8 détail, AC18 Retour ferme E8 puis E6
  // bureau : détail embarqué dans le panneau, pas de bouton ni d'écran E8
  if (await page.getByRole('button', { name: t.details }).count()) {
    await page.getByRole('button', { name: t.details }).click();
    await expect(page.getByRole('heading', { level: 2 })).toBeVisible();
    expect(await axe(page), 'axe E8').toEqual([]);
    await page.goBack();
  }
  await expect(page.getByTestId('headline')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: t.find })).toBeVisible();
});

test('GPX téléchargé depuis le résultat (AC14, D27 : plus d’envoi montre)', async ({ page }, info) => {
  const t = T();
  await setup(page);
  // ordinateur : téléchargement direct, même si le navigateur sait partager (Safari sur Mac) ; mobile : branche téléchargement forcée
  if (info.project.use.isMobile) await page.addInitScript(() => { Object.defineProperty(navigator, 'canShare', { value: undefined }); });
  else await page.addInitScript(() => { Object.defineProperty(navigator, 'canShare', { value: () => true }); Object.defineProperty(navigator, 'share', { value: async () => { throw new Error('partage appelé sur ordinateur'); } }); });
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: t.download }).click();
  const d = await dl;
  expect(d.suggestedFilename()).toBe('optrail-10.5km-424m.gpx');
  const body = await (await d.createReadStream()).toArray();
  const gpx = Buffer.concat(body).toString();
  expect(gpx).toContain('<trk>');
  expect(gpx).toContain('<ele>');
  expect(gpx).toContain('© IGN');
});

test('GPX partagé quand le système sait partager un fichier (canShare, écran tactile)', async ({ page }, info) => {
  test.skip(!info.project.use.isMobile, 'partage de fichier : écran tactile seulement');
  const t = T();
  await setup(page);
  await page.addInitScript(() => {
    const w = window as unknown as { shared: { name: string; text: string } | null };
    w.shared = null;
    Object.defineProperty(navigator, 'canShare', { value: (d: { files?: File[] }) => !!d.files?.length });
    Object.defineProperty(navigator, 'share', { value: async (d: { files: File[] }) => { w.shared = { name: d.files[0].name, text: await d.files[0].text() }; } });
  });
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await page.getByRole('button', { name: isFr() ? 'Télécharger le GPX' : 'Download GPX' }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { shared: { name: string } | null }).shared?.name)).toBe('optrail-10.5km-424m.gpx');
  expect(await page.evaluate(() => (window as unknown as { shared: { text: string } }).shared.text)).toContain('<trk>');
});

test('GPX : partage refusé par le système (type, certificat local) → téléchargement', async ({ page }, info) => {
  test.skip(!info.project.use.isMobile, 'partage de fichier : écran tactile seulement');
  const t = T();
  await setup(page);
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'canShare', { value: () => true });
    Object.defineProperty(navigator, 'share', { value: async () => { throw new DOMException('refusé', 'NotAllowedError'); } });
  });
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: t.download }).click();
  const d = await dl;
  expect(d.suggestedFilename()).toBe('optrail-10.5km-424m.gpx');
  const gpx = Buffer.concat(await (await d.createReadStream()).toArray()).toString();
  expect(gpx).toMatch(/^<\?xml[\s\S]*<trkpt lat="[\d.]+" lon="[\d.]+"><ele>/);
});

test.describe('sombre', () => {
  test.use({ colorScheme: 'dark' });
  test('axe E3/E6/E12 en sombre, carte claire (AC19)', async ({ page }) => {
    const t = T();
    await setup(page);
    await mockPlan(page);
    await page.goto('/');
    await placeStart(page);
    expect(await axe(page), 'axe E3 sombre').toEqual([]);
    await page.getByRole('button', { name: t.find }).click();
    await expect(page.getByTestId('headline')).toBeVisible();
    expect(await axe(page), 'axe E6 sombre').toEqual([]);
    await page.getByRole('button', { name: t.history }).first().click();
    expect(await axe(page), 'axe E12 sombre').toEqual([]);
    expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).not.toBe('rgb(255, 255, 255)');
  });
});

test('zone : 2 points → Valider désactivé ; départ hors zone : zone validée, erreur au lancement (AC7)', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: isFr() ? 'Zone' : 'Area', exact: true }).click();
  const validate = page.getByTestId('zone-validate');
  await expect(page.locator('.count')).toHaveText('0/50'); // terra-draw monté (composant chargé à la demande)
  await expect(validate).toBeDisabled();
  const box = (await page.locator('.map').boundingBox())!;
  const P = (fx: number, fy: number) => [box.x + box.width * fx, box.y + box.height * fy] as const;
  // triangle dans le coin gauche, loin du départ (centre, 30 % de hauteur)
  const pts = [P(0.1, 0.08), P(0.3, 0.08), P(0.2, 0.2)];
  for (const [i, [x, y]] of pts.entries()) await zoneClick(page, x, y, i + 1);
  await expect(validate).toBeDisabled(); // pas encore fermée
  await zoneClick(page, ...pts[0], 3); // fermeture
  await expect(validate).toBeEnabled();
  // départ hors zone : indice non bloquant, la zone se valide quand même
  await expect(page.getByTestId('zone-outside')).toBeVisible();
  await validate.click();
  const find = page.getByRole('button', { name: t.find });
  await expect(find).toBeVisible(); // retour en E3, zone gardée
  await expect(page.locator('.hint .warn-inline')).toBeVisible();
  // l'erreur n'arrive qu'au lancement, sans appel au serveur, avec la sortie de secours
  await find.click();
  const alert = page.locator('.alert');
  await expect(alert).toContainText(isFr() ? 'Ton départ est hors de la zone : déplace-le ou modifie la zone.' : 'Your start is outside the area: move it or edit the area.');
  expect(calls.length).toBe(0);
  await alert.getByRole('button', { name: isFr() ? 'Modifier la zone' : 'Edit the area' }).click();
  await expect(page.getByTestId('zone-validate')).toBeEnabled();
});
