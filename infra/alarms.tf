# Alarmes → e-mail ; coupe-circuit (D8, D32 B2/E3), voir README « Coupe-circuit » :
# - calculs ACCEPTÉS (filtre de métriques sur le log `plan`, accepted=true) > seuil sur 5 min
#   → SNS → optrail-killswitch : concurrence 0, reprise automatique 1 h plus tard (Scheduler) ;
# - Budgets (coût brut, crédits exclus) → même Lambda : concurrence 0, reprise MANUELLE.
# Les jetons Turnstile faux ne comptent pas (sinon n'importe qui couperait le service, E3).

resource "aws_sns_topic" "alerts" {
  name = "optrail-alerts"
}

resource "aws_sns_topic" "killswitch" {
  name = "optrail-killswitch"
}

resource "aws_sns_topic_subscription" "email" {
  for_each  = { alerts = aws_sns_topic.alerts.arn, killswitch = aws_sns_topic.killswitch.arn }
  topic_arn = each.value
  protocol  = "email"
  endpoint  = var.alert_email
}

resource "aws_sns_topic_policy" "killswitch" {
  arn = aws_sns_topic.killswitch.arn
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = ["budgets.amazonaws.com", "cloudwatch.amazonaws.com"] }
      Action    = "SNS:Publish"
      Resource  = aws_sns_topic.killswitch.arn
      Condition = { StringEquals = { "aws:SourceAccount" = local.account } }
    }]
  })
}

# --- Alarmes d'information -----------------------------------------------------

locals {
  fn = { FunctionName = aws_lambda_function.api.function_name }
}

resource "aws_cloudwatch_metric_alarm" "errors" {
  alarm_name          = "optrail-api-errors"
  namespace           = "AWS/Lambda"
  metric_name         = "Errors"
  dimensions          = local.fn
  statistic           = "Sum"
  period              = 300
  evaluation_periods  = 1
  threshold           = 0
  comparison_operator = "GreaterThanThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
}

resource "aws_cloudwatch_metric_alarm" "throttles" {
  alarm_name          = "optrail-api-throttles"
  namespace           = "AWS/Lambda"
  metric_name         = "Throttles"
  dimensions          = local.fn
  statistic           = "Sum"
  period              = 300
  evaluation_periods  = 1
  threshold           = 0
  comparison_operator = "GreaterThanThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
}

resource "aws_cloudwatch_metric_alarm" "duration_p95" {
  alarm_name          = "optrail-api-duration-p95"
  alarm_description   = "p95 au-delà du budget serveur (~15 s)."
  namespace           = "AWS/Lambda"
  metric_name         = "Duration"
  dimensions          = local.fn
  extended_statistic  = "p95"
  period              = 900
  evaluation_periods  = 1
  threshold           = 15000
  comparison_operator = "GreaterThanThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
}

# --- Coupe-circuit -------------------------------------------------------------

# Contrat d'exploitation avec le handler (api.md § Exploitation) : une ligne JSON par requête
# /api/plan, `accepted: true` dès que Turnstile est passé (ou appel interne), `compute_s` en s.
resource "aws_cloudwatch_log_metric_filter" "accepted" {
  name           = "optrail-accepted-compute"
  log_group_name = aws_cloudwatch_log_group.api.name
  pattern        = "{ $.msg = \"plan\" && $.accepted IS TRUE }"
  metric_transformation {
    namespace = "optrail"
    name      = "AcceptedComputeSeconds"
    value     = "$.compute_s"
    unit      = "Seconds"
  }
}

resource "aws_cloudwatch_metric_alarm" "killswitch" {
  alarm_name          = "optrail-api-killswitch"
  alarm_description   = "Calculs acceptés sur 5 min > seuil : pause 1 h (concurrence 0)."
  namespace           = "optrail"
  metric_name         = aws_cloudwatch_log_metric_filter.accepted.metric_transformation[0].name
  statistic           = "Sum"
  period              = 300
  evaluation_periods  = 1
  threshold           = var.killswitch_compute_s
  comparison_operator = "GreaterThanThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.killswitch.arn]
}

# Filet E3 : durée BRUTE (jetons faux compris), alerte e-mail seulement, jamais de coupure.
resource "aws_cloudwatch_metric_alarm" "duration_sum" {
  alarm_name          = "optrail-api-duration-hour"
  alarm_description   = "Somme des durées (toutes requêtes) sur 1 h > seuil : regarder les logs (abus de jetons ?)."
  namespace           = "AWS/Lambda"
  metric_name         = "Duration"
  dimensions          = local.fn
  statistic           = "Sum"
  period              = 3600
  evaluation_periods  = 1
  threshold           = var.duration_alert_s * 1000
  comparison_operator = "GreaterThanThreshold"
  treat_missing_data  = "notBreaching"
  alarm_actions       = [aws_sns_topic.alerts.arn]
}

resource "aws_budgets_budget" "monthly" {
  name         = "optrail-monthly"
  budget_type  = "COST"
  limit_amount = max(var.budget_usd...)
  limit_unit   = "USD"
  time_unit    = "MONTHLY"

  # Plan Free : l'usage hors free tier est payé par les crédits ; avec les crédits inclus le coût
  # net resterait 0 et le budget ne sonnerait jamais. On suit donc le coût BRUT.
  cost_types {
    include_credit = false
    include_refund = false
  }

  dynamic "notification" {
    for_each = var.budget_usd
    content {
      notification_type         = "ACTUAL"
      comparison_operator       = "GREATER_THAN"
      threshold                 = notification.value
      threshold_type            = "ABSOLUTE_VALUE"
      subscriber_sns_topic_arns = [aws_sns_topic.killswitch.arn]
    }
  }
}

# Pause : SNS (alarme → reprise planifiée à +1 h ; budget → reprise manuelle, planif. annulée).
# Reprise : événement {"action": "resume"} du Scheduler (planification unique, auto-supprimée).
data "archive_file" "killswitch" {
  type        = "zip"
  output_path = "${path.module}/.build/killswitch.zip"
  source {
    filename = "index.py"
    content  = <<-PY
      import datetime as dt
      import json
      import os

      import boto3

      FN = os.environ["TARGET"]
      SCHEDULE = "optrail-resume"
      lam = boto3.client("lambda")
      sch = boto3.client("scheduler")


      def log(**kw):
          print(json.dumps({"msg": "killswitch", "target": FN, **kw}))


      def handler(event, context):
          if event.get("action") == "resume":
              lam.delete_function_concurrency(FunctionName=FN)
              log(op="resume")
              return
          raw = event["Records"][0]["Sns"]["Message"]
          try:
              alarm = json.loads(raw).get("AlarmName")
          except ValueError:
              alarm = None  # Budgets : message texte
          lam.put_function_concurrency(FunctionName=FN, ReservedConcurrentExecutions=0)
          if not alarm:
              try:
                  sch.delete_schedule(Name=SCHEDULE)
              except sch.exceptions.ResourceNotFoundException:
                  pass
              log(op="pause", source="budget", resume="manual")
              return
          at = (dt.datetime.now(dt.timezone.utc) + dt.timedelta(seconds=int(os.environ["PAUSE_S"])))
          args = dict(
              Name=SCHEDULE,
              ScheduleExpression=f"at({at:%Y-%m-%dT%H:%M:%S})",
              ScheduleExpressionTimezone="UTC",
              FlexibleTimeWindow={"Mode": "OFF"},
              ActionAfterCompletion="DELETE",
              Target={"Arn": context.invoked_function_arn, "RoleArn": os.environ["SCHEDULER_ROLE"],
                      "Input": json.dumps({"action": "resume"})},
          )
          try:
              sch.create_schedule(**args)
          except sch.exceptions.ConflictException:
              sch.update_schedule(**args)
          log(op="pause", source=alarm, resume_at=args["ScheduleExpression"])
    PY
  }
}

resource "aws_iam_role" "killswitch" {
  name                 = "optrail-killswitch"
  assume_role_policy   = data.aws_iam_policy_document.lambda_trust.json
  permissions_boundary = local.boundary
}

resource "aws_iam_role_policy" "killswitch" {
  name = "killswitch"
  role = aws_iam_role.killswitch.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      { Effect = "Allow", Action = ["lambda:PutFunctionConcurrency", "lambda:DeleteFunctionConcurrency"], Resource = aws_lambda_function.api.arn },
      { Effect = "Allow", Action = ["scheduler:CreateSchedule", "scheduler:UpdateSchedule", "scheduler:DeleteSchedule"], Resource = "arn:aws:scheduler:eu-north-1:${local.account}:schedule/default/optrail-resume" },
      { Effect = "Allow", Action = "iam:PassRole", Resource = aws_iam_role.scheduler.arn },
      { Effect = "Allow", Action = ["logs:CreateLogStream", "logs:PutLogEvents"], Resource = "${aws_cloudwatch_log_group.killswitch.arn}:*" },
    ]
  })
}

resource "aws_cloudwatch_log_group" "killswitch" {
  name              = "/aws/lambda/optrail-killswitch"
  retention_in_days = 14
}

resource "aws_lambda_function" "killswitch" {
  function_name    = "optrail-killswitch"
  role             = aws_iam_role.killswitch.arn
  architectures    = ["arm64"]
  runtime          = "python3.13"
  handler          = "index.handler"
  filename         = data.archive_file.killswitch.output_path
  source_code_hash = data.archive_file.killswitch.output_base64sha256
  memory_size      = 128
  timeout          = 10
  environment {
    variables = {
      TARGET         = aws_lambda_function.api.function_name
      SCHEDULER_ROLE = aws_iam_role.scheduler.arn
      PAUSE_S        = tostring(var.killswitch_pause_s)
    }
  }
  logging_config {
    log_format = "Text"
    log_group  = aws_cloudwatch_log_group.killswitch.name
  }
}

resource "aws_lambda_permission" "killswitch_sns" {
  statement_id  = "sns"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.killswitch.function_name
  principal     = "sns.amazonaws.com"
  source_arn    = aws_sns_topic.killswitch.arn
}

resource "aws_sns_topic_subscription" "killswitch" {
  topic_arn = aws_sns_topic.killswitch.arn
  protocol  = "lambda"
  endpoint  = aws_lambda_function.killswitch.arn
}

# Rôle du Scheduler pour la reprise : n'invoque que optrail-killswitch.
resource "aws_iam_role" "scheduler" {
  name                 = "optrail-scheduler"
  permissions_boundary = local.boundary
  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect    = "Allow"
      Principal = { Service = "scheduler.amazonaws.com" }
      Action    = "sts:AssumeRole"
      Condition = { StringEquals = { "aws:SourceAccount" = local.account } }
    }]
  })
}

resource "aws_iam_role_policy" "scheduler" {
  name = "resume"
  role = aws_iam_role.scheduler.id
  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Action   = "lambda:InvokeFunction"
      Resource = aws_lambda_function.killswitch.arn
    }]
  })
}
