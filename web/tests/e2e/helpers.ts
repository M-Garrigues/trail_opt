import { readFileSync } from 'node:fs';
import { expect, test, type Page, type Route } from '@playwright/test';

export const plan1 = readFileSync(new URL('../fixtures/plan1.json', import.meta.url), 'utf8');
export const plan4 = readFileSync(new URL('../fixtures/plan4.json', import.meta.url), 'utf8');

export const isFr = () => test.info().project.name.endsWith('-fr');
export const T = (fr = isFr()) => fr
  ? { go: 'C’est parti', find: 'Trouver ma sortie', cancel: 'Annuler', details: 'Détail', back: 'Retour', other: 'Autres sorties', history: 'Mes sorties', clear: 'Effacer l’historique', share: 'Partager', create: 'Créer le lien', menu: 'Menu', download: 'Télécharger le GPX' }
  : { go: 'Let’s go', find: 'Find my run', cancel: 'Cancel', details: 'Details', back: 'Back', other: 'Other runs', history: 'My runs', clear: 'Clear history', share: 'Share', create: 'Create link', menu: 'Menu', download: 'Download GPX' };


/** Départ mémorisé à Massy (cadre la carte) ; Turnstile coupé (jeton absent, accepté en local). */
export async function setup(page: Page, opts: { intro?: boolean; start?: { lat: number; lon: number } } = {}) {
  await page.addInitScript(([intro, start]) => {
    if (!sessionStorage.getItem('seeded')) {
      sessionStorage.setItem('seeded', '1');
      localStorage.setItem('optrail.lastStart', JSON.stringify(start));
      if (!intro) localStorage.setItem('optrail.intro', 'true');
    }
    // AC2 : compte les appels de géolocalisation
    (window as unknown as { geoCalls: number }).geoCalls = 0;
    const g = navigator.geolocation;
    if (g) {
      const orig = g.getCurrentPosition.bind(g);
      g.getCurrentPosition = (...a: Parameters<Geolocation['getCurrentPosition']>) => { (window as unknown as { geoCalls: number }).geoCalls++; return orig(...a); };
    }
  }, [opts.intro ?? false, opts.start ?? { lat: 48.73, lon: 2.27 }] as const);
  await page.route('https://challenges.cloudflare.com/**', (r) => r.abort());
}

export type Calls = URL[];
/** Mock de GET /api/plan : n_candidates=1 → plan1, sinon plan4. */
export async function mockPlan(page: Page, handler?: (route: Route, url: URL) => Promise<boolean | void>): Promise<Calls> {
  const calls: Calls = [];
  await page.route('**/api/plan?**', async (route) => {
    const url = new URL(route.request().url());
    calls.push(url);
    if (handler && (await handler(route, url))) return;
    await route.fulfill({ contentType: 'application/json', body: url.searchParams.get('n_candidates') === '1' ? plan1 : plan4 });
  });
  return calls;
}

/** Pose un sommet de zone et attend que le compteur l'ait pris en compte (pas de délai fixe : AC7 stable en parallèle). */
export async function zoneClick(page: Page, x: number, y: number, count: number) {
  await page.mouse.click(x, y);
  await expect(page.locator('.count')).toHaveText(`${count}/50`);
}

export async function placeStart(page: Page) {
  await page.locator('.map[data-ready]').waitFor(); // style chargé, sources posées (pas 'load' : attend toutes les tuiles)

  const box = (await page.locator('.map').boundingBox())!;
  // tiers supérieur de la carte : jamais sous la feuille ni les boutons
  const [x, y] = [box.x + box.width / 2, box.y + box.height * 0.3];
  // feuille encore en animation (couche refermée) : attendre que la carte soit bien sous le point
  await expect.poll(() => page.evaluate(([x, y]) => !!document.elementFromPoint(x, y)?.closest('.map'), [x, y])).toBe(true);
  await page.mouse.click(x, y);
  await expect(page.locator('.start-marker')).toBeVisible();
}

/** Options (montées, pente, voies…) : mobile = page « Options » de la feuille ; bureau = « Plus d'options » déplié dans le panneau. */
export async function openOptions(page: Page) {
  if (await page.locator('.sheet').count()) await page.locator('.bar .btn.ico').first().click();
  else if (await page.locator('details.more:not([open])').count()) await page.locator('details.more summary').click();
  await expect(page.locator('#roads')).toBeVisible();
}
/** Mobile : referme la page Options (« Terminé ») ; bureau : sans effet. */
export async function closeOptions(page: Page) {
  if (!(await page.locator('.sheet').count())) return;
  await page.locator('.bar .btn.primary').click();
  await expect(page.locator('.start-row')).toBeVisible();
}
