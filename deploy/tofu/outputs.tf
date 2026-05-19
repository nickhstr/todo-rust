output "node_ipv4" {
  description = "Public IPv4 addresses of the k3s nodes (index 0 is the cluster-init node)."
  value       = module.cluster.node_ipv4
}

output "first_node_ipv4" {
  value = module.cluster.node_ipv4[0]
}

output "k3s_token" {
  description = "Cluster-join token (sensitive)."
  value       = module.cluster.k3s_token
  sensitive   = true
}

output "domain_fqdn_root" {
  description = "Root domain for app records (e.g., 'todo.example.com')."
  value       = "${var.domain_prefix}.${var.cloudflare_zone_name}"
}
