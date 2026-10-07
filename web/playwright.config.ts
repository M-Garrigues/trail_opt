import { defineConfig, devices } from '@playwright/test';

// Pile locale : Vite (npm run dev). L'API est mockée par page.route avec des réponses réelles du handler
// (tests/fixtures/*.json, capturées via scripts/dev.sh) ; fond de carte IGN réel (réseau).
// Projets : Pixel 7 (Chromium), iPhone 14 (WebKit), bureau (Chromium).
// BASE_URL=https://localhost:5173 : réutilise la pile `LAN=1 scripts/dev.sh` (HTTPS auto-signé).
const baseURL = process.env.BASE_URL ?? 'http://localhost:5173';
export default defineConfig({
  testDir: 'tests/e2e',
  timeout: 60_000,
  fullyParallel: true,
  // CI : fond de carte IGN réel (réseau) → 1 nouvel essai
  retries: process.env.CI ? 1 : 0,
  forbidOnly: !!process.env.CI,
  reporter: [['list']],
  // CI : runner sans carte graphique, les animations de caméra (3D) n'y aboutissent pas à temps ;
  // le front respecte prefers-reduced-motion (caméra instantanée), même rendu final.
  use: { baseURL, ignoreHTTPSErrors: true, trace: 'retain-on-failure', ...(process.env.CI ? { reducedMotion: 'reduce' as const } : {}) },
  webServer: { command: 'npm run dev', url: baseURL, ignoreHTTPSErrors: true, reuseExistingServer: true, timeout: 60_000 },
  projects: [
    { name: 'mobile-fr', use: { ...devices['Pixel 7'], locale: 'fr-FR' } },
    { name: 'iphone-fr', use: { ...devices['iPhone 14'], locale: 'fr-FR' } },
    { name: 'desktop-en', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 }, locale: 'en-US' } },
  ],
});
