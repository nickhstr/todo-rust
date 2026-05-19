output "private_network_id" {
  value = hcloud_network.main.id
}

output "private_subnet_id" {
  value = hcloud_network_subnet.nodes.id
}

output "firewall_id" {
  value = hcloud_firewall.nodes.id
}
