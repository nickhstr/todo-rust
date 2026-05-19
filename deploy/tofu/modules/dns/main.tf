locals {
  records = var.lb_ipv4 == "" ? [] : [
    var.domain_prefix,              # todo.example.com (prod)
    "staging.${var.domain_prefix}", # staging.todo.example.com
    "*.${var.domain_prefix}",       # *.todo.example.com (previews + grafana)
    "grafana",                      # grafana.example.com (Plan 3)
  ]
}

resource "cloudflare_record" "app" {
  for_each = toset(local.records)
  zone_id  = var.zone_id
  name     = each.value
  value    = var.lb_ipv4
  type     = "A"
  ttl      = 300
  proxied  = false # cert-manager DNS-01 needs unproxied A for verification path
}
