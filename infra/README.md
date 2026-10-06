# Infrastructure optrail

OpenTofu ≥ 1.10, AWS **eu-north-1** (Stockholm, UE ; seule région permise par les SCP du compte géré, D36)
+ us-east-1 pour le certificat ACM / CloudFront. Coût visé : 0 € (free tier).

```
navigateur ─► CloudFront ─┬─ /*      ─► S3 site (OAC)            shared/ expire à 90 j
                          │  (fonction optrail-spa : route sans extension → /index.html)
                          ├─ /api/loops* ─┐ toutes méthodes (POST : x-amz-content-sha256 requis)
                          └─ /api/*  ─────┴► Function URL AWS_IAM (OAC) ─► Lambda optrail-api:live
                                        GET, sans cache            arm64, 30 s, R/W shared/*,
                                                                   lecture tiles/* (artefacts) en mode s3
CloudWatch (logs 14 j, alarmes) ─► SNS e-mail
Calculs acceptés 5 min │ Budgets 1 $ / 5 $ ─► SNS ─► optrail-killswitch ─► concurrence de l'API = 0
                         (alarme : reprise auto 1 h plus tard via EventBridge Scheduler ; budget : manuelle)
```

En-têtes (politique `optrail-security`) : HSTS, nosniff, X-Frame-Options DENY, Referrer-Policy,
Permissions-Policy (`geolocation=(self)`, le reste coupé), CSP en **Report-Only** (cdn.tf, `local.csp`) :
vérifier la console sur le site, puis la passer en `Content-Security-Policy` (bloquante).
`/shared/*` n'est jamais servi par CloudFront (403) : les boucles partagées passent par `/api/loops/<id>`.

- `bootstrap/` : état local, appliqué une fois à la main. Bucket d'état (TLS obligatoire), bucket
  d'artefacts, boundary `optrail-lambda-boundary` des rôles Lambda. Fournisseur OIDC GitHub + rôle
  `optrail-deploy` seulement si `enable_github_oidc = true` (faux : OIDC refusé par les SCP, D36).
- `./` : tout le reste, appliqué par `scripts/deploy.sh` depuis le poste du fondateur (session `aws login`).
  `.github/workflows/deploy.yml` est désactivé (nécessite l'OIDC) ; la CI GitHub (`ci.yml`) n'a aucun accès AWS.
- Configuration non secrète de prod : `infra/prod.env`. Secrets : `~/.config/optrail/private.env` (600, hors dépôt).
- Domaine : `ENABLE_CUSTOM_DOMAIN=false` par défaut (site sur `*.cloudfront.net`).

## Mise en service (une fois)

0. **Compte AWS** `Optrail` (698766075762), plan Free (passer en Paid avant le 2027-04-04). Compte
   « géré » (rôle `managed/AccountFullAccessRole`) : SCP non modifiables (eu-north-1 seule + us-east-1
   pour les services globaux ; OIDC refusé). Accès CLI : `aws login --profile optrail`
   (sessions courtes), jamais de clés longues ; toujours `AWS_PROFILE=optrail` (le profil
   `default` de cette machine vise un autre compte).
   Quota : `aws lambda get-account-settings --region eu-north-1 --query AccountLimit.ConcurrentExecutions`
   (10 sur un compte neuf ; s'il dépasse 10, `LAMBDA_RESERVED_CONCURRENCY=10` dans `infra/prod.env`).
1. **Bootstrap** :
   ```bash
   cd infra/bootstrap && AWS_PROFILE=optrail tofu init \
     && AWS_PROFILE=optrail tofu plan -out=bootstrap.tfplan \
     && AWS_PROFILE=optrail tofu apply bootstrap.tfplan
   ```
   Garder `terraform.tfstate` hors du dépôt (il est ignoré par git, ne contient pas de secret).
2. **Secrets** dans `~/.config/optrail/private.env` (600) : `TURNSTILE_SITEKEY` (publique mais propre à
   la prod ; absente ou clé de test `1x0000…` → refus), `TURNSTILE_SECRET`, `ALERT_EMAIL` (e-mail perso :
   jamais dans le dépôt), `CLOUDFLARE_API_TOKEN` (avec le domaine seulement).
   - `LOOP_SIGNING_KEY` (signature des boucles, M3) : rien à créer, générée par Tofu
     (`random_password.loop_signing`, état chiffré). Rotation :
     `tofu apply -replace=random_password.loop_signing` (boucles calculées pas encore partagées → 400).
   - `TURNSTILE_HOSTNAMES` (siteverify) = `optrail.eu` + `CLOUDFRONT_HOSTNAME` tant que
     `ENABLE_CUSTOM_DOMAIN` est faux. Après le premier déploiement : `CLOUDFRONT_HOSTNAME` = domaine de la
     sortie `url` (`dxxxx.cloudfront.net`) dans `infra/prod.env`, puis redéployer ; sans lui, Turnstile
     échoue sur *.cloudfront.net (`bot_check_failed`).
3. **Dalles** (D10) : `data.yml` publie la release GitHub `tiles-<v>` (`tiles-<v>-NN.zip` de 300 dalles,
   `manifest.json`, `pois.json`, `coverage.geojson` ; v = `data_version` du manifeste, France ≈ 1,2 Go) ;
   copie vers S3 depuis le poste (pas d'OIDC) :
   ```sh
   AWS_PROFILE=optrail scripts/tiles_to_s3.sh bdtopo-wfs-2026-10e   # exemple ; ~10 min, ≤ 0,6 Go de disque
   ```
   Le script traite un zip à la fois (télécharge, vérifie les sha256 du manifeste, `aws s3 sync`, supprime),
   copie le manifeste en dernier, puis compare tailles et noms sur S3 au manifeste. Relançable. Préfixe neuf,
   jamais réécrit ni supprimé (rollback = remettre l'ancienne `DATA_VERSION`). Puis `DATA_VERSION` := `$v`
   dans `infra/prod.env` et redéployer ; deploy.sh échoue si `tiles/$v/manifest.json` manque. La France
   entière ne tient pas dans le zip Lambda : ne basculer sur une version France qu'avec `TILES_SOURCE=s3`
   dans `infra/prod.env` (lecture S3 à la demande). Procédure complète : pipeline/README.md.
4. **Cloudflare** (zone optrail.eu, DNSSEC actif) : supprimer les enregistrements importés d'OVH
   à l'apex (A/AAAA, TXT de redirection) avant le premier déploiement avec domaine, sinon le CNAME
   apex échoue. Widget Turnstile pour `optrail.eu` + `localhost` (+ `dxxxx.cloudfront.net` tant que
   le domaine perso est inactif).
5. Pousser sur `main` (ruleset : CI verte exigée) → `AWS_PROFILE=optrail scripts/deploy.sh`.
   Confirmer les deux e-mails d'abonnement SNS.
6. Après le premier apply : vérifier que le budget voit un coût (Billing → Budgets → `optrail-monthly`,
   coût brut, crédits exclus) et que le filtre de métriques compte (CloudWatch → Métriques → `optrail`
   → `AcceptedComputeSeconds` après un calcul réel ; exige `accepted: true` dans le log du handler).

## Actions fondateur restantes (revue sécurité 2026-10-04)

- **E2** : supprimer l'utilisateur IAM `mathieu` (AdministratorAccess, sans MFA) du compte 698766075762.
- **M1** : fait le 2026-10-05 (environnement `prod` limité à `main`, ruleset `main`). Reste : passer la
  clé de déploiement locale « mac local » en lecture seule ou la remplacer par le compte gh **personnel**.
- **M4** : supprimer la clé longue `AKIA…` du profil `default` (compte 684578650133) au profit d'`aws login`.
- **M5** : passer le compte en plan Paid avant ouverture large (et avant le 2027-04-04).

## Déploiement (`scripts/deploy.sh`, D36)

`AWS_PROFILE=optrail scripts/deploy.sh [<commit>]` (défaut `origin/main`) :
1. Contrôles : compte 698766075762, commit poussé, CI GitHub verte sur ce commit (`SKIP_CI_CHECK=1` pour forcer).
2. Build du commit **exact** dans un `git worktree` jetable (l'arbre de travail n'est pas touché),
   **sans secret** dans l'environnement des scripts de build : Lambda arm64 (Rust épinglé, cargo-lambda/zig)
   et front (`npm ci --ignore-scripts`, clé de site Turnstile de prod).
3. Zip + dalles → S3, `tofu apply` (confirmation demandée ; `AUTO_APPROVE=1` sinon), smoke interne de la
   nouvelle version, archive du front dans `s3://<artefacts>/site/<sha>/`, **release** (`infra/release.sh` :
   alias `live` + site, assets à empreinte `immutable` jamais supprimés, `index.html` & co en `no-cache`),
   **smoke public** via CloudFront (`GET /` → 200, `GET /api/plan` sans jeton → 403 `bot_check_failed`),
   puis pointeurs `s3://<artefacts>/releases/{current,previous}.json`. Smoke public raté → la release
   précédente (front + alias) est remise automatiquement.

## Exploitation

- **Rollback** manuel : `AWS_PROFILE=optrail scripts/deploy.sh rollback` → remet `previous.json`
  (front ET alias Lambda) ; il devient la release courante (un seul niveau de retour).

### Coupe-circuit (D32 B2/E3)

- Mesure : filtre de métriques sur le log JSON `plan` du handler, `{ $.msg = "plan" && $.accepted IS TRUE }`,
  somme de `compute_s` → `optrail/AcceptedComputeSeconds`. Seuls les calculs **acceptés** (Turnstile
  passé, ou appel interne) comptent : des jetons faux ne peuvent pas couper le service.
- Seuil (`killswitch_compute_s`, 900 s sur 5 min) : le free tier Lambda (400 000 Go-s/mois) vaut
  132 979 s/mois à 3,008 Go, ≈ 4 400 s/jour. 900 s/5 min = 3 calculs en parallèle sans arrêt
  (≈ 180 calculs de 5 s) : hors de portée d'un trafic de lancement. En saturation (quota 10),
  ≈ 4 000 s brûlées par cycle (détection ≤ 2 périodes + 1 h de pause), soit ≈ 1,6 $/jour brut au pire :
  payé par les crédits du plan Free et arrêté par le budget.
- Déclenchement : alarme `optrail-api-killswitch` → SNS → `optrail-killswitch` met la concurrence
  réservée à 0 et planifie `optrail-resume` (EventBridge Scheduler, unique, auto-supprimé) à +1 h, qui
  la retire. Nouveau déclenchement pendant la pause → reprise repoussée.
- Budgets `optrail-monthly` (1 $ et 5 $, coût **brut** : `include_credit = false`, sinon le plan Free à
  crédits afficherait 0) → même Lambda : pause **sans reprise automatique** (planification annulée).
  Reprise manuelle : `aws lambda delete-function-concurrency --function-name optrail-api`.
- Filet E3 : `optrail-api-duration-hour` (Duration brute > 3 600 s/h, jetons faux compris) → e-mail
  seulement.
- Pendant la pause, la Function URL renvoie **429** (throttle Lambda) : le front affiche `service_paused`
  (429 = pause ou quota saturé, indistinguables). Un déploiement ne rétablit jamais la concurrence.

### Couper le site entier (M5 : CloudFront et S3 ne sont pas couverts par le coupe-circuit)

```bash
export AWS_PROFILE=<profil du compte Optrail>
id=$(aws cloudfront list-distributions --query "DistributionList.Items[?Comment=='optrail'].Id | [0]" --output text)
aws cloudfront get-distribution-config --id "$id" > dc.json
jq '.DistributionConfig.Enabled = false | .DistributionConfig' dc.json > off.json
aws cloudfront update-distribution --id "$id" --if-match "$(jq -r .ETag dc.json)" --distribution-config file://off.json
```
Réactivation : même chose avec `Enabled = true`. Tant qu'elle est coupée, ne pas pousser sur `main`
(l'apply la remet en service : `enabled = true` dans cdn.tf, et le smoke public échouerait).

### Divers
- Le certificat par défaut `*.cloudfront.net` impose TLSv1 minimum : passer au domaine (TLSv1.2_2021) vite.
- Les vieux `assets/` restent dans le bucket du site (quelques centaines de Ko par déploiement) ;
  nettoyage manuel éventuel, jamais `--delete`.
- Vérifier localement : `tofu fmt -check -recursive infra` puis, dans chaque racine,
  `tofu init -backend=false && tofu validate`.
