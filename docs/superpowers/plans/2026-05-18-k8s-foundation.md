# K8s Foundation — Plan 1 of 5

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a self-managed 3-node k3s HA cluster on Hetzner Cloud with ArgoCD GitOps-managing the platform layer (ingress-nginx, cert-manager, external-secrets-operator, 1Password Connect) and end-to-end secret round-trip from 1Password through external-secrets to a native Kubernetes Secret.

**Architecture:** OpenTofu provisions the network, three CX22 nodes (cloud-init bootstraps k3s in HA mode with embedded etcd), Cloudflare DNS records, and an object-storage bucket for Tofu state + Postgres WAL backups (later). After the cluster boots, a one-shot Helm install brings up ArgoCD, then an App-of-Apps root takes over and reconciles all platform components from the `deploy/argocd/` folder in this repo. 1Password Connect runs in-cluster; external-secrets-operator (ESO) pulls secrets through it into native k8s Secrets.

**Tech Stack:**
- OpenTofu 1.7+ (Terraform-compatible IaC)
- Hetzner Cloud + Cloudflare DNS
- k3s 1.30+ HA (embedded etcd)
- Helm 3 (initial platform installs), then ArgoCD reconciles all of it
- ingress-nginx (controller behind Hetzner LB)
- cert-manager + Let's Encrypt + Cloudflare DNS-01 (wildcards for preview envs in Plan 4)
- external-secrets-operator + 1Password Connect

**Spec:** `docs/superpowers/specs/2026-05-18-k8s-deploy-design.md`

**Plan position:** This is Plan 1 of 5. Subsequent plans (write when this one is done):
- Plan 2 — App + CI/CD (CNPG + Valkey + todo-app to staging+prod + GHA pipelines)
- Plan 3 — Observability (kube-prometheus-stack + Loki + Tempo + Alertmanager)
- Plan 4 — Preview environments (ApplicationSet PR generator)
- Plan 5 — Local k3d path (`just up-k8s`)

---

## Prerequisites (one-time, manual — do these before Task 1)

These cannot be code; they are dashboard / web UI steps that gate the rest of the plan.

1. **Local CLI tools** installed on your dev box:
   ```bash
   brew install opentofu kubectl helm k3d jq 1password-cli kubeconform kustomize
   tofu version          # expect >= 1.7
   kubectl version --client
   helm version
   op --version
   ```

2. **Hetzner Cloud**:
   - Create an account at https://console.hetzner.cloud
   - Add billing.
   - Create a project named `todo-app`.
   - Inside the project: Security → API Tokens → Generate API Token (Read & Write). Save it in 1Password as `hetzner-api-token`.
   - Note your default location: `nbg1` (Nuremberg) is assumed throughout this plan.

3. **Cloudflare**:
   - Add your domain (e.g., `<yourdomain>`) to Cloudflare; update nameservers at your registrar.
   - Cloudflare dashboard → My Profile → API Tokens → Create Token. Use the "Edit zone DNS" template, scope it to the zone for `<yourdomain>`. Save as `cloudflare-api-token` in 1Password.

4. **1Password vault**:
   - Create a new vault named `todo-app`.
   - Inside it, store the two API tokens above. We'll add more items in later tasks.

5. **Domain plan**:
   - You'll dedicate a subdomain to this. The plan uses `todo.<yourdomain>` for prod, `staging.todo.<yourdomain>` for staging, and `*.todo.<yourdomain>` for preview envs and Grafana. Pick your actual subdomain now and substitute `<yourdomain>` and `<subdomain>` in commands throughout this plan. (Example: `nickhstr.dev` and `todo.nickhstr.dev`.)

6. **Email for alerts** (used in Plan 3, but plan-ahead): pick a Gmail address you'll receive alerts at, and generate an App Password at https://myaccount.google.com/apppasswords — store it as `smtp-gmail` in 1Password.

---

## File Structure

This plan creates (and the engineer should imagine) the following tree. Files are introduced over the course of the tasks; this map is for orientation only.

```
deploy/
├── tofu/
│   ├── README.md
│   ├── bootstrap-object-storage.sh    # one-shot, runs once before `tofu init`
│   ├── .gitignore
│   ├── versions.tf                    # provider pinning
│   ├── backend.tf                     # S3-compatible backend on Hetzner Object Storage
│   ├── variables.tf                   # input variables
│   ├── outputs.tf                     # cluster IPs, kubeconfig hints
│   ├── main.tf                        # composes the three modules
│   ├── terraform.tfvars.example
│   └── modules/
│       ├── network/{main,variables,outputs}.tf
│       ├── cluster/
│       │   ├── main.tf
│       │   ├── variables.tf
│       │   ├── outputs.tf
│       │   └── templates/cloud-init.yaml.tpl
│       └── dns/{main,variables,outputs}.tf
├── bootstrap/
│   ├── README.md
│   ├── install-argocd.sh
│   ├── argocd-values.yaml
│   ├── ingress-nginx-values.yaml
│   ├── cert-manager-values.yaml
│   ├── hcloud-ccm-values.yaml
│   ├── hcloud-csi-values.yaml
│   └── cluster-issuers.yaml
└── argocd/
    ├── apps/
    │   ├── root.yaml                  # the App-of-Apps root
    │   └── platform/
    │       ├── argocd.yaml
    │       ├── hcloud-ccm.yaml
    │       ├── hcloud-csi.yaml
    │       ├── ingress-nginx.yaml
    │       ├── cert-manager.yaml
    │       ├── cert-issuers.yaml
    │       ├── external-secrets.yaml
    │       └── onepassword-connect.yaml
    └── manifests/
        ├── platform/
        │   ├── argocd/values.yaml
        │   ├── hcloud-ccm/values.yaml
        │   ├── hcloud-csi/values.yaml
        │   ├── ingress-nginx/values.yaml
        │   ├── cert-manager/values.yaml
        │   ├── cert-issuers/{kustomization,issuer-staging,issuer-prod}.yaml
        │   ├── external-secrets/{values.yaml,cluster-secret-store.yaml}
        │   └── onepassword-connect/values.yaml
        └── smoke/                     # delete at end of plan
            ├── kustomization.yaml
            └── external-secret.yaml
```

Plus updates to `justfile` (new recipes) and `README.md` (link to the deploy guide).

---

## Task 1: Stand up the OpenTofu state bucket

**Files:**
- Create: `deploy/tofu/bootstrap-object-storage.sh`
- Create: `deploy/tofu/.gitignore`
- Create: `deploy/tofu/README.md`

The Tofu S3 backend needs a bucket *before* `tofu init` runs. This is the only piece of infrastructure created outside Tofu, deliberately, because state cannot bootstrap itself.

- [ ] **Step 1: Write the bucket bootstrap script**

Create `deploy/tofu/bootstrap-object-storage.sh`:

```bash
#!/usr/bin/env bash
# One-shot: create the Hetzner Object Storage bucket that holds OpenTofu state
# and (later) Postgres WAL backups. Run this ONCE per environment.
#
# Prereqs: HCLOUD_TOKEN env var or `op` access to "hetzner-api-token".
# Requires `aws` CLI (Hetzner Object Storage is S3-compatible).

set -euo pipefail

BUCKET="${BUCKET:-todo-app-tofu-state}"
REGION="${REGION:-nbg1}"
ENDPOINT="https://${REGION}.your-objectstorage.com"

if ! command -v aws >/dev/null; then
  echo "aws CLI required. brew install awscli" >&2
  exit 1
fi

if [[ -z "${AWS_ACCESS_KEY_ID:-}" ]]; then
  echo "Set AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY from a Hetzner Object Storage credential pair."
  echo "Create one at: Console → Security → S3 credentials."
  exit 1
fi

aws --endpoint-url="$ENDPOINT" s3 mb "s3://${BUCKET}" --region "$REGION"
aws --endpoint-url="$ENDPOINT" s3api put-bucket-versioning \
  --bucket "$BUCKET" \
  --versioning-configuration Status=Enabled

echo "Bucket s3://${BUCKET} ready at ${ENDPOINT}"
```

- [ ] **Step 2: Make the script executable and write the gitignore**

```bash
chmod +x deploy/tofu/bootstrap-object-storage.sh
```

Create `deploy/tofu/.gitignore`:

```
# Never commit:
terraform.tfvars
*.tfstate
*.tfstate.backup
.terraform/
.terraform.lock.hcl
```

- [ ] **Step 3: Write the deploy/tofu README**

Create `deploy/tofu/README.md`:

```markdown
# OpenTofu — Hetzner Cloud infrastructure

Provisions the network, k3s nodes, Cloudflare DNS records, and the state bucket
that backs this very directory's state. Run order on a fresh setup:

1. `./bootstrap-object-storage.sh` — one-shot, creates the state bucket.
2. `tofu init` — pulls providers, configures S3 backend.
3. `tofu plan` — preview.
4. `tofu apply` — apply.

State is stored in `s3://todo-app-tofu-state/cluster.tfstate` via Hetzner's
S3-compatible object storage. The bucket is versioned so state mistakes are
recoverable.

Inputs live in `terraform.tfvars` (gitignored). Copy `terraform.tfvars.example`
and fill in your tokens.
```

- [ ] **Step 4: Create Hetzner Object Storage credentials & run the bootstrap**

In the Hetzner Console: Security → S3 credentials → Generate. Save the access key + secret in 1Password as `hetzner-s3-creds`. Then:

```bash
export AWS_ACCESS_KEY_ID="$(op item get hetzner-s3-creds --field access_key)"
export AWS_SECRET_ACCESS_KEY="$(op item get hetzner-s3-creds --field secret_key)"
cd deploy/tofu
./bootstrap-object-storage.sh
```

Expected output:
```
make_bucket: todo-app-tofu-state
Bucket s3://todo-app-tofu-state ready at https://nbg1.your-objectstorage.com
```

- [ ] **Step 5: Commit**

```bash
git add deploy/tofu/bootstrap-object-storage.sh deploy/tofu/.gitignore deploy/tofu/README.md
git commit -m "$(cat <<'EOF'
infra: scaffold tofu directory with state bucket bootstrap

Adds the one-shot script that creates the Hetzner Object Storage bucket
backing the OpenTofu S3 state. Gitignores tfvars + state. Tofu modules
land in subsequent commits.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Tofu root scaffolding

**Files:**
- Create: `deploy/tofu/versions.tf`
- Create: `deploy/tofu/backend.tf`
- Create: `deploy/tofu/variables.tf`
- Create: `deploy/tofu/outputs.tf`
- Create: `deploy/tofu/main.tf`
- Create: `deploy/tofu/terraform.tfvars.example`

Wires the provider versions, S3 backend, input variables, and an empty `main.tf` that we'll fill module-by-module in Tasks 3–5.

- [ ] **Step 1: Provider versions**

Create `deploy/tofu/versions.tf`:

```hcl
terraform {
  required_version = ">= 1.7.0"

  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.49"
    }
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 4.40"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
  }
}
```

- [ ] **Step 2: S3 backend**

Create `deploy/tofu/backend.tf`:

```hcl
terraform {
  backend "s3" {
    bucket = "todo-app-tofu-state"
    key    = "cluster.tfstate"
    region = "us-east-1"               # required by AWS SDK; Hetzner ignores it
    endpoints = {
      s3 = "https://nbg1.your-objectstorage.com"
    }
    skip_credentials_validation = true
    skip_metadata_api_check     = true
    skip_region_validation      = true
    skip_requesting_account_id  = true
    force_path_style            = true
    use_path_style              = true
  }
}
```

- [ ] **Step 3: Input variables**

Create `deploy/tofu/variables.tf`:

```hcl
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
```

- [ ] **Step 4: Outputs**

Create `deploy/tofu/outputs.tf`:

```hcl
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
```

- [ ] **Step 5: Main composition (module stubs)**

Create `deploy/tofu/main.tf`:

```hcl
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
  source            = "./modules/cluster"
  location          = var.location
  node_count        = var.node_count
  node_type         = var.node_type
  k3s_version       = var.k3s_version
  ssh_admin_pubkey  = var.ssh_admin_pubkey
  network_id        = module.network.private_network_id
  firewall_id       = module.network.firewall_id
}

module "dns" {
  source         = "./modules/dns"
  zone_id        = data.cloudflare_zone.main.id
  domain_prefix  = var.domain_prefix
  # Once ingress-nginx provisions a Hetzner LB, run `tofu apply` again with
  # `lb_ipv4` set via -var (or by un-commenting the data source below) to
  # populate the DNS records. Phase-2 of this plan handles that.
  lb_ipv4        = ""
}
```

- [ ] **Step 6: tfvars example**

Create `deploy/tofu/terraform.tfvars.example`:

```hcl
hcloud_token          = "REDACTED — paste from 1Password hetzner-api-token"
cloudflare_api_token  = "REDACTED — paste from 1Password cloudflare-api-token"
cloudflare_zone_name  = "example.com"
domain_prefix         = "todo"
ssh_admin_pubkey      = "ssh-ed25519 AAAA... your@host"
ssh_admin_source_ipv4 = "203.0.113.5"  # `curl ifconfig.me`
```

- [ ] **Step 7: Commit (don't init yet — modules come next)**

```bash
git add deploy/tofu/{versions,backend,variables,outputs,main}.tf deploy/tofu/terraform.tfvars.example
git commit -m "$(cat <<'EOF'
infra: tofu root scaffolding — providers, backend, vars, outputs

Composes three child modules (network, cluster, dns) which land in
the next commits.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Network module

**Files:**
- Create: `deploy/tofu/modules/network/main.tf`
- Create: `deploy/tofu/modules/network/variables.tf`
- Create: `deploy/tofu/modules/network/outputs.tf`

Private network for east-west traffic plus a firewall that:
- Allows SSH and kube-API from your IP only
- Allows HTTP/HTTPS from the world (the Hetzner LB lives on the world side)
- Allows VXLAN/flannel/etcd between nodes (cluster-internal)

- [ ] **Step 1: variables.tf**

Create `deploy/tofu/modules/network/variables.tf`:

```hcl
variable "ssh_admin_source_ipv4" {
  description = "Admin /32 allowed to reach SSH and kube-API."
  type        = string
}
```

- [ ] **Step 2: main.tf**

Create `deploy/tofu/modules/network/main.tf`:

```hcl
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
    direction = "in"
    protocol  = "tcp"
    port      = "22"
    source_ips = ["${var.ssh_admin_source_ipv4}/32"]
    description = "SSH from admin"
  }

  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "6443"
    source_ips = ["${var.ssh_admin_source_ipv4}/32"]
    description = "Kubernetes API from admin"
  }

  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "80"
    source_ips = ["0.0.0.0/0", "::/0"]
    description = "HTTP (ingress)"
  }

  rule {
    direction = "in"
    protocol  = "tcp"
    port      = "443"
    source_ips = ["0.0.0.0/0", "::/0"]
    description = "HTTPS (ingress)"
  }

  rule {
    direction = "in"
    protocol  = "icmp"
    source_ips = ["0.0.0.0/0", "::/0"]
    description = "ICMP for diagnostics"
  }

  # Cluster-internal traffic flows over the private network (10.0.0.0/16)
  # which Hetzner doesn't firewall by default. No explicit rules needed
  # for VXLAN/flannel/etcd here — only the public-facing edges.
}
```

- [ ] **Step 3: outputs.tf**

Create `deploy/tofu/modules/network/outputs.tf`:

```hcl
output "private_network_id" {
  value = hcloud_network.main.id
}

output "private_subnet_id" {
  value = hcloud_network_subnet.nodes.id
}

output "firewall_id" {
  value = hcloud_firewall.nodes.id
}
```

- [ ] **Step 4: Commit**

```bash
git add deploy/tofu/modules/network/
git commit -m "$(cat <<'EOF'
infra: tofu network module — private subnet + firewall

Allows SSH and kube-API from admin /32 only; HTTP/HTTPS open to the
world (terminated at the Hetzner LB).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Cluster module with cloud-init bootstrap

**Files:**
- Create: `deploy/tofu/modules/cluster/main.tf`
- Create: `deploy/tofu/modules/cluster/variables.tf`
- Create: `deploy/tofu/modules/cluster/outputs.tf`
- Create: `deploy/tofu/modules/cluster/templates/cloud-init.yaml.tpl`

The cluster module creates `node_count` Hetzner servers and bootstraps k3s in HA mode with embedded etcd. Node 0 runs `--cluster-init`; nodes 1..N wait for node 0's API to be reachable and then join.

- [ ] **Step 1: variables.tf**

Create `deploy/tofu/modules/cluster/variables.tf`:

```hcl
variable "location"         { type = string }
variable "node_count"       { type = number }
variable "node_type"        { type = string }
variable "k3s_version"      { type = string }
variable "ssh_admin_pubkey" { type = string }
variable "network_id"       { type = number }
variable "firewall_id"      { type = number }
```

- [ ] **Step 2: cloud-init template**

Create `deploy/tofu/modules/cluster/templates/cloud-init.yaml.tpl`:

```yaml
#cloud-config
write_files:
  - path: /etc/rancher/k3s/config.yaml
    content: |
      disable:
        - traefik           # we run ingress-nginx
        - servicelb         # we use hcloud-cloud-controller-manager
      tls-san:
        - ${tls_san}
      kube-apiserver-arg:
        - "audit-log-path=/var/log/k3s/audit.log"

runcmd:
  - |
      set -euo pipefail

      # Wait for the Hetzner Cloud private network interface to attach.
      # The systemd predictable name varies by base image (enp7s0 / ens10 / etc.),
      # so detect by destination subnet instead of by name.
      while ! ip -o -4 route show to match 10.0.0.0/16 | grep -q .; do
        echo "waiting for private network..."; sleep 3
      done
      PRIVATE_IFACE=$(ip -o -4 route show to match 10.0.0.0/16 | awk '{print $5}' | head -1)
      NODE_IP=$(ip -4 -o addr show "$PRIVATE_IFACE" | awk '{print $4}' | cut -d/ -f1)
      echo "Using private iface=$PRIVATE_IFACE node-ip=$NODE_IP"

%{ if is_init ~}
      curl -sfL https://get.k3s.io | \
        INSTALL_K3S_VERSION=${k3s_version} \
        K3S_TOKEN=${k3s_token} \
        sh -s - server \
          --cluster-init \
          --node-ip="$NODE_IP" \
          --flannel-iface="$PRIVATE_IFACE"
%{ else ~}
      # Wait for the init node's API to be reachable over the private network.
      until curl -ksf --max-time 5 "https://${first_node_private_ipv4}:6443/livez?verbose" >/dev/null; do
        echo "waiting for k3s API on ${first_node_private_ipv4}:6443..."; sleep 5
      done
      curl -sfL https://get.k3s.io | \
        INSTALL_K3S_VERSION=${k3s_version} \
        K3S_TOKEN=${k3s_token} \
        sh -s - server \
          --server=https://${first_node_private_ipv4}:6443 \
          --node-ip="$NODE_IP" \
          --flannel-iface="$PRIVATE_IFACE"
%{ endif ~}
```

Notes:
- `disable: traefik, servicelb` matters: we install ingress-nginx and use the Hetzner cloud-controller-manager for LoadBalancer Services.
- We deliberately do NOT taint any node — all 3 nodes are schedulable (the spec calls for HA with all nodes running workloads). The control-plane role is implicit from `k3s server`.
- `--node-ip` forces flannel + kubelet onto the private network; intra-cluster traffic stays off the public NICs (and outside the public-facing firewall rules) entirely.

- [ ] **Step 3: main.tf**

Create `deploy/tofu/modules/cluster/main.tf`:

```hcl
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
      is_init                  = count.index == 0
      k3s_version              = var.k3s_version
      k3s_token                = random_password.k3s_token.result
      tls_san                  = "todo-app.k8s.internal"
      first_node_private_ipv4  = local.node_private_ips[0]
    }
  )
}

resource "hcloud_server_network" "node" {
  count      = var.node_count
  server_id  = hcloud_server.node[count.index].id
  network_id = var.network_id
  ip         = local.node_private_ips[count.index]
}
```

Note: Tofu's `count` block can't dynamically depend on prior-index resources via `depends_on`, but the cloud-init `until curl ...` loop in nodes 1..N waits for node 0's API on its known private IP before joining, which makes parallel creation safe. The private-IP attachment happens slightly after the server starts; cloud-init explicitly waits for it (`while ! ip -o -4 route show to match 10.0.0.0/16 | grep -q .`).

- [ ] **Step 4: outputs.tf**

Create `deploy/tofu/modules/cluster/outputs.tf`:

```hcl
output "node_ipv4" {
  value = [for s in hcloud_server.node : s.ipv4_address]
}

output "k3s_token" {
  value     = random_password.k3s_token.result
  sensitive = true
}
```

- [ ] **Step 5: Commit**

```bash
git add deploy/tofu/modules/cluster/
git commit -m "$(cat <<'EOF'
infra: tofu cluster module — 3-node k3s HA via cloud-init

Init node runs --cluster-init; join nodes poll node 0's API and then
join. Token generated by random_password, lives in state (encrypted at
rest by Hetzner Object Storage).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: DNS module

**Files:**
- Create: `deploy/tofu/modules/dns/main.tf`
- Create: `deploy/tofu/modules/dns/variables.tf`
- Create: `deploy/tofu/modules/dns/outputs.tf`

Cloudflare records for prod/staging/preview hosts. The `lb_ipv4` is empty on the first apply (we don't have an LB yet); after Task 9 you'll re-apply with the real LB IP.

- [ ] **Step 1: variables.tf**

Create `deploy/tofu/modules/dns/variables.tf`:

```hcl
variable "zone_id"       { type = string }
variable "domain_prefix" { type = string }   # e.g., "todo"
variable "lb_ipv4" {
  type        = string
  default     = ""
  description = "Hetzner LB public IPv4. Empty on first apply, set after ingress-nginx is up."
}
```

- [ ] **Step 2: main.tf**

Create `deploy/tofu/modules/dns/main.tf`:

```hcl
locals {
  records = var.lb_ipv4 == "" ? [] : [
    var.domain_prefix,                       # todo.example.com (prod)
    "staging.${var.domain_prefix}",          # staging.todo.example.com
    "*.${var.domain_prefix}",                # *.todo.example.com (previews + grafana)
  ]
}

resource "cloudflare_record" "app" {
  for_each = toset(local.records)
  zone_id  = var.zone_id
  name     = each.value
  value    = var.lb_ipv4
  type     = "A"
  ttl      = 300
  proxied  = false   # cert-manager DNS-01 needs unproxied A for verification path; flip later if you want Cloudflare proxy
}
```

- [ ] **Step 3: outputs.tf**

Create `deploy/tofu/modules/dns/outputs.tf`:

```hcl
output "record_names" {
  value = [for r in cloudflare_record.app : r.hostname]
}
```

- [ ] **Step 4: Commit**

```bash
git add deploy/tofu/modules/dns/
git commit -m "$(cat <<'EOF'
infra: tofu dns module — cloudflare A records for app hosts

Empty on first apply; populated after ingress-nginx provisions the
Hetzner LB.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Initialize Tofu and dry-run the plan

**Files:** none (commands only)

- [ ] **Step 1: Copy tfvars and fill in real values**

```bash
cd deploy/tofu
cp terraform.tfvars.example terraform.tfvars
# Edit terraform.tfvars; fill in:
#   - hcloud_token (from 1Password: op item get hetzner-api-token --field credential)
#   - cloudflare_api_token (from 1Password)
#   - cloudflare_zone_name (e.g., example.com)
#   - domain_prefix (e.g., todo)
#   - ssh_admin_pubkey (cat ~/.ssh/id_ed25519.pub)
#   - ssh_admin_source_ipv4 ($(curl -s ifconfig.me))
```

- [ ] **Step 2: tofu init**

```bash
export AWS_ACCESS_KEY_ID="$(op item get hetzner-s3-creds --field access_key)"
export AWS_SECRET_ACCESS_KEY="$(op item get hetzner-s3-creds --field secret_key)"
tofu init
```

Expected output:
```
Initializing modules...
- cluster in modules/cluster
- dns in modules/dns
- network in modules/network
Initializing the backend...
Successfully configured the backend "s3"!
...
Terraform has been successfully initialized!
```

- [ ] **Step 3: tofu plan**

```bash
tofu plan
```

Expected: `Plan: 11 to add, 0 to change, 0 to destroy.` (1 network, 1 subnet, 1 firewall, 1 SSH key, 1 random_password, 3 servers, 3 server-network attachments). No DNS records yet because `lb_ipv4 == ""`.

- [ ] **Step 4: tofu validate (sanity)**

```bash
tofu validate
```

Expected: `Success! The configuration is valid.`

No commit — this is verification of prior tasks.

---

## Task 7: First apply — provision the cluster

**Files:** none (commands + verification)

- [ ] **Step 1: Apply**

```bash
cd deploy/tofu
tofu apply        # type 'yes' when prompted
```

Expected: completes in ~5–8 minutes. Output prints `node_ipv4 = ["1.2.3.4", "1.2.3.5", "1.2.3.6"]`.

- [ ] **Step 2: Wait for k3s to converge**

Cloud-init runs after server reboot. SSH in to confirm:

```bash
NODE0=$(tofu output -raw first_node_ipv4)
ssh root@$NODE0 'systemctl is-active k3s'
```

Expected: `active`. If `activating`, wait 30s and retry — the init node finishes installing first.

```bash
ssh root@$NODE0 'k3s kubectl get nodes'
```

Expected within 3–5 minutes (after join nodes finish): three lines, all `Ready`, all `Roles: control-plane,etcd,master`.

- [ ] **Step 3: Verify etcd quorum**

```bash
ssh root@$NODE0 'k3s kubectl get pods -n kube-system | grep etcd'
```

Expected: three pods (one per node), all `Running`.

- [ ] **Step 4: Snapshot the apply log**

(Optional — keep a paste in your local notes; don't commit to repo.)

No commit — apply is a state mutation, not a code change.

---

## Task 8: Fetch kubeconfig

**Files:**
- Manual create: `~/.kube/config-todo` (not committed)
- Update: `deploy/tofu/README.md` (add the kubeconfig step)

- [ ] **Step 1: Pull kubeconfig from node 0**

```bash
NODE0=$(cd deploy/tofu && tofu output -raw first_node_ipv4)
scp root@$NODE0:/etc/rancher/k3s/k3s.yaml ~/.kube/config-todo
sed -i.bak "s|server: https://127.0.0.1:6443|server: https://${NODE0}:6443|" ~/.kube/config-todo
chmod 600 ~/.kube/config-todo
```

- [ ] **Step 2: Verify kubectl access**

```bash
export KUBECONFIG=~/.kube/config-todo
kubectl get nodes
```

Expected: three nodes all `Ready`.

```bash
kubectl get pods --all-namespaces
```

Expected: coredns, metrics-server, local-path-provisioner all `Running`. (We disabled traefik + servicelb in cloud-init, so those should be absent.)

- [ ] **Step 3: Append kubeconfig steps to deploy/tofu/README.md**

Add at the bottom of `deploy/tofu/README.md`:

```markdown

## After apply

```bash
NODE0=$(tofu output -raw first_node_ipv4)
scp root@$NODE0:/etc/rancher/k3s/k3s.yaml ~/.kube/config-todo
sed -i.bak "s|server: https://127.0.0.1:6443|server: https://${NODE0}:6443|" ~/.kube/config-todo
chmod 600 ~/.kube/config-todo
export KUBECONFIG=~/.kube/config-todo
kubectl get nodes
```

Add `export KUBECONFIG=~/.kube/config-todo` to your shell profile or use
`kubectl --kubeconfig=...` per command.
```

- [ ] **Step 4: Commit**

```bash
git add deploy/tofu/README.md
git commit -m "$(cat <<'EOF'
docs: kubeconfig retrieval step for tofu README

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Install hcloud-cloud-controller-manager + hcloud-csi-driver

**Files:**
- Create: `deploy/bootstrap/hcloud-ccm-values.yaml`
- Create: `deploy/bootstrap/hcloud-csi-values.yaml`
- Create: `deploy/bootstrap/README.md`

These two controllers translate k8s LoadBalancer Services into real Hetzner LBs, and PVCs into Hetzner Cloud Volumes. They're prerequisites for ingress-nginx (which wants a LoadBalancer Service) and for CNPG (Plan 2, needs PVCs).

- [ ] **Step 1: Create the hcloud secret**

```bash
export KUBECONFIG=~/.kube/config-todo
kubectl create namespace hcloud-system

kubectl create secret generic hcloud \
  -n hcloud-system \
  --from-literal=token="$(op item get hetzner-api-token --field credential)" \
  --from-literal=network="todo-app"
```

- [ ] **Step 2: Write CCM Helm values**

Create `deploy/bootstrap/hcloud-ccm-values.yaml`:

```yaml
networking:
  enabled: true
  clusterCIDR: 10.42.0.0/16

env:
  HCLOUD_LOAD_BALANCERS_LOCATION: { value: nbg1 }
  HCLOUD_LOAD_BALANCERS_USE_PRIVATE_IP: { value: "true" }
  HCLOUD_LOAD_BALANCERS_ENABLED: { value: "true" }
  HCLOUD_LOAD_BALANCERS_NETWORK_ZONE: { value: eu-central }
```

- [ ] **Step 3: Install CCM**

```bash
helm repo add hcloud https://charts.hetzner.cloud
helm repo update

helm upgrade --install hcloud-cloud-controller-manager hcloud/hcloud-cloud-controller-manager \
  --namespace hcloud-system \
  --version 1.20.0 \
  -f deploy/bootstrap/hcloud-ccm-values.yaml
```

Verify:

```bash
kubectl -n hcloud-system get pods -w
# Expected: hcloud-cloud-controller-manager-... Running
```

- [ ] **Step 4: Write CSI Helm values**

Create `deploy/bootstrap/hcloud-csi-values.yaml`:

```yaml
storageClasses:
  - name: hcloud-volumes
    defaultStorageClass: true
    reclaimPolicy: Retain
    extraParameters:
      "csi.storage.k8s.io/fstype": ext4
```

- [ ] **Step 5: Install CSI**

```bash
helm upgrade --install hcloud-csi hcloud/hcloud-csi \
  --namespace hcloud-system \
  --version 2.10.0 \
  -f deploy/bootstrap/hcloud-csi-values.yaml
```

Verify:

```bash
kubectl get storageclass
# Expected: hcloud-volumes (default)
```

- [ ] **Step 6: Write the bootstrap README**

Create `deploy/bootstrap/README.md`:

```markdown
# Cluster bootstrap

Initial installs of platform components, run *once* against a fresh cluster.
After ArgoCD comes up (Task 14), it takes over reconciliation of all of these.

Order:
1. `hcloud-ccm` + `hcloud-csi` (Task 9)        — cloud integration
2. `ingress-nginx` (Task 10)                    — provisions a Hetzner LB
3. (back to tofu) DNS records pointing at LB IP (Task 11)
4. `cert-manager` (Task 12)                     — TLS automation
5. ClusterIssuers for Let's Encrypt (Task 13)
6. Cert smoke test (Task 14)
7. ArgoCD bootstrap (Task 15)
```

- [ ] **Step 7: Commit**

```bash
git add deploy/bootstrap/hcloud-ccm-values.yaml deploy/bootstrap/hcloud-csi-values.yaml deploy/bootstrap/README.md
git commit -m "$(cat <<'EOF'
bootstrap: helm values for hcloud-ccm and hcloud-csi

Cloud-controller-manager translates LoadBalancer Services into Hetzner
LBs; CSI driver backs PVCs with Hetzner Cloud Volumes. Both installed
directly via Helm initially; ArgoCD assumes management in a later task.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Install ingress-nginx and let it provision the Hetzner LB

**Files:**
- Create: `deploy/bootstrap/ingress-nginx-values.yaml`

- [ ] **Step 1: Helm values**

Create `deploy/bootstrap/ingress-nginx-values.yaml`:

```yaml
controller:
  ingressClassResource:
    name: nginx
    enabled: true
    default: true
  service:
    type: LoadBalancer
    annotations:
      load-balancer.hetzner.cloud/name: "todo-app-ingress"
      load-balancer.hetzner.cloud/location: "nbg1"
      load-balancer.hetzner.cloud/use-private-ip: "true"
      load-balancer.hetzner.cloud/network-zone: "eu-central"
      load-balancer.hetzner.cloud/type: "lb11"
  config:
    use-proxy-protocol: "false"
    use-forwarded-headers: "true"
    proxy-real-ip-cidr: "10.0.0.0/16"
  metrics:
    enabled: true
    serviceMonitor:
      enabled: false    # enable later when kube-prometheus-stack is up (Plan 3)
```

- [ ] **Step 2: Install**

```bash
helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx
helm repo update

kubectl create namespace ingress-nginx
helm upgrade --install ingress-nginx ingress-nginx/ingress-nginx \
  --namespace ingress-nginx \
  --version 4.11.2 \
  -f deploy/bootstrap/ingress-nginx-values.yaml
```

- [ ] **Step 3: Wait for the Hetzner LB to be created**

```bash
kubectl -n ingress-nginx get svc ingress-nginx-controller -w
```

Expected, after ~60–90s: `EXTERNAL-IP` populates with a real public IPv4. Verify in the Hetzner Console (Load Balancers tab) that a `todo-app-ingress` LB exists pointing at all three nodes on port 80/443.

- [ ] **Step 4: Save the LB IP for the next task**

```bash
export LB_IPV4=$(kubectl -n ingress-nginx get svc ingress-nginx-controller -o jsonpath='{.status.loadBalancer.ingress[0].ip}')
echo "LB IPv4: $LB_IPV4"
```

- [ ] **Step 5: Commit (values only)**

```bash
git add deploy/bootstrap/ingress-nginx-values.yaml
git commit -m "$(cat <<'EOF'
bootstrap: ingress-nginx helm values

LoadBalancer Service annotated for hcloud-ccm; ServiceMonitor disabled
until kube-prometheus-stack lands in Plan 3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Tofu re-apply to populate DNS records

**Files:** none

- [ ] **Step 1: Re-apply with the LB IP**

```bash
cd deploy/tofu
tofu apply -var "lb_ipv4=${LB_IPV4}"   # type 'yes'
```

Expected: `Plan: 3 to add, 0 to change, 0 to destroy.` (three Cloudflare records).

- [ ] **Step 2: Bake the LB IP into terraform.tfvars**

Add to `deploy/tofu/terraform.tfvars` (still gitignored):

```hcl
lb_ipv4 = "203.0.113.42"   # whatever LB_IPV4 was
```

And add `lb_ipv4` to `terraform.tfvars.example` (with a redacted placeholder):

```hcl
lb_ipv4 = "REDACTED — set after ingress-nginx is up (Task 10/11)"
```

Then re-run `tofu apply` from `terraform.tfvars` directly (no `-var`).

- [ ] **Step 3: Verify DNS resolves**

```bash
dig +short todo.<yourdomain> @1.1.1.1
dig +short staging.todo.<yourdomain> @1.1.1.1
dig +short test123.todo.<yourdomain> @1.1.1.1   # wildcard
```

All three should return `${LB_IPV4}`.

- [ ] **Step 4: Commit the example update**

```bash
git add deploy/tofu/terraform.tfvars.example
git commit -m "$(cat <<'EOF'
infra: document lb_ipv4 variable in tfvars example

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Install cert-manager

**Files:**
- Create: `deploy/bootstrap/cert-manager-values.yaml`

- [ ] **Step 1: Helm values**

Create `deploy/bootstrap/cert-manager-values.yaml`:

```yaml
crds:
  enabled: true

prometheus:
  enabled: false   # turn on with Plan 3

# Cloudflare DNS-01 solver needs the API token mounted; we'll create the
# secret separately (Task 13).

extraArgs:
  - --dns01-recursive-nameservers=1.1.1.1:53,8.8.8.8:53
  - --dns01-recursive-nameservers-only
```

- [ ] **Step 2: Install**

```bash
helm repo add jetstack https://charts.jetstack.io
helm repo update

kubectl create namespace cert-manager
helm upgrade --install cert-manager jetstack/cert-manager \
  --namespace cert-manager \
  --version v1.15.3 \
  -f deploy/bootstrap/cert-manager-values.yaml
```

- [ ] **Step 3: Verify**

```bash
kubectl -n cert-manager get pods -w
```

Expected: `cert-manager`, `cert-manager-webhook`, `cert-manager-cainjector` all `Running`.

- [ ] **Step 4: Commit**

```bash
git add deploy/bootstrap/cert-manager-values.yaml
git commit -m "$(cat <<'EOF'
bootstrap: cert-manager helm values

Configures recursive DNS nameservers for DNS-01 solver checks so we
don't depend on the cluster's default resolver (which would chase the
record back through Cloudflare's authoritative servers via the LB).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Cloudflare ClusterIssuers (LE staging + production)

**Files:**
- Create: `deploy/bootstrap/cluster-issuers.yaml`

- [ ] **Step 1: Create the Cloudflare API token secret**

```bash
kubectl -n cert-manager create secret generic cloudflare-api-token \
  --from-literal=api-token="$(op item get cloudflare-api-token --field credential)"
```

- [ ] **Step 2: Write the ClusterIssuer manifest**

Create `deploy/bootstrap/cluster-issuers.yaml`:

```yaml
---
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-staging
spec:
  acme:
    server: https://acme-staging-v02.api.letsencrypt.org/directory
    email: REPLACE_WITH_YOUR_EMAIL
    privateKeySecretRef:
      name: letsencrypt-staging-account
    solvers:
      - dns01:
          cloudflare:
            apiTokenSecretRef:
              name: cloudflare-api-token
              key: api-token
---
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-prod
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: REPLACE_WITH_YOUR_EMAIL
    privateKeySecretRef:
      name: letsencrypt-prod-account
    solvers:
      - dns01:
          cloudflare:
            apiTokenSecretRef:
              name: cloudflare-api-token
              key: api-token
```

Replace `REPLACE_WITH_YOUR_EMAIL` with your real address.

- [ ] **Step 3: Apply**

```bash
kubectl apply -f deploy/bootstrap/cluster-issuers.yaml
```

- [ ] **Step 4: Verify both issuers**

```bash
kubectl get clusterissuers
```

Expected: both `letsencrypt-staging` and `letsencrypt-prod` show `READY: True`. If they show `False`, run `kubectl describe clusterissuer letsencrypt-staging` and resolve the error (most common: wrong scope on the Cloudflare token; needs `Zone.DNS:Edit` plus `Zone:Read`).

- [ ] **Step 5: Commit**

```bash
git add deploy/bootstrap/cluster-issuers.yaml
git commit -m "$(cat <<'EOF'
bootstrap: letsencrypt staging + prod ClusterIssuers via cloudflare DNS-01

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Smoke test — issue a real certificate end-to-end

**Files:**
- Create: `deploy/bootstrap/smoke-cert-test.yaml` (delete at end of task)

- [ ] **Step 1: Write the smoke test manifest**

Create `deploy/bootstrap/smoke-cert-test.yaml`:

```yaml
---
apiVersion: v1
kind: Namespace
metadata:
  name: smoke
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: hello
  namespace: smoke
spec:
  replicas: 1
  selector: { matchLabels: { app: hello } }
  template:
    metadata: { labels: { app: hello } }
    spec:
      containers:
        - name: hello
          image: ghcr.io/nginxinc/nginx-unprivileged:1.27-alpine
          ports: [{ containerPort: 8080 }]
---
apiVersion: v1
kind: Service
metadata:
  name: hello
  namespace: smoke
spec:
  selector: { app: hello }
  ports: [{ port: 80, targetPort: 8080 }]
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: hello
  namespace: smoke
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-staging
spec:
  ingressClassName: nginx
  rules:
    - host: hello.todo.<yourdomain>          # SUBSTITUTE
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: hello
                port: { number: 80 }
  tls:
    - hosts: [hello.todo.<yourdomain>]       # SUBSTITUTE
      secretName: hello-tls
```

Substitute `<yourdomain>` in both places.

- [ ] **Step 2: Apply and watch the Certificate**

```bash
kubectl apply -f deploy/bootstrap/smoke-cert-test.yaml
kubectl -n smoke get certificate hello-tls -w
```

Expected within 60–120 seconds: `READY: True`. The Cloudflare DNS-01 solver writes a TXT record (look in Cloudflare DNS dashboard: a `_acme-challenge.hello.todo.<yourdomain>` TXT briefly appears, then disappears).

If stuck at `READY: False`, inspect:

```bash
kubectl -n smoke describe certificate hello-tls
kubectl -n smoke describe order $(kubectl -n smoke get order -o name | head -1)
kubectl -n smoke describe challenge $(kubectl -n smoke get challenge -o name | head -1)
```

Common causes: Cloudflare token missing `Zone:Read`; wrong zone scope; recursive resolver issues.

- [ ] **Step 3: Verify the cert works over HTTPS**

```bash
curl -sv https://hello.todo.<yourdomain>/ 2>&1 | head -25
```

Expected: TLS handshake succeeds (note the staging cert is signed by `STAGING Let's Encrypt`, so `curl` will complain about the chain unless you pass `-k`. That's expected — switch to `letsencrypt-prod` next.)

- [ ] **Step 4: Flip to production issuer**

```bash
kubectl -n smoke annotate ingress hello cert-manager.io/cluster-issuer=letsencrypt-prod --overwrite
kubectl -n smoke delete secret hello-tls          # force re-issuance
kubectl -n smoke delete certificate hello-tls
```

Re-create the Certificate by re-applying the Ingress (the annotation change is captured but cert-manager only re-issues if the secret/cert is gone):

```bash
kubectl apply -f deploy/bootstrap/smoke-cert-test.yaml
kubectl -n smoke get certificate hello-tls -w
```

Expected: `READY: True` again, this time with a real LE cert. `curl -sv https://hello.todo.<yourdomain>/` should now succeed without `-k`.

- [ ] **Step 5: Tear down the smoke test, keep the manifest in the repo for posterity**

```bash
kubectl delete -f deploy/bootstrap/smoke-cert-test.yaml
```

(The file stays in the repo as documentation.)

- [ ] **Step 6: Commit**

```bash
git add deploy/bootstrap/smoke-cert-test.yaml
git commit -m "$(cat <<'EOF'
bootstrap: cert-manager smoke test (Ingress -> staging -> prod)

End-to-end verification that LE DNS-01 issuance works. Applied and
torn down during bootstrap; lives in repo for future fresh-cluster
runs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: Bootstrap ArgoCD via Helm

**Files:**
- Create: `deploy/bootstrap/argocd-values.yaml`
- Create: `deploy/bootstrap/install-argocd.sh`

- [ ] **Step 1: ArgoCD Helm values**

Create `deploy/bootstrap/argocd-values.yaml`:

```yaml
global:
  domain: argocd.todo.<yourdomain>           # SUBSTITUTE

configs:
  params:
    server.insecure: false
  cm:
    timeout.reconciliation: 30s
    application.instanceLabelKey: argocd.argoproj.io/instance

server:
  ingress:
    enabled: true
    ingressClassName: nginx
    hostname: argocd.todo.<yourdomain>       # SUBSTITUTE
    annotations:
      cert-manager.io/cluster-issuer: letsencrypt-prod
    tls: true

dex:
  enabled: false       # we'll wire SSO later if desired

# Pin app sync to manual-sync by default; we'll opt into auto-sync per app.
controller:
  metrics:
    enabled: true
    serviceMonitor:
      enabled: false   # Plan 3
```

Substitute `<yourdomain>`.

- [ ] **Step 2: Install script**

Create `deploy/bootstrap/install-argocd.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

CHART_VERSION="${CHART_VERSION:-7.5.2}"
NAMESPACE="argocd"

helm repo add argo https://argoproj.github.io/argo-helm
helm repo update

kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

helm upgrade --install argocd argo/argo-cd \
  --namespace "$NAMESPACE" \
  --version "$CHART_VERSION" \
  -f deploy/bootstrap/argocd-values.yaml

echo "Waiting for argocd-server..."
kubectl -n "$NAMESPACE" rollout status deployment/argocd-server --timeout=180s

echo
echo "Initial admin password:"
kubectl -n "$NAMESPACE" get secret argocd-initial-admin-secret \
  -o jsonpath='{.data.password}' | base64 -d
echo
echo "Save this to 1Password (vault todo-app, item 'argocd-admin')."
```

- [ ] **Step 3: Run it**

```bash
chmod +x deploy/bootstrap/install-argocd.sh
./deploy/bootstrap/install-argocd.sh
```

Expected: rollout status reports `successfully rolled out`. The initial admin password prints — save it to 1Password as `argocd-admin`.

- [ ] **Step 4: Verify access**

Wait ~60 seconds for cert-manager to provision the cert, then:

```bash
kubectl -n argocd get ingress
kubectl -n argocd get certificate
```

`Certificate` should show `READY: True`. Visit `https://argocd.todo.<yourdomain>` in a browser — log in as `admin` with the password from Step 3.

- [ ] **Step 5: Commit**

```bash
git add deploy/bootstrap/argocd-values.yaml deploy/bootstrap/install-argocd.sh
git commit -m "$(cat <<'EOF'
bootstrap: install ArgoCD via helm

One-shot install script + values. ArgoCD will assume self-management
in Task 16.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: App-of-Apps root + ArgoCD self-management

**Files:**
- Create: `deploy/argocd/apps/root.yaml`
- Create: `deploy/argocd/apps/platform/argocd.yaml`
- Create: `deploy/argocd/manifests/platform/argocd/values.yaml`

From here on, every cluster change flows through git + ArgoCD reconciliation.

- [ ] **Step 1: Move ArgoCD values into the GitOps manifests path**

Copy `deploy/bootstrap/argocd-values.yaml` to `deploy/argocd/manifests/platform/argocd/values.yaml` (this is the canonical location ArgoCD will reconcile from):

```bash
mkdir -p deploy/argocd/manifests/platform/argocd
cp deploy/bootstrap/argocd-values.yaml deploy/argocd/manifests/platform/argocd/values.yaml
```

(The bootstrap copy stays for fresh-cluster runs.)

- [ ] **Step 2: ArgoCD self-management Application**

Create `deploy/argocd/apps/platform/argocd.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: argocd
  namespace: argocd
  finalizers:
    - resources-finalizer.argocd.argoproj.io
spec:
  project: default
  sources:
    - repoURL: https://argoproj.github.io/argo-helm
      chart: argo-cd
      targetRevision: 7.5.2
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/argocd/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: argocd
  syncPolicy:
    automated:
      prune: false       # never self-prune; protects against accidental deletes
      selfHeal: true
    syncOptions:
      - ServerSideApply=true
      - CreateNamespace=true
```

- [ ] **Step 3: App-of-Apps root**

Create `deploy/argocd/apps/root.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: root
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/apps/platform
    directory:
      recurse: true
  destination:
    server: https://kubernetes.default.svc
    namespace: argocd
  syncPolicy:
    automated:
      prune: true        # auto-prune Applications removed from git
      selfHeal: true
    syncOptions:
      - ServerSideApply=true
```

- [ ] **Step 4: Push these to the feature branch and apply the root**

```bash
git add deploy/argocd/
git commit -m "$(cat <<'EOF'
gitops: app-of-apps root + argocd self-management

Root Application watches deploy/argocd/apps/platform/. ArgoCD takes over
its own management from here.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

Then apply the root (still manual — this is the last kubectl apply outside ArgoCD):

```bash
kubectl apply -f deploy/argocd/apps/root.yaml
```

- [ ] **Step 5: Verify Argo reconciles itself**

In the ArgoCD UI (or `kubectl -n argocd get app`), watch:
- `root` Application appears, `Synced + Healthy`
- `argocd` Application appears, `Synced + Healthy`

Run `helm list -n argocd` — should still show `argocd` (now reconciled by Argo, not Helm-managed directly).

---

## Task 17: Migrate hcloud-ccm and hcloud-csi into ArgoCD

**Files:**
- Create: `deploy/argocd/apps/platform/hcloud-ccm.yaml`
- Create: `deploy/argocd/apps/platform/hcloud-csi.yaml`
- Create: `deploy/argocd/manifests/platform/hcloud-ccm/values.yaml`
- Create: `deploy/argocd/manifests/platform/hcloud-csi/values.yaml`

- [ ] **Step 1: Copy values into the GitOps path**

```bash
mkdir -p deploy/argocd/manifests/platform/hcloud-ccm
mkdir -p deploy/argocd/manifests/platform/hcloud-csi
cp deploy/bootstrap/hcloud-ccm-values.yaml deploy/argocd/manifests/platform/hcloud-ccm/values.yaml
cp deploy/bootstrap/hcloud-csi-values.yaml deploy/argocd/manifests/platform/hcloud-csi/values.yaml
```

- [ ] **Step 2: Applications**

Create `deploy/argocd/apps/platform/hcloud-ccm.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: hcloud-ccm
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://charts.hetzner.cloud
      chart: hcloud-cloud-controller-manager
      targetRevision: 1.20.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/hcloud-ccm/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: hcloud-system
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

Create `deploy/argocd/apps/platform/hcloud-csi.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: hcloud-csi
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://charts.hetzner.cloud
      chart: hcloud-csi
      targetRevision: 2.10.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/hcloud-csi/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: hcloud-system
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

- [ ] **Step 3: Commit + push**

```bash
git add deploy/argocd/apps/platform/hcloud-{ccm,csi}.yaml \
        deploy/argocd/manifests/platform/hcloud-{ccm,csi}/
git commit -m "$(cat <<'EOF'
gitops: migrate hcloud-ccm and hcloud-csi to argo management

No state change — Argo adopts the existing helm release shape and reconciles.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 4: Verify**

In ArgoCD UI: both `hcloud-ccm` and `hcloud-csi` Applications appear, `Synced + Healthy`. Pods in `hcloud-system` namespace continue running (no churn).

If Argo reports `OutOfSync` for trivial diffs (e.g., labels), inspect with the UI Diff view; common fix is to add `argocd.argoproj.io/compare-options: IgnoreExtraneous` to the Helm release annotations, or add `respectIgnoreDifferences` in the Application spec.

---

## Task 18: Migrate ingress-nginx and cert-manager into ArgoCD

**Files:**
- Create: `deploy/argocd/apps/platform/ingress-nginx.yaml`
- Create: `deploy/argocd/apps/platform/cert-manager.yaml`
- Create: `deploy/argocd/apps/platform/cert-issuers.yaml`
- Create: `deploy/argocd/manifests/platform/ingress-nginx/values.yaml`
- Create: `deploy/argocd/manifests/platform/cert-manager/values.yaml`
- Create: `deploy/argocd/manifests/platform/cert-issuers/{kustomization,issuer-staging,issuer-prod}.yaml`

- [ ] **Step 1: Copy values into the GitOps path**

```bash
mkdir -p deploy/argocd/manifests/platform/ingress-nginx
mkdir -p deploy/argocd/manifests/platform/cert-manager
mkdir -p deploy/argocd/manifests/platform/cert-issuers
cp deploy/bootstrap/ingress-nginx-values.yaml deploy/argocd/manifests/platform/ingress-nginx/values.yaml
cp deploy/bootstrap/cert-manager-values.yaml  deploy/argocd/manifests/platform/cert-manager/values.yaml
```

- [ ] **Step 2: Split cluster-issuers.yaml into Kustomize**

Create `deploy/argocd/manifests/platform/cert-issuers/issuer-staging.yaml`:

```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-staging
spec:
  acme:
    server: https://acme-staging-v02.api.letsencrypt.org/directory
    email: REPLACE_WITH_YOUR_EMAIL
    privateKeySecretRef: { name: letsencrypt-staging-account }
    solvers:
      - dns01:
          cloudflare:
            apiTokenSecretRef:
              name: cloudflare-api-token
              key: api-token
```

Create `deploy/argocd/manifests/platform/cert-issuers/issuer-prod.yaml`:

```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-prod
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: REPLACE_WITH_YOUR_EMAIL
    privateKeySecretRef: { name: letsencrypt-prod-account }
    solvers:
      - dns01:
          cloudflare:
            apiTokenSecretRef:
              name: cloudflare-api-token
              key: api-token
```

Create `deploy/argocd/manifests/platform/cert-issuers/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
# No `namespace:` — ClusterIssuer is a cluster-scoped resource.
# The cloudflare-api-token Secret it references lives in cert-manager,
# which we set on the ClusterIssuer's solver spec, not via Kustomize.
resources:
  - issuer-staging.yaml
  - issuer-prod.yaml
```

- [ ] **Step 3: Applications**

Create `deploy/argocd/apps/platform/ingress-nginx.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: ingress-nginx
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://kubernetes.github.io/ingress-nginx
      chart: ingress-nginx
      targetRevision: 4.11.2
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/ingress-nginx/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: ingress-nginx
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

Create `deploy/argocd/apps/platform/cert-manager.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: cert-manager
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://charts.jetstack.io
      chart: cert-manager
      targetRevision: v1.15.3
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/cert-manager/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: cert-manager
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

Create `deploy/argocd/apps/platform/cert-issuers.yaml` — this one is Kustomize, not Helm:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: cert-issuers
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/platform/cert-issuers
  destination:
    server: https://kubernetes.default.svc
    namespace: cert-manager
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 4: Commit + push + verify**

```bash
git add deploy/argocd/apps/platform/{ingress-nginx,cert-manager,cert-issuers}.yaml \
        deploy/argocd/manifests/platform/{ingress-nginx,cert-manager,cert-issuers}/
git commit -m "$(cat <<'EOF'
gitops: migrate ingress-nginx, cert-manager, cluster-issuers to argo

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD UI, all three new Applications appear `Synced + Healthy`. `kubectl get clusterissuers` continues to show both as `READY: True`.

---

## Task 19: Install external-secrets-operator via ArgoCD

**Files:**
- Create: `deploy/argocd/apps/platform/external-secrets.yaml`
- Create: `deploy/argocd/manifests/platform/external-secrets/values.yaml`

- [ ] **Step 1: Helm values**

Create `deploy/argocd/manifests/platform/external-secrets/values.yaml`:

```yaml
installCRDs: true

certController:
  resources:
    requests: { cpu: 10m, memory: 32Mi }
    limits: { cpu: 100m, memory: 128Mi }

webhook:
  resources:
    requests: { cpu: 10m, memory: 32Mi }
    limits: { cpu: 100m, memory: 128Mi }

resources:
  requests: { cpu: 10m, memory: 64Mi }
  limits: { cpu: 100m, memory: 256Mi }
```

- [ ] **Step 2: Application**

Create `deploy/argocd/apps/platform/external-secrets.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: external-secrets-helm
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://charts.external-secrets.io
      chart: external-secrets
      targetRevision: 0.10.4
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/external-secrets/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: external-secrets
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

(The Application name is `external-secrets-helm` to leave space for the sibling `external-secrets-css` Application added in Task 22.)

- [ ] **Step 3: Commit + push**

```bash
git add deploy/argocd/apps/platform/external-secrets.yaml \
        deploy/argocd/manifests/platform/external-secrets/
git commit -m "$(cat <<'EOF'
gitops: install external-secrets-operator

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 4: Verify**

In ArgoCD UI: `external-secrets` Application `Synced + Healthy`. `kubectl get crd | grep external-secrets` should list `clustersecretstores.external-secrets.io`, `externalsecrets.external-secrets.io`, `secretstores.external-secrets.io`, etc.

---

## Task 20: 1Password Connect — prepare credentials

**Files:** none (manual setup + one kubectl create secret)

This is the only manual secret in the cluster from now on. Every other secret flows through ESO.

- [ ] **Step 1: Create a 1Password Connect server**

In the 1Password web UI:
- Integrations → Directory → 1Password Connect → Create
- Name: `todo-app-cluster`
- Vault access: grant access to `todo-app` vault only
- Save the generated `1password-credentials.json` (downloads to your computer)
- Save the access token (shown once — copy now)

- [ ] **Step 2: Stash the credentials back in 1Password (DR)**

Create a 1Password item `op-connect-bootstrap` with:
- File attachment: `1password-credentials.json`
- Field: `access_token` (the long string)

- [ ] **Step 3: Create the bootstrap secret in-cluster**

```bash
mkdir -p ~/secure
op document get op-connect-bootstrap --output ~/secure/1password-credentials.json
# Confirm the file is JSON (starts with {)

kubectl create namespace onepassword-connect

kubectl -n onepassword-connect create secret generic op-credentials \
  --from-file=1password-credentials.json=$HOME/secure/1password-credentials.json

kubectl -n onepassword-connect create secret generic op-access-token \
  --from-literal=token="$(op item get op-connect-bootstrap --field access_token)"

# Wipe the local copy
shred -u ~/secure/1password-credentials.json 2>/dev/null || rm -P ~/secure/1password-credentials.json
```

- [ ] **Step 4: Verify both secrets exist**

```bash
kubectl -n onepassword-connect get secrets
```

Expected: both `op-credentials` and `op-access-token`.

No commit — these secrets are runtime state, not code.

---

## Task 21: Install 1Password Connect via ArgoCD

**Files:**
- Create: `deploy/argocd/apps/platform/onepassword-connect.yaml`
- Create: `deploy/argocd/manifests/platform/onepassword-connect/values.yaml`

- [ ] **Step 1: Helm values**

Create `deploy/argocd/manifests/platform/onepassword-connect/values.yaml`:

```yaml
# The 1Password Connect chart ("@1password/connect" on Helm) reads the
# credentials.json file from a Secret we created manually (Task 20).
connect:
  credentialsKey: 1password-credentials.json
  credentialsName: op-credentials

operator:
  create: false   # we use the standalone external-secrets-operator, not the 1P operator

service:
  type: ClusterIP
  port: 8080
```

- [ ] **Step 2: Application**

Create `deploy/argocd/apps/platform/onepassword-connect.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: onepassword-connect
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://1password.github.io/connect-helm-charts
      chart: connect
      targetRevision: 1.16.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/onepassword-connect/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: onepassword-connect
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/apps/platform/onepassword-connect.yaml \
        deploy/argocd/manifests/platform/onepassword-connect/
git commit -m "$(cat <<'EOF'
gitops: install 1password connect

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD UI: `onepassword-connect` Application `Synced + Healthy`. The Connect pod should be `Running`.

```bash
kubectl -n onepassword-connect get pods
kubectl -n onepassword-connect logs deployment/onepassword-connect-connect | head -20
```

Expected: no errors, log line `Connect API listening on :8080`.

---

## Task 22: ClusterSecretStore — wire ESO to 1Password Connect

**Files:**
- Create: `deploy/argocd/manifests/platform/external-secrets/cluster-secret-store.yaml`
- Update: `deploy/argocd/apps/platform/external-secrets.yaml` (add Kustomize source alongside Helm)

The Helm-only Application from Task 19 just installs ESO. We need to also apply a `ClusterSecretStore` resource, which is a separate manifest. We'll add it via a Kustomize sub-Application.

- [ ] **Step 1: ClusterSecretStore manifest**

Create `deploy/argocd/manifests/platform/external-secrets/cluster-secret-store.yaml`:

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ClusterSecretStore
metadata:
  name: onepassword-connect
spec:
  provider:
    onepassword:
      connectHost: http://onepassword-connect-connect.onepassword-connect.svc.cluster.local:8080
      vaults:
        todo-app: 1
      auth:
        secretRef:
          connectTokenSecretRef:
            name: op-access-token
            namespace: onepassword-connect
            key: token
```

- [ ] **Step 2: Wrap it in a Kustomize sub-app**

(The Helm Application from Task 19 was already named `external-secrets-helm.yaml`, so no rename is needed — we just add a second Application file alongside it.)

Create `deploy/argocd/apps/platform/external-secrets-css.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: external-secrets-css
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/platform/external-secrets
    # Kustomize will pick up the cluster-secret-store.yaml. Add a kustomization.yaml
    # to make this explicit:
  destination:
    server: https://kubernetes.default.svc
    namespace: external-secrets
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

Also create `deploy/argocd/manifests/platform/external-secrets/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - cluster-secret-store.yaml
# Note: values.yaml is consumed by the Helm Application; Kustomize doesn't see it.
```

- [ ] **Step 3: Commit + push**

```bash
git add deploy/argocd/apps/platform/external-secrets-css.yaml \
        deploy/argocd/manifests/platform/external-secrets/cluster-secret-store.yaml \
        deploy/argocd/manifests/platform/external-secrets/kustomization.yaml
git commit -m "$(cat <<'EOF'
gitops: wire ESO ClusterSecretStore to 1password connect

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 4: Verify**

```bash
kubectl get clustersecretstore
```

Expected: `onepassword-connect` exists, status `STATUS: Valid` and `READY: True`. If `Ready: False`, inspect:

```bash
kubectl describe clustersecretstore onepassword-connect
kubectl -n onepassword-connect logs deployment/onepassword-connect-connect --tail=50
```

Most common failures: typo in `connectHost`, missing `op-access-token` secret, token expired.

---

## Task 23: Smoke test — round-trip a secret from 1Password to a Kubernetes Secret

**Files:**
- Create: `deploy/argocd/manifests/smoke/kustomization.yaml`
- Create: `deploy/argocd/manifests/smoke/external-secret.yaml`
- Create: `deploy/argocd/apps/platform/smoke.yaml`

- [ ] **Step 1: Add a test item to the 1Password vault**

In 1Password (vault `todo-app`):
- Create item type **Password**
- Name: `smoke-test`
- Add a field `secret` with value `hello-world-1234`

- [ ] **Step 2: Smoke ExternalSecret**

Create `deploy/argocd/manifests/smoke/external-secret.yaml`:

```yaml
---
apiVersion: v1
kind: Namespace
metadata:
  name: smoke
---
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: smoke-test
  namespace: smoke
spec:
  refreshInterval: 1m
  secretStoreRef:
    name: onepassword-connect
    kind: ClusterSecretStore
  target:
    name: smoke-test
    creationPolicy: Owner
  data:
    - secretKey: secret
      remoteRef:
        key: smoke-test
        property: secret
```

Create `deploy/argocd/manifests/smoke/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources:
  - external-secret.yaml
```

- [ ] **Step 3: Application**

Create `deploy/argocd/apps/platform/smoke.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: smoke
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/smoke
  destination:
    server: https://kubernetes.default.svc
    namespace: smoke
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

- [ ] **Step 4: Commit + push**

```bash
git add deploy/argocd/manifests/smoke/ deploy/argocd/apps/platform/smoke.yaml
git commit -m "$(cat <<'EOF'
gitops: end-to-end smoke test for 1password -> ESO -> native secret

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 5: Verify the round-trip**

In ArgoCD UI: `smoke` Application `Synced + Healthy`. Then:

```bash
kubectl -n smoke get externalsecret smoke-test
```

Expected status `SYNCED`.

```bash
kubectl -n smoke get secret smoke-test -o jsonpath='{.data.secret}' | base64 -d
```

Expected: `hello-world-1234`.

**This is the success criterion for Plan 1.** A secret stored in 1Password materializes as a native k8s Secret without any human touch beyond the original 1Password write.

- [ ] **Step 6: Tear down the smoke Application**

Once verified, you can leave the smoke Application running indefinitely (it's tiny and serves as a continuous check), or remove it:

```bash
# To remove: delete the Application file and push
git rm deploy/argocd/apps/platform/smoke.yaml deploy/argocd/manifests/smoke/*.yaml
git commit -m "smoke: remove ESO round-trip test (verified)"
git push
```

(Keeping it around is fine; recommended even — it's an early-warning canary if 1Password Connect breaks.)

---

## Task 24: Justfile recipes for cluster lifecycle

**Files:**
- Modify: `justfile`

Add convenience commands so future-you doesn't need to remember exact paths.

- [ ] **Step 1: Append k8s + tofu recipes to justfile**

Open `justfile` and append:

```make
# --- Tofu / cluster lifecycle ---

# tofu plan against the prod cluster state
tofu-plan:
    cd deploy/tofu && tofu plan

# tofu apply (prompts for yes)
tofu-apply:
    cd deploy/tofu && tofu apply

# Show cluster outputs (node IPs, LB IP, etc.)
tofu-outputs:
    cd deploy/tofu && tofu output

# --- Kubernetes ---

# Use the cluster kubeconfig for the current shell
k8s-export:
    @echo "export KUBECONFIG=~/.kube/config-todo"

# Pull a fresh kubeconfig from node 0
k8s-kubeconfig:
    NODE0=$$(cd deploy/tofu && tofu output -raw first_node_ipv4) && \
        scp root@$$NODE0:/etc/rancher/k3s/k3s.yaml ~/.kube/config-todo && \
        sed -i.bak "s|server: https://127.0.0.1:6443|server: https://$$NODE0:6443|" ~/.kube/config-todo && \
        chmod 600 ~/.kube/config-todo

# Verify cluster health
k8s-ps:
    KUBECONFIG=~/.kube/config-todo kubectl get nodes
    KUBECONFIG=~/.kube/config-todo kubectl get pods --all-namespaces -o wide

# ArgoCD UI port-forward (fallback if ingress is broken)
k8s-argocd-pf:
    KUBECONFIG=~/.kube/config-todo kubectl -n argocd port-forward svc/argocd-server 8080:443

# Validate all argocd manifests offline
k8s-validate:
    kustomize build deploy/argocd/manifests/platform/cert-issuers   | kubeconform -strict -ignore-missing-schemas -summary
    kustomize build deploy/argocd/manifests/platform/external-secrets | kubeconform -strict -ignore-missing-schemas -summary
    kustomize build deploy/argocd/manifests/smoke                    | kubeconform -strict -ignore-missing-schemas -summary
```

- [ ] **Step 2: Verify a couple of recipes**

```bash
just tofu-outputs
just k8s-ps
just k8s-validate
```

All three should run without error.

- [ ] **Step 3: Commit**

```bash
git add justfile
git commit -m "$(cat <<'EOF'
just: add cluster lifecycle recipes (tofu-* and k8s-*)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 25: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a deploy section to README**

In `README.md`, near the bottom (before any existing "Contributing" / "License" section), add:

```markdown
## Production deployment (Hetzner k3s)

The production deployment runs on a self-managed 3-node k3s HA cluster on
Hetzner Cloud, managed via OpenTofu + ArgoCD. Secrets live in 1Password.

- **Design spec:** `docs/superpowers/specs/2026-05-18-k8s-deploy-design.md`
- **Implementation plans:** `docs/superpowers/plans/2026-05-18-k8s-foundation.md`
  is Plan 1 of 5 (cluster + platform + secrets); subsequent plans land the app,
  CI/CD, observability, preview environments, and local k3d parity.
- **Day-2 ops:** see `deploy/tofu/README.md` for cluster provisioning and
  `deploy/bootstrap/README.md` for the initial install order. After ArgoCD is
  up, all changes flow via git: edit `deploy/argocd/manifests/...`, commit,
  push.

Quick commands:

```bash
just tofu-plan          # preview infra changes
just tofu-apply         # apply infra changes
just k8s-kubeconfig     # pull a fresh kubeconfig
just k8s-ps             # cluster overview
just k8s-validate       # offline manifest validation
```
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs: link production deployment plan from main README

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final verification

After all 25 tasks are complete, run this end-to-end checklist:

- [ ] `kubectl get nodes` — three nodes `Ready`, all `control-plane,etcd,master`
- [ ] `kubectl -n argocd get app` — all Applications `Synced + Healthy`:
  - `root`
  - `argocd`
  - `hcloud-ccm`, `hcloud-csi`
  - `ingress-nginx`, `cert-manager`, `cert-issuers`
  - `external-secrets-helm`, `external-secrets-css`
  - `onepassword-connect`
- [ ] `kubectl get clusterissuers` — `letsencrypt-staging` and `letsencrypt-prod` both `READY: True`
- [ ] `kubectl get clustersecretstore` — `onepassword-connect` `READY: True`
- [ ] Open `https://argocd.todo.<yourdomain>` in a browser — valid LE production cert, ArgoCD UI loads
- [ ] (If smoke retained) `kubectl -n smoke get secret smoke-test -o jsonpath='{.data.secret}' | base64 -d` returns `hello-world-1234`
- [ ] `just k8s-validate` exits 0

When all of the above pass, this plan is complete. Hand off to **Plan 2 (App + CI/CD)** by invoking the writing-plans skill with the next milestone.
