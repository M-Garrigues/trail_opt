# Amorçage, appliqué UNE fois à la main (état local, voir infra/README.md) : bucket d'état OpenTofu,
# bucket d'artefacts (zip Lambda, dalles), boundary des rôles Lambda. Fournisseur OIDC GitHub + rôle
# optrail-deploy seulement si enable_github_oidc (D36 : refusés par les SCP du compte géré ; le
# déploiement passe par scripts/deploy.sh avec la session `aws login`).

terraform {
  required_version = ">= 1.10"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }
}

provider "aws" {
  region = "eu-north-1"
  default_tags {
    tags = { project = "optrail", managed_by = "opentofu-bootstrap" }
  }
}

# Le dépôt utilise les `sub` immuables de GitHub (use_immutable_subject = true) : le segment repo
# porte les identifiants numériques (propriétaire@id/dépôt@id), non réattribuables. Valeur lue par
# `gh api repos/M-Garrigues/trail_opt/actions/oidc/customization/sub --jq .sub_claim_prefix`.
variable "github_sub_prefix" {
  description = "Préfixe `sub` OIDC du dépôt autorisé à déployer (sub_claim_prefix de GitHub)."
  type        = string
  default     = "repo:M-Garrigues@22774745/trail_opt@1402132975"
}

variable "enable_github_oidc" {
  description = "Crée l'OIDC GitHub et le rôle optrail-deploy (deploy.yml). Faux tant que les SCP refusent l'OIDC (D36)."
  type        = bool
  default     = false
}

data "aws_caller_identity" "me" {}

locals {
  account = data.aws_caller_identity.me.account_id
  # Le `sub` du jeton OIDC exige la personnalisation du dépôt (claims repo, context, ref ;
  # voir README) : sans elle, un job d'environnement n'expose pas la branche.
  oidc_sub = "${var.github_sub_prefix}:environment:prod:ref:refs/heads/main"
}

# --- Buckets -----------------------------------------------------------------

resource "aws_s3_bucket" "state" {
  bucket = "optrail-tfstate-${local.account}"
  lifecycle {
    prevent_destroy = true
  }
}

resource "aws_s3_bucket" "artifacts" {
  bucket = "optrail-artifacts-${local.account}"
}

resource "aws_s3_bucket_versioning" "state" {
  bucket = aws_s3_bucket.state.id
  versioning_configuration {
    status = "Enabled"
  }
}

# L'état contient le secret Turnstile : jamais en clair sur le réseau.
resource "aws_s3_bucket_policy" "state" {
  bucket = aws_s3_bucket.state.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Sid       = "TLSOnly"
      Effect    = "Deny"
      Principal = "*"
      Action    = "s3:*"
      Resource  = [aws_s3_bucket.state.arn, "${aws_s3_bucket.state.arn}/*"]
      Condition = { Bool = { "aws:SecureTransport" = "false" } }
    }]
  })
}

resource "aws_s3_bucket_public_access_block" "all" {
  for_each                = { state = aws_s3_bucket.state.id, artifacts = aws_s3_bucket.artifacts.id }
  bucket                  = each.value
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# Volume négligeable : on borne quand même les vieilles versions d'état et les vieux zips
# (une version Lambda publiée garde sa propre copie du code, le rollback n'en dépend pas).
resource "aws_s3_bucket_lifecycle_configuration" "state" {
  bucket = aws_s3_bucket.state.id
  rule {
    id     = "old-state-versions"
    status = "Enabled"
    filter {}
    noncurrent_version_expiration {
      noncurrent_days = 90
    }
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "artifacts" {
  bucket = aws_s3_bucket.artifacts.id
  rule {
    id     = "old-lambda-zips"
    status = "Enabled"
    filter {
      prefix = "lambda/"
    }
    expiration {
      days = 30
    }
  }
}

# --- OIDC GitHub + rôle de déploiement ---------------------------------------

resource "aws_iam_openid_connect_provider" "github" {
  count          = var.enable_github_oidc ? 1 : 0
  url            = "https://token.actions.githubusercontent.com"
  client_id_list = ["sts.amazonaws.com"]
}

data "aws_iam_policy_document" "trust" {
  count = var.enable_github_oidc ? 1 : 0
  statement {
    actions = ["sts:AssumeRoleWithWebIdentity"]
    principals {
      type        = "Federated"
      identifiers = [aws_iam_openid_connect_provider.github[0].arn]
    }
    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:aud"
      values   = ["sts.amazonaws.com"]
    }
    # Égalité stricte : ni PR, ni fork, ni autre branche, ni autre environnement.
    condition {
      test     = "StringEquals"
      variable = "token.actions.githubusercontent.com:sub"
      values   = [local.oidc_sub]
    }
  }
}

resource "aws_iam_role" "deploy" {
  count                = var.enable_github_oidc ? 1 : 0
  name                 = "optrail-deploy"
  assume_role_policy   = data.aws_iam_policy_document.trust[0].json
  max_session_duration = 3600
}

# Plafond imposé à tout rôle créé par le déploiement (rôles des Lambda) : empêche le rôle
# de déploiement de fabriquer un rôle plus puissant que lui (escalade de privilèges).
data "aws_iam_policy_document" "boundary" {
  statement {
    actions   = ["logs:CreateLogStream", "logs:PutLogEvents"]
    resources = ["arn:aws:logs:eu-north-1:${local.account}:log-group:/aws/lambda/optrail-*"]
  }
  # Coupe-circuit : pause / reprise de l'API.
  statement {
    actions   = ["lambda:PutFunctionConcurrency", "lambda:GetFunctionConcurrency", "lambda:DeleteFunctionConcurrency"]
    resources = ["arn:aws:lambda:eu-north-1:${local.account}:function:optrail-*"]
  }
  # Reprise différée d'1 h : planification unique EventBridge Scheduler, rôle du Scheduler.
  statement {
    actions   = ["scheduler:CreateSchedule", "scheduler:UpdateSchedule", "scheduler:DeleteSchedule"]
    resources = ["arn:aws:scheduler:eu-north-1:${local.account}:schedule/default/optrail-*"]
  }
  statement {
    actions   = ["iam:PassRole"]
    resources = ["arn:aws:iam::${local.account}:role/optrail-scheduler"]
    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["scheduler.amazonaws.com"]
    }
  }
  statement {
    actions   = ["lambda:InvokeFunction"]
    resources = ["arn:aws:lambda:eu-north-1:${local.account}:function:optrail-killswitch"]
  }
  # Boucles partagées seulement (le reste du site est écrit par scripts/deploy.sh, pas par une Lambda).
  statement {
    actions   = ["s3:GetObject", "s3:PutObject"]
    resources = ["arn:aws:s3:::optrail-site-*/shared/*"]
  }
}

resource "aws_iam_policy" "boundary" {
  name   = "optrail-lambda-boundary"
  policy = data.aws_iam_policy_document.boundary.json
}

data "aws_iam_policy_document" "deploy" {
  count = var.enable_github_oidc ? 1 : 0
  # État OpenTofu (verrou natif S3 : objet .tflock).
  statement {
    actions   = ["s3:ListBucket"]
    resources = [aws_s3_bucket.state.arn]
  }
  statement {
    actions   = ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"]
    resources = ["${aws_s3_bucket.state.arn}/optrail/*"]
  }

  # Ressources de l'application : tout est préfixé optrail-.
  statement {
    actions = ["s3:*"]
    resources = [
      "arn:aws:s3:::optrail-site-*", "arn:aws:s3:::optrail-site-*/*",
      aws_s3_bucket.artifacts.arn, "${aws_s3_bucket.artifacts.arn}/*",
    ]
  }
  statement {
    actions   = ["lambda:*"]
    resources = ["arn:aws:lambda:eu-north-1:${local.account}:function:optrail-*"]
  }
  statement {
    actions   = ["logs:DescribeLogGroups"]
    resources = ["*"]
  }
  statement {
    actions   = ["logs:*"]
    resources = ["arn:aws:logs:eu-north-1:${local.account}:log-group:/aws/lambda/optrail-*"]
  }
  statement {
    actions   = ["cloudwatch:DescribeAlarms"]
    resources = ["*"]
  }
  statement {
    actions   = ["cloudwatch:*"]
    resources = ["arn:aws:cloudwatch:eu-north-1:${local.account}:alarm:optrail-*"]
  }
  statement {
    actions   = ["sns:*"]
    resources = ["arn:aws:sns:eu-north-1:${local.account}:optrail-*"]
  }
  statement {
    actions   = ["budgets:*"]
    resources = ["arn:aws:budgets::${local.account}:budget/optrail-*"]
  }
  # CloudFront (distribution, OAC, politique d'en-têtes, invalidations) : identifiants aléatoires,
  # pas de nommage par préfixe → liste d'actions fermée (ni clés de signature, ni KeyValueStore,
  # ni origines VPC, ni journaux temps réel) ; les fonctions sont, elles, limitées à optrail-*.
  statement {
    actions   = ["cloudfront:Get*", "cloudfront:List*", "cloudfront:Describe*"]
    resources = ["*"]
  }
  statement {
    actions = [
      "cloudfront:CreateDistribution", "cloudfront:UpdateDistribution", "cloudfront:DeleteDistribution",
      "cloudfront:TagResource", "cloudfront:UntagResource", "cloudfront:CreateInvalidation",
      "cloudfront:CreateOriginAccessControl", "cloudfront:UpdateOriginAccessControl", "cloudfront:DeleteOriginAccessControl",
      "cloudfront:CreateResponseHeadersPolicy", "cloudfront:UpdateResponseHeadersPolicy", "cloudfront:DeleteResponseHeadersPolicy",
      "cloudfront:CreateOriginRequestPolicy", "cloudfront:UpdateOriginRequestPolicy", "cloudfront:DeleteOriginRequestPolicy",
    ]
    resources = ["*"]
  }
  statement {
    actions = [
      "cloudfront:CreateFunction", "cloudfront:UpdateFunction", "cloudfront:DeleteFunction", "cloudfront:PublishFunction",
    ]
    resources = ["arn:aws:cloudfront::${local.account}:function/optrail-*"]
  }
  statement {
    actions   = ["acm:*"]
    resources = ["*"]
    condition {
      test     = "StringEquals"
      variable = "aws:RequestedRegion"
      values   = ["us-east-1"]
    }
  }

  # Rôles des Lambda : seulement optrail-*, seulement sous la boundary ci-dessus.
  statement {
    actions = [
      "iam:GetRole", "iam:DeleteRole", "iam:TagRole", "iam:UntagRole", "iam:UpdateAssumeRolePolicy",
      "iam:GetRolePolicy", "iam:DeleteRolePolicy", "iam:ListRolePolicies",
      "iam:ListAttachedRolePolicies", "iam:ListInstanceProfilesForRole",
    ]
    resources = ["arn:aws:iam::${local.account}:role/optrail-*"]
  }
  statement {
    actions   = ["iam:CreateRole", "iam:PutRolePolicy", "iam:PutRolePermissionsBoundary"]
    resources = ["arn:aws:iam::${local.account}:role/optrail-*"]
    condition {
      test     = "StringEquals"
      variable = "iam:PermissionsBoundary"
      values   = [aws_iam_policy.boundary.arn]
    }
  }
  statement {
    actions   = ["iam:PassRole"]
    resources = ["arn:aws:iam::${local.account}:role/optrail-*"]
    condition {
      test     = "StringEquals"
      variable = "iam:PassedToService"
      values   = ["lambda.amazonaws.com"]
    }
  }
  statement {
    effect    = "Deny"
    actions   = ["iam:DeleteRolePermissionsBoundary", "iam:AttachRolePolicy", "iam:CreatePolicyVersion"]
    resources = ["*"]
  }

  # E1 : le rôle ne peut ni se modifier, ni se supprimer, ni changer sa confiance (pas de persistance).
  statement {
    effect    = "Deny"
    actions   = ["iam:*"]
    resources = [aws_iam_role.deploy[0].arn]
  }
  # E1 / D23 : Function URL toujours en AWS_IAM, jamais d'invocation publique.
  statement {
    effect    = "Deny"
    actions   = ["lambda:CreateFunctionUrlConfig", "lambda:UpdateFunctionUrlConfig"]
    resources = ["*"]
    condition {
      test     = "StringNotEquals"
      variable = "lambda:FunctionUrlAuthType"
      values   = ["AWS_IAM"]
    }
  }
  statement {
    effect    = "Deny"
    actions   = ["lambda:AddPermission"]
    resources = ["*"]
    condition {
      test     = "StringEquals"
      variable = "lambda:Principal"
      values   = ["*"]
    }
  }
  statement {
    effect    = "Deny"
    actions   = ["lambda:AddPermission"]
    resources = ["*"]
    condition {
      test     = "StringEquals"
      variable = "lambda:FunctionUrlAuthType"
      values   = ["NONE"]
    }
  }
}

resource "aws_iam_role_policy" "deploy" {
  count  = var.enable_github_oidc ? 1 : 0
  name   = "optrail-deploy"
  role   = aws_iam_role.deploy[0].id
  policy = data.aws_iam_policy_document.deploy[0].json
}

# --- Sorties (TF_STATE_BUCKET / ARTIFACTS_BUCKET : lues par scripts/deploy.sh) --

output "AWS_DEPLOY_ROLE_ARN" {
  value = one(aws_iam_role.deploy[*].arn)
}

output "TF_STATE_BUCKET" {
  value = aws_s3_bucket.state.id
}

output "ARTIFACTS_BUCKET" {
  value = aws_s3_bucket.artifacts.id
}

output "LAMBDA_BOUNDARY_ARN" {
  value = aws_iam_policy.boundary.arn
}
