resource "hcloud_network" "main" {
  name     = "todo-app"
  ip_range = "10.0.0.0/16"
}

resource "hcloud_network_subnet" "nodes" {
  type         = "cloud"
  network_id   = hcloud_network.main.id
  network_zone = "eu-central"
  ip_range     = "10.0.1.0/24"
}

resource "hcloud_firewall" "nodes" {
  name = "todo-app-nodes"

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "22"
    source_ips  = ["${var.ssh_admin_source_ipv4}/32"]
    description = "SSH from admin"
  }

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "6443"
    source_ips  = ["${var.ssh_admin_source_ipv4}/32"]
    description = "Kubernetes API from admin"
  }

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "80"
    source_ips  = ["0.0.0.0/0", "::/0"]
    description = "HTTP (ingress)"
  }

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "443"
    source_ips  = ["0.0.0.0/0", "::/0"]
    description = "HTTPS (ingress)"
  }

  rule {
    direction   = "in"
    protocol    = "icmp"
    source_ips  = ["0.0.0.0/0", "::/0"]
    description = "ICMP for diagnostics"
  }

  # Cluster-internal traffic flows over the private network (10.0.0.0/16)
  # which Hetzner doesn't firewall by default. No explicit rules needed
  # for VXLAN/flannel/etcd here — only the public-facing edges.
}
