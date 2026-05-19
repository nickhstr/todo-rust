variable "zone_id" { type = string }
variable "domain_prefix" { type = string } # e.g., "todo"
variable "lb_ipv4" {
  type        = string
  default     = ""
  description = "Hetzner LB public IPv4. Empty on first apply, set after ingress-nginx is up."
}
