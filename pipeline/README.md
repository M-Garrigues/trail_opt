# pipeline — dalles `tiles/1`

Format : `.team/contracts/tiles.md` (non versionné) ; version des données : `DATA_VERSION` dans
`build.py`, recopiée dans `manifest.json`.

```sh
python -m pipeline build    --tiles 32_342,33_342 --out tiles      # ou bbox 'sud,ouest,nord,est'
python -m pipeline check    --tiles tiles --list pipeline/tiles_v1.txt [--previous ancien/manifest.json]
python -m pipeline clip     --tiles tiles --out engine/tests/data/tiles --disk lat,lon,rayon_m …
python -m pipeline coverage --tiles tiles --out web/public/coverage.geojson
python -m pipeline pois     --tiles tiles      # cols/sommets -> tiles/pois.json (après chaque build)
```

`build` reprend où il s'est arrêté (dalle intacte = sautée) et accepte plusieurs processus sur le
même dossier : `xargs -P 3 -n 1 python -m pipeline build --out tiles --tiles < pipeline/tiles_v1.txt`
(~25 min pour IdF + Isère + Lyon, 87 dalles). `check` sort en code 1 si une dalle manque, si un sha256 est
faux, si `nodata_frac` > 1 %, `jump_gt10` > 0,5 % ou `node_fallback` > 0,2 % des tronçons, ou si le
nombre de tronçons d'une dalle bouge de plus de 5 % par rapport à la version précédente.

## Mise à jour trimestrielle (manuelle)

1. **Quand** : chaque trimestre, après la nouvelle édition de la BD TOPO (le WFS sert toujours
   l'édition courante). Sur une branche : `DATA_VERSION = "bdtopo-wfs-AAAA-MM"` dans `build.py`.
2. **Construire** : GitHub → Actions → `data` → *Run workflow* sur cette branche (liste par défaut
   `pipeline/tiles_v1.txt`). Le job refuse une version déjà publiée, construit à 3 processus,
   lance `pipeline check` contre la dernière release `tiles-*`, puis publie la release
   `tiles-<DATA_VERSION>` (zip + `manifest.json`). Pas d'accès AWS dans ce workflow.
3. **Vérifier** à la main avant de servir : totaux du manifeste dans le log ; parité sur le zip
   décompressé (`PYTHONPATH=. python scripts/parity_prep.py --tiles <dossier>`) ; une boucle Massy
   et une boucle Bourg-d'Oisans dans l'app locale (`scripts/dev.sh`).
4. **Dalle de test** : depuis le zip, régénérer `engine/tests/data/tiles` (commande dans son
   README) et `web/public/coverage.geojson` si la couverture change ; `pytest` (qui exige
   `data_version` de la dalle de test = `DATA_VERSION`) et `cargo test` verts ; fusionner.
5. **Publier en prod** (infra, `infra/README.md`) : copier le zip de la release vers
   `s3://<ARTIFACTS_BUCKET>/tiles/<DATA_VERSION>/` (préfixe neuf, jamais réécrit), mettre la
   `DATA_VERSION` de `infra/prod.env` à la nouvelle valeur, redéployer (`scripts/deploy.sh`).
6. **Rollback** : remettre `DATA_VERSION` de `infra/prod.env` à la version précédente et
   redéployer. Ne jamais supprimer une release `tiles-*` ni un préfixe S3 servi un jour.

## Reproductibilité

Non rejouable : le WFS BD TOPO et le WMS-R altimétrique de la Géoplateforme servent l'état du jour
(pas d'édition datée ; la couverture LiDAR HD s'étend et change les altitudes). Une version de
dalles ne peut donc pas être reconstruite plus tard depuis le réseau. Rejouable : à sources égales
(cache `TRAILOPT_CACHE_DIR`), `build` est déterministe (dalles pilotes reconstruites hors ligne en
10b : sha256 identiques). Piste retenue, la plus simple : l'artefact de chaque version est archivé
(release GitHub `tiles-<DATA_VERSION>`, ~85 Mo, conservée ; préfixe S3 immuable). Plus tard si
besoin : reconstruire depuis les éditions BD TOPO datées téléchargeables (GeoPackage par
département) plutôt que le WFS.

Données : BD TOPO®, RGE ALTI®, LiDAR HD — IGN, Licence Ouverte Etalab 2.0.
