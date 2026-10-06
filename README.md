# optrail

Sorties trail sur mesure : tu choisis un type d'entraînement (Max D+, Cible distance + D+, Le plus
court pour un D+ donné), un départ et éventuellement une zone ; optrail trace la sortie qui y répond
le mieux sur les chemins de la BD TOPO® de l'IGN. Résultat : tracé sur le Plan IGN, profil d'altitude
coloré par pente, GPX, lien de partage (90 jours).
Interface FR/EN, sans compte ni cookie. Couverture actuelle : Île-de-France, Isère et Lyon (France
entière en préparation).
Site : [optrail.eu](https://optrail.eu).

## Architecture

```
navigateur (Svelte 5 + Vite, MapLibre, terra-draw)          tuiles Plan IGN, géocodage : Géoplateforme
   │  GET /api/plan, POST|GET /api/loops (+ jeton Cloudflare Turnstile)
   ▼
CloudFront ─► S3 (site, boucles partagées)
          └─► Lambda arm64 (moteur Rust) ─► dalles pré-calculées (zip Lambda)
```

| Dossier | Contenu |
|---|---|
| `engine/` | Moteur Rust : préparation du graphe depuis les dalles, solveurs (recuit classique, recuit par faces), modes, handler Lambda (`src/bin/lambda.rs`), API (`src/api.rs`). Codes d'erreur stables : `engine/codes.json`. |
| `web/` | Front Svelte 5 (PWA). Textes dans `src/i18n/fr.ts` (fait foi) et `en.ts`. |
| `pipeline/` | Construction des dalles `tiles/1` depuis la BD TOPO et le MNT IGN (Python). |
| `infra/` | OpenTofu (AWS, Cloudflare DNS) et déploiement : voir [`infra/README.md`](infra/README.md). |
| `trailopt/` | Moteur Python historique, oracle de tests (voir plus bas). |

Une boucle est un sous-graphe connexe à degrés pairs contenant le départ ; son D+ est la somme
des poids `(montée + descente) / 2` des tronçons. Les dalles stockent le profil d'altitude de
chaque tronçon tous les 5 m (LiDAR HD, repli RGE ALTI).

## Lancer en local

Prérequis : Rust stable, Node LTS, `cargo-lambda` (`pip install cargo-lambda`), dalles
dans `scripts/experiments/tiles_v1` (non versionnées, construites par `pipeline`, voir plus bas ; ou `TILES_DIR=…`).

```bash
(cd web && npm ci)
scripts/dev.sh        # API sur http://localhost:9000, front sur http://localhost:5173
```

Clés Turnstile de test, partage stocké dans `engine/target/shared`. Options en tête de
`scripts/dev.sh` (`FORCE_TURNSTILE=1`, `WEB=0`, ports). Dalles :
`python -m pipeline build --tiles <ix_iy,…|sud,ouest,nord,est> --out <dossier>`.

## Tests

```bash
(cd engine && cargo fmt --check && cargo clippy --release --all-targets -- -D warnings && cargo test --release)
python -m pytest -q             # Python + parité avec le moteur Rust (binaire release ci-dessus)
(cd web && npm test && npm run build && npx playwright test)
```

La CI (`.github/workflows/ci.yml`) lance le tout ; le déploiement part de `main` quand elle est
verte (`infra/README.md`).

## Moteur Python historique

Le premier prototype (`trailopt/`, Python) reste dans le dépôt comme **oracle de tests** : la
parité Rust/Python est vérifiée en CI. Usage en CLI :
`python -m trailopt --start 48.7309,2.2713 --distance 10 --out boucle.gpx`.

## Contribuer

Contributions bienvenues : lis [CONTRIBUTING.md](CONTRIBUTING.md) (lancer en local, tests, conventions,
clause de contribution). Une faille ? [SECURITY.md](SECURITY.md). Pour une idée ou un bug : une issue.

## Licence

Code : [PolyForm Noncommercial 1.0.0](LICENSE) (usage non commercial libre ; usage commercial :
me contacter via [GitHub](https://github.com/M-Garrigues) ou contact@optrail.eu).

Données : BD TOPO®, RGE ALTI®, MNT LiDAR HD et Plan IGN © IGN, Licence Ouverte Etalab 2.0 ; les
dalles dérivées sont publiées sous la même licence. Si des données OpenStreetMap sont ajoutées :
© contributeurs OpenStreetMap (ODbL). Ces licences s'appliquent aux données et à leurs dérivés,
indépendamment de celle du code.
