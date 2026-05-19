variable "hcloud_token" {
  description = "Hetzner Cloud API token (read/write)."
  type        = string
  sensitive   = true
}

variable "cloudflare_api_token" {
  description = "Cloudflare API token scoped to the target zone."
  type        = string
  sensitive   = true
}

variable "cloudflare_zone_name" {
  description = "Apex zone (e.g., 'example.com')."
  type        = string
}

variable "domain_prefix" {
  description = "Subdomain prefix for app hosts (e.g., 'todo' yields todo.example.com)."
  type        = string
}

variable "location" {
  description = "Hetzner datacenter location."
  type        = string
  default     = "nbg1"
}

variable "node_count" {
  description = "Number of k3s server nodes."
  type        = number
  default     = 3
  validation {
    condition     = var.node_count >= 1 && var.node_count % 2 == 1
    error_message = "node_count must be odd (for HA quorum) and >= 1."
  }
}

variable "node_type" {
  description = "Hetzner server type."
  type        = string
  default     = "cx22"
}

variable "k3s_version" {
  description = "k3s install version (https://github.com/k3s-io/k3s/releases)."
  type        = string
  default     = "v1.30.5+k3s1"
}

variable "ssh_admin_pubkey" {
  description = "Public SSH key authorized for root login on all nodes."
  type        = string
}

variable "ssh_admin_source_ipv4" {
  description = "Your /32 IPv4 source address for SSH + Kubernetes API."
  type        = string
}

variable "lb_ipv4" {
  description = "Hetzner LB public IPv4. Empty on first apply, set after ingress-nginx is up."
  type        = string
  default     = ""
}
