#!/usr/bin/env bash
# Serveur de développement de l'API : `cargo lambda watch` sert le handler (binaire `lambda`)
# avec recompilation à chaud, dalles locales et clés Turnstile de TEST Cloudflare.
#
#   scripts/dev.sh                       # API sur http://localhost:$API_PORT
#   TILES_DIR=scripts/experiments/tiles_v1 scripts/dev.sh
#
# Appel : curl -H 'X-Turnstile-Token: XXXX.DUMMY.TOKEN.XXXX' \
#   "http://localhost:9000/lambda-url/lambda/api/plan?lat=48.7309&lon=2.2713&distance_km=10"
# Attention : `cargo lambda watch` n'émule pas `requestContext.authorizer`, donc chaque appel y est
# « interne » et Turnstile n'est pas vérifié (en prod, la Function URL AWS_IAM l'impose, D23).
# Pour tester Turnstile de bout en bout : FORCE_TURNSTILE=1 scripts/dev.sh (tout appel est alors
# traité comme externe : jeton exigé, siteverify appelé). Avec la clé secrète de test par défaut,
# tout jeton non vide passe ; TURNSTILE_SECRET=2x0000000000000000000000000000000AA les refuse tous
# (→ 403 bot_check_failed).
#
# Partage (POST /api/loops, GET /api/loops/<id>) : sans SHARED_BUCKET, les boucles sont écrites dans
# $SHARE_DIR (défaut engine/target/shared, rien n'expire en local).
#
# Front : si web/node_modules existe (cd web && npm ci), Vite est lancé aussi sur http://localhost:$WEB_PORT
# (proxy /api → cette API dans web/vite.config.ts, même origine qu'en prod). WEB=0 pour l'API seule.
# Côté navigateur, clé de site Turnstile de test invisible 1x00000000000000000000BB (passe toujours).
#
# Profil cargo `local` (release + debug_assertions) : seul build qui accepte les clés de test,
# FORCE_TURNSTILE et TURNSTILE_DISABLED (ignorés/refusés en release, T28). Signature des boucles :
# clé aléatoire à chaque démarrage si LOOP_SIGNING_KEY est absent.
#
# Prérequis : cargo-lambda (`pip install cargo-lambda`, zig inclus) dans le PATH.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
export API_PORT=${API_PORT:-9000}
export WEB_PORT=${WEB_PORT:-5173}
TILES_DIR=${TILES_DIR:-$ROOT/scripts/experiments/tiles_pilote}
TILES_DIR=$(cd "$TILES_DIR" && pwd) # chemin absolu (cargo lambda change de dossier)
# Clé secrète de test Cloudflare : siteverify répond toujours « success ».
TURNSTILE_SECRET=${TURNSTILE_SECRET:-1x0000000000000000000000000000000AA}
FORCE_TURNSTILE=${FORCE_TURNSTILE:-0}
SHARE_DIR=${SHARE_DIR:-$ROOT/engine/target/shared}

command -v cargo-lambda >/dev/null || { echo "cargo-lambda introuvable : pip install cargo-lambda" >&2; exit 1; }
test -f "$TILES_DIR/manifest.json" || { echo "pas de manifest.json dans $TILES_DIR" >&2; exit 1; }

echo "API : http://localhost:$API_PORT/lambda-url/lambda/api/plan  (dalles : $TILES_DIR, partage : $SHARE_DIR, FORCE_TURNSTILE=$FORCE_TURNSTILE)"
if [ "${WEB:-1}" = 1 ] && [ -d "$ROOT/web/node_modules" ]; then
  # LAN=1 : écoute sur le réseau local en HTTPS auto-signé (test sur téléphone)
  if [ "${LAN:-0}" = 1 ]; then host_opt=--host; scheme=https; else host_opt=; scheme=http; fi
  (cd "$ROOT/web" && npm run dev -- --port "$WEB_PORT" --strictPort $host_opt) &
  trap 'trap - EXIT INT TERM; kill 0 2>/dev/null' EXIT INT TERM
  echo "Front : $scheme://localhost:$WEB_PORT"
  [ "${LAN:-0}" = 1 ] && echo "Réseau local : $scheme://$(ipconfig getifaddr en0 2>/dev/null || hostname -I | cut -d' ' -f1):$WEB_PORT"
fi
cd "$ROOT/engine"
cargo lambda watch --profile local --bin lambda --invoke-port "$API_PORT" \
  --env-var "TILES_DIR=$TILES_DIR" \
  --env-var "TURNSTILE_SECRET=$TURNSTILE_SECRET" \
  --env-var "FORCE_TURNSTILE=$FORCE_TURNSTILE" \
  --env-var "SHARE_DIR=$SHARE_DIR"
