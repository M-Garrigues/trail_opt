# Boucle de trail à D+ optimal

Application web (Streamlit) qui génère une boucle de trail au départ d'un point, dans une
zone dessinée sur la carte. La boucle a une distance donnée et soit **maximise le D+**,
soit vise un couple **(distance, D+) cible**. Sortie : tracé, profil altimétrique, GPX.

Périmètre : France (altitude IGN). Données © IGN, © contributeurs OpenStreetMap (ODbL).

## Lancer en local

```bash
python3.12 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt
streamlit run app.py                     # interface
python -m trailopt --start 48.7303,2.2725 --distance 10 --out boucle.gpx --debug   # CLI
pytest -q                                # tests, hors ligne (fixtures)
```

Options CLI utiles : `--mode target --dplus 300`, `--max-grade 35`, `--polygon zone.geojson`,
`--roads unpaved|pedestrian|minor|all`, `--solver auto|exact|anneal`, `--no-revisit`, `--no-limits`.

Variables d'environnement :

| Variable | Rôle | Défaut |
|---|---|---|
| `TRAILOPT_OVERPASS_URLS` | endpoints Overpass, séparés par des virgules, essayés dans l'ordre | OSM France, puis overpass-api.de |
| `TRAILOPT_CACHE_DIR` | cache disque (tronçons IGN, réponses Overpass, dalles d'altitude) | `/tmp/trailopt_cache` |
| `TRAILOPT_OFFLINE` | `1` : toute requête réseau échoue (tests) | — |

## Déployer

### Streamlit Community Cloud (cible v1)

1. Pousser le repo sur GitHub.
2. share.streamlit.io → *New app* → fichier principal `app.py`.
3. *Advanced settings* → Python **3.12**.

`requirements.txt` installe les dépendances Python. `packages.txt` installe `libexpat1`,
dont la roue rasterio a besoin et qui manque dans les images Debian slim.

### Repli : Oracle Cloud Always Free (ARM, 2 OCPU, 12 Go)

```bash
docker build -t trailopt .
docker run -d --restart unless-stopped -p 8501:8501 trailopt
```

L'image a été construite et testée en arm64 : une boucle y est calculée hors ligne.

## Fonctionnement

1. **Zone** : polygone dessiné par clics successifs sur la carte (fermé automatiquement dès
   3 sommets), ou disque de rayon D/2 centré sur le départ (plafonné à 6,9 km de rayon).
   Elle est ensuite **coupée par le disque de rayon Lmax/2 autour du départ**, car une boucle
   de longueur ≤ Lmax ne s'en éloigne jamais davantage. Le plafond d'aire s'applique après
   cette découpe, et la requête Overpass ne porte que sur la zone utile.
2. **Réseau de chemins**, deux sources au choix (`--source ign|osm`) :
   - **IGN BD TOPO (défaut)** : tronçons de route du WFS Géoplateforme, sans clé, par pages
     de 5 000 avec champs restreints. Les tronçons sont déjà coupés aux carrefours : les nœuds
     sont leurs extrémités. Ponts et tunnels viennent de la position par rapport au sol.
     Tronçons privés, réservés aux ayants droit ou hors service exclus. Pas de trottoirs ni de
     petites voies piétonnes urbaines.
   - **OpenStreetMap** : requête Overpass JSON brute, sans osmnx. Nœuds = extrémités de ways
     et nœuds partagés. `oneway` est ignoré. Plus complet en ville et sur les sentiers informels.

   Dans les deux cas, les nœuds de degré 2 sont ensuite contractés. Types de voies :

   | Option | IGN BD TOPO | OpenStreetMap |
   |---|---|---|
   | Sentiers (non revêtus) | Sentier, Chemin, Route empierrée | `surface` non revêtue ; sans `surface`, path, bridleway et track sauf `tracktype=grade1` |
   | Voies piétonnes (toujours OSM) | non utilisé | path, footway (trottoirs inclus), track, bridleway, steps, pedestrian, cycleway |
   | + petites routes (défaut) | + Escalier, Piste cyclable, routes d'importance 4 à 6 | + residential, service, unclassified, tertiary… |
   | + toutes routes | + importance 1 à 3, hors type autoroutier | + secondary, primary |

   La BD TOPO n'a presque aucune voie piétonne en ville. « Voies piétonnes » passe donc
   toujours par OpenStreetMap, quelle que soit la source choisie.
3. **Altitude** : WMS-R Géoplateforme en GeoTIFF float32 à 5 m, en dalles de 1 000 px.
   La couche principale est le MNT LiDAR HD, avec repli RGE ALTI sur les trous. Interpolation
   bilinéaire, échantillonnage tous les 5 m, lissage sur 3 points avec extrémités fixées.
   Profil linéaire sur les ponts et tunnels.
4. **Découpe à la zone** : un tronçon qui traverse le bord de la zone est coupé au bord, la
   partie intérieure est gardée. Les tronçons IGN sont longs : les écarter en entier ferait
   disparaître des voies visiblement dans la zone.
4. **Accès par aller-retour** : si le réseau bouclable du type de voies choisi est à plus de
   30 m du point cliqué, la boucle part quand même de ce point. Elle rejoint le réseau par le
   plus court chemin sur toutes les voies de la même source, en aller-retour compté dans la
   distance et le D+, limité à 60 % de la distance. L'entrée dans le réseau est la jonction la
   plus proche par la route. Si l'aller-retour rend la boucle impossible, on passe au départ
   de repli.
4. **Départ de repli** : si aucune boucle de la bonne distance n'est possible depuis le point
   cliqué, les sous-réseaux 2-arête-connexes (seuls endroits où une boucle existe) sont triés
   par distance au point cliqué. Ceux dont le réseau atteignable est plus court que la distance
   minimale sont écartés sans calcul. Les autres sont résolus dans l'ordre, chacun avec le
   solveur adapté à sa taille et le budget restant. La première boucle à la bonne distance est
   retenue, avec un avertissement et le départ effectif sur la carte. À Massy, 10 km en
   sentiers non revêtus : 25 sous-réseaux écartés, départ déplacé de 2,1 km vers le bois de
   Verrières.
5. **Élagage** sans perte d'optimalité : ponts au sens de Tarjan, composante de s, portée
   d(s,u) + l + d(v,s) ≤ Lmax, puis filtre de pente et nouvel élagage.
6. **Solveur** : boucle = sous-graphe connexe pair contenant s, D+ = Σ (montée+descente)/2.
   - ≤ `EXACT_MAX_EDGES` arêtes : 20 % du budget en recuit pour un warm-start, puis CP-SAT
     (parité, connexité par flot, 2 workers) ;
   - au-delà : recuit simulé sur tout le budget.

### Rayon libre autour du départ et option « carrefours uniques »

- **Rayon libre de 200 m, dans tous les modes.** Toute arête entièrement à moins de 200 m du
  départ est doublée par une copie parallèle. La boucle peut donc reprendre la même route près
  du départ, ce qui est nécessaire quand le départ est dans une impasse. Ces arêtes ne sont plus
  des ponts et survivent à l'élagage. Dans CP-SAT, une copie n'est utilisable que si l'original
  l'est, ce qui casse la symétrie sans perte. En pratique à Massy sur 10 km, une seule arête
  est parcourue deux fois.
- **« Ne jamais repasser par un carrefour »** (case à cocher, `--no-revisit`). Chaque nœud à
  plus de 200 m du départ a un degré au plus 2 dans la boucle. Dans CP-SAT, c'est une borne
  sur k_v ; le modèle de flot et de parité est inchangé, sans recours à `AddCircuit`. Dans le
  recuit, les plus courts chemins interdisent en transit les nœuds déjà traversés par le reste
  de la boucle. Les nœuds du rayon libre peuvent être repassés.

### Couloirs parallèles

Deux trottoirs d'une même rue, ou un chemin large saisi deux fois dans OSM, forment un seul
couloir. La boucle n'en emprunte qu'un, dans tous les modes ; le rayon libre de 200 m est
exempté. Deux tronçons sont jumeaux s'ils restent à moins de 15 m l'un de l'autre, dans la
même direction à 25° près, sur au moins 30 m **et** au moins 60 % du plus court. Les petits
bouts qui se touchent à un carrefour, ou ne se longent que partiellement, ne sont pas
concernés : `tests/test_parallel.py` couvre ces cas. La détection est vectorisée (cKDTree sur
les points densifiés) : 0,15 s pour 9 800 tronçons, 1 s pour 55 000. Seuils dans
`trailopt/graph.py` (`PARALLEL_*`).

## Interface

- Un clic place le départ (épingle verte) ou un sommet de zone, selon le mode choisi au-dessus
  de la carte. Le curseur change avec le mode.
- « Tout effacer » apparaît en haut dès qu'un départ ou un sommet est posé.
- Recherche d'adresse, de commune, de lieu (col, forêt…) ou de coordonnées « lat, lon » via le
  géocodage Géoplateforme : la carte se recentre, et « Départ ici » place le départ.
- « Calculer » apparaît sous la recherche dès qu'un départ est posé. Une barre de progression
  avance selon le budget de calcul. À la fin, la carte se recentre sur la boucle, et distance,
  D+ et bouton GPX s'affichent juste sous la carte.
- Clavier : après une recherche, Entrée place le départ sur le résultat, puis Entrée lance le
  calcul. Échap annule un calcul en cours et arrête réellement le solveur.
- Le tracé affiché reste en place quand on modifie le départ ou la zone, jusqu'au prochain
  calcul. Cliquer sur le profil centre la carte sur le point.
- Le profil est coloré selon la pente, mesurée sur environ 50 m : dégradé continu du vert (plat) au
  jaune, à l'orange, au rouge, jusqu'au rouge sombre à 40 % et plus. Le survol affiche distance, altitude et pente.
- Le profil altimétrique est dessiné dans la carte et lié au tracé : survoler le profil montre
  le point sur la carte, survoler le tracé montre le point sur le profil. Tout se passe dans le
  navigateur, sans aller-retour serveur.
- Fonds : Plan IGN vectoriel (MapLibre GL, par défaut), Plan IGN raster, photos aériennes.
  Relief : ombrage LiDAR HD en surimpression (mode « multiply », activable dans le sélecteur
  de couches). Le fond n'est jamais rechargé : seuls les calques dynamiques changent, donc la
  vue reste en place.

## Benchmark du seuil `EXACT_MAX_EDGES`

Script : `PYTHONPATH=. python scripts/bench_exact.py --dists 5,10,20 --roads trails,minor`.
Départ : centre de Massy. Budget de 20 s, 2 workers, 2 graines. Mesures du 2 octobre 2026
sur un Mac à 8 cœurs, avec d'autres calculs en parallèle.

| distance | voies | arêtes | recuit seul (D+ m) | warm-start + CP-SAT (D+ m) | statut CP-SAT |
|---|---|---|---|---|---|
| 5 km | sentiers | 163 | 120,8 / 120,8 | **122,3** / **122,3** | OPTIMAL |
| 7,5 km | sentiers | 329 | 148,6 / 146,7 | 148,5 / 147,0 | FEASIBLE |
| 10 km | sentiers | 329 | 189,2 / 195,3 | 191,1 / 189,2 | FEASIBLE |
| 15 km | sentiers | 329 | 237,1 / 237,8 | **241,7** / **241,5** | OPTIMAL / FEASIBLE |
| 2 km | + petites routes | 618 | 98,9 / 98,9 | 98,9 / 98,9 | FEASIBLE |
| 3 km | + petites routes | 1 447 | 116,4 / 116,4 | 140,3 / 114,6 | FEASIBLE |
| 5 km | + petites routes | 3 300 | **205,9** / **219,9** | 187,0 / 205,4 | FEASIBLE |
| 10 km | + petites routes | 9 619 | **435,7** / **444,8** | 401,5 / 400,5 | FEASIBLE |
| 20 km | + petites routes | 44 609 | **1 293** / **961** | 276 / 253 | FEASIBLE |
| 20 km | sentiers | 329 | — | — | INFEASIBLE (prouvé) |

**Seuil retenu : 1 000 arêtes.**
- Jusqu'à ~600 arêtes, la voie exacte est toujours au moins aussi bonne. Elle prouve souvent
  l'optimum, et sinon donne une borne et un écart.
- Au-delà de ~3 000 arêtes, le recuit seul gagne nettement.
- Entre les deux, les résultats sont mêlés et dominés par la variance du recuit.

Deux graines par point, c'est peu : le seuil est indicatif. En zone urbaine dense comme Massy
avec petites routes, presque tout passe par le recuit. CP-SAT sert surtout en mode
« sentiers seuls » ou sur des zones dessinées petites.

## Plafonds retenus

| Plafond | Valeur | Raison |
|---|---|---|
| distance | 2 à 25 km | anti-abus |
| aire de la zone utile | 150 km² | anti-abus ; le disque par défaut est plafonné à ~148 km² (rayon 6,9 km) |
| budget solveur | 5 à 60 s, défaut 20 s | 2 cœurs partagés |
| calculs simultanés | 1 | verrou global, le second utilisateur est prié de réessayer |

Mémoire mesurée sur le pire cas autorisé : 25 km, zone par défaut de 148 km² à Massy avec
petites routes, soit environ 39 000 arêtes après élagage.

| Mesure | Valeur |
|---|---|
| pic RSS du calcul (process neuf, recuit, disque par défaut) | 445 Mo |
| pic RSS avec l'ancien demi-disque (55 500 arêtes) | 463 Mo |
| pic RSS observé avec CP-SAT forcé sur 44 600 arêtes | 1,1 Go |

C'est loin des 2,7 Go de Streamlit Cloud, donc les plafonds ne sont pas abaissés. CP-SAT
n'est jamais lancé au-delà du seuil en mode automatique.

## Limites connues

- **Artefacts du MNT** : un passage sous une voie ferrée ou une route non taguée `bridge` ou
  `tunnel` produit des pics de quelques mètres. Le mode « Maximiser le D+ » peut les exploiter.
  Le lissage sur 3 points ne les efface pas.
- **Grands graphes** : à 55 000 arêtes, le recuit ne fait que ~2 700 itérations en 20 s. Le
  résultat est valide mais probablement loin de l'optimum. Des redémarrages multiples aideraient,
  car un warm-start de 4 s bat parfois un recuit de 20 s.
- **Overpass** : les instances publiques renvoient souvent 504 sous charge. L'ordre des
  endpoints et les tentatives avec backoff limitent le problème. Le cache disque est éphémère
  sur Streamlit Cloud.
- **Mode cible** : « hors de portée » est signalé par trois indicateurs. Le premier est une
  borne de sac à dos fractionnaire sur le D+ atteignable. Le deuxième est la borne CP-SAT sur
  l'erreur quand CP-SAT tourne. Le troisième est un écart de D+ supérieur à 10 %.

## Hors périmètre v1

Envoi vers Garmin ou Komoot, comptes
utilisateurs, multi-départs, préférence de surface, choix du sens de parcours.
