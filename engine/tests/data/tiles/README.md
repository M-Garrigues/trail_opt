# Dalle de test (tiles/1)

Extrait de quelques disques des dalles IdF + Isère, au format et à la `data_version` de
`manifest.json` : Massy (plaine, 6 km), Bourg-d'Oisans (7,5 km) et Alpes (6 km). Utilisé par les
tests bout en bout du moteur (`tests/common/mod.rs`) et vérifié par `tests/test_tiles.py`.

Régénérer (après un changement de `DATA_VERSION`, depuis un dossier complet à jour) :

    python -m pipeline clip --tiles <dossier tiles/1> --out engine/tests/data/tiles \
      --disk 48.7309,2.2713,6000 --disk 45.0555,6.0310,7500 --disk 45.0920,6.0700,6000

Données dérivées de la BD TOPO®, du RGE ALTI® et du LiDAR HD de l'IGN, sous Licence Ouverte
Etalab 2.0 (https://www.etalab.gouv.fr/licence-ouverte-open-licence/). Source : IGN, données
téléchargées en octobre 2026 ; modifiées (découpage, altitudes lissées et corrigées).
