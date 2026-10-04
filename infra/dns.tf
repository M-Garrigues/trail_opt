# Domaine personnalisé (D15), inactif tant que enable_custom_domain = false.
# DNS chez Cloudflare (gratuit), proxy désactivé : CloudFront fait TLS et CDN. Pas de Route 53.

resource "aws_acm_certificate" "site" {
  count             = local.domain_on ? 1 : 0
  provider          = aws.us_east_1
  domain_name       = var.domain
  validation_method = "DNS"
  lifecycle {
    create_before_destroy = true
  }
}

resource "cloudflare_dns_record" "acm" {
  for_each = local.domain_on ? {
    for o in aws_acm_certificate.site[0].domain_validation_options : o.domain_name => o
  } : {}
  zone_id = var.cloudflare_zone_id
  name    = trimsuffix(each.value.resource_record_name, ".")
  type    = each.value.resource_record_type
  content = trimsuffix(each.value.resource_record_value, ".")
  ttl     = 1
  proxied = false
}

resource "aws_acm_certificate_validation" "site" {
  count                   = local.domain_on ? 1 : 0
  provider                = aws.us_east_1
  certificate_arn         = aws_acm_certificate.site[0].arn
  validation_record_fqdns = [for r in cloudflare_dns_record.acm : r.name]
}

# Apex en CNAME : Cloudflare l'aplatit automatiquement (CNAME flattening).
resource "cloudflare_dns_record" "apex" {
  count   = local.domain_on ? 1 : 0
  zone_id = var.cloudflare_zone_id
  name    = var.domain
  type    = "CNAME"
  content = aws_cloudfront_distribution.main.domain_name
  ttl     = 1
  proxied = false
}
