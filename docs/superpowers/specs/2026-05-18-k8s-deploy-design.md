# Production deployment on Kubernetes — design

Owner: nickhstr
Date: 2026-05-18
Status: approved (brainstorming) — implementation plan to follow via writing-plans

## Context

`todo-rust` runs production-shaped today via `docker compose`: axum + Postgres + Valkey, cookie sessions, OTel → Tempo / Loki / Prometheus / Grafana, multi-stage release image landing in distroless. The next step is moving the production deployment to Kubernetes, primarily as a learning exercise. The app likely has zero real users; the deployment is the point.

## Goals

1. Run the app on a real Kubernetes cluster the owner manages, gaining hands-on k8s ops experience.
2. Self-host the cluster (or as close to it as practical) to also exercise infra ops.
3. Approximate the production deployment locally for verification work.
4. Use GitHub Actions for CI/CD.
5. Support per-PR preview environments.
6. Manage secrets via the owner's existing 1Password subscription.
7. Keep cost in the range of "reasonable for a personal learning project" — single-digit-to-low-double-digit dollars per month is fine; AWS-scale is not justified.

## Non-goals

- Multi-region or DR-grade resilience. Single Hetzner DC is fine.
- Real-traffic optimization. The compose stack already meets app-level perf goals; the k8s version inherits the same Rust binary.
- Migration from the existing dev compose workflow. Compose stays the daily inner loop.
- Replacing the existing observability stack with a hosted vendor; we re-host the same components in-cluster.

## Decisions

| Area | Decision | Reasoning |
|---|---|---|
| Cluster host | Hetzner Cloud (Nuremberg DC `nbg1` by default), self-managed (not managed k8s, not bare-metal home server) | Cheapest path to real internet-facing infrastructure with maximum k8s ops learning. Bare-metal at home was rejected because constrained hardware limits scale-out experiments. |
| Cluster size | 3× CX22 nodes in k3s HA mode (3 servers, embedded etcd) | Real HA cluster (survives 1-node loss), enables learning rolling updates / drainage / PDBs / anti-affinity. ~$13.50/mo total. |
| Kubernetes distribution | k3s | Conformant k8s, lighter to install/operate than upstream, largest ecosystem coverage for self-hosters, traditional Linux node admin still visible (SSH + systemd + containerd) so the owner learns nodes too. Talos was rejected as too unusual; vanilla kubeadm as too time-sunk vs payoff for this stage. |
| IaC | OpenTofu | Open-source Terraform fork, same syntax, no HashiCorp license drama. State stored in Hetzner Object Storage (S3-compatible) with locking. |
| Data layer | In-cluster: CloudNativePG operator for Postgres, StatefulSet for Valkey. PVCs backed by Hetzner CSI driver (cloud volumes). | Maximum stateful-workload learning (StatefulSets, PVCs, CSI, operators, backups, point-in-time recovery). CNPG is the industry-standard Postgres operator. |
| GitOps | ArgoCD | Pull-based; has a UI which is valuable while learning (visible sync status, drift, app health); native PR-driven preview environments via ApplicationSet PullRequestGenerator; biggest community footprint. |
| Manifest formats | Helm for upstream charts (platform), Kustomize for app overlays | Don't rewrite upstream charts. Own-app manifests don't need template power; Kustomize overlays handle per-env diffs cleanly. |
| Local production-parity | Compose remains the inner-loop; `just up-k8s` spins up k3d for k8s-path validation | Don't sacrifice the well-tuned compose dev experience. k3d is the escape hatch for "does this k8s manifest actually work." |
| DNS | Cloudflare (using existing owner-provided domain) | Free, fast API, first-class cert-manager DNS-01 solver, supports wildcard certs needed for preview envs. |
| TLS | cert-manager + Let's Encrypt + Cloudflare DNS-01 solver | Wildcard cert `*.<subdomain>.<domain>` covers prod, staging, and all preview envs without per-PR issuance churn. |
| Secrets | 1Password Connect (in-cluster) + external-secrets-operator | Cluster-internal access (no public 1Password endpoint), real "secrets in vault, materialized as k8s Secret" learning. |
| CI/CD | GitHub Actions; CI commits manifest tag bumps directly back to repo | Simplest mental model, every deploy traceable to a single CI run, easy rollback via `git revert`. Argo Image Updater can be swapped in later as a learning exercise. |
| Image registry | GHCR | Free for public repo; private repo would need a pull secret in-cluster, easy via ESO/1Password. |
| Preview environments | ArgoCD ApplicationSet PullRequestGenerator, ephemeral per-PR namespace (own CNPG cluster, own Valkey, own PVCs); torn down on PR close | Ephemeral isolation maximizes both correctness fidelity and learning vs a shared dev DB. |
| Alerts | Alertmanager → Gmail SMTP via App Password (cred sourced from 1Password through ESO) | No third-party signup, free, integrates with the chosen secrets path. |

## Architecture overview

```
                  ┌──────────────────────────────────────────────┐
GitHub ── push ─▶│         GitHub Actions (CI)                  │
                  │  build → push image → bump manifest         │
                  └──────────────┬───────────────────────────────┘
                                 │ git commit (deploy/ subfolder)
                                 ▼
                 ┌────────────────────────────────────────────────┐
Cloudflare DNS  │       Hetzner Cloud (Nuremberg)                │
   *.app.you ──▶│  k3s HA: 3× CX22 (server + server + server)    │
                │                                                │
                │  Platform:  ArgoCD · ingress-nginx ·            │
                │             cert-manager · CloudNativePG op ·   │
                │             external-secrets-operator ·         │
                │             1Password Connect ·                 │
                │             metrics-server                      │
                │                                                │
                │  Apps:      todo-app-prod (ns)                  │
                │             todo-app-staging (ns)               │
                │             todo-app-pr-<N> (ns, ephemeral)     │
                │                                                │
                │  Data:      CNPG cluster per ns                 │
                │             valkey StatefulSet per ns           │
                │                                                │
                │  Obs (ns observability):                        │
                │             kube-prometheus-stack · loki ·      │
                │             tempo · grafana · otel-collector ·  │
                │             alertmanager → Gmail SMTP           │
                └────────────────────────────────────────────────┘
                                  │
                       Hetzner CSI│ → Cloud Volumes for PVCs
                                  │
                       Hetzner LB │ → public IP for ingress
```

Three environments share one cluster:

| Env | Namespace | Hostname | App replicas | Postgres | Notes |
|---|---|---|---|---|---|
| prod | `todo-app-prod` | `todo.<domain>` | 2 (HPA 2–6) | CNPG, primary + 1 replica, 10Gi PVC, WAL archive to Hetzner Object Storage | The real one |
| staging | `todo-app-staging` | `staging.todo.<domain>` | 1 | CNPG, primary only, 5Gi PVC | Mirrors prod manifests; gets every `main` build |
| preview | `todo-app-pr-<N>` | `pr-<N>.todo.<domain>` | 1 | CNPG, primary only, 1Gi PVC, no backups | Ephemeral, torn down on PR close |

## A. Infrastructure provisioning (OpenTofu)

```
deploy/tofu/
├── network/      Hetzner private network (10.0.0.0/16) + firewall:
│                   - SSH (22): owner IP only
│                   - k8s API (6443): owner IP only
│                   - HTTP (80): world (LB-only ingress)
│                   - HTTPS (443): world (LB-only ingress)
│                   - VXLAN (8472/udp), Wireguard k3s flannel: internal only
├── cluster/      3× CX22 nodes; cloud-init joins them to k3s HA:
│                   - First node: `k3s server --cluster-init --tls-san <LB-IP>`
│                   - Other two: `k3s server --server https://<first-node>:6443 --token <shared>`
│                   - Token: random secret, stored in Hetzner Object Storage + 1Password
│                 Each node tagged `role=server` (no dedicated agents at this size).
├── dns/          Cloudflare DNS records:
│                   - `todo.<domain>` A → LB IP
│                   - `staging.todo.<domain>` A → LB IP
│                   - `*.todo.<domain>` A → LB IP (covers preview envs)
│                   - `grafana.<domain>` A → LB IP
└── object-storage/  Hetzner Object Storage bucket:
                       - `<bucket>/tofu-state/` (Terraform state, locked)
                       - `<bucket>/wal/` (CNPG WAL archive)
                       - `<bucket>/backups/` (CNPG base backups)
```

`tofu apply` is idempotent and reproducible: lose a node, rebuild it the same way. State in object storage; the bucket itself is created outside Tofu (one-shot via Hetzner CLI) so we have somewhere for state to land before the first apply.

## B. Cluster platform & bootstrap order

### Bootstrap order

1. `tofu apply` creates network + 3 nodes + DNS records. Cloud-init installs k3s HA.
2. Owner fetches kubeconfig from first server: `scp root@<node1>:/etc/rancher/k3s/k3s.yaml ~/.kube/config-todo`, edits `server:` to LB IP.
3. One-shot bootstrap script (`deploy/bootstrap/install-argocd.sh`) `kubectl apply`s a Helm-templated ArgoCD manifest.
4. ArgoCD comes up. Apply a single root `Application` (`deploy/argocd/apps/root.yaml`) pointing at `deploy/argocd/apps/`. This is the App-of-Apps pattern: that folder contains child `Application`s for every platform component + the app itself.
5. ArgoCD self-manages from this point. Bootstrap script is run exactly once per cluster.

### Platform components (each is its own ArgoCD Application)

| Component | Chart | Purpose |
|---|---|---|
| ingress-nginx | `kubernetes/ingress-nginx` | HTTP/HTTPS controller; exposed via Hetzner LoadBalancer Service (`hcloud-load-balancer` annotations) |
| cert-manager | `jetstack/cert-manager` | Cert lifecycle; ClusterIssuer = Let's Encrypt with Cloudflare DNS-01 solver |
| cloudnative-pg | `cnpg/cloudnative-pg` | Postgres operator: primary/replica, failover, WAL archive, PITR |
| external-secrets | `external-secrets/external-secrets` | Watches `ExternalSecret`s, fetches from 1Password Connect |
| 1password-connect | `1password/connect` | In-cluster 1Password integration server (Deployment) |
| kube-prometheus-stack | `prometheus-community/kube-prometheus-stack` | Prometheus operator + Prometheus + Alertmanager + Grafana + node-exporter + kube-state-metrics |
| loki | `grafana/loki` | Logs; single-binary mode, filesystem on Hetzner volume |
| tempo | `grafana/tempo` | Traces; monolithic mode, filesystem on Hetzner volume |
| opentelemetry-collector | `open-telemetry/opentelemetry-collector` | OTLP ingest → tempo / loki / prom |
| metrics-server | (k3s ships it; enable) | Pod metrics for HPA |

Storage retention: Prometheus 7d, Loki 7d, Tempo 3d. Total observability footprint ~25Gi.

### Manifest layout (in this repo)

```
deploy/
├── tofu/                    OpenTofu modules (see Section A)
├── bootstrap/               One-shot: ArgoCD initial install
└── argocd/
    ├── apps/                App-of-Apps root + per-component Applications
    │   ├── root.yaml
    │   ├── platform/        One Application per platform component
    │   └── todo-app/        ApplicationSet for prod + previews; Application for staging
    └── manifests/
        ├── platform/        Helm values + raw YAML per chart
        │   ├── ingress-nginx/
        │   ├── cert-manager/
        │   ├── external-secrets/
        │   ├── cloudnative-pg/
        │   └── observability/
        └── todo-app/
            ├── base/        Kustomize base: Deployment, Service, Ingress, HPA, PDB, CNPG Cluster, Valkey StatefulSet, ExternalSecret, ServiceMonitor
            └── overlays/
                ├── prod/
                ├── staging/
                ├── preview/   Template for PR generator
                └── local/     k3d / non-1Password fallback
```

Single-repo layout: ArgoCD watches `https://github.com/nickhstr/todo-rust.git` path `deploy/argocd/`. Can be extracted to a separate `infra` repo later if scope grows; not needed at this stage.

## C. Application & data layer

### Per-namespace contents

```
todo-app-<env>
├── Deployment todo-app
│   ├── replicas: prod 2, staging 1, preview 1
│   ├── topology spread constraint (nodes)
│   ├── PodDisruptionBudget min=1 (prod only)
│   ├── resources: req 100m/128Mi, lim 500m/256Mi
│   ├── readinessProbe → /readyz
│   ├── livenessProbe → /healthz
│   ├── envFrom: ConfigMap todo-app-config + Secret todo-app-secrets + Secret <cnpg-cluster>-app
│   └── image: ghcr.io/nickhstr/todo-app:<tag>
├── HorizontalPodAutoscaler (prod): 2–6 replicas, target CPU 70%
├── Service ClusterIP
├── Ingress (ingress-nginx class)
│   ├── tls: secretName <env>-wildcard-cert (managed by cert-manager Certificate)
│   └── host: per env (see env table)
├── ServiceMonitor scrapes /metrics
├── CNPG Cluster <env>-postgres
│   ├── instances: prod 2, staging 1, preview 1
│   ├── storage.size: 10Gi / 5Gi / 1Gi
│   ├── bootstrap.initdb (no template needed; app runs sqlx migrations at startup)
│   └── backup (prod only): WAL archive to Hetzner Object Storage via barmanObjectStore
├── Valkey StatefulSet
│   ├── 1 replica, 1Gi PVC, AOF enabled
│   └── Service ClusterIP (`valkey.<ns>.svc.cluster.local`)
└── ExternalSecret todo-app-secrets
    └── pulls SESSION_KEY from 1Password vault item `<env>/SESSION_KEY`
```

### Application configuration

Existing env-var contract from compose ports straight over:

| Env var | Source |
|---|---|
| `DATABASE_URL` | CNPG-generated secret `<cluster>-app` (operator manages credentials) |
| `REDIS_URL` | ConfigMap; value `redis://valkey.<ns>.svc.cluster.local:6379` |
| `APP__AUTH__SESSION_KEY` | ExternalSecret → 1Password item `<env>/SESSION_KEY` |
| `APP__AUTH__COOKIE_SECURE` | ConfigMap; `"true"` everywhere (ingress terminates TLS, including preview envs) |
| `APP__OBSERVABILITY__OTEL_ENDPOINT` | ConfigMap; `http://otel-collector.observability.svc:4317` |
| `APP__OBSERVABILITY__OTEL_ENABLED` | ConfigMap; `"true"` |
| `APP__OBSERVABILITY__LOG_FORMAT` | ConfigMap; `"json"` |
| `RUST_LOG` | ConfigMap |

`X-App-Version` continues to work — `GIT_SHA` is passed as a build-arg by GHA exactly like `just up` does today.

### Migrations

App already calls `MIGRATOR.run()` on startup (`crates/storage/src/lib.rs`). No initContainer required. If belt-and-braces becomes desirable later, add `sqlx migrate run` as an initContainer reusing the same image.

## D. Secrets, CI/CD, preview environments

### Secrets — 1Password → ESO

One-time bootstrap:
1. Create vault `todo-app` in 1Password.
2. Create a 1Password Connect server in that vault; download `1password-credentials.json` + access token; stash both back in the vault for DR.
3. `kubectl create secret -n external-secrets generic op-credentials --from-file=...` once. This is the only manually-created secret in the cluster.
4. ArgoCD deploys 1Password Connect (Helm) referencing that bootstrap secret.
5. ESO `ClusterSecretStore` named `onepassword-connect` wired to Connect's in-cluster Service.

Runtime:
```yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: todo-app-secrets
spec:
  refreshInterval: 1h
  secretStoreRef: { name: onepassword-connect, kind: ClusterSecretStore }
  target: { name: todo-app-secrets }
  data:
    - secretKey: APP__AUTH__SESSION_KEY
      remoteRef: { key: todo-app/SESSION_KEY }
```

Items in the `todo-app` vault, namespaced by env:
- `prod/SESSION_KEY` — hex-encoded, ≥64 bytes decoded; see `Config::decoded_session_key`
- `staging/SESSION_KEY`
- `preview/SESSION_KEY` (shared across all preview envs)
- `smtp-gmail` (Gmail App Password for Alertmanager)
- `cloudflare-api-token` (cert-manager DNS-01 solver)
- `github-pr-token` (read-only token for ArgoCD ApplicationSet PR generator to poll GitHub PRs)
- `gh-deploy-token` (write-scoped, used in GHA to commit manifest bumps — lives in GitHub repo secrets, not 1Password directly; rotated periodically from a 1Password mirror)

Single vault for now; split later if scope demands.

### CI/CD — three GitHub Actions workflows

**`pr-validate.yml`** (trigger: `pull_request`):
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` (testcontainers work on GHA)
- `docker buildx build` (no push — smoke test the multistage)
- `kustomize build deploy/argocd/manifests/todo-app/overlays/staging | kubeconform -strict` (manifest validation)

**`main-deploy.yml`** (trigger: push to `main`):
- `cargo test --workspace --lib --bins` (unit-only; integration ran in PR)
- `docker buildx build --push --tag ghcr.io/nickhstr/todo-app:<sha> --build-arg GIT_SHA=<sha>`
- `cd deploy/argocd/manifests/todo-app/overlays/staging && kustomize edit set image todo-app=ghcr.io/nickhstr/todo-app:<sha>`
- `git commit -am "staging: deploy <sha>"` + `git push`
- Guard against self-trigger on `main-deploy`: the bot's commits to `deploy/` would otherwise re-fire `main-deploy`. Either gate with `if: github.actor != 'github-actions[bot]'`, or scope `paths-ignore: [deploy/**]` on the `main-deploy` push trigger. `pr-validate` doesn't need this guard (it fires on `pull_request`, not push).

**`preview-build.yml`** (trigger: `pull_request` open/synchronize):
- `docker buildx build --push --tag ghcr.io/nickhstr/todo-app:pr-<N>-<sha> --build-arg GIT_SHA=<sha>`
- No manifest commit; ArgoCD ApplicationSet picks up the new image via templated tag

**`promote-prod.yml`** (trigger: `workflow_dispatch` with `sha` input, or `v*` tag):
- `cd deploy/argocd/manifests/todo-app/overlays/prod && kustomize edit set image todo-app=ghcr.io/nickhstr/todo-app:<sha>`
- Commit + push; ArgoCD reconciles prod

Auth: a GitHub App or fine-scoped PAT in repo secrets (`GH_DEPLOY_TOKEN`) with write access to this repo only.

### Preview env wiring — ApplicationSet PR generator

```yaml
apiVersion: argoproj.io/v1alpha1
kind: ApplicationSet
metadata:
  name: todo-app-previews
spec:
  generators:
    - pullRequest:
        github:
          owner: nickhstr
          repo: todo-rust
          tokenRef: { secretName: github-pr-token, key: token }
        requeueAfterSeconds: 60
  template:
    metadata: { name: 'todo-pr-{{number}}' }
    spec:
      source:
        repoURL: https://github.com/nickhstr/todo-rust.git
        targetRevision: HEAD
        path: deploy/argocd/manifests/todo-app/overlays/preview
        kustomize:
          namePrefix: 'pr-{{number}}-'
          images: ['todo-app=ghcr.io/nickhstr/todo-app:pr-{{number}}-{{head_sha}}']
          commonAnnotations: { preview/pr: '{{number}}' }
      destination:
        server: https://kubernetes.default.svc
        namespace: 'todo-app-pr-{{number}}'
      syncPolicy:
        automated: { prune: true, selfHeal: true }
        syncOptions: [CreateNamespace=true]
```

PR open → ApplicationSet generates Application → app + own CNPG cluster + own Valkey + ingress at `pr-<N>.todo.<domain>` (wildcard cert covers it). PR close → ApplicationSet removes Application → ArgoCD prune deletes namespace + PVCs reclaimed.

## E. Observability

The dev compose stack ports over 1:1, all installed as platform charts:

| Dev compose | Prod equivalent |
|---|---|
| `prometheus` | `kube-prometheus-stack` (operator + Prom + Alertmanager + Grafana + node-exporter + kube-state-metrics) |
| `grafana` | bundled in `kube-prometheus-stack`; dashboards via ConfigMap (`grafana_dashboard: "1"` label — sidecar auto-loads) |
| `loki` | `grafana/loki` chart (single-binary, filesystem on Hetzner volume) |
| `tempo` | `grafana/tempo` chart (monolithic, filesystem on Hetzner volume) |
| `otel-collector` | `open-telemetry/opentelemetry-collector` chart |
| (new) `promtail` or `grafana/alloy` | Tails container stdout to Loki |

Existing `docker/grafana/dashboards/app.json` ports over unchanged via the sidecar.

### Alertmanager — email via Gmail SMTP

- `Alertmanager.config.global.smtp_*` reads from a Secret materialized by ESO from 1Password `smtp-gmail`
- Default route → email receiver pointing at owner's address
- Starter `PrometheusRule`s:
  - `app_pod_crashloop`: pod restarts > 3 in 10min
  - `app_5xx_burn_rate`: `rate(http_requests_total{status=~"5.."}[5m]) / rate(http_requests_total[5m]) > 0.05` for 10min
  - `postgres_down`: `up{job="postgres-exporter"} == 0` for 5min
  - `disk_pressure`: node `node_filesystem_free_bytes` < 10%
  - `cert_expiring`: `certmanager_certificate_expiration_timestamp_seconds - time() < 14*86400`

### Access

- Grafana fronted by ingress at `grafana.<domain>`, behind Basic Auth (initial; can swap to oauth2-proxy → GitHub OAuth as a follow-up learning exercise)
- Prometheus + Alertmanager UIs not exposed publicly; reach via `kubectl port-forward`

## F. Local approximation

Compose remains the daily inner loop (`just up`). New recipe `just up-k8s`:

```
just up-k8s:
  k3d cluster create todo --port 8080:80@loadbalancer --port 8443:443@loadbalancer
  kubectl apply -k deploy/argocd/bootstrap/                # ArgoCD install (local-aware values)
  kubectl wait deployment/argocd-server -n argocd --for=condition=available --timeout=120s
  kubectl apply -k deploy/argocd/apps/                     # root App-of-Apps
```

Local-mode caveats:
- ArgoCD pulls from your GitHub branch — push to a feature branch to test a manifest change. Acceptable for "do these manifests work on a real cluster" workflows.
- Hetzner CSI driver is not available locally; k3d falls back to k3s local-path provisioner. CNPG works on it; no real volume backing, persistence lost on `k3d cluster delete`.
- `deploy/argocd/manifests/todo-app/overlays/local/` overrides ExternalSecret → plain Secret with junk dev values (no 1Password locally by default; can opt-in to the prod path later).
- Escape hatch: `just k8s-apply staging` does direct `kubectl apply -k overlays/staging` against the local cluster, bypassing ArgoCD entirely. Use for "I want to try a manifest change without pushing."

## Cost summary (USD/month)

| Item | Cost |
|---|---|
| 3× Hetzner CX22 nodes | ~$13.50 |
| Hetzner LoadBalancer (LB11) | ~$6.00 |
| Hetzner Object Storage (state + WAL backups, ~10GB) | ~$1.50 |
| Hetzner Cloud Volumes (~50GB across PVCs) | ~$2.50 |
| Cloudflare DNS, Let's Encrypt, GHA minutes, GHCR | $0 |
| Domain renewal (amortized, $15/yr) | ~$1.25 |
| **Total** | **~$25/mo** |

For reference: comparable AWS EKS would run ~$150+/mo; DigitalOcean Kubernetes managed ~$30–40/mo.

## Phased rollout

| Phase | Deliverable |
|---|---|
| 1 | OpenTofu modules (network + nodes + DNS + object-storage bucket); `tofu apply` produces a running 3-node k3s HA cluster |
| 2 | Manual bootstrap: kubectl access, ingress-nginx + cert-manager with ACME staging, smoke test on a "hello world" Deployment |
| 3 | ArgoCD installed; App-of-Apps wired to `deploy/argocd/`; ESO + 1Password Connect; round-trip a test secret end-to-end |
| 4 | todo-app deployed to `todo-app-staging` via Argo: CNPG cluster, Valkey, real cert from LE production issuer |
| 5 | GHA: `pr-validate` + `main-deploy` workflows green; image push + manifest bump confirmed end-to-end |
| 6 | `todo-app-prod` env; `promote-prod` workflow |
| 7 | Observability stack in `observability` ns; existing dashboard ported; Alertmanager → Gmail SMTP wired |
| 8 | Preview envs: ApplicationSet PR generator; wildcard cert; preview-build workflow |
| 9 | Local k3d path: `just up-k8s` + `local` overlay + docs |

## Open questions / future work

- **vcluster for preview env density**: if PR volume ever grows enough that per-PR CNPG clusters strain the node pool, vcluster could give cheaper isolation (shared etcd/control-plane, per-tenant namespace tree). Defer until it matters.
- **Argo Image Updater swap-in**: replacing GHA-driven manifest commits with cluster-pull image updates is a self-contained learning exercise; manifests don't change, only what writes to them.
- **Grafana auth upgrade**: oauth2-proxy → GitHub OAuth replaces Basic Auth. Small, well-scoped follow-up.
- **Extracting manifests to a separate `infra` repo**: blast-radius improvement for "this PR can never touch deploy paths," not needed at this stage.
- **External etcd**: currently embedded with k3s for simplicity. External would be the next "real production correctness" step.
- **Multi-region / DR**: not in scope. The data backup story (CNPG WAL archive to object storage) is the floor on disaster tolerance.
- **Cluster autoscaler**: not needed at this scale (fixed 3-node pool). Could be a fun add later via the Hetzner cluster autoscaler provider.
- **Network policies**: cluster currently runs without `NetworkPolicy` resources. Adding default-deny + per-namespace allowlists is a reasonable hardening pass once the system is stable.
- **Cluster autoupdates**: how to do k3s minor-version upgrades safely (drain + replace, or system-upgrade-controller). Worth its own brainstorm later.
