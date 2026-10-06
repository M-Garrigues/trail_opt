// Installation de l'app : entrée permanente du menu + suggestion unique après le 3e itinéraire généré (jamais au chargement).
// Android : `beforeinstallprompt` simulé ; iPhone : consigne « Partager → Sur l'écran d'accueil ».
import { test, expect, type Page } from '@playwright/test';
import { setup, mockPlan, placeStart, T } from './helpers';

const L = { title: 'Installer l’app', action: 'Installer', later: 'Plus tard', ios: /Partager.*Sur l’écran d’accueil/ };

/** Premier calcul fait ; `more()` = un calcul de plus (« Nouvelles propositions » compte comme un recalcul). */
async function toRoute(page: Page) {
  const t = T();
  await setup(page); await mockPlan(page);
  await page.goto('/');
  await placeStart(page);
  await page.getByRole('button', { name: t.find }).click();
  await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
  return async () => {
    await page.getByRole('button', { name: /Nouvelles propositions|New suggestions/ }).click();
    await expect(page.getByTestId('headline')).toBeVisible({ timeout: 20_000 });
  };
}
/** Simule l'événement de Chrome ; `window.prompted` compte les appels à prompt(). */
const fireBip = (page: Page) => page.evaluate(() => {
  const e = Object.assign(new Event('beforeinstallprompt', { cancelable: true }), { prompt: async () => { (window as unknown as { prompted: number }).prompted = 1; } });
  dispatchEvent(e);
  return e.defaultPrevented;
});

test('Android : rien au chargement, suggestion après le premier GPX, une seule fois', async ({ page }, info) => {
  test.skip(info.project.name !== 'mobile-fr');
  const more = await toRoute(page);
  expect(await fireBip(page)).toBe(true); // pas de bandeau du navigateur
  const hint = page.getByTestId('install-hint');
  await expect(hint).toHaveCount(0);
  await more(); // 2e
  await expect(hint).toHaveCount(0);
  await more(); // 3e
  await expect(hint).toBeVisible();
  await page.screenshot({ path: `../.team/handoffs/img/install-hint-${info.project.name}.png` });
  await hint.getByRole('button', { name: L.later }).click();
  await expect(hint).toHaveCount(0);
  await more(); // 4e : une seule fois
  await expect(hint).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('optrail.install'))).toBe('true');
  // l'entrée du menu reste, et déclenche l'invite du navigateur
  await page.goto('/');
  await fireBip(page);
  await page.getByRole('button', { name: T().menu }).click();
  await page.getByRole('button', { name: L.title }).click();
  expect(await page.evaluate(() => (window as unknown as { prompted: number }).prompted)).toBe(1);
  await expect(page.getByRole('button', { name: L.title })).toHaveAttribute('aria-expanded', 'false'); // invite consommée → consigne générique
});

test('Android : « Installer » de la suggestion ouvre l’invite', async ({ page }, info) => {
  test.skip(info.project.name !== 'mobile-fr');
  const more = await toRoute(page);
  await fireBip(page);
  await more(); await more();
  await page.getByTestId('install-hint').getByRole('button', { name: L.action }).click();
  expect(await page.evaluate(() => (window as unknown as { prompted: number }).prompted)).toBe(1);
  await expect(page.getByTestId('install-hint')).toHaveCount(0);
});

test('iPhone : consigne Partager dans la suggestion et dans le menu', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-fr');
  const more = await toRoute(page);
  await expect(page.getByTestId('install-hint')).toHaveCount(0);
  await more(); await more();
  const hint = page.getByTestId('install-hint');
  await expect(hint).toContainText(L.ios);
  await expect(hint.locator('svg.share')).toBeVisible();
  await page.screenshot({ path: `../.team/handoffs/img/install-hint-${info.project.name}.png` });
  await hint.getByRole('button', { name: L.later }).click();
  await page.goto('/');
  await page.getByRole('button', { name: T().menu }).click();
  const entry = page.getByRole('button', { name: L.title });
  await expect(entry).toHaveAttribute('aria-expanded', 'false');
  await entry.click();
  await expect(page.locator('#install-steps')).toContainText(L.ios);
  await page.screenshot({ path: `../.team/handoffs/img/install-menu-${info.project.name}.png` });
});

test('déjà installée (standalone) ou bureau sans invite : rien dans le menu', async ({ page }, info) => {
  if (info.project.name !== 'desktop-en') {
    await page.addInitScript(() => {
      const mm = window.matchMedia.bind(window);
      window.matchMedia = (q: string) => (q.includes('display-mode') ? ({ matches: true, addEventListener() {}, removeEventListener() {} } as unknown as MediaQueryList) : mm(q));
    });
  }
  await setup(page);
  await page.goto('/');
  await page.getByRole('button', { name: T().menu }).click();
  await expect(page.getByRole('heading', { name: 'Menu' })).toBeVisible();
  await expect(page.getByRole('button', { name: /Installer l’app|Install the app/ })).toHaveCount(0);
});
