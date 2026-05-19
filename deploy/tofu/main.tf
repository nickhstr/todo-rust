provider "hcloud" {
  token = var.hcloud_token
}

provider "cloudflare" {
  api_token = var.cloudflare_api_token
}

data "cloudflare_zone" "main" {
  name = var.cloudflare_zone_name
}

module "network" {
  source                = "./modules/network"
  ssh_admin_source_ipv4 = var.ssh_admin_source_ipv4
}

module "cluster" {
  source           = "./modules/cluster"
  location         = var.location
  node_count       = var.node_count
  node_type        = var.node_type
  k3s_version      = var.k3s_version
  ssh_admin_pubkey = var.ssh_admin_pubkey
  network_id       = module.network.private_network_id
  firewall_id      = module.network.firewall_id
}

module "dns" {
  source        = "./modules/dns"
  zone_id       = data.cloudflare_zone.main.id
  domain_prefix = var.domain_prefix
  # Once ingress-nginx provisions a Hetzner LB, run `tofu apply` again with
  # `lb_ipv4` set via -var (or via terraform.tfvars) to populate DNS records.
  lb_ipv4 = var.lb_ipv4
}
