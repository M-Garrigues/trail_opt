#!/usr/bin/env bash
# Copie la release GitHub `tiles-<version>` vers s3://<bucket>/tiles/<version>/ depuis le poste du CEO
# (pas d'OIDC, D36), un zip à la fois (télécharge, décompresse, vérifie les sha256 du manifeste, sync,
# supprime) : jamais plus de ~2 zips de disque local. Le manifeste est copié EN DERNIER : tant que la
# copie n'est pas finie, deploy.sh refuse la version. Relançable (--size-only) ; préfixe neuf, jamais
# réécrit (refus si un manifeste différent s'y trouve déjà).
#
#   AWS_PROFILE=optrail scripts/tiles_to_s3.sh bdtopo-wfs-2026-10e
# Prérequis : gh (connecté), aws (session `aws login --profile optrail`), jq, unzip, shasum.
set -euo pipefail

BUCKET=${ARTIFACTS_BUCKET:-optrail-artifacts-698766075762}
ACCOUNT=698766075762
export AWS_REGION=eu-north-1 AWS_DEFAULT_REGION=eu-north-1

die() { echo "tiles_to_s3: $*" >&2; exit 1; }
v=${1:-}
[[ $v =~ ^bdtopo-wfs-[0-9]{4}-[0-9]{2}[a-z]?$ ]] || die "usage : $0 <data_version> (ex. bdtopo-wfs-2026-10e)"
[[ ${AWS_PROFILE:-} && ${AWS_PROFILE:-} != default ]] || die "AWS_PROFILE=optrail requis (le profil default vise un autre compte)"
[[ $(aws sts get-caller-identity --query Account --output text) == "$ACCOUNT" ]] || die "mauvais compte AWS"

dest="s3://$BUCKET/tiles/$v"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh release download "tiles-$v" -p manifest.json -p pois.json -D "$work"
man=$work/manifest.json
[[ $(jq -r .data_version "$man") == "$v" ]] || die "data_version du manifeste différente de $v"
if aws s3 cp --only-show-errors "$dest/manifest.json" "$work/remote.json" 2>/dev/null; then
  cmp -s "$man" "$work/remote.json" || die "$dest contient déjà un AUTRE manifeste : préfixe jamais réécrit"
fi
jq -r '.tiles | to_entries[] | "\(.value.sha256)  \(.key).npz"' "$man" | sort -k2 > "$work/sums"
echo "$(jq -r .pois.sha256 "$man")  pois.json" | (cd "$work" && shasum -a 256 -c --quiet -) || die "pois.json : sha256 faux"

zips=$(gh release view "tiles-$v" --json assets -q '.assets[].name' | grep -E "^tiles-$v-[0-9]+\.zip$" | sort || true)
[[ $zips ]] || die "release tiles-$v : aucun zip de dalles"

: > "$work/seen"
for z in $zips; do
  echo "== $z"
  gh release download "tiles-$v" -p "$z" -D "$work"
  unzip -q -o "$work/$z" -d "$work/x"
  rm -f "$work/$z"
  (cd "$work/x" && find . -name '*.npz' | sed 's|^\./||' | sort) > "$work/names"
  # sha256 de chaque dalle du zip contre le manifeste (une dalle hors manifeste = erreur)
  join -1 2 -2 1 -o 1.1,1.2 "$work/sums" "$work/names" | sed 's/ /  /' > "$work/part"
  [[ $(wc -l < "$work/part") -eq $(wc -l < "$work/names") ]] || die "$z : dalle absente du manifeste"
  (cd "$work/x" && shasum -a 256 -c --quiet "$work/part") || die "$z : sha256 faux"
  cat "$work/names" >> "$work/seen"
  aws s3 sync --only-show-errors --size-only "$work/x/" "$dest/"
  rm -rf "$work/x"
done
[[ $(sort -u "$work/seen" | wc -l) -eq $(wc -l < "$work/sums") ]] || die "les zips ne contiennent pas toutes les dalles du manifeste"

# Repères puis manifeste (dernier).
aws s3 cp --only-show-errors "$work/pois.json" "$dest/pois.json"
aws s3 cp --only-show-errors "$man" "$dest/manifest.json"

# Vérification finale côté S3 : chaque dalle du manifeste avec sa taille, plus pois.json et manifest.json.
{ jq -r --arg v "$v" '.tiles | to_entries[] | "tiles/\($v)/\(.key).npz \(.value.bytes)"' "$man"
  jq -r --arg v "$v" '"tiles/\($v)/pois.json \(.pois.bytes)"' "$man"
  echo "tiles/$v/manifest.json $(wc -c < "$man" | tr -d ' ')"
} | sort > "$work/expected"
aws s3 ls --recursive "$dest/" | awk '{print $4, $3}' | sort > "$work/actual"
missing=$(comm -23 "$work/expected" "$work/actual" | wc -l)
((missing == 0)) || { comm -23 "$work/expected" "$work/actual" | head; die "$missing objets manquants ou de taille différente"; }
echo "OK : $(wc -l < "$work/expected" | tr -d ' ') objets dans $dest (sha256 vérifiés avant envoi, tailles après)"
echo "Suite : infra/README.md § Dalles (DATA_VERSION=$v dans infra/prod.env, puis scripts/deploy.sh)"
