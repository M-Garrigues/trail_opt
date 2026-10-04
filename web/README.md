# optrail — front web (v1)

SPA Svelte 5 + Vite + TypeScript, MapLibre GL (Plan IGN vectoriel + ombrage LiDAR HD), terra-draw (zone),
profil d'altitude en canvas, i18n maison (`src/i18n/fr.ts` fait foi, `en.ts` typé dessus). Spécification :
`contracts/ui-spec.md` ; API : `contracts/api.md` (dossier d'équipe).

```sh
npm ci
../scripts/dev.sh          # API locale (cargo lambda watch) + Vite sur http://localhost:5173
npm run dev                # front seul (proxy /api → http://localhost:${API_PORT:-9000})
npm test                   # Vitest : i18n (couverture de engine/codes.json), GPX, catalogue, historique
npx playwright test        # parcours critiques, mobile (Pixel 7, fr-FR) + bureau (1280 px, en-US), API mockée
npm run build              # svelte-check + build dans dist/
```

- Turnstile : `VITE_TURNSTILE_SITEKEY` au build ; défaut = clé de test Cloudflare invisible (passe toujours).
- Captures du handoff : `SCREENS=1 npx playwright test screens`.
- Rien n'est envoyé sans action : pas de cookie, pas de géolocalisation à froid ; réglages et historique en localStorage.
