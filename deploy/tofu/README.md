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
