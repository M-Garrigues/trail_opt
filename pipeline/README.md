# pipeline — dalles `tiles/1`

Format : `.team/contracts/tiles.md` (non versionné) ; version des données : `DATA_VERSION` dans
`build.py`, recopiée dans `manifest.json`.

```sh
python -m pipeline build    --tiles 32_342,33_342 --out tiles      # ou bbox 'sud,ouest,nord,est'
python -m pipeline check    --tiles tiles --list pipeline/tiles_v1.txt [--previous ancien/manifest.json]
python -m pipeline clip     --tiles tiles --out engine/tests/data/tiles --disk lat,lon,rayon_m …
python -m pipeline coverage --tiles tiles --out web/public/coverage.geojson
python -m pipeline pois     --tiles tiles      # cols/sommets -> tiles/pois.json (après chaque build)
python -m pipeline merge    --tiles tiles      # data.yml : fusionne manifest-<k>.json / pois-<k>.json des shards
python -m pipeline enrich   --tiles copie [--osm france.osm.pbf …]   # étiquettes calm, osm_hike, osm_water (en place)
```

Listes : `tiles_france.txt` (France métropolitaine, Corse et îles : 1 561 dalles, produite par
`scripts/tile_list_fxx.py` = dalles qui touchent les départements ADMIN EXPRESS élargis de 1 km ; quelques
dalles de bord sont vides, 2,8 Ko), `tiles_v1.txt` (pilote IdF + Isère + Lyon, 87 dalles, version 10d),
`tiles_essai.txt` (6 dalles autour de Chamonix, pour essayer le workflow), `tiles_dom.txt` (66 dalles :
Réunion 16, Guadeloupe 19, Martinique 10, Mayotte 5, Guyane littorale 16 ; clés `<zone>/<ix>_<iy>`, repère
UTM de la zone, `scripts/tile_list.py <zone>`). Les DOM ne sont PAS dans la liste par défaut : à ajouter à
une campagne quand le moteur lit les zones (`zones` au manifeste, contrat tiles.md).

`build` reprend où il s'est arrêté (dalle intacte = sautée) et accepte plusieurs processus sur le
même dossier : `xargs -P 3 -n 1 python -m pipeline build --out tiles --tiles < pipeline/tiles_v1.txt`
(~25 min pour IdF + Isère + Lyon, 87 dalles). `check` sort en code 1 si une dalle manque, si un sha256 est
faux, si `nodata_frac` > 1 %, `jump_gt10` > 0,5 % ou `node_fallback` > 0,2 % des tronçons, ou si le
nombre de tronçons d'une dalle bouge de plus de 5 % par rapport à la version précédente.

## Mise à jour trimestrielle (manuelle)

1. **Quand** : chaque trimestre, après la nouvelle édition de la BD TOPO (le WFS sert toujours
   l'édition courante). Sur une branche : `DATA_VERSION = "bdtopo-wfs-AAAA-MM"` dans `build.py`.
2. **Construire** : GitHub → Actions → `data` → *Run workflow* sur cette branche (liste par défaut
   `pipeline/tiles_france.txt`, 32 shards dont 8 à la fois ; détail ci-dessous). Le workflow refuse une
   version déjà publiée, construit les shards, les fusionne, lance `pipeline check` sur l'ENSEMBLE
   (dont l'altitude unique des nœuds entre dalles) contre la dernière release `tiles-*`, puis publie la
   release `tiles-<DATA_VERSION>` : `tiles-<v>-NN.zip` (300 dalles chacun), `manifest.json`, `pois.json`,
   `coverage.geojson`. Pas d'accès AWS dans ce workflow.
3. **Vérifier** à la main avant de servir : totaux du manifeste dans le log ; parité sur le zip
   décompressé (`PYTHONPATH=. python scripts/parity_prep.py --tiles <dossier>`) ; une boucle Massy
   et une boucle Bourg-d'Oisans dans l'app locale (`scripts/dev.sh`).
4. **Front et dalle de test** : si la couverture change, `gh release download tiles-<v> -p coverage.geojson
   -D web/public --clobber`. La dalle de test `engine/tests/data/tiles` porte sa propre `data_version` :
   ne la régénérer (commande dans son README) que si le format ou le calcul des dalles change.
   `pytest` et `cargo test` verts ; fusionner.
5. **Publier en prod** (infra, `infra/README.md`) : `AWS_PROFILE=optrail scripts/tiles_to_s3.sh <v>`
   (release → `s3://<ARTIFACTS_BUCKET>/tiles/<v>/`, préfixe neuf, jamais réécrit ; sha256 et tailles
   vérifiés), mettre la `DATA_VERSION` de `infra/prod.env` à la nouvelle valeur, redéployer (`scripts/deploy.sh`).
6. **Rollback** : remettre `DATA_VERSION` de `infra/prod.env` à la version précédente et
   redéployer. Ne jamais supprimer une release `tiles-*` ni un préfixe S3 servi un jour.

## Workflow `data` (France entière)

- **Shards** : la ligne i de la liste va au shard `i mod N` (deux dalles voisines tombent dans des shards
  différents). Chaque shard construit ses dalles à 3 processus, lance `pois` et `check` sur ses dalles,
  puis dépose un artefact `shard-<k>`. Le job `merge` rassemble tout, `pipeline merge`, `pipeline check`
  sur l'ensemble, `coverage`, zips ; `publish` crée la release.
- **Altitude aux bords** : `tile_columns` calcule l'altitude d'un nœud sur tous les tronçons qui le
  touchent (voisins et chaînes de ponts/tunnels compris, dalle ± 2 km), donc deux shards donnent la même
  valeur. Vérifié le 2026-10-06 : 4 dalles voisines deux à deux (32_342/32_343, 46_321/46_322)
  construites dans 4 processus et 4 caches séparés, fusionnées : `check` OK (322 nœuds partagés, 0 écart),
  et sha256 identiques aux dalles 10d.
- **Mesures à froid** (cache vide, 1 processus, poste local, 2026-10-06) : montagne 38–40 s, urbain dense
  51–77 s (RAM 2,1 Go), rural 68 s ; ~190 requêtes et ~110 Mo de cache par dalle. France : 21,5 M de
  tronçons au WFS (`resultType=hits` par dalle) → ~20,7 M en dalles, 1,0–1,25 Go, `pois.json` ~5–6 Mo
  (48 762 cols/pics/sommets). 32 shards de ~49 dalles ≈ 25 min chacun ; à 8 simultanés ≈ 1 h 45 d'horloge,
  ~14 h-runner (gratuites, dépôt public).
- **Quotas Géoplateforme** (cartes.gouv.fr, « limites d'usage », par IP) : WFS 30 req/s, WMS-Raster
  40 req/s ; au-delà, HTTP 429 pendant 5 s. Un shard émet ~15 req/s (une IP par runner) ; chaque requête
  est reprise 4 fois (1, 2, 4, 8 s), puis le shard relance ses dalles manquantes 2 fois (après 2 et 8 min).
  Baisser `parallel` (4) si des 429 apparaissent dans les logs.
- **Shard en échec** : `gh run rerun <id> --failed` ne refait que ce shard (ses dalles, les autres
  artefacts restent 7 jours), puis fusion et publication. Ne jamais compléter une version avec des
  dalles d'une autre campagne.
- **Essai** : `gh workflow run data.yml --ref <branche> -f tiles_list=pipeline/tiles_essai.txt -f shards=2
  -f parallel=2 -f previous=none -f publish=false` (6 dalles, ~6 min, artefact `release` seul).

## Étiquettes d'agrément (`enrich`, workflow `enrich`)

Post-traitement d'un dossier de dalles déjà construites, sans requête IGN (colonnes et sens : contrat tiles.md
§ Étiquettes). Il réécrit les `.npz` EN PLACE : toujours sur une copie ou sur les zips d'une release.

- `calm` (0–15) : éloignement des routes importantes, depuis les dalles seules (la dalle et ses 8 voisines
  doivent être dans le dossier ; `check` recalcule et refuse sinon).
- Avec `--osm` : `osm_hike` (0/1/2, relations `route=hiking|foot`) et `osm_water` (0–15, part de la longueur
  à moins de 50 m d'une rivière, d'un canal, d'un plan d'eau ou de la côte). © les contributeurs
  d'OpenStreetMap, ODbL ; étiquettes seulement, absence = neutre.
- `--osm` prend des extraits Geofabrik `.osm.pbf` (réduits par `osmium tags-filter` puis
  `add-locations-to-ways` en un texte `.opl` gardé à côté ; paquet `osmium-tool`, `brew install osmium-tool`)
  ou des `.opl` déjà filtrés. Le texte OPL est lu par la bibliothèque standard : pas de dépendance Python.
- Version : `<base>.<n>` (`bdtopo-wfs-2026-10e` → `…-10e.1`), `derived_from`, `columns` (source et licence par
  colonne) au manifeste. Relançable (les colonnes sont remplacées, la version avance).

Workflow : `gh workflow run enrich.yml --ref <branche> -f base=bdtopo-wfs-2026-10e` (défaut : extrait
`europe/france`, publication de `tiles-<base>.<n>` ; `-f publish=false` pour un essai, `-f osm=` pour `calm`
seul ; DOM : ajouter `europe/france/reunion europe/france/guadeloupe europe/france/martinique
europe/france/guyane europe/france/mayotte` à `osm`). Un seul job : télécharge les zips de la release
d'entrée, étiquette, `check` (dont `--previous` = manifeste d'entrée : mêmes tronçons), refait les zips. Puis
`scripts/tiles_to_s3.sh <base>.<n>` comme pour une version construite.

Mesures (2026-10-06, poste local, 9 dalles autour de la Chartreuse, 186 577 tronçons) : extrait Rhône-Alpes
530 Mo → filtre osmium 8 s et 1,6 Go de RAM (ensembles d'identifiants : même ordre de grandeur pour la France),
129 Mo de texte OPL, 3,1 M de sommets ; lecture 3 s ; ~1 s par dalle pour les trois étiquettes ; `check` 0,3 s par
dalle ; +0,56 o/tronçon (+0,95 %). France (extrait 5,1 Go, 1 561 dalles, 20,7 M de tronçons) : ~1,2 Go de texte
filtré, 2 à 3 Go de RAM Python (494 Mo mesurés pour Rhône-Alpes), ~25 min d'étiquetage et ~10 min de contrôle sur le poste (compter le double sur un
runner), +12 Mo de dalles, ~9 Go de disque au plus haut.

Eau par la BD TOPO (non retenu) : `troncon_hydrographique` + `surface_hydrographique` au WFS = 2 requêtes et
5 Mo par dalle (≈ 3 300 requêtes pour la France, loin du quota), mais `classe_de_largeur` vaut « 0 à 5 m » sur
92 % des tronçons et les rivières larges sont des surfaces : pas de tri rivière / ruisseau, là où OSM le donne
(`river` contre `stream`) sans une requête.

## Reproductibilité

Non rejouable : le WFS BD TOPO et le WMS-R altimétrique de la Géoplateforme servent l'état du jour
(pas d'édition datée ; la couverture LiDAR HD s'étend et change les altitudes). Une version de
dalles ne peut donc pas être reconstruite plus tard depuis le réseau. Rejouable : à sources égales
(cache `TRAILOPT_CACHE_DIR`), `build` est déterministe (dalles pilotes reconstruites hors ligne en
10b : sha256 identiques). Piste retenue, la plus simple : l'artefact de chaque version est archivé
(release GitHub `tiles-<DATA_VERSION>`, ~1,2 Go pour la France, conservée ; préfixe S3 immuable). Plus tard si
besoin : reconstruire depuis les éditions BD TOPO datées téléchargeables (GeoPackage par
département) plutôt que le WFS.

Données : BD TOPO®, RGE ALTI®, LiDAR HD — IGN, Licence Ouverte Etalab 2.0. Colonnes `osm_*` des versions
enrichies : © les contributeurs d'OpenStreetMap, ODbL 1.0.
