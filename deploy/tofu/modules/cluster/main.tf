resource "random_password" "k3s_token" {
  length  = 64
  special = false
}

resource "hcloud_ssh_key" "admin" {
  name       = "todo-app-admin"
  public_key = var.ssh_admin_pubkey
}

locals {
  # Predictable private IPs so cloud-init knows where node 0 is before it boots.
  # 10.0.1.10 -> node 0 (cluster-init), 10.0.1.11 -> node 1, 10.0.1.12 -> node 2.
  node_private_ips = [for i in range(var.node_count) : "10.0.1.${10 + i}"]
}

resource "hcloud_server" "node" {
  count        = var.node_count
  name         = format("todo-app-%02d", count.index + 1)
  image        = "debian-12"
  server_type  = var.node_type
  location     = var.location
  ssh_keys     = [hcloud_ssh_key.admin.id]
  firewall_ids = [var.firewall_id]

  # Note: we deliberately do NOT use the inline `network {}` block here —
  # we attach the private network via a separate hcloud_server_network
  # resource (next), which lets us pin the private IP deterministically
  # and avoid a circular dep with cloud-init.

  labels = {
    role    = "k3s-server"
    cluster = "todo-app"
  }

  user_data = templatefile(
    "${path.module}/templates/cloud-init.yaml.tpl",
    {
      is_init                 = count.index == 0
      k3s_version             = var.k3s_version
      k3s_token               = random_password.k3s_token.result
      tls_san                 = "todo-app.k8s.internal"
      first_node_private_ipv4 = local.node_private_ips[0]
    }
  )
}

resource "hcloud_server_network" "node" {
  count      = var.node_count
  server_id  = hcloud_server.node[count.index].id
  network_id = var.network_id
  ip         = local.node_private_ips[count.index]
}
