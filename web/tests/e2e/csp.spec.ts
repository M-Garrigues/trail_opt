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
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 }); // Turnstile réel (jusqu'à 8 s)
  await page.getByRole('button', { name: isFr() ? 'Vue 3D' : '3D view' }).click();
  await expect(page.getByRole('button', { name: isFr() ? 'Vue 2D' : '2D view' })).toHaveAttribute('aria-pressed', 'true');
  await page.waitForTimeout(2000); // tuiles de relief (Mapterhorn)
  await page.getByRole('button', { name: t.details }).first().click({ timeout: 3000 }).catch(() => {}); // bureau : détail déjà embarqué
  await page.goto('/');
  await page.getByRole('button', { name: t.menu }).click();
  await page.waitForTimeout(500);
  expect(await violations()).toEqual([]);
});
