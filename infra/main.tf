# optrail, infrastructure de production (un seul environnement, D1/D4/D8/D10/D15).
# Appliquée par scripts/deploy.sh (D36) ; état S3 créé par infra/bootstrap.

terraform {
  required_version = ">= 1.10"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 5.0"
    }
    archive = {
      source  = "hashicorp/archive"
      version = "~> 2.0"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.0"
    }
  }
  # bucket passé à l'init : tofu init -backend-config=bucket=optrail-tfstate-<compte>
  backend "s3" {
    key          = "optrail/prod.tfstate"
    region       = "eu-north-1"
    encrypt      = true
    use_lockfile = true
  }
}

provider "aws" {
  region = "eu-north-1"
  default_tags {
    tags = { project = "optrail", managed_by = "opentofu" }
  }
}

# ACM pour CloudFront : us-east-1 obligatoirement.
provider "aws" {
  alias  = "us_east_1"
  region = "us-east-1"
  default_tags {
    tags = { project = "optrail", managed_by = "opentofu" }
  }
}

provider "cloudflare" {
  # Domaine désactivé : jeton factice au bon format, aucune ressource Cloudflare n'est créée.
  api_token = var.enable_custom_domain ? var.cloudflare_api_token : "0000000000000000000000000000000000000000"
}

data "aws_caller_identity" "me" {}

locals {
  account  = data.aws_caller_identity.me.account_id
  boundary = "arn:aws:iam::${local.account}:policy/optrail-lambda-boundary" # créée par bootstrap
}
