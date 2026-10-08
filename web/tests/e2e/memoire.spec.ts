// 2026-10-08 (retour du fondateur) : la 3D sur téléphone peut épuiser la mémoire ; rien ne doit se perdre.
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr } from './helpers';

test('onglet rechargé : la sortie revient sans recalcul ; « Retour » efface cet état', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  const headline = await page.getByTestId('headline').innerText();
  expect(calls.length).toBe(1);
  await page.reload();
  await expect(page.getByTestId('headline')).toHaveText(headline); // restaurée
  expect(calls.length).toBe(1); // sans recalcul (ni Turnstile ni serveur)
  // nouvelle recherche (retour) : l'état gardé est effacé, un rechargement ne le ramène plus
  await page.getByRole('button', { name: t.back }).first().click();
  await expect(page.getByRole('button', { name: t.find })).toBeVisible();
  expect(await page.evaluate(() => sessionStorage.getItem('optrail.session'))).toBeNull();
  await page.reload();
  await expect(page.getByRole('button', { name: t.find })).toBeVisible();
  await expect(page.getByTestId('headline')).toHaveCount(0);
});

test('contexte WebGL perdu en 3D : retour en 2D, message, sortie conservée', async ({ page, browserName }) => {
  test.skip(browserName === 'webkit', 'WEBGL_lose_context simulé sous Chromium');
  test.slow();
  const t = T();
  await setup(page);
  await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  const headline = await page.getByTestId('headline').innerText();
  await page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getPitch()), { timeout: 30_000 }).toBeGreaterThan(40);
  await page.evaluate(() => {
    const c = (window as any).tmap.map.getCanvas() as HTMLCanvasElement;
    const gl = (c.getContext('webgl2') ?? c.getContext('webgl')) as WebGLRenderingContext;
    gl.getExtension('WEBGL_lose_context')!.loseContext();
  });
  await expect(page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' })).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByText(isFr() ? 'Vue 3D interrompue (mémoire)' : '3D view stopped (memory)')).toBeVisible();
  await expect(page.getByTestId('headline')).toHaveText(headline);
});

// Longue suite de mouvements en 3D (rotation, inclinaison, zoom, changement de sortie, survol du profil, 2D/3D) :
// aucune erreur JavaScript, aucun rechargement, aucune remise à zéro de l'application.
test('3D : longue suite de mouvements sans erreur ni remise à zéro', async ({ page }, info) => {
  test.slow();
  test.setTimeout(240_000);
  const t = T();
  const errors: string[] = [], lost: string[] = [];
  let loads = 0;
  page.on('pageerror', (e) => { if (!/Importing a module script failed/.test(e.message)) errors.push(`pageerror: ${e.message}`); });
  page.on('console', async (m) => {
    if (m.type() !== 'error' || /Failed to load resource|net::ERR/.test(m.text())) return;
    // erreur passée en objet (MapLibre : événement `error`) : message et pile pour le diagnostic
    const detail = await Promise.all(m.args().map((a) => a.evaluate((v: unknown) => (v instanceof Error ? `${v.name}: ${v.message} @ ${(v.stack ?? '').split('\n').slice(0, 4).join(' | ')}` : v && typeof v === 'object' && 'error' in v ? String((v as { error: Error }).error?.stack ?? (v as { error: unknown }).error) : String(v))).catch(() => '?')));
    const text = `${m.text()} ${detail.join(' ; ')}`;
    // tuiles du fond ou du relief indisponibles (502, 400 du serveur IGN) : pas une erreur de l'appli ;
    // worker de MapLibre refusé par WebKit en développement (certificat local) : propre à l'environnement de test
    if (/AJAXError|data\.geopf|tiles\.mapterhorn|Worker failed to load/.test(text)) return;
    errors.push(`console: ${text}`);
  });
  page.on('load', () => loads++);
  await setup(page);
  await mockPlan(page, async (route) => { await route.fulfill({ contentType: 'application/json', body: (await import('./helpers')).plan4 }); return true; });
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
  await page.evaluate(() => (window as any).tmap.map.on('webglcontextlost', () => ((window as any).lost = ((window as any).lost ?? 0) + 1)));
  const loadsBefore = loads;
  const radios = page.getByRole('radio', { name: /^(Choisir la sortie|Choose run)/ });
  const btn3d = page.locator('.view3d');
  await btn3d.click();
  await expect.poll(() => page.evaluate(() => (window as any).tmap.map.getPitch()), { timeout: 30_000 }).toBeGreaterThan(40);
  for (let k = 0; k < 24; k++) {
    await page.evaluate((k) => {
      const m = (window as any).tmap.map;
      m.easeTo({ bearing: m.getBearing() + 47, pitch: 30 + (k * 13) % 45, zoom: m.getZoom() + (k % 3 === 0 ? 1.2 : k % 3 === 1 ? -0.8 : 0.3), duration: 300 });
    }, k);
    if (k % 4 === 1) await radios.nth(k % 4).click();
    if (k % 6 === 2) { const s = page.getByRole('slider').first(); const b = await s.boundingBox(); if (b) for (let x = 0.1; x < 1; x += 0.2) await page.mouse.move(b.x + b.width * x, b.y + b.height / 2); }
    if (k % 8 === 7) { await btn3d.click(); await page.waitForTimeout(500); await btn3d.click(); }
    await page.waitForTimeout(400);
  }
  await page.waitForTimeout(2000);
  expect(errors, errors.join('\n')).toEqual([]);
  expect(loads, 'rechargement de la page').toBe(loadsBefore);
  expect(await page.evaluate(() => (window as any).lost ?? 0), 'contexte WebGL perdu').toBe(0);
  await expect(page.getByTestId('headline')).toBeVisible(); // pas de remise à zéro
});

// Geste « retour » du navigateur pendant la 3D (glissé du pavé tactile ou du bord de l'écran) : c'était la
// « remise à zéro » ; la sortie est gardée et « suivant » la rouvre, sans recalcul.
test('geste retour du navigateur : la sortie est gardée et « suivant » la rouvre', async ({ page }) => {
  const t = T();
  await setup(page);
  const calls = await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  const headline = await page.getByTestId('headline').innerText();
  await page.goBack();
  await expect(page.getByTestId('headline')).toHaveCount(0);
  expect(await page.evaluate(() => sessionStorage.getItem('optrail.session'))).not.toBeNull();
  await page.goForward();
  await expect(page.getByTestId('headline')).toHaveText(headline);
  expect(calls.length).toBe(1);
  // et un rechargement après le geste la rouvre aussi
  await page.goBack();
  await page.reload();
  await expect(page.getByTestId('headline')).toHaveText(headline);
  expect(calls.length).toBe(1);
});
