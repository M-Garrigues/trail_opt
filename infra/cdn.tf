# Site statique (S3 privé) + API (Function URL), même origine derrière CloudFront avec OAC.
# CloudFront à l'usage (always free 1 To / 10 M req), sans plan forfaitaire ni WAF (D4).

resource "aws_s3_bucket" "site" {
  bucket = "optrail-site-${local.account}"
}

resource "aws_s3_bucket_public_access_block" "site" {
  bucket                  = aws_s3_bucket.site.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# shared/ : boucles partagées (v2, PUT par la Lambda), conservées 90 jours.
resource "aws_s3_bucket_lifecycle_configuration" "site" {
  bucket = aws_s3_bucket.site.id
  rule {
    id     = "shared-90d"
    status = "Enabled"
    filter {
      prefix = "shared/"
    }
    expiration {
      days = 90
    }
  }
  # salt/ : sel quotidien des visiteurs uniques (D51), oublié après 2 jours (donnée anonyme)
  rule {
    id     = "salt-2d"
    status = "Enabled"
    filter {
      prefix = "salt/"
    }
    expiration {
      days = 2
    }
  }
}

# ListBucket : un fichier absent rend 404 (et non 403). Pas de listing possible : CloudFront ne
# transmet pas la query string au site (CachingOptimized, sans politique de requête d'origine).
# shared/ (boucles partagées) n'est jamais servi en direct : seulement via GET /api/loops/<id> ;
# salt/ (sel quotidien des visiteurs, D51) jamais servi.
resource "aws_s3_bucket_policy" "site" {
  bucket = aws_s3_bucket.site.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect    = "Allow"
        Principal = { Service = "cloudfront.amazonaws.com" }
        Action    = ["s3:GetObject", "s3:ListBucket"]
        Resource  = [aws_s3_bucket.site.arn, "${aws_s3_bucket.site.arn}/*"]
        Condition = { StringEquals = { "AWS:SourceArn" = aws_cloudfront_distribution.main.arn } }
      },
      {
        Effect    = "Deny"
        Principal = { Service = "cloudfront.amazonaws.com" }
        Action    = "s3:GetObject"
        Resource  = ["${aws_s3_bucket.site.arn}/shared/*", "${aws_s3_bucket.site.arn}/salt/*"]
      },
    ]
  })
}

# Routage SPA (ui-spec §8) : une route sans extension (/, /b/<id>…) sert index.html ; les fichiers
# (/assets/x.js, /coverage.geojson…) gardent leur vrai 404. Seulement sur le comportement par
# défaut : /api/* n'est jamais réécrit (erreurs et 429 intacts). Pas de custom_error_response,
# qui masquerait aussi les erreurs de l'API. Free tier : 2 M invocations/mois.
resource "aws_cloudfront_function" "spa" {
  name    = "optrail-spa"
  runtime = "cloudfront-js-2.0"
  publish = true
  code    = <<-EOT
    function handler(event) {
      var r = event.request;
      if (r.uri.indexOf('.', r.uri.lastIndexOf('/')) === -1) r.uri = '/index.html';
      return r;
    }
  EOT
}

resource "aws_cloudfront_origin_access_control" "s3" {
  name                              = "optrail-s3"
  origin_access_control_origin_type = "s3"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

resource "aws_cloudfront_origin_access_control" "lambda" {
  name                              = "optrail-lambda"
  origin_access_control_origin_type = "lambda"
  signing_behavior                  = "always"
  signing_protocol                  = "sigv4"
}

# Politiques gérées par AWS.
data "aws_cloudfront_cache_policy" "optimized" {
  name = "Managed-CachingOptimized"
}

data "aws_cloudfront_cache_policy" "disabled" {
  name = "Managed-CachingDisabled"
}

# Vers la Function URL : jamais Host (signature OAC). Liste fermée des en-têtes lus par le
# handler + CloudFront-Viewer-Address (remoteip de siteverify, E3), absent de la politique gérée
# AllViewerExceptHostHeader (allExcept host : en-têtes du visiteur seulement).
# Admin (D47, contracts/admin.md) : pays/région du visiteur (journaux), user-agent (hachage des
# visiteurs uniques de /api/hit), x-admin-key (Authorization est écrasé par la signature OAC).
# Une politique de requête d'origine ne change pas la clé de cache (/api/* : CachingDisabled).
resource "aws_cloudfront_origin_request_policy" "api" {
  name = "optrail-api"
  headers_config {
    header_behavior = "whitelist"
    headers {
      items = [
        "x-turnstile-token", "content-type", "CloudFront-Viewer-Address",
        "CloudFront-Viewer-Country", "CloudFront-Viewer-Country-Region", "user-agent", "x-admin-key",
      ]
    }
  }
  query_strings_config {
    query_string_behavior = "all"
  }
  cookies_config {
    cookie_behavior = "none"
  }
}

# En-têtes de sécurité (M2) : ceux de Managed-SecurityHeadersPolicy + Permissions-Policy + CSP.
# CSP BLOQUANTE (D61) depuis le 2026-10-07 : suite e2e complète (3 projets) sur le build servi avec ces
# en-têtes, zéro violation ; web/tests/e2e/csp.spec.ts échoue à toute violation. Une source ajoutée au
# front doit l'être ici aussi.
# Origines : tuiles/style/géocodage IGN (data.geopf.fr), MNT 3D (tiles.mapterhorn.com),
# Turnstile (challenges.cloudflare.com, script + iframe), worker MapLibre (self, blob:).
locals {
  csp = join("; ", [
    "default-src 'self'",
    "script-src 'self' https://challenges.cloudflare.com",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob: https://data.geopf.fr https://tiles.mapterhorn.com",
    "font-src 'self' https://data.geopf.fr",
    "connect-src 'self' https://data.geopf.fr https://tiles.mapterhorn.com",
    "worker-src 'self' blob:",
    "child-src 'self' blob:",
    "frame-src https://challenges.cloudflare.com",
    "manifest-src 'self'",
    "base-uri 'self'",
    "form-action 'self'",
    "object-src 'none'",
  ])
}

resource "aws_cloudfront_response_headers_policy" "security" {
  name = "optrail-security"

  security_headers_config {
    strict_transport_security {
      access_control_max_age_sec = 31536000
      include_subdomains         = true
      override                   = true
    }
    content_type_options {
      override = true
    }
    frame_options {
      frame_option = "DENY"
      override     = true
    }
    referrer_policy {
      referrer_policy = "strict-origin-when-cross-origin"
      override        = true
    }
    xss_protection {
      protection = true
      mode_block = true
      override   = true
    }
    content_security_policy {
      content_security_policy = local.csp
      override                = true
    }
  }

  custom_headers_config {
    items {
      header   = "Permissions-Policy"
      value    = "geolocation=(self), camera=(), microphone=(), payment=(), usb=(), interest-cohort=()"
      override = true
    }
  }
}

locals {
  domain_on = var.enable_custom_domain
}

resource "aws_cloudfront_distribution" "main" {
  enabled             = true
  comment             = "optrail"
  default_root_object = "index.html"
  http_version        = "http2and3"
  price_class         = "PriceClass_100"
  aliases             = local.domain_on ? [var.domain] : []

  origin {
    origin_id                = "site"
    domain_name              = aws_s3_bucket.site.bucket_regional_domain_name
    origin_access_control_id = aws_cloudfront_origin_access_control.s3.id
  }

  origin {
    origin_id                = "api"
    domain_name              = trimsuffix(trimprefix(aws_lambda_function_url.api.function_url, "https://"), "/")
    origin_access_control_id = aws_cloudfront_origin_access_control.lambda.id
    custom_origin_config {
      http_port              = 80
      https_port             = 443
      origin_protocol_policy = "https-only"
      origin_ssl_protocols   = ["TLSv1.2"]
      origin_read_timeout    = 35 # > timeout Lambda (30 s)
    }
  }

  default_cache_behavior {
    target_origin_id           = "site"
    viewer_protocol_policy     = "redirect-to-https"
    allowed_methods            = ["GET", "HEAD"]
    cached_methods             = ["GET", "HEAD"]
    compress                   = true
    cache_policy_id            = data.aws_cloudfront_cache_policy.optimized.id
    response_headers_policy_id = aws_cloudfront_response_headers_policy.security.id
    function_association {
      event_type   = "viewer-request"
      function_arn = aws_cloudfront_function.spa.arn
    }
  }

  # Partage (POST /api/loops, GET /api/loops/{id}). CloudFront n'accepte que des jeux de méthodes
  # complets ; la Lambda refuse le reste (405). OAC → Lambda signe le corps seulement si le
  # navigateur envoie x-amz-content-sha256 (SHA-256 hex du corps), calculé côté front. CloudFront
  # le lit sans qu'il figure dans la politique d'origine (il y est refusé en liste blanche).
  ordered_cache_behavior {
    path_pattern               = "/api/loops*"
    target_origin_id           = "api"
    viewer_protocol_policy     = "https-only"
    allowed_methods            = ["GET", "HEAD", "OPTIONS", "PUT", "POST", "PATCH", "DELETE"]
    cached_methods             = ["GET", "HEAD"]
    compress                   = true
    cache_policy_id            = data.aws_cloudfront_cache_policy.disabled.id
    origin_request_policy_id   = aws_cloudfront_origin_request_policy.api.id
    response_headers_policy_id = aws_cloudfront_response_headers_policy.security.id
  }

  # Mesure d'audience (POST /api/hit, D47) : même principe que /api/loops* (jeu de méthodes
  # complet, x-amz-content-sha256 calculé par le front), jamais en cache. Avant /api/*.
  ordered_cache_behavior {
    path_pattern               = "/api/hit"
    target_origin_id           = "api"
    viewer_protocol_policy     = "https-only"
    allowed_methods            = ["GET", "HEAD", "OPTIONS", "PUT", "POST", "PATCH", "DELETE"]
    cached_methods             = ["GET", "HEAD"]
    compress                   = true
    cache_policy_id            = data.aws_cloudfront_cache_policy.disabled.id
    origin_request_policy_id   = aws_cloudfront_origin_request_policy.api.id
    response_headers_policy_id = aws_cloudfront_response_headers_policy.security.id
  }

  # API : GET seulement (HEAD imposé par CloudFront), jamais en cache. Couvre aussi
  # GET /api/admin/stats (D47 : no-store posé par la Lambda, x-admin-key transmis par la politique).
  ordered_cache_behavior {
    path_pattern               = "/api/*"
    target_origin_id           = "api"
    viewer_protocol_policy     = "https-only"
    allowed_methods            = ["GET", "HEAD"]
    cached_methods             = ["GET", "HEAD"]
    compress                   = true
    cache_policy_id            = data.aws_cloudfront_cache_policy.disabled.id
    origin_request_policy_id   = aws_cloudfront_origin_request_policy.api.id
    response_headers_policy_id = aws_cloudfront_response_headers_policy.security.id
  }

  restrictions {
    geo_restriction {
      restriction_type = "none"
    }
  }

  viewer_certificate {
    cloudfront_default_certificate = !local.domain_on
    acm_certificate_arn            = local.domain_on ? aws_acm_certificate_validation.site[0].certificate_arn : null
    ssl_support_method             = local.domain_on ? "sni-only" : null
    minimum_protocol_version       = local.domain_on ? "TLSv1.2_2021" : "TLSv1"
  }
}
