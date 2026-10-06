variable "lambda_s3_key" {
  description = "Clé du zip Lambda dans le bucket d'artefacts (lambda/<sha>.zip), déposé par scripts/deploy.sh."
  type        = string
}

variable "lambda_memory_mb" {
  description = "Mémoire de la Lambda de calcul (quota d'un compte neuf : 3 008 Mo, D1)."
  type        = number
  default     = 3008
  validation {
    condition     = var.lambda_memory_mb >= 128 && var.lambda_memory_mb <= 3008
    error_message = "lambda_memory_mb doit être entre 128 et 3008."
  }
}

variable "lambda_env" {
  description = "Variables d'environnement non secrètes de la Lambda (fixées par le CTO : DATA_VERSION…)."
  type        = map(string)
  default     = {}
}

variable "reserved_concurrency" {
  description = "Concurrence réservée à la création (-1 = aucune : le quota du compte, 10 sur un compte neuf, sert de plafond). Ensuite pilotée hors Tofu (coupe-circuit)."
  type        = number
  default     = -1
}

variable "turnstile_secret" {
  description = "Clé secrète Cloudflare Turnstile (secret GitHub TF_VAR_turnstile_secret, D8)."
  type        = string
  sensitive   = true
}

variable "cloudfront_hostname" {
  description = "Domaine *.cloudfront.net de la distribution (sortie `url` du premier apply), accepté par Turnstile tant que le domaine perso est inactif. Vide au premier apply."
  type        = string
  default     = ""
}

variable "alert_email" {
  description = "Adresse qui reçoit alarmes, budgets et coupe-circuit (abonnement SNS à confirmer)."
  type        = string
  sensitive   = true # e-mail perso : jamais dans les logs du plan (secret GitHub ALERT_EMAIL)
}

# Calage (README « Coupe-circuit ») : free tier Lambda 400 000 Go-s/mois = 132 979 s à 3,008 Go,
# soit ≈ 4 400 s/jour. 900 s sur 5 min = 3 calculs en parallèle sans interruption (≈ 180 calculs
# de 5 s) : jamais atteint par un trafic de lancement, atteint en ≤ 2 périodes par une saturation
# (quota 10). Pire cas en saturation : ≈ 4 000 s par cycle de ~70 min, ≈ 1,6 $/jour brut, payé par
# les crédits du plan Free et arrêté par le budget 1 $ (reprise manuelle).
variable "killswitch_compute_s" {
  description = "Coupe-circuit : secondes de calcul ACCEPTÉ sur 5 min au-delà desquelles l'API est mise en pause."
  type        = number
  default     = 900
}

variable "killswitch_pause_s" {
  description = "Durée de la pause avant reprise automatique (s)."
  type        = number
  default     = 3600
}

variable "duration_alert_s" {
  description = "Alerte e-mail (sans coupure) : somme de Duration brute sur 1 h, jetons rejetés compris (≈ 1 calcul continu)."
  type        = number
  default     = 3600
}

variable "budget_usd" {
  description = "Seuils Budgets en USD, coût brut hors crédits (M5 : ≈ 1 $) : chacun met l'API en pause, reprise manuelle."
  type        = list(number)
  default     = [1, 5]
}

variable "enable_custom_domain" {
  description = "Faux : site servi sur *.cloudfront.net. Vrai (D25, zone Cloudflare prête) : ACM + DNS Cloudflare."
  type        = bool
  default     = false
}

variable "domain" {
  type    = string
  default = "optrail.eu"
}

variable "cloudflare_zone_id" {
  description = "Identifiant de la zone Cloudflare du domaine (requis si enable_custom_domain)."
  type        = string
  default     = ""
}

variable "cloudflare_api_token" {
  description = "Jeton Cloudflare limité à Zone.DNS:Edit sur la zone (requis si enable_custom_domain)."
  type        = string
  default     = ""
  sensitive   = true
}
