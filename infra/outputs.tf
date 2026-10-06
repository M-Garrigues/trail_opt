output "url" {
  value = var.enable_custom_domain ? "https://${var.domain}" : "https://${aws_cloudfront_distribution.main.domain_name}"
}

output "distribution_id" {
  value = aws_cloudfront_distribution.main.id
}

output "site_bucket" {
  value = aws_s3_bucket.site.id
}

output "function_name" {
  value = aws_lambda_function.api.function_name
}

# Version publiée par cet apply : smoke-testée puis promue sur `live` par scripts/deploy.sh.
output "lambda_version" {
  value = aws_lambda_function.api.version
}
