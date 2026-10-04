#!/usr/bin/env bash
# Mesure des durées de calcul sur la Lambda RÉELLE (I1, D18/D32), à lancer après l'apply.
# Invoque directement une version publiée (IAM, événement Function URL sans authorizer = appel
# interne, comme le smoke test de deploy.yml : pas de Turnstile), Massy 10/50/100 km × n = 1/2.
#
#   AWS_PROFILE=<profil du compte optrail> scripts/lambda_bench.sh <version|alias> [répétitions]
#
# Sortie (TSV) : km, n, essai, statut, compute_s (moteur), durée Lambda (ms, REPORT), mémoire max (Mo),
# init (ms, démarrage à froid seulement), longueur, D+. Le premier essai de chaque série peut être
# à froid. Coût : ~18 appels × ≤ 15 s × 3 Go ≈ 800 Go-s (offre gratuite : 400 000 Go-s/mois).
# Ensuite : recaler `estimateS` (api.md § Durée estimée) et les cibles D18 (p50 < 2 s à 10 km,
# < 6 s à 100 km) sur ces valeurs.
set -euo pipefail

QUALIFIER=${1:?usage : AWS_PROFILE=… $0 <version|alias> [répétitions]}
REPS=${2:-3}
FUNCTION=${FUNCTION:-optrail-api}
REGION=${AWS_REGION:-eu-west-3}
: "${AWS_PROFILE:?passer le profil AWS explicitement (le profil default vise un autre compte, D28)}"
command -v jq >/dev/null || { echo "jq requis" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf 'km\tn\trep\tstatus\tcompute_s\tduration_ms\tmax_mem_mb\tinit_ms\tlength_m\tdplus_m\n'
for km in 10 50 100; do
  for n in 1 2; do
    q="lat=48.7309&lon=2.2713&distance_km=$km&n_candidates=$n"
    jq -n --arg q "$q" '{
      version: "2.0", routeKey: "$default", rawPath: "/api/plan", rawQueryString: $q,
      headers: {}, isBase64Encoded: false,
      queryStringParameters: ($q | split("&") | map(split("=") | {(.[0]): .[1]}) | add),
      requestContext: {routeKey: "$default", stage: "$default", requestId: "bench",
        http: {method: "GET", path: "/api/plan", protocol: "HTTP/1.1", sourceIp: "127.0.0.1", userAgent: "bench"}}
    }' > "$tmp/event.json"
    for rep in $(seq 1 "$REPS"); do
      aws lambda invoke --region "$REGION" --function-name "$FUNCTION" --qualifier "$QUALIFIER" \
        --cli-binary-format raw-in-base64-out --cli-read-timeout 60 --log-type Tail \
        --payload "file://$tmp/event.json" "$tmp/out.json" > "$tmp/meta.json"
      report=$(jq -r '.LogResult // ""' "$tmp/meta.json" | base64 --decode | grep '^REPORT' || true)
      field() { sed -nE "s/.*$1: ([0-9.]+).*/\\1/p" <<<"$report"; }
      body=$(jq -r '.body // "{}"' "$tmp/out.json")
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$km" "$n" "$rep" \
        "$(jq -r '.statusCode // "?"' "$tmp/out.json")" \
        "$(jq -r '.compute_s // "" | tostring | .[0:5]' <<<"$body")" \
        "$(field Duration)" "$(field 'Max Memory Used')" "$(field 'Init Duration')" \
        "$(jq -r '.candidates[0].length_m // ""' <<<"$body")" "$(jq -r '.candidates[0].dplus_m // ""' <<<"$body")"
    done
  done
done
