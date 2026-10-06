// Captures pour le handoff (pas un test d'acceptation) : SCREENS=1 npx playwright test screens
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T } from './helpers';

const OUT = '../.team/handoffs/img/';
test.skip(!process.env.SCREENS, 'captures seulement avec SCREENS=1');

for (const [name, scheme] of [['mobile-clair', 'light'], ['mobile-sombre', 'dark']] as const) {
  test.describe(name, () => {
    test.use({ colorScheme: scheme });
    test(name, async ({ page }, info) => {
      test.skip(info.project.name !== 'mobile-fr');
      const t = T(true);
      await setup(page);
      await mockPlan(page);
      await page.goto('/');
      await placeStart(page);
      await page.waitForTimeout(4000);
      await page.screenshot({ path: `${OUT}ux-v1-${name}-reglages.png` });
      await page.getByRole('button', { name: t.find }).click();
      await expect(page.getByTestId('headline')).toBeVisible();
      await page.waitForTimeout(4000);
      await page.screenshot({ path: `${OUT}ux-v1-${name}-resultat.png` });
    });
  });
}

test('bureau', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-en');
  const t = T(false);
  await setup(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  await page.getByRole('button', { name: t.other, exact: true }).click();
  await expect(page.locator('.loops .loop')).toHaveCount(4);
  await page.waitForTimeout(4000);
  const prof = page.getByRole('slider').first();
  const b = (await prof.boundingBox())!;
  await page.mouse.move(b.x + b.width * 0.6, b.y + b.height / 2);
  await page.screenshot({ path: `${OUT}ux-v1-bureau-resultat.png` });
});

// T21 (D27) : directions « nature ». DESIGNS=a,b,c ; OUT_DIR pour itérer hors du handoff.
const DOUT = process.env.OUT_DIR ?? OUT;
for (const d of (process.env.DESIGNS ?? 'a,b,c').split(',')) {
  for (const scheme of ['light', 'dark'] as const) {
    const sc = scheme === 'light' ? 'clair' : 'sombre';
    test(`design ${d} mobile ${sc}`, async ({ page }, info) => {
      test.skip(info.project.name !== 'mobile-fr');
      await page.emulateMedia({ colorScheme: scheme });
      await setup(page);
      await mockPlan(page);
      await page.goto(`/?design=${d}`);
      await placeStart(page);
      await page.waitForTimeout(6000);
      await page.screenshot({ path: `${DOUT}design-${d}-mobile-${sc}-reglages.png` });
      await page.getByRole('button', { name: T(true).find }).click();
      await expect(page.getByTestId('headline')).toBeVisible();
      await page.waitForTimeout(6000);
      await page.screenshot({ path: `${DOUT}design-${d}-mobile-${sc}-resultat.png` });
    });
    test(`design ${d} bureau ${sc}`, async ({ page }, info) => {
      test.skip(info.project.name !== 'desktop-en');
      await page.emulateMedia({ colorScheme: scheme });
      await setup(page);
      await mockPlan(page);
      await page.goto(`/?design=${d}&lang=fr`);
      await placeStart(page);
      await page.waitForTimeout(6000);
      await page.screenshot({ path: `${DOUT}design-${d}-bureau-${sc}-reglages.png` });
      await page.getByRole('button', { name: T(true).find }).click();
      await expect(page.getByTestId('headline')).toBeVisible();
      await page.waitForTimeout(6000);
      const prof = page.getByRole('slider').first();
      const b = (await prof.boundingBox())!;
      await page.mouse.move(b.x + b.width * 0.6, b.y + b.height / 2);
      await page.screenshot({ path: `${DOUT}design-${d}-bureau-${sc}-resultat.png` });
    });
  }
}

// Spike 3D : API locale réelle (relief des Alpes, pas de mock), SHOT3D=1.
for (const proj of ['desktop-en', 'mobile-fr']) {
  test(`design 3D ${proj}`, async ({ page }, info) => {
    test.skip(info.project.name !== proj || !process.env.SHOT3D);
    test.setTimeout(180_000);
    await page.addInitScript(() => {
      localStorage.setItem('optrail.intro', 'true');
      localStorage.setItem('optrail.lastStart', JSON.stringify({ lat: 45.06, lon: 6.03 }));
    });
    await page.route('https://challenges.cloudflare.com/**', (r) => r.abort());
    await page.goto('/?design=a&lang=fr');
    await placeStart(page);
    await page.getByRole('button', { name: T(true).find }).click();
    await expect(page.getByTestId('headline')).toBeVisible({ timeout: 120_000 });
    await page.waitForTimeout(4000);
    const n = proj === 'desktop-en' ? 'bureau' : 'mobile';
    await page.screenshot({ path: `${DOUT}design-3d-${n}-avant.png` });
    await page.getByRole('button', { name: 'Vue 3D' }).click();
    await page.waitForTimeout(9000);
    await page.screenshot({ path: `${DOUT}design-3d-${n}.png` });
  });
}

// Niveaux : la feuille aux 3 crans (plus dépliée = plus haute), assemblés ensuite (scripts : voir handoff).
for (const d of (process.env.DESIGNS ?? 'a,b,c').split(',')) {
  test(`design ${d} niveaux`, async ({ page }, info) => {
    test.skip(info.project.name !== 'mobile-fr');
    await setup(page);
    await mockPlan(page);
    await page.goto(`/?design=${d}`);
    await placeStart(page);
    await page.getByRole('button', { name: T(true).find }).click();
    await expect(page.getByTestId('headline')).toBeVisible();
    await page.waitForTimeout(5000);
    const handle = page.locator('.sheet .handle');
    for (const k of [1, 2, 0]) {
      await page.screenshot({ path: `${DOUT}design-${d}-niveau-${k}.png` });
      await handle.click();
      await page.waitForTimeout(900);
    }
  });
}

// T36 (D34) : captures avant/après. SCREENS=1 T36=avant|apres [QS='?design=a'] npx playwright test screens -g t36 --workers=1
const T36 = process.env.T36;
const QS = process.env.QS ?? '';
const qs = (extra: string) => (QS ? `${QS}&${extra}` : `?${extra}`);
for (const scheme of ['light', 'dark'] as const) {
  const sc = scheme === 'light' ? 'clair' : 'sombre';
  test(`t36 mobile ${sc}`, async ({ page }, info) => {
    test.skip(info.project.name !== 'mobile-fr' || !T36);
    await page.emulateMedia({ colorScheme: scheme });
    await setup(page, { intro: true });
    await mockPlan(page);
    await page.goto(`/${qs('lang=fr')}`);
    await page.waitForTimeout(4000);
    await page.screenshot({ path: `${OUT}t36-${T36}-mobile-${sc}-accueil.png` });
    await page.getByRole('button', { name: T(true).go }).click();
    await placeStart(page);
    await page.waitForTimeout(4000);
    await page.screenshot({ path: `${OUT}t36-${T36}-mobile-${sc}-reglages.png` });
    await page.getByRole('button', { name: T(true).find }).click();
    await expect(page.getByTestId('headline')).toBeVisible();
    await page.waitForTimeout(4000);
    await page.screenshot({ path: `${OUT}t36-${T36}-mobile-${sc}-resultat.png` });
  });
  test(`t36 bureau ${sc}`, async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-en' || !T36);
    await page.emulateMedia({ colorScheme: scheme });
    await setup(page);
    await mockPlan(page);
    await page.goto(`/${qs('lang=fr')}`);
    await placeStart(page);
    await page.waitForTimeout(4000);
    await page.screenshot({ path: `${OUT}t36-${T36}-bureau-${sc}-reglages.png` });
    await page.getByRole('button', { name: T(true).find }).click();
    await expect(page.getByTestId('headline')).toBeVisible();
    await page.waitForTimeout(4000);
    await page.screenshot({ path: `${OUT}t36-${T36}-bureau-${sc}-resultat.png` });
  });
}
// 3D en forte pente (API locale réelle) : SHOT3D=1 T36=apres [START=lat,lon] [KM=12]. Même caméra : ligne drapée
// (avant, couches MapLibre) puis ruban surélevé (après, trail3d.ts).
test('t36 3d', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-en' || !T36 || !process.env.SHOT3D);
  test.setTimeout(240_000);
  const [lat, lon] = (process.env.START ?? '45.06,6.03').split(',').map(Number);
  await page.addInitScript(([lat, lon, km]) => {
    localStorage.setItem('optrail.intro', 'true');
    localStorage.setItem('optrail.lastStart', JSON.stringify({ lat, lon }));
    localStorage.setItem('optrail.settings', JSON.stringify({ typeId: 'max_dplus', values: { max_dplus: { distance_km: km } } }));
  }, [lat, lon, Number(process.env.KM ?? 10)] as const);
  await page.route('https://challenges.cloudflare.com/**', (r) => r.abort());
  await page.goto(`/${qs('lang=fr')}`);
  await page.locator('.map[data-ready]').waitFor();
  await page.waitForTimeout(1500);
  await placeStart(page);
  await page.getByRole('button', { name: T(true).find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 180_000 });
  await page.getByRole('button', { name: 'Vue 3D' }).click();
  await page.waitForTimeout(12000);
  await page.screenshot({ path: `${OUT}t36-${T36}-3d-ruban.png` });
  // mesure : temps moyen d'une image (rendu forcé) avec le ruban
  const ms = async () => page.evaluate(async () => {
    const m = (window as unknown as { tmap: { map: { triggerRepaint(): void; once(e: string, f: () => void): void } } }).tmap.map;
    const t0 = performance.now();
    for (let i = 0; i < 60; i++) await new Promise<void>((ok) => { m.once('render', () => ok()); m.triggerRepaint(); });
    return (performance.now() - t0) / 60;
  });
  const withRibbon = await ms();
  await page.evaluate(() => {
    const m = (window as unknown as { tmap: { map: { removeLayer(id: string): void; setLayoutProperty(l: string, k: string, v: string): void } } }).tmap.map;
    m.removeLayer('trail-3d');
    for (const l of ['loops-casing', 'loops-sel']) m.setLayoutProperty(l, 'visibility', 'visible');
  });
  await page.waitForTimeout(3000);
  await page.screenshot({ path: `${OUT}t36-${T36}-3d-drape.png` });
  const draped = await ms();
  console.log(`T36 3D ms/image : ruban ${withRibbon.toFixed(1)} · drapé ${draped.toFixed(1)}`);
});
