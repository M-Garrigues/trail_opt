# Infrastructure optrail

OpenTofu ≥ 1.10, AWS eu-west-3 (+ us-east-1 pour le certificat ACM). Coût visé : 0 € (free tier).

```
navigateur ─► CloudFront ─┬─ /*      ─► S3 site (OAC)            shared/ expire à 90 j
                          │  (fonction optrail-spa : route sans extension → /index.html)
                          ├─ /api/loops* ─┐ toutes méthodes (POST : x-amz-content-sha256 requis)
                          └─ /api/*  ─────┴► Function URL AWS_IAM (OAC) ─► Lambda optrail-api:live
                                        GET, sans cache            arm64, 30 s, R/W shared/*
CloudWatch (logs 14 j, alarmes) ─► SNS e-mail
Calculs acceptés 5 min │ Budgets 1 $ / 5 $ ─► SNS ─► optrail-killswitch ─► concurrence de l'API = 0
                         (alarme : reprise auto 1 h plus tard via EventBridge Scheduler ; budget : manuelle)
```

En-têtes (politique `optrail-security`) : HSTS, nosniff, X-Frame-Options DENY, Referrer-Policy,
Permissions-Policy (`geolocation=(self)`, le reste coupé), CSP en **Report-Only** (cdn.tf, `local.csp`) :
vérifier la console sur le site, puis la passer en `Content-Security-Policy` (bloquante).
`/shared/*` n'est jamais servi par CloudFront (403) : les boucles partagées passent par `/api/loops/<id>`.

- `bootstrap/` : état local, appliqué une fois à la main. Bucket d'état (TLS obligatoire), bucket
  d'artefacts, fournisseur OIDC GitHub, rôle `optrail-deploy` (main + environnement `prod` seulement ;
  ne peut ni se modifier, ni ouvrir une Function URL hors AWS_IAM, ni rendre une Lambda publique).
- `./` : tout le reste, appliqué par `.github/workflows/deploy.yml`.
- Domaine : `enable_custom_domain = false` par défaut (site sur `*.cloudfront.net`).

## Mise en service (fondateur, une fois)

Détail ordonné, qui fait quoi : `.team/handoffs/2026-10-04-infra-mise-en-ligne.md` (hors dépôt).

0. **Compte AWS** `Optrail` (698766075762), plan Free (passer en Paid avant le 2027-04-04),
   membre de l'organisation du fondateur. Accès CLI : `aws login` (sessions courtes), jamais de clés
   longues. Toutes les commandes : `export AWS_PROFILE=<profil du compte Optrail>` (le profil
   `default` de cette machine vise un autre compte). Les SCP de l'organisation doivent autoriser
   eu-west-3 (+ us-east-1 pour ACM/CloudFront) et `iam:*OpenIDConnectProvider*` sur ce compte.
   Quota : `aws lambda get-account-settings --region eu-west-3 --query AccountLimit.ConcurrentExecutions`
   (10 sur un compte neuf ; s'il dépasse 10, variable GitHub `LAMBDA_RESERVED_CONCURRENCY=10`).
1. **Bootstrap** :
   ```bash
   cd infra/bootstrap && tofu init && tofu plan -out=bootstrap.tfplan && tofu apply bootstrap.tfplan
   ```
   Garder `terraform.tfstate` hors du dépôt (il est ignoré par git, ne contient pas de secret).
2. **GitHub** (dépôt `M-Garrigues/trail_opt`, avec le compte gh PERSONNEL) :
   - `sub` OIDC avec la branche (exigé par la politique de confiance). Le dépôt est en `sub`
     immuable (`use_immutable_subject: true`) : la confiance attend
     `repo:M-Garrigues@22774745/trail_opt@1402132975:environment:prod:ref:refs/heads/main`
     (variable `github_sub_prefix` du bootstrap). Personnalisation, puis vérification :
     ```bash
     gh api -X PUT repos/M-Garrigues/trail_opt/actions/oidc/customization/sub \
       -F use_default=false -f 'include_claim_keys[]=repo' \
       -f 'include_claim_keys[]=context' -f 'include_claim_keys[]=ref'
     gh api repos/M-Garrigues/trail_opt/actions/oidc/customization/sub
     # attendu : use_default=false, use_immutable_subject=true, même sub_claim_prefix
     ```
     Si `use_immutable_subject` repasse à false, remettre `github_sub_prefix = "repo:M-Garrigues/trail_opt"`.
   - Environnement `prod` : branches de déploiement = `main` seulement.
   - Variables de `prod` : `AWS_DEPLOY_ROLE_ARN`, `TF_STATE_BUCKET`, `ARTIFACTS_BUCKET`
     (sorties du bootstrap), `ALERT_EMAIL`, `DATA_VERSION`.
     Domaine : `ENABLE_CUSTOM_DOMAIN=true`, `CLOUDFLARE_ZONE_ID`.
   - Variable du **dépôt** (pas de l'environnement : le build tourne sans environnement ni accès AWS) :
     `TURNSTILE_SITEKEY` (publique). Absente ou clé de test (`1x0000…`) → le build échoue.
   - Secrets de `prod` : `TURNSTILE_SECRET`, `CLOUDFLARE_API_TOKEN`
     (jeton limité à Zone → DNS → Edit sur la seule zone optrail.eu).
   - `LOOP_SIGNING_KEY` (signature des boucles, M3) : aucun secret à créer, générée par Tofu
     (`random_password.loop_signing`, état chiffré). Rotation :
     `tofu apply -replace=random_password.loop_signing` (boucles calculées pas encore partagées → 400).
   - `TURNSTILE_HOSTNAMES` (siteverify) = `optrail.eu` + `CLOUDFRONT_HOSTNAME` tant que
     `ENABLE_CUSTOM_DOMAIN` est faux. Après le premier apply : variable de `prod`
     `CLOUDFRONT_HOSTNAME` = domaine de la sortie `url` (`dxxxx.cloudfront.net`), puis redéployer ;
     sans elle, Turnstile échoue sur *.cloudfront.net (`bot_check_failed`).
3. **Dalles pilotes** (D10), tant que `data.yml` n'est pas branché :
   `aws s3 sync <dossier tiles/1> s3://<ARTIFACTS_BUCKET>/tiles/<DATA_VERSION>/`
4. **Cloudflare** (zone optrail.eu, DNSSEC actif) : supprimer les enregistrements importés d'OVH
   à l'apex (A/AAAA, TXT de redirection) avant le premier déploiement avec domaine, sinon le CNAME
   apex échoue. Widget Turnstile pour `optrail.eu` + `localhost` (+ `dxxxx.cloudfront.net` tant que
   le domaine perso est inactif).
5. Pousser sur `main` → `ci` → (verte) → `deploy`. Confirmer les deux e-mails d'abonnement SNS.
6. Après le premier apply : vérifier que le budget voit un coût (Billing → Budgets → `optrail-monthly`,
   coût brut, crédits exclus) et que le filtre de métriques compte (CloudWatch → Métriques → `optrail`
   → `AcceptedComputeSeconds` après un calcul réel ; exige `accepted: true` dans le log du handler).

## Actions fondateur restantes (revue sécurité 2026-10-04)

- **E2** : supprimer l'utilisateur IAM `mathieu` (AdministratorAccess, sans MFA) du compte 698766075762 ;
  accès par Identity Center seulement, root géré centralement depuis le compte de gestion.
- **M1** : environnement GitHub `prod` (branche `main` seule, relecteur requis = soi-même si voulu) ;
  ruleset sur `main` (pas de force-push ni suppression, CI `ci` requise) ; clé de déploiement locale
  retirée ; travailler avec le compte gh **personnel** (`gh auth switch`), pas `MGarrigues-pasqal`.
- **M4** : supprimer la clé longue `AKIA…` du profil `default` (compte 684578650133) au profit d'`aws login`.
- **SCP** (D28) : ne lever le refus OIDC que pour `iam:CreateOpenIDConnectProvider`,
  `iam:UpdateOpenIDConnectProviderThumbprint`, `iam:AddClientIDToOpenIDConnectProvider`,
  `iam:TagOpenIDConnectProvider`, `iam:DeleteOpenIDConnectProvider` (`iam:*OpenIDConnectProvider*`)
  et pour le seul compte 698766075762 (condition `aws:PrincipalAccount`).
- **M5** : passer le compte en plan Paid avant ouverture large (et avant le 2027-04-04).

## Déploiement (`.github/workflows/deploy.yml`)

`ci` verte sur un push de `main` → `deploy` (événement `workflow_run`) :
1. `gate` : le commit est encore la tête de `main` (sinon rien).
2. `build`, **sans jeton OIDC ni secret** : Lambda arm64 (Rust épinglé, cargo-lambda/zig installés
   par hash) et front (`npm ci --ignore-scripts`), passés en artefacts.
3. `deploy` (environnement `prod`) : zip + dalles → S3, `tofu apply` (nouvelle version Lambda),
   smoke interne de cette version, archive du front dans `s3://<artefacts>/site/<sha>/`,
   **release** (`infra/release.sh` : alias `live` + site, assets à empreinte `immutable` jamais
   supprimés, `index.html` & co en `no-cache`), **smoke public** via CloudFront (`GET /` → 200,
   `GET /api/plan` sans jeton → 403 `bot_check_failed`), puis pointeurs
   `s3://<artefacts>/releases/{current,previous}.json`. Smoke public raté → la release précédente
   (front + alias) est remise automatiquement.

Redéployer un commit : « Re-run all jobs » sur son exécution de `deploy`.

## Exploitation

- **Rollback** manuel : Actions → deploy → Run workflow (sur `main`) → remet `previous.json`
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
