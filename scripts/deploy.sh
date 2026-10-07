#!/usr/bin/env bash
# Déploiement prod depuis le poste du fondateur (D36 : pas d'OIDC GitHub, SCP du compte géré).
# Reprend .github/workflows/deploy.yml : build (Lambda arm64 + front) du commit EXACT dans un
# git worktree jetable → zip S3 → tofu apply (nouvelle version) → smoke interne de cette version →
# release (alias live + site, infra/release.sh) → smoke public via CloudFront → sinon rollback auto.
#
#   AWS_PROFILE=optrail scripts/deploy.sh            # déploie origin/main
#   AWS_PROFILE=optrail scripts/deploy.sh <commit>   # autre commit (doit être poussé, CI verte)
#   AWS_PROFILE=optrail scripts/deploy.sh rollback   # release précédente (front + alias)
#
# Session : `aws login --profile optrail` (courte, aucune clé longue). Le compte est vérifié.
# Config non secrète : infra/prod.env. Secrets : ~/.config/optrail/private.env (600) :
#   TURNSTILE_SITEKEY, TURNSTILE_SECRET, ALERT_EMAIL, ADMIN_KEY (≥ 32 car., vide = admin désactivé)
#   [, CLOUDFLARE_API_TOKEN si ENABLE_CUSTOM_DOMAIN]. Aucune n'est visible des builds (revue infra H1).
# tofu apply demande confirmation (AUTO_APPROVE=1 pour l'éviter).
# Prérequis : rustup, cargo-lambda (pip, zig inclus), node/npm, tofu ≥ 1.10, jq, zip, aws ≥ 2.32.
set -euo pipefail

ACCOUNT=698766075762
REGION=eu-north-1
FUNCTION=optrail-api
RUST_VERSION=1.99.0 # même toolchain que ci.yml
# Smoke interne : Massy 10 km, D+ attendu ~430 m.
SMOKE_QUERY='lat=48.7309&lon=2.2713&distance_km=10'
SMOKE_MIN_DPLUS=300 SMOKE_MIN_M=9000 SMOKE_MAX_M=11000

die() { echo "deploy: $*" >&2; exit 1; }
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
export PATH="$HOME/.cargo/bin:$PATH" AWS_REGION=$REGION AWS_DEFAULT_REGION=$REGION FUNCTION

# --- Compte : jamais le profil default (autre compte) ---------------------------------------------
[[ ${AWS_PROFILE:-} && $AWS_PROFILE != default ]] || die "AWS_PROFILE requis (profil du compte Optrail, pas default)"
acct=$(aws sts get-caller-identity --query Account --output text) || die "session expirée : aws login --profile $AWS_PROFILE"
[[ $acct == "$ACCOUNT" ]] || die "mauvais compte : $acct (attendu $ACCOUNT)"
export ARTIFACTS="optrail-artifacts-$ACCOUNT"
STATE_BUCKET="optrail-tfstate-$ACCOUNT"

if [[ ${1:-} == rollback ]]; then
  tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
  aws s3 cp --only-show-errors "s3://$ARTIFACTS/releases/previous.json" "$tmp/prev.json" || die "pas de release précédente"
  export SITE_BUCKET="optrail-site-$ACCOUNT"
  DISTRIBUTION=$(aws cloudfront list-distributions --output text \
    --query "DistributionList.Items[?Comment=='optrail'].Id | [0]"); export DISTRIBUTION
  "$repo/infra/release.sh" "$(jq -r .sha "$tmp/prev.json")" "$(jq -r .version "$tmp/prev.json")"
  aws s3 cp --only-show-errors "$tmp/prev.json" "s3://$ARTIFACTS/releases/current.json"
  aws s3 rm --only-show-errors "s3://$ARTIFACTS/releases/previous.json"
  echo "rollback : $(jq -c . "$tmp/prev.json") remise en service"; exit 0
fi

# --- Configuration et secrets (jamais affichés) ---------------------------------------------------
set -a
# shellcheck source=/dev/null
. "$repo/infra/prod.env"
priv="$HOME/.config/optrail/private.env"
[[ -f $priv && $(stat -f %Lp "$priv" 2>/dev/null || stat -c %a "$priv") == 600 ]] || die "$priv absent ou pas en 600"
# shellcheck source=/dev/null
. "$priv"
set +a
# Revue infra H1 : les builds (build.rs, proc-macros, scripts npm) ne voient AUCUNE variable de private.env :
# les noms sont relus dans le fichier (jamais les valeurs), plus la liste connue par sûreté.
nobuild=(-u TURNSTILE_SECRET -u ALERT_EMAIL -u CLOUDFLARE_API_TOKEN -u ADMIN_KEY -u DEV_TURNSTILE_SECRET -u CLOUDFLARE_ACCOUNT_ID)
while IFS= read -r v; do nobuild+=(-u "$v"); done < <(sed -nE 's/^(export[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*)=.*/\2/p' "$priv")
case "${TURNSTILE_SITEKEY:-}" in "" | [123]x0000*) die "TURNSTILE_SITEKEY absente ou clé de test (private.env)" ;; esac
[[ ${TURNSTILE_SECRET:-} && ${ALERT_EMAIL:-} && ${DATA_VERSION:-} ]] || die "TURNSTILE_SECRET, ALERT_EMAIL ou DATA_VERSION manquant"
TILES_SOURCE=${TILES_SOURCE:-zip}
[[ $TILES_SOURCE == zip || $TILES_SOURCE == s3 ]] || die "TILES_SOURCE : zip ou s3 (infra/prod.env)"
[[ $ENABLE_CUSTOM_DOMAIN != true || ${CLOUDFLARE_API_TOKEN:-} ]] || die "CLOUDFLARE_API_TOKEN requis avec le domaine"

# --- Commit : poussé, CI verte (le ruleset de main l'exige déjà pour main) -----------------------
git -C "$repo" fetch -q origin
SHA=$(git -C "$repo" rev-parse --verify "${1:-origin/main}^{commit}")
git -C "$repo" branch -r --contains "$SHA" | grep -q . || die "$SHA n'est sur aucune branche poussée"
ci=$(gh api "repos/M-Garrigues/trail_opt/commits/$SHA/check-runs" \
  --jq '[.check_runs[] | select(.app.slug == "github-actions")] | if length == 0 then "absente" elif all(.conclusion == "success") then "success" else "rouge" end')
[[ $ci == success ]] || [[ ${SKIP_CI_CHECK:-} == 1 ]] || die "CI de $SHA : $ci (SKIP_CI_CHECK=1 pour forcer)"
[[ $SHA == "$(git -C "$repo" rev-parse origin/main)" ]] || echo "⚠ $SHA n'est pas origin/main"
echo "déploiement de $SHA (données $DATA_VERSION, dalles : $TILES_SOURCE) sur $acct/$REGION"

work=$(mktemp -d)
cleanup() { git -C "$repo" worktree remove --force "$work/src" 2>/dev/null || true; rm -rf "$work"; }
trap cleanup EXIT
git -C "$repo" worktree add -q --detach "$work/src" "$SHA"
src="$work/src"

# --- Build (aucun secret dans l'environnement des scripts de build, sauf la clé de site publique) --
rustup toolchain install "$RUST_VERSION" --profile minimal -t aarch64-unknown-linux-gnu >/dev/null
(cd "$src/engine" && env "${nobuild[@]}" \
  RUSTUP_TOOLCHAIN=$RUST_VERSION CARGO_TARGET_DIR="$repo/engine/target/deploy" \
  cargo lambda build --release --arm64 --locked --bin lambda)
(cd "$src/web" && env "${nobuild[@]}" \
  VITE_TURNSTILE_SITEKEY="$TURNSTILE_SITEKEY" sh -c 'npm ci --ignore-scripts --no-audit --no-fund && npm run build')

# --- Zip vers S3 : binaire seul (TILES_SOURCE=s3, D43) ou binaire + dalles (zip, D10) -------------
aws s3api head-object --bucket "$ARTIFACTS" --key "tiles/$DATA_VERSION/manifest.json" >/dev/null 2>&1 \
  || die "s3://$ARTIFACTS/tiles/$DATA_VERSION/manifest.json absent (infra/README.md étape 3)"
mkdir -p "$work/pkg"
install -m 755 "$repo/engine/target/deploy/lambda/lambda/bootstrap" "$work/pkg/bootstrap"
if [[ $TILES_SOURCE == zip ]]; then
  aws s3 sync --only-show-errors "s3://$ARTIFACTS/tiles/$DATA_VERSION/" "$work/pkg/tiles/"
  TF_VAR_lambda_env=$(jq -nc --arg v "$DATA_VERSION" '{DATA_VERSION: $v, TILES_DIR: "/var/task/tiles"}')
else
  TF_VAR_lambda_env=$(jq -nc --arg v "$DATA_VERSION" --arg b "$ARTIFACTS" \
    '{DATA_VERSION: $v, TILES_S3: "s3://\($b)/tiles/\($v)/"}')
fi
(cd "$work/pkg" && zip -qr9 ../lambda.zip .)
aws s3 cp --only-show-errors "$work/lambda.zip" "s3://$ARTIFACTS/lambda/$SHA.zip"

# --- tofu apply (infra du commit déployé) ---------------------------------------------------------
export TF_VAR_lambda_s3_key="lambda/$SHA.zip" TF_VAR_alert_email="$ALERT_EMAIL" \
  TF_VAR_turnstile_secret="$TURNSTILE_SECRET" TF_VAR_cloudflare_api_token="${CLOUDFLARE_API_TOKEN:-}" \
  TF_VAR_reserved_concurrency="$LAMBDA_RESERVED_CONCURRENCY" TF_VAR_enable_custom_domain="$ENABLE_CUSTOM_DOMAIN" \
  TF_VAR_cloudflare_zone_id="$CLOUDFLARE_ZONE_ID" TF_VAR_cloudfront_hostname="$CLOUDFRONT_HOSTNAME"
export TF_VAR_lambda_env TF_VAR_tiles_source="$TILES_SOURCE" TF_VAR_admin_key="${ADMIN_KEY:-}" # vide : admin désactivé
tf() { tofu -chdir="$src/infra" "$@"; }
tf init -input=false -backend-config="bucket=$STATE_BUCKET" >/dev/null
if [[ ${AUTO_APPROVE:-} == 1 ]]; then tf apply -input=false -auto-approve; else tf apply; fi
VERSION=$(tf output -raw lambda_version)
SITE_BUCKET=$(tf output -raw site_bucket); DISTRIBUTION=$(tf output -raw distribution_id)
export SITE_BUCKET DISTRIBUTION
URL=$(tf output -raw url)

# --- Smoke interne de la version publiée (appel IAM direct, sans authorizer = appel interne) -----
# Démarrage à froid : en mode s3, manifeste + repères à l'init puis la dalle de Massy téléchargée
# (≈ 1 s en plus) ; la réussite prouve la lecture S3 (zip sans dalles). Délai client 60 s > timeout
# Lambda 30 s.
jq -n --arg q "$SMOKE_QUERY" '{
  version: "2.0", routeKey: "$default", rawPath: "/api/plan", rawQueryString: $q,
  headers: {}, isBase64Encoded: false,
  queryStringParameters: ($q | split("&") | map(split("=") | {(.[0]): .[1]}) | add),
  requestContext: {routeKey: "$default", stage: "$default", requestId: "smoke",
    http: {method: "GET", path: "/api/plan", protocol: "HTTP/1.1", sourceIp: "127.0.0.1", userAgent: "smoke"}}
}' > "$work/event.json"
aws lambda invoke --function-name "$FUNCTION:$VERSION" --cli-binary-format raw-in-base64-out \
  --cli-read-timeout 60 --payload "file://$work/event.json" "$work/out.json" > "$work/meta.json"
if jq -e '.FunctionError' "$work/meta.json" >/dev/null || ! jq -e '.statusCode == 200' "$work/out.json" >/dev/null; then
  cat "$work/out.json"; die "smoke interne raté (version $VERSION, alias live inchangé)"
fi
jq -c '.body | fromjson | .candidates[0] | {length_m, dplus_m}' "$work/out.json"
jq -e --argjson d "$SMOKE_MIN_DPLUS" --argjson lo "$SMOKE_MIN_M" --argjson hi "$SMOKE_MAX_M" \
  '.body | fromjson | .candidates[0] | .dplus_m >= $d and .length_m >= $lo and .length_m <= $hi' \
  "$work/out.json" >/dev/null || die "smoke interne : boucle hors bornes (alias live inchangé)"

# --- Archive du site de ce commit (source des releases et rollbacks) ------------------------------
aws s3 cp --recursive --only-show-errors "$src/web/dist/assets/" "s3://$ARTIFACTS/site/$SHA/assets/" \
  --cache-control "public, max-age=31536000, immutable"
aws s3 cp --recursive --only-show-errors --exclude 'assets/*' "$src/web/dist/" "s3://$ARTIFACTS/site/$SHA/" \
  --cache-control "no-cache"
aws s3 cp --only-show-errors "s3://$ARTIFACTS/releases/current.json" "$work/prev.json" 2>/dev/null \
  || { rm -f "$work/prev.json"; echo "première release : pas de rollback possible"; }

# --- Release, smoke public, rollback automatique --------------------------------------------------
rollback() {
  [[ -f $work/prev.json ]] || die "smoke public raté, pas de release précédente : site à réparer à la main"
  "$src/infra/release.sh" "$(jq -r .sha "$work/prev.json")" "$(jq -r .version "$work/prev.json")"
  die "smoke public raté : release précédente remise en service"
}
"$src/infra/release.sh" "$SHA" "$VERSION" || rollback
public_ok() {
  local code
  for _ in $(seq 10); do
    code=$(curl -s -o "$work/index.html" -w '%{http_code}' "$URL/") && [[ $code == 200 ]] && break
    echo "GET / → $code, nouvel essai"; sleep 15
  done
  [[ $code == 200 ]] && grep -qi '<!doctype html' "$work/index.html" || return 1
  code=$(curl -s -o "$work/api.json" -w '%{http_code}' "$URL/api/plan?$SMOKE_QUERY")
  echo "GET /api/plan sans jeton → $code $(head -c 300 "$work/api.json")"
  [[ $code == 403 ]] && jq -e '.error.code == "bot_check_failed"' "$work/api.json" >/dev/null
}
public_ok || rollback

if [[ -f $work/prev.json ]]; then aws s3 cp --only-show-errors "$work/prev.json" "s3://$ARTIFACTS/releases/previous.json"; fi
jq -n --arg sha "$SHA" --arg v "$VERSION" '{sha: $sha, version: $v}' > "$work/current.json"
aws s3 cp --only-show-errors "$work/current.json" "s3://$ARTIFACTS/releases/current.json"
echo "OK : $URL (version $VERSION, $SHA)"
