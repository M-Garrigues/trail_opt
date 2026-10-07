# Lambda de calcul (D1, D10) : binaire Rust arm64 provided.al2023, zip (binaire, + dalles en mode
# tiles_source = "zip") déposé dans S3 par scripts/deploy.sh. Chaque changement publie une version ; l'alias `live` est
# basculé par scripts/deploy.sh après le smoke test (hors Tofu, d'où ignore_changes).

locals {
  artifacts_bucket = "optrail-artifacts-${local.account}" # créé par bootstrap
  api_name         = "optrail-api"
}

data "aws_iam_policy_document" "lambda_trust" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "api" {
  name                 = local.api_name
  assume_role_policy   = data.aws_iam_policy_document.lambda_trust.json
  permissions_boundary = local.boundary
}

resource "aws_iam_role_policy" "api" {
  name = "logs"
  role = aws_iam_role.api.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["logs:CreateLogStream", "logs:PutLogEvents"]
      Resource = "${aws_cloudwatch_log_group.api.arn}:*"
    }]
  })
}

# Boucles partagées (ui-spec §8) : lecture/écriture limitées à shared/ du bucket site. Sans
# ListBucket, un id absent rend AccessDenied (403) : le handler le traite comme 404.
# salt/<AAAA-MM-JJ> : sel quotidien aléatoire des visiteurs uniques (POST /api/hit, D51), créé par
# écriture conditionnelle à la première visite du jour, supprimé après 2 jours (cycle de vie).
resource "aws_iam_role_policy" "api_shared" {
  name = "shared-loops"
  role = aws_iam_role.api.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["s3:GetObject", "s3:PutObject"]
      Resource = ["${aws_s3_bucket.site.arn}/shared/*", "${aws_s3_bucket.site.arn}/salt/*"]
    }]
  })
}

# Dalles lues à la demande (D43, tiles_source = "s3") : GetObject sur tiles/* du bucket d'artefacts
# seulement. Sans ListBucket : une dalle absente rend 403, traité comme une erreur de source.
# Exige la même autorisation dans la boundary (infra/bootstrap/main.tf), sinon l'accès est refusé.
resource "aws_iam_role_policy" "api_tiles" {
  count = var.tiles_source == "s3" ? 1 : 0
  name  = "tiles-read"
  role  = aws_iam_role.api.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = ["s3:GetObject"]
      Resource = "arn:aws:s3:::${local.artifacts_bucket}/tiles/*"
    }]
  })
}

# Rétention 13 mois (D47 : statistiques admin) ; 395 j n'est pas une valeur admise par CloudWatch,
# 400 est la plus proche (≤ 25 mois de conservation exigés par la CNIL pour la mesure d'audience).
resource "aws_cloudwatch_log_group" "api" {
  name              = "/aws/lambda/${local.api_name}"
  retention_in_days = 400
}

# Admin (D47, contracts/admin.md) : GET /api/admin/stats lance des requêtes Logs Insights sur ce
# seul groupe. GetQueryResults n'accepte pas de ressource : « * » (lit un queryId connu seulement).
resource "aws_iam_role_policy" "api_stats" {
  name = "admin-stats"
  role = aws_iam_role.api.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = "logs:StartQuery"
        Resource = [aws_cloudwatch_log_group.api.arn, "${aws_cloudwatch_log_group.api.arn}:*"]
      },
      { Effect = "Allow", Action = "logs:GetQueryResults", Resource = "*" },
    ]
  })
}

# Clé HMAC des boucles (M3, api.md v1.3) : générée par Tofu, gardée dans l'état chiffré (S3,
# TLS seul), aucun secret GitHub à gérer. Rotation : tofu apply -replace=random_password.loop_signing
# (seules les boucles calculées et pas encore partagées deviennent impartageables).
resource "random_password" "loop_signing" {
  length  = 64
  special = false
}

# Noms d'hôte acceptés de siteverify (E3). Le domaine CloudFront ne peut pas venir de la
# distribution (cycle Lambda → URL → distribution) : variable remplie après le premier apply.
locals {
  turnstile_hostnames = join(",", compact([
    var.domain,
    var.enable_custom_domain ? "" : var.cloudfront_hostname,
  ]))
}

resource "aws_lambda_function" "api" {
  function_name = local.api_name
  role          = aws_iam_role.api.arn
  architectures = ["arm64"]
  runtime       = "provided.al2023"
  handler       = "bootstrap"
  s3_bucket     = local.artifacts_bucket
  s3_key        = var.lambda_s3_key
  memory_size   = var.lambda_memory_mb
  timeout       = 30
  publish       = true

  reserved_concurrent_executions = var.reserved_concurrency

  # /tmp : cache des dalles (plafond du moteur : TILES_CACHE_MB, 1 500 Mo par défaut) ; 512 Mo
  # (inclus dans le prix) en mode zip.
  ephemeral_storage {
    size = var.tiles_source == "s3" ? 2048 : 512
  }

  environment {
    variables = merge(var.lambda_env, {
      TURNSTILE_SECRET    = var.turnstile_secret
      TURNSTILE_HOSTNAMES = local.turnstile_hostnames
      LOOP_SIGNING_KEY    = random_password.loop_signing.result
      SHARED_BUCKET       = aws_s3_bucket.site.id # boucles partagées sous shared/
      ADMIN_KEY           = var.admin_key         # vide : routes /api/admin/* en 404
    })
  }

  logging_config {
    log_format = "Text"
    log_group  = aws_cloudwatch_log_group.api.name
  }

  # Le droit de lire les dalles existe avant la version qui s'en sert (smoke interne à froid).
  depends_on = [aws_iam_role_policy.api_tiles]

  lifecycle {
    # Le coupe-circuit met la concurrence à 0 : un déploiement ne doit pas la rétablir.
    ignore_changes = [reserved_concurrent_executions]
  }
}

resource "aws_lambda_alias" "live" {
  name             = "live"
  function_name    = aws_lambda_function.api.function_name
  function_version = aws_lambda_function.api.version
  lifecycle {
    ignore_changes = [function_version] # basculé par scripts/deploy.sh (smoke test) et rollback
  }
}

resource "aws_lambda_function_url" "api" {
  function_name      = aws_lambda_function.api.function_name
  qualifier          = aws_lambda_alias.live.name
  authorization_type = "AWS_IAM" # seul CloudFront (OAC) peut l'appeler
  invoke_mode        = "BUFFERED"
}

# Les Function URL exigent les deux permissions (InvokeFunctionUrl et InvokeFunction).
resource "aws_lambda_permission" "cloudfront_url" {
  statement_id           = "cloudfront-url"
  action                 = "lambda:InvokeFunctionUrl"
  function_name          = aws_lambda_function.api.function_name
  qualifier              = aws_lambda_alias.live.name
  principal              = "cloudfront.amazonaws.com"
  source_arn             = aws_cloudfront_distribution.main.arn
  function_url_auth_type = "AWS_IAM"
}

resource "aws_lambda_permission" "cloudfront_invoke" {
  statement_id             = "cloudfront-invoke"
  action                   = "lambda:InvokeFunction"
  function_name            = aws_lambda_function.api.function_name
  qualifier                = aws_lambda_alias.live.name
  principal                = "cloudfront.amazonaws.com"
  source_arn               = aws_cloudfront_distribution.main.arn
  invoked_via_function_url = true
}
