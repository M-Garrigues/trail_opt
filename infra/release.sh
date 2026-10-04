#!/usr/bin/env bash
# Met en service une release complète : alias Lambda `live` + site, depuis l'archive
# s3://$ARTIFACTS/site/<sha>/ écrite par deploy.yml. Sert au déploiement, au rollback automatique
# (smoke public raté) et au rollback manuel : front et API reviennent toujours ensemble.
# Env : ARTIFACTS, SITE_BUCKET, DISTRIBUTION, FUNCTION. Usage : release.sh <sha> <version Lambda>
set -euo pipefail
sha=$1 version=$2
[[ $sha =~ ^[0-9a-f]{40}$ && $version =~ ^[0-9]+$ ]] || { echo "::error::release invalide : '$sha' '$version'"; exit 1; }
src="s3://$ARTIFACTS/site/$sha"

aws lambda update-alias --function-name "$FUNCTION" --name live --function-version "$version" --query FunctionVersion
# assets/ d'abord : noms à empreinte, jamais supprimés (un onglet ouvert sur une ancienne version les
# charge encore). Puis index.html et le reste. `cp` et non `sync` : un rollback copie des objets plus
# anciens que ceux en place. Cache-Control vient de l'archive (immutable / no-cache), métadonnées copiées.
aws s3 cp --recursive --only-show-errors "$src/assets/" "s3://$SITE_BUCKET/assets/"
aws s3 cp --recursive --only-show-errors --exclude 'assets/*' "$src/" "s3://$SITE_BUCKET/"
aws cloudfront create-invalidation --distribution-id "$DISTRIBUTION" --paths '/*' --query Invalidation.Id
