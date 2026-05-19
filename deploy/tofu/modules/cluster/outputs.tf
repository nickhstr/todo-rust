output "node_ipv4" {
  value = [for s in hcloud_server.node : s.ipv4_address]
}

output "k3s_token" {
  value     = random_password.k3s_token.result
  sensitive = true
}
