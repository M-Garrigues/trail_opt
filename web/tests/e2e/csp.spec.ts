// M2 (T27/T31) : la CSP servie par CloudFront (infra/cdn.tf, local.csp, seule source) appliquée au
// document en Report-Only ; le parcours (carte, Turnstile réel en clé de test, calcul, détail, 3D, menu)
// ne doit lever aucune violation. Prérequis pour passer la CSP en mode bloquant.
import { readFileSync } from 'node:fs';
import { test, expect } from '@playwright/test';
import { setup, mockPlan, placeStart, T, isFr } from './helpers';

const tf = readFileSync(new URL('../../../infra/cdn.tf', import.meta.url), 'utf8');
const csp = [...tf.match(/csp = join\("; ", \[([\s\S]*?)\]\)/)![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]).join('; ');

test('CSP de prod : aucune violation sur le parcours principal', async ({ page }) => {
  const t = T();
  await setup(page);
  await page.unroute('https://challenges.cloudflare.com/**'); // Turnstile réel : script + iframe
  await mockPlan(page);
  await page.route('**/*', async (route) => {
    const own = new URL(route.request().url()).origin === new URL(test.info().project.use.baseURL!).origin;
    if (route.request().resourceType() !== 'document' || !own) return route.fallback();
    const res = await route.fetch();
    await route.fulfill({ response: res, headers: { ...res.headers(), 'content-security-policy-report-only': csp } });
  });
  await page.addInitScript(() => {
    const w = window as unknown as { cspViolations: string[] };
    w.cspViolations = [];
    document.addEventListener('securitypolicyviolation', (e) => w.cspViolations.push(`${e.effectiveDirective} ${e.blockedURI}`));
  });
  const violations = () => page.evaluate(() => (window as unknown as { cspViolations: string[] }).cspViolations);

  await page.goto('/?design=a');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible();
  await page.getByRole('button', { name: t.details }).first().click();
  const v3 = page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' });
  if (await v3.count()) {
    await v3.first().click();
    await page.waitForTimeout(2000); // tuiles de relief
  }
  await page.goto('/');
  await page.getByRole('button', { name: t.menu }).click();
  await page.waitForTimeout(500);
  expect(await violations()).toEqual([]);
});
