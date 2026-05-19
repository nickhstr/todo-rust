# Local k3d parity

The compose stack (`just up`) is the daily inner loop. This directory adds
a `just up-k8s` recipe that spins up the production Kustomize manifests on
a local k3d cluster — useful for validating a manifest change before
pushing it to a real cluster.

## Differences from prod

| Prod | Local |
|---|---|
| ArgoCD reconciles | Direct `kubectl apply -k` |
| 1Password → ESO → Secret | Plain Secret with a throwaway session key |
| Hetzner CSI volumes | k3s local-path provisioner |
| ingress-nginx + cert-manager + LB | `kubectl port-forward` to the Service |
| 2 app replicas | 1 |
| CNPG 2-instance + WAL archive | CNPG 1-instance, no backup |
| OTel → in-cluster collector | OTel disabled (no observability stack locally) |

## Usage

```bash
just up-k8s            # bring up cluster + app
just down-k8s          # tear down everything

# After 'up':
just fwd-k8s           # port-forward 8080 -> svc/todo-app
# Open http://localhost:8080
```

## When to use this vs compose

- **compose** (`just up`): daily dev. Hot reload, fast inner loop.
- **k3d** (`just up-k8s`): validating manifests, debugging CRD interactions,
  practicing k8s troubleshooting commands. Slower but real Kubernetes.
- **PR preview** (label PR `preview`): validating against the real cluster.
  Slowest, most realistic.
