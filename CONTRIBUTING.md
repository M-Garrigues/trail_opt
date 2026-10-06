# Contribuer à optrail

Contributions bienvenues. Le dépôt est sous licence PolyForm Noncommercial 1.0.0 (voir `LICENSE`).

## Lancer en local
Prérequis : Rust stable, Node LTS, `cargo-lambda` (`pip install cargo-lambda`), Python 3.12 (`pip install -r requirements-dev.txt`)
et des dalles (`scripts/experiments/tiles_v1`, non versionnées, construites par `python -m pipeline build`, voir `pipeline/README.md`).
```bash
(cd web && npm ci)
TILES_DIR=scripts/experiments/tiles_v1 scripts/dev.sh      # API :9000, front :5173 (options en tête du script)
```

## Tests
```bash
(cd engine && cargo fmt --check && cargo clippy --release --all-targets -- -D warnings && cargo test --release)
python -m pytest -q                       # pipeline + parité avec le moteur Rust
(cd web && npm test && npm run build && npx playwright test)
```

## Conventions
- Simplicité d'abord : diff court, pas d'abstraction pour un seul cas, pas de nouvelle dépendance sans raison.
- Code et commentaires en français, comme l'existant.
- Textes de l'interface dans `web/src/i18n/fr.ts` (fait foi) puis `en.ts`.
- Codes d'erreur de l'API : `engine/codes.json` (stables).

## Proposer un changement
1. Un correctif simple : une PR directe. Tout le reste (fonctionnalité, changement d'API ou de données) : ouvre d'abord une issue.
2. PR petite et ciblée, modèle de PR rempli, tests lancés.
3. La CI doit être verte (le ruleset de `main` l'exige) ; pas de push direct sur `main`.

## Non accepté
- Clés, jetons, secrets, adresses e-mail personnelles.
- Données IGN brutes ou fichiers lourds (les dalles se reconstruisent avec `pipeline/`).

## Clause de contribution (à signer par chaque commit)
Ajoute `Signed-off-by: Prénom Nom <e-mail>` à chaque commit (`git commit -s`). Par cette mention, tu certifies :
1. que tu es l'auteur de la contribution, ou que tu as le droit de la soumettre (Developer Certificate of Origin 1.1, <https://developercertificate.org>) ;
2. que tu acceptes qu'elle soit publiée sous la licence du projet (PolyForm Noncommercial 1.0.0) ;
3. que tu accordes à Mathieu Garrigues, titulaire du projet, le droit perpétuel et mondial de la reproduire, la modifier et la
   relicencier sous d'autres conditions, y compris commerciales.
