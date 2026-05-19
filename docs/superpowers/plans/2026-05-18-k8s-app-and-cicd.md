# K8s App + CI/CD — Plan 2 of 5

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy the todo-app to the Hetzner k3s cluster (built in Plan 1) on a `staging` namespace and a `prod` namespace, each with its own CloudNativePG-managed Postgres cluster and Valkey StatefulSet. Wire three GitHub Actions workflows so `pull_request` validates, push-to-`main` auto-deploys staging, and a manual workflow promotes a specific image SHA to prod.

**Architecture:** Application manifests live as a Kustomize base under `deploy/argocd/manifests/todo-app/base/` with per-env overlays (`staging`, `prod`, `preview` template, `local`). ArgoCD has one Application per non-preview env, watching its overlay. CNPG operator manages Postgres per env; Valkey is a simple StatefulSet. Image tags are bumped by GitHub Actions committing back to `main` (push-based GitOps).

**Tech Stack:**
- CloudNativePG operator (v1.24+)
- Kustomize 5+
- ArgoCD multi-source Applications (continues the Plan 1 pattern)
- GitHub Actions (`actions/checkout@v4`, `docker/buildx-action@v3`, `actions/setup-buildx@v3`, `docker/login-action@v3`)
- GHCR (image registry)

**Spec:** `docs/superpowers/specs/2026-05-18-k8s-deploy-design.md`

**Plan position:** Plan 2 of 5. Predecessor: Plan 1 (foundation) must be complete and verified. Followups: Plan 3 (observability), Plan 4 (preview envs), Plan 5 (local k3d).

---

## Prerequisites

- Plan 1 (`docs/superpowers/plans/2026-05-18-k8s-foundation.md`) complete; cluster is up, ArgoCD healthy, 1Password→ESO round-trip verified.
- `KUBECONFIG=~/.kube/config-todo` exported in your shell.
- GitHub repo `nickhstr/todo-rust` (substitute your own); admin access to add Actions secrets.
- A 1Password item per environment for the cookie session key:
  - Item `staging/SESSION_KEY` (Password type, field `value` = `<128 hex chars>`)
  - Item `prod/SESSION_KEY` (same shape)
  - Generate values: `openssl rand -hex 64` → store in 1Password.

---

## File Structure

```
.github/workflows/
├── pr-validate.yml
├── main-deploy.yml
└── promote-prod.yml

deploy/argocd/
├── apps/
│   ├── platform/
│   │   └── cloudnative-pg.yaml         # CNPG operator (extension of Plan 1's platform)
│   └── todo-app/
│       ├── staging.yaml                # Application: staging overlay
│       └── prod.yaml                   # Application: prod overlay
└── manifests/
    ├── platform/
    │   └── cloudnative-pg/
    │       └── values.yaml             # Helm values for the operator
    └── todo-app/
        ├── base/
        │   ├── kustomization.yaml
        │   ├── deployment.yaml
        │   ├── service.yaml
        │   ├── ingress.yaml
        │   ├── configmap.yaml
        │   ├── external-secret.yaml
        │   ├── hpa.yaml
        │   ├── pdb.yaml
        │   ├── postgres-cluster.yaml   # CNPG Cluster
        │   └── valkey.yaml             # StatefulSet + Service
        └── overlays/
            ├── staging/
            │   ├── kustomization.yaml
            │   ├── patches.yaml        # 1 replica, smaller PG, staging host
            │   └── external-secret.yaml  # staging-namespaced 1P item refs
            ├── prod/
            │   ├── kustomization.yaml
            │   ├── patches.yaml        # 2 replicas + HPA, 2-instance PG, prod host
            │   ├── external-secret.yaml
            │   └── postgres-backup.yaml  # WAL archive to Hetzner Object Storage
            └── preview/                # template — used by Plan 4's ApplicationSet
                ├── kustomization.yaml
                ├── patches.yaml
                └── external-secret.yaml
```

Plus updates to `justfile` and `README.md`.

---

## Task 1: Install CloudNativePG operator via ArgoCD

**Files:**
- Create: `deploy/argocd/apps/platform/cloudnative-pg.yaml`
- Create: `deploy/argocd/manifests/platform/cloudnative-pg/values.yaml`

- [ ] **Step 1: Helm values**

Create `deploy/argocd/manifests/platform/cloudnative-pg/values.yaml`:

```yaml
crds:
  create: true

monitoring:
  podMonitorEnabled: false   # turn on with Plan 3
  grafanaDashboard:
    create: false

resources:
  requests: { cpu: 50m, memory: 128Mi }
  limits: { cpu: 500m, memory: 256Mi }

# The operator runs cluster-wide; one instance manages clusters in any namespace.
```

- [ ] **Step 2: Application**

Create `deploy/argocd/apps/platform/cloudnative-pg.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: cloudnative-pg
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://cloudnative-pg.github.io/charts
      chart: cloudnative-pg
      targetRevision: 0.22.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/cloudnative-pg/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: cnpg-system
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/apps/platform/cloudnative-pg.yaml \
        deploy/argocd/manifests/platform/cloudnative-pg/values.yaml
git commit -m "$(cat <<'EOF'
gitops: install CloudNativePG operator

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD UI: `cloudnative-pg` Application appears, `Synced + Healthy`. Verify CRDs:

```bash
kubectl get crd | grep cnpg
# Expected: clusters.postgresql.cnpg.io, poolers.postgresql.cnpg.io, backups.postgresql.cnpg.io, ...
```

---

## Task 2: todo-app Kustomize base — scaffolding

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/kustomization.yaml`

Sets up the base directory; subsequent tasks add resources one at a time. Resources are listed up front so Kustomize finds them as we go.

- [ ] **Step 1: Base kustomization.yaml**

Create `deploy/argocd/manifests/todo-app/base/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

# Per-overlay we set the namespace; base intentionally leaves it unset.

commonLabels:
  app.kubernetes.io/name: todo-app
  app.kubernetes.io/part-of: todo-app

resources:
  - configmap.yaml
  - external-secret.yaml
  - postgres-cluster.yaml
  - valkey.yaml
  - deployment.yaml
  - service.yaml
  - ingress.yaml
  - hpa.yaml
  - pdb.yaml

images:
  # Image tag is overridden per overlay; placeholder is the registry path.
  - name: todo-app
    newName: ghcr.io/nickhstr/todo-app
    newTag: bootstrap
```

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/kustomization.yaml
git commit -m "$(cat <<'EOF'
gitops: scaffold todo-app kustomize base

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: ConfigMap (non-secret app configuration)

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/configmap.yaml`

Carries everything from the dev compose env block that *isn't* secret.

- [ ] **Step 1: Write the ConfigMap**

Create `deploy/argocd/manifests/todo-app/base/configmap.yaml`:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: todo-app-config
data:
  RUST_LOG: "info,todo_app=info,sqlx=warn,tower_http=info"

  # REDIS_URL points at the in-namespace Valkey service.
  REDIS_URL: "redis://valkey:6379"

  APP__AUTH__COOKIE_SECURE: "true"

  # OTel deliberately off until Plan 3 lands the collector. App startup logs
  # remain useful via JSON-formatted stdout.
  APP__OBSERVABILITY__OTEL_ENABLED: "false"
  APP__OBSERVABILITY__LOG_FORMAT: "json"

  # template autoreload off in prod-shaped envs
  APP__TEMPLATE_AUTORELOAD: "false"
```

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/configmap.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app ConfigMap with non-secret config

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: ExternalSecret (cookie session key from 1Password)

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/external-secret.yaml`

The 1Password item key is templated by overlay (`staging/SESSION_KEY` vs `prod/SESSION_KEY`). Base uses a placeholder; overlays patch.

- [ ] **Step 1: Write the ExternalSecret**

Create `deploy/argocd/manifests/todo-app/base/external-secret.yaml`:

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: todo-app-secrets
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: onepassword-connect
    kind: ClusterSecretStore
  target:
    name: todo-app-secrets
    creationPolicy: Owner
  data:
    - secretKey: APP__AUTH__SESSION_KEY
      remoteRef:
        # Overridden by per-overlay patches (staging or prod).
        key: BASE-PLACEHOLDER/SESSION_KEY
        property: value
```

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/external-secret.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app ExternalSecret (session key from 1password)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: CNPG Cluster manifest (Postgres)

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/postgres-cluster.yaml`

A 1-instance Postgres by default; overlays scale prod to 2 and add WAL archive.

- [ ] **Step 1: Write the Cluster**

Create `deploy/argocd/manifests/todo-app/base/postgres-cluster.yaml`:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: todo-postgres
spec:
  instances: 1
  imageName: ghcr.io/cloudnative-pg/postgresql:16.4
  storage:
    size: 5Gi
    storageClass: hcloud-volumes

  bootstrap:
    initdb:
      database: todo
      owner: todo
      # postInitSQL runs once at cluster init; we deliberately leave it empty —
      # the app runs sqlx migrations on startup, which is the canonical source
      # of truth for schema.

  postgresql:
    parameters:
      max_connections: "100"
      shared_buffers: "256MB"
      work_mem: "8MB"
      log_min_duration_statement: "500"

  monitoring:
    enablePodMonitor: false   # Plan 3 turns this on

  resources:
    requests: { cpu: 50m, memory: 256Mi }
    limits: { cpu: 1000m, memory: 1Gi }
```

The CNPG operator auto-creates two Secrets in the namespace:
- `todo-postgres-app` — non-superuser app credentials (this is what the app uses)
- `todo-postgres-superuser` — for ops

The `todo-postgres-app` Secret has these keys: `username`, `password`, `host`, `port`, `dbname`, `uri`, `jdbc-uri`. The app's Deployment will pull `uri` → `DATABASE_URL`.

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/postgres-cluster.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app CNPG Postgres cluster (1-instance base)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Valkey StatefulSet + Service

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/valkey.yaml`

- [ ] **Step 1: Write Valkey manifest**

Create `deploy/argocd/manifests/todo-app/base/valkey.yaml`:

```yaml
---
apiVersion: v1
kind: Service
metadata:
  name: valkey
spec:
  type: ClusterIP
  selector: { app.kubernetes.io/component: valkey }
  ports:
    - name: redis
      port: 6379
      targetPort: 6379
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: valkey
spec:
  serviceName: valkey
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/component: valkey
  template:
    metadata:
      labels:
        app.kubernetes.io/component: valkey
    spec:
      containers:
        - name: valkey
          image: valkey/valkey:7-alpine
          args: ["valkey-server", "--appendonly", "yes"]
          ports: [{ containerPort: 6379, name: redis }]
          readinessProbe:
            exec: { command: ["valkey-cli", "ping"] }
            periodSeconds: 5
          livenessProbe:
            tcpSocket: { port: 6379 }
            periodSeconds: 10
          volumeMounts:
            - name: data
              mountPath: /data
          resources:
            requests: { cpu: 10m, memory: 32Mi }
            limits: { cpu: 200m, memory: 128Mi }
  volumeClaimTemplates:
    - metadata: { name: data }
      spec:
        accessModes: [ReadWriteOnce]
        storageClassName: hcloud-volumes
        resources:
          requests: { storage: 1Gi }
```

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/valkey.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app Valkey StatefulSet + Service

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Deployment

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/deployment.yaml`

- [ ] **Step 1: Write the Deployment**

Create `deploy/argocd/manifests/todo-app/base/deployment.yaml`:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: todo-app
spec:
  replicas: 1
  selector:
    matchLabels:
      app.kubernetes.io/component: web
  template:
    metadata:
      labels:
        app.kubernetes.io/component: web
    spec:
      topologySpreadConstraints:
        - maxSkew: 1
          topologyKey: kubernetes.io/hostname
          whenUnsatisfiable: ScheduleAnyway
          labelSelector:
            matchLabels:
              app.kubernetes.io/component: web
      containers:
        - name: todo-app
          image: todo-app   # transformed by kustomize images:
          ports:
            - containerPort: 3000
              name: http
          envFrom:
            - configMapRef: { name: todo-app-config }
            - secretRef: { name: todo-app-secrets }
          env:
            - name: DATABASE_URL
              valueFrom:
                secretKeyRef:
                  name: todo-postgres-app
                  key: uri
          readinessProbe:
            httpGet:
              path: /readyz
              port: http
            initialDelaySeconds: 5
            periodSeconds: 5
            failureThreshold: 6     # allow ~30s for first-boot migrations
          livenessProbe:
            httpGet:
              path: /healthz
              port: http
            initialDelaySeconds: 30
            periodSeconds: 10
            failureThreshold: 3
          resources:
            requests: { cpu: 100m, memory: 128Mi }
            limits: { cpu: 500m, memory: 256Mi }
          securityContext:
            runAsNonRoot: true
            runAsUser: 65532
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: ["ALL"]
```

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/deployment.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app Deployment

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Service + Ingress + HPA + PDB

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/service.yaml`
- Create: `deploy/argocd/manifests/todo-app/base/ingress.yaml`
- Create: `deploy/argocd/manifests/todo-app/base/hpa.yaml`
- Create: `deploy/argocd/manifests/todo-app/base/pdb.yaml`

- [ ] **Step 1: Service**

Create `deploy/argocd/manifests/todo-app/base/service.yaml`:

```yaml
apiVersion: v1
kind: Service
metadata:
  name: todo-app
spec:
  type: ClusterIP
  selector:
    app.kubernetes.io/component: web
  ports:
    - name: http
      port: 80
      targetPort: 3000
```

- [ ] **Step 2: Ingress (host templated by overlay)**

Create `deploy/argocd/manifests/todo-app/base/ingress.yaml`:

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: todo-app
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  ingressClassName: nginx
  rules:
    - host: BASE-PLACEHOLDER
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: todo-app
                port: { number: 80 }
  tls:
    - hosts: [BASE-PLACEHOLDER]
      secretName: todo-app-tls
```

(Overlays patch the host.)

- [ ] **Step 3: HPA**

Create `deploy/argocd/manifests/todo-app/base/hpa.yaml`:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: todo-app
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: todo-app
  minReplicas: 1
  maxReplicas: 6
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
```

- [ ] **Step 4: PDB**

Create `deploy/argocd/manifests/todo-app/base/pdb.yaml`:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: todo-app
spec:
  minAvailable: 1
  selector:
    matchLabels:
      app.kubernetes.io/component: web
```

- [ ] **Step 5: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/{service,ingress,hpa,pdb}.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app Service, Ingress, HPA, PDB

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Staging overlay

**Files:**
- Create: `deploy/argocd/manifests/todo-app/overlays/staging/kustomization.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/staging/patches.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/staging/external-secret.yaml`

- [ ] **Step 1: kustomization.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/staging/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

namespace: todo-app-staging

resources:
  - ../../base

patches:
  - path: patches.yaml
  - path: external-secret.yaml
    target:
      kind: ExternalSecret
      name: todo-app-secrets

images:
  - name: todo-app
    newName: ghcr.io/nickhstr/todo-app
    newTag: bootstrap   # GHA's main-deploy workflow bumps this
```

- [ ] **Step 2: patches.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/staging/patches.yaml`:

```yaml
---
# Replicas: 1 (HPA still active 1→6 but base load is low)
apiVersion: apps/v1
kind: Deployment
metadata: { name: todo-app }
spec:
  replicas: 1
---
# Set the staging host on the Ingress
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata: { name: todo-app }
spec:
  rules:
    - host: staging.todo.<yourdomain>     # SUBSTITUTE
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: todo-app
                port: { number: 80 }
  tls:
    - hosts: [staging.todo.<yourdomain>]  # SUBSTITUTE
      secretName: todo-app-tls
---
# CNPG cluster: 1 instance, 5Gi (same as base — explicit for clarity)
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata: { name: todo-postgres }
spec:
  instances: 1
  storage:
    size: 5Gi
```

- [ ] **Step 3: external-secret.yaml patch**

Create `deploy/argocd/manifests/todo-app/overlays/staging/external-secret.yaml`:

```yaml
- op: replace
  path: /spec/data/0/remoteRef/key
  value: staging/SESSION_KEY
```

(This is a JSON 6902 patch operating on the ExternalSecret.)

- [ ] **Step 4: Verify locally**

```bash
kustomize build deploy/argocd/manifests/todo-app/overlays/staging | kubeconform -strict -ignore-missing-schemas -summary
```

Expected: no errors. Check the rendered manifest by eye to confirm `<yourdomain>` substitutions are in place.

- [ ] **Step 5: Commit**

```bash
git add deploy/argocd/manifests/todo-app/overlays/staging/
git commit -m "$(cat <<'EOF'
gitops: todo-app staging overlay

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Prod overlay

**Files:**
- Create: `deploy/argocd/manifests/todo-app/overlays/prod/kustomization.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/prod/patches.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/prod/external-secret.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/prod/postgres-backup.yaml`

Differences from staging: 2 replicas, 2-instance CNPG, larger storage, WAL archive enabled, prod host.

- [ ] **Step 1: kustomization.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/prod/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

namespace: todo-app-prod

resources:
  - ../../base
  - postgres-backup.yaml      # extra resource (ObjectStore + ScheduledBackup) only in prod

patches:
  - path: patches.yaml
  - path: external-secret.yaml
    target:
      kind: ExternalSecret
      name: todo-app-secrets

images:
  - name: todo-app
    newName: ghcr.io/nickhstr/todo-app
    newTag: bootstrap   # GHA's promote-prod workflow bumps this
```

- [ ] **Step 2: patches.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/prod/patches.yaml`:

```yaml
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: todo-app }
spec:
  replicas: 2
---
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata: { name: todo-app }
spec:
  rules:
    - host: todo.<yourdomain>             # SUBSTITUTE
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: todo-app
                port: { number: 80 }
  tls:
    - hosts: [todo.<yourdomain>]          # SUBSTITUTE
      secretName: todo-app-tls
---
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata: { name: todo-postgres }
spec:
  instances: 2
  storage:
    size: 10Gi
  backup:
    barmanObjectStore:
      destinationPath: s3://todo-app-tofu-state/wal/prod
      endpointURL: https://nbg1.your-objectstorage.com
      s3Credentials:
        accessKeyId:
          name: cnpg-s3-creds
          key: access_key
        secretAccessKey:
          name: cnpg-s3-creds
          key: secret_key
      wal:
        compression: gzip
        maxParallel: 4
    retentionPolicy: "30d"
```

- [ ] **Step 3: external-secret.yaml patch**

Create `deploy/argocd/manifests/todo-app/overlays/prod/external-secret.yaml`:

```yaml
- op: replace
  path: /spec/data/0/remoteRef/key
  value: prod/SESSION_KEY
```

- [ ] **Step 4: postgres-backup.yaml (ObjectStore secret + ScheduledBackup)**

Create `deploy/argocd/manifests/todo-app/overlays/prod/postgres-backup.yaml`:

```yaml
---
# S3 credentials for CNPG to write WAL + base backups.
# Sourced from 1Password via ESO.
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: cnpg-s3-creds
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: onepassword-connect
    kind: ClusterSecretStore
  target:
    name: cnpg-s3-creds
    creationPolicy: Owner
  data:
    - secretKey: access_key
      remoteRef: { key: hetzner-s3-creds, property: access_key }
    - secretKey: secret_key
      remoteRef: { key: hetzner-s3-creds, property: secret_key }
---
apiVersion: postgresql.cnpg.io/v1
kind: ScheduledBackup
metadata:
  name: todo-postgres-daily
spec:
  schedule: "0 0 2 * * *"   # 02:00 UTC daily
  backupOwnerReference: self
  cluster:
    name: todo-postgres
```

- [ ] **Step 5: Verify**

```bash
kustomize build deploy/argocd/manifests/todo-app/overlays/prod | kubeconform -strict -ignore-missing-schemas -summary
```

- [ ] **Step 6: Commit**

```bash
git add deploy/argocd/manifests/todo-app/overlays/prod/
git commit -m "$(cat <<'EOF'
gitops: todo-app prod overlay (2 replicas, 2-instance PG, WAL archive)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Preview overlay template (used by Plan 4)

**Files:**
- Create: `deploy/argocd/manifests/todo-app/overlays/preview/kustomization.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/preview/patches.yaml`
- Create: `deploy/argocd/manifests/todo-app/overlays/preview/external-secret.yaml`

This overlay is referenced (and customized per-PR) by the ApplicationSet PR generator in Plan 4. We write it now because the patterns are the same as staging/prod.

- [ ] **Step 1: kustomization.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/preview/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

# namespace and namePrefix are injected by the ApplicationSet template
# (kustomize.namePrefix: 'pr-<N>-') in Plan 4. We leave them unset here.

resources:
  - ../../base

patches:
  - path: patches.yaml
  - path: external-secret.yaml
    target:
      kind: ExternalSecret
      name: todo-app-secrets

images:
  - name: todo-app
    newName: ghcr.io/nickhstr/todo-app
    newTag: bootstrap   # ApplicationSet templates the real tag per PR
```

- [ ] **Step 2: patches.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/preview/patches.yaml`:

```yaml
---
apiVersion: apps/v1
kind: Deployment
metadata: { name: todo-app }
spec:
  replicas: 1
---
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata: { name: todo-postgres }
spec:
  instances: 1
  storage:
    size: 1Gi
  # No backup config — previews are ephemeral.
---
# Host is replaced per-PR by the ApplicationSet via Argo's templating in Plan 4.
# We leave a placeholder here that Kustomize alone would render as-is; Plan 4
# uses kustomize.commonAnnotations + a post-render patch or just relies on
# the rendered preview being thrown away. The simpler approach is to set
# the host via an `nameSuffix`-aware patch in Plan 4. Defer the host
# substitution to the ApplicationSet template.
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata: { name: todo-app }
spec:
  rules:
    - host: preview-placeholder.todo.<yourdomain>   # SUBSTITUTE base domain
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: todo-app
                port: { number: 80 }
  tls:
    - hosts: [preview-placeholder.todo.<yourdomain>]
      secretName: todo-app-tls
```

(Plan 4 will substitute `preview-placeholder` for `pr-<N>` via the ApplicationSet's Argo templating layer.)

- [ ] **Step 3: external-secret.yaml**

Create `deploy/argocd/manifests/todo-app/overlays/preview/external-secret.yaml`:

```yaml
- op: replace
  path: /spec/data/0/remoteRef/key
  value: preview/SESSION_KEY
```

(All preview envs share the same `preview/SESSION_KEY` 1Password item — it's a learning project, not multi-tenant.)

- [ ] **Step 4: Add the `preview/SESSION_KEY` 1Password item**

In 1Password, vault `todo-app`, add a Password item:
- Name: `preview/SESSION_KEY`
- Field `value`: `openssl rand -hex 64` output

- [ ] **Step 5: Verify and commit**

```bash
kustomize build deploy/argocd/manifests/todo-app/overlays/preview | kubeconform -strict -ignore-missing-schemas -summary
git add deploy/argocd/manifests/todo-app/overlays/preview/
git commit -m "$(cat <<'EOF'
gitops: todo-app preview overlay template (used by Plan 4 PR generator)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Manually build & push the first image

**Files:** none (one-shot commands)

The Kustomize overlays reference `:bootstrap`. Build that tag once so the first reconcile succeeds, then GitHub Actions takes over.

- [ ] **Step 1: Authenticate Docker to GHCR**

```bash
echo "<your-github-PAT-with-write:packages>" | docker login ghcr.io -u <yourgithub> --password-stdin
```

If you don't already have a PAT with `write:packages`, create one at https://github.com/settings/tokens (classic, scope `write:packages`, `read:packages`). Save in 1Password as `ghcr-write-token`.

- [ ] **Step 2: Build + push**

```bash
git fetch origin
GIT_SHA=$(git rev-parse --short HEAD)
docker buildx build \
  --platform linux/amd64 \
  --tag ghcr.io/nickhstr/todo-app:bootstrap \
  --tag "ghcr.io/nickhstr/todo-app:${GIT_SHA}" \
  --build-arg "GIT_SHA=${GIT_SHA}" \
  --file docker/Dockerfile \
  --push \
  .
```

Verify in `https://github.com/nickhstr?tab=packages` that `todo-app` exists with both tags.

- [ ] **Step 3: Make the package public (optional, easier)**

In GHCR UI: package settings → Change visibility → Public. Otherwise the cluster needs an image pull secret; see Task 13 alternative.

No commit — image build is runtime state.

---

## Task 13: Image pull secret if package is private (skip if public)

**Files:** none

If you kept the GHCR package private, the cluster needs to authenticate.

- [ ] **Step 1: Add a 1Password item**

In 1Password vault `todo-app`, add a Password item:
- Name: `ghcr-pull-token`
- Field `username`: your GitHub username
- Field `token`: a PAT scoped to `read:packages`

- [ ] **Step 2: Create an ExternalSecret in each app namespace**

Append to `deploy/argocd/manifests/todo-app/base/external-secret.yaml` (separator first):

```yaml
---
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: ghcr-pull
spec:
  refreshInterval: 6h
  secretStoreRef:
    name: onepassword-connect
    kind: ClusterSecretStore
  target:
    name: ghcr-pull
    template:
      type: kubernetes.io/dockerconfigjson
      data:
        .dockerconfigjson: |
          {
            "auths": {
              "ghcr.io": {
                "username": "{{ .username }}",
                "password": "{{ .token }}",
                "auth": "{{ printf "%s:%s" .username .token | b64enc }}"
              }
            }
          }
  data:
    - secretKey: username
      remoteRef: { key: ghcr-pull-token, property: username }
    - secretKey: token
      remoteRef: { key: ghcr-pull-token, property: token }
```

Then patch the Deployment to use it. In `deploy/argocd/manifests/todo-app/base/deployment.yaml`, add inside `spec.template.spec`:

```yaml
      imagePullSecrets:
        - name: ghcr-pull
```

- [ ] **Step 3: Commit**

```bash
git add deploy/argocd/manifests/todo-app/base/external-secret.yaml \
        deploy/argocd/manifests/todo-app/base/deployment.yaml
git commit -m "$(cat <<'EOF'
gitops: ghcr image pull secret via ESO (private package path)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

(Skip Task 13 entirely if the package is public — that's the simpler default.)

---

## Task 14: ArgoCD Application for staging

**Files:**
- Create: `deploy/argocd/apps/todo-app/staging.yaml`

- [ ] **Step 1: Application**

Create `deploy/argocd/apps/todo-app/staging.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: todo-app-staging
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/todo-app/overlays/staging
  destination:
    server: https://kubernetes.default.svc
    namespace: todo-app-staging
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

- [ ] **Step 2: Update root App-of-Apps to include todo-app subdir**

In `deploy/argocd/apps/root.yaml`, change `path: deploy/argocd/apps/platform` to point at the parent of both platform/ and todo-app/:

```yaml
spec:
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/apps
    directory:
      recurse: true
```

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/apps/root.yaml deploy/argocd/apps/todo-app/staging.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app staging Application + root expanded to scan apps/

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD UI: `todo-app-staging` Application appears. Watch sync:
- `todo-app-staging` namespace gets created
- CNPG Cluster comes up (takes ~60–90s for first instance)
- todo-app Pod comes up; first boot runs migrations, may flap Ready/NotReady briefly
- Ingress materializes; cert-manager issues cert

```bash
kubectl -n todo-app-staging get all
kubectl -n todo-app-staging get cluster todo-postgres
kubectl -n todo-app-staging get certificate
```

All Healthy/Ready within ~3 minutes.

Visit `https://staging.todo.<yourdomain>` — should render the signup page.

---

## Task 15: ArgoCD Application for prod

**Files:**
- Create: `deploy/argocd/apps/todo-app/prod.yaml`

- [ ] **Step 1: Application**

Create `deploy/argocd/apps/todo-app/prod.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: todo-app-prod
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/todo-app/overlays/prod
  destination:
    server: https://kubernetes.default.svc
    namespace: todo-app-prod
  syncPolicy:
    automated:
      prune: false       # manual prune for prod safety
      selfHeal: true
    syncOptions: [ServerSideApply=true, CreateNamespace=true]
```

(`prune: false` on prod is a guardrail: if a resource disappears from git, we want to be explicit about deleting it from prod.)

- [ ] **Step 2: Commit + push + verify**

```bash
git add deploy/argocd/apps/todo-app/prod.yaml
git commit -m "$(cat <<'EOF'
gitops: todo-app prod Application (prune disabled)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

Verify `todo-app-prod` Application appears in ArgoCD, syncs, Postgres + app come up healthy. Hit `https://todo.<yourdomain>`.

---

## Task 16: GHA — `pr-validate.yml`

**Files:**
- Create: `.github/workflows/pr-validate.yml`

- [ ] **Step 1: Workflow**

Create `.github/workflows/pr-validate.yml`:

```yaml
name: pr-validate

on:
  pull_request:
    branches: [main]

permissions:
  contents: read

jobs:
  rust:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - name: fmt
        run: cargo fmt --all --check

      - name: clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: unit tests
        run: cargo test --workspace --lib --bins

      - name: integration tests (testcontainers, needs docker)
        run: cargo test --workspace

  docker:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4

      - uses: docker/setup-buildx-action@v3

      - name: docker build (no push)
        uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/Dockerfile
          push: false
          cache-from: type=gha
          cache-to: type=gha,mode=max
          build-args: |
            GIT_SHA=${{ github.event.pull_request.head.sha }}

  manifests:
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4

      - name: install kustomize + kubeconform
        run: |
          curl -sLo /tmp/kustomize.tar.gz https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v5.4.3/kustomize_v5.4.3_linux_amd64.tar.gz
          tar -xzf /tmp/kustomize.tar.gz -C /usr/local/bin
          curl -sLo /tmp/kubeconform.tar.gz https://github.com/yannh/kubeconform/releases/download/v0.6.7/kubeconform-linux-amd64.tar.gz
          tar -xzf /tmp/kubeconform.tar.gz -C /usr/local/bin

      - name: validate staging overlay
        run: |
          kustomize build deploy/argocd/manifests/todo-app/overlays/staging \
            | kubeconform -strict -ignore-missing-schemas -summary

      - name: validate prod overlay
        run: |
          kustomize build deploy/argocd/manifests/todo-app/overlays/prod \
            | kubeconform -strict -ignore-missing-schemas -summary

      - name: validate preview overlay
        run: |
          kustomize build deploy/argocd/manifests/todo-app/overlays/preview \
            | kubeconform -strict -ignore-missing-schemas -summary
```

- [ ] **Step 2: Push as a PR to test**

Create a throwaway PR (e.g., a no-op README change) to verify all three jobs go green. Note: the first run will be slow because Rust cache is cold.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/pr-validate.yml
git commit -m "$(cat <<'EOF'
ci: add pr-validate workflow (fmt, clippy, tests, docker build, manifests)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 17: Set up GH_DEPLOY_TOKEN for manifest commits

**Files:** none (GitHub UI + 1Password)

`main-deploy.yml` and `promote-prod.yml` commit Kustomize tag bumps back to the repo. The default `GITHUB_TOKEN` cannot trigger downstream workflows on its own commits — we use a fine-grained PAT for the bot's write path.

- [ ] **Step 1: Create the PAT**

GitHub → Settings → Developer settings → Personal access tokens → Fine-grained tokens → Generate.
- Token name: `todo-rust-deploy-bot`
- Expiration: 1 year (note in 1Password to rotate)
- Repository access: Only select repositories → `todo-rust`
- Permissions: Contents (Read & Write), Actions (Read), Metadata (Read)
- Generate, copy the value.

- [ ] **Step 2: Store the PAT in 1Password**

In 1Password vault `todo-app`:
- Item type: API Credential
- Name: `gh-deploy-token`
- Field `token`: the PAT value

- [ ] **Step 3: Add to repo secrets**

GitHub → repo → Settings → Secrets and variables → Actions → New repository secret:
- Name: `GH_DEPLOY_TOKEN`
- Value: paste from 1Password

No commit — this is GitHub state.

---

## Task 18: GHA — `main-deploy.yml`

**Files:**
- Create: `.github/workflows/main-deploy.yml`

- [ ] **Step 1: Workflow**

Create `.github/workflows/main-deploy.yml`:

```yaml
name: main-deploy

on:
  push:
    branches: [main]
    paths-ignore:
      - 'deploy/**'        # avoid self-retrigger from manifest bumps
      - 'docs/**'
      - 'README.md'
      - '.github/**'

permissions:
  contents: write          # we git push
  packages: write          # we push to ghcr

jobs:
  build-and-deploy-staging:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
        with:
          token: ${{ secrets.GH_DEPLOY_TOKEN }}
          fetch-depth: 0

      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: unit tests
        run: cargo test --workspace --lib --bins

      - uses: docker/setup-buildx-action@v3

      - name: log in to ghcr
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: docker build + push
        uses: docker/build-push-action@v6
        with:
          context: .
          file: docker/Dockerfile
          push: true
          tags: ghcr.io/nickhstr/todo-app:${{ github.sha }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
          build-args: |
            GIT_SHA=${{ github.sha }}

      - name: install kustomize
        run: |
          curl -sLo /tmp/kustomize.tar.gz https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v5.4.3/kustomize_v5.4.3_linux_amd64.tar.gz
          tar -xzf /tmp/kustomize.tar.gz -C /usr/local/bin

      - name: bump staging image tag
        working-directory: deploy/argocd/manifests/todo-app/overlays/staging
        run: |
          kustomize edit set image todo-app=ghcr.io/nickhstr/todo-app:${{ github.sha }}

      - name: commit and push
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add deploy/argocd/manifests/todo-app/overlays/staging/kustomization.yaml
          if git diff --staged --quiet; then
            echo "No image tag change; skipping commit."
          else
            git commit -m "staging: deploy ${{ github.sha }}"
            git push
          fi
```

- [ ] **Step 2: Commit + push to trigger**

```bash
git add .github/workflows/main-deploy.yml
git commit -m "$(cat <<'EOF'
ci: main-deploy workflow — build, push, bump staging tag

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 3: Verify**

When the PR (Task 16) merges (or you push the workflow to main directly), watch:
1. GitHub Actions → main-deploy run completes
2. A new commit `staging: deploy <sha>` appears on main from `github-actions[bot]`
3. ArgoCD detects the change and syncs `todo-app-staging` to the new image
4. `kubectl -n todo-app-staging get pods` shows a rolling update; new pod with the new image
5. Visit `https://staging.todo.<yourdomain>/` — `X-App-Version` response header should show the new SHA

---

## Task 19: GHA — `promote-prod.yml`

**Files:**
- Create: `.github/workflows/promote-prod.yml`

- [ ] **Step 1: Workflow**

Create `.github/workflows/promote-prod.yml`:

```yaml
name: promote-prod

on:
  workflow_dispatch:
    inputs:
      sha:
        description: 'Git SHA to deploy to prod (must already be built and pushed to ghcr)'
        required: true
        type: string

permissions:
  contents: write

jobs:
  promote:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
        with:
          token: ${{ secrets.GH_DEPLOY_TOKEN }}
          fetch-depth: 0

      - name: verify image exists in ghcr
        env:
          TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          # GHCR returns 200 if the manifest exists, 404 otherwise.
          STATUS=$(curl -sL -o /dev/null -w "%{http_code}" \
            -H "Authorization: Bearer $(echo -n $TOKEN | base64)" \
            "https://ghcr.io/v2/nickhstr/todo-app/manifests/${{ inputs.sha }}")
          if [ "$STATUS" != "200" ]; then
            echo "Image ghcr.io/nickhstr/todo-app:${{ inputs.sha }} not found (HTTP $STATUS)"
            exit 1
          fi

      - name: install kustomize
        run: |
          curl -sLo /tmp/kustomize.tar.gz https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v5.4.3/kustomize_v5.4.3_linux_amd64.tar.gz
          tar -xzf /tmp/kustomize.tar.gz -C /usr/local/bin

      - name: bump prod image tag
        working-directory: deploy/argocd/manifests/todo-app/overlays/prod
        run: |
          kustomize edit set image todo-app=ghcr.io/nickhstr/todo-app:${{ inputs.sha }}

      - name: commit and push
        run: |
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add deploy/argocd/manifests/todo-app/overlays/prod/kustomization.yaml
          if git diff --staged --quiet; then
            echo "Prod is already on this SHA."
          else
            git commit -m "prod: deploy ${{ inputs.sha }}"
            git push
          fi
```

- [ ] **Step 2: Commit + push**

```bash
git add .github/workflows/promote-prod.yml
git commit -m "$(cat <<'EOF'
ci: promote-prod workflow (manual workflow_dispatch by sha)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 3: Verify end-to-end**

In GitHub: Actions → promote-prod → Run workflow → enter the same SHA that just deployed to staging.

Watch:
1. Workflow completes
2. New commit `prod: deploy <sha>` lands on main
3. ArgoCD syncs `todo-app-prod`
4. `https://todo.<yourdomain>/` `X-App-Version` shows the new SHA

---

## Task 20: justfile additions for app inspection

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Append app recipes**

Append to `justfile`:

```make
# --- App inspection (in-cluster) ---

# Logs for the staging app pod.
k8s-logs env='staging':
    KUBECONFIG=~/.kube/config-todo \
        kubectl -n todo-app-{{env}} logs -f deployment/todo-app

# Status snapshot for an environment.
k8s-status env='staging':
    @KUBECONFIG=~/.kube/config-todo kubectl -n todo-app-{{env}} get all,certificate,externalsecret,cluster.postgresql.cnpg.io

# psql shell into the in-cluster Postgres.
k8s-psql env='staging':
    KUBECONFIG=~/.kube/config-todo \
        kubectl -n todo-app-{{env}} exec -it todo-postgres-1 -c postgres -- psql -U todo -d todo

# Trigger a manual sync of an Application.
k8s-sync app:
    KUBECONFIG=~/.kube/config-todo \
        kubectl -n argocd patch application {{app}} \
            --type=merge --patch '{"metadata":{"annotations":{"argocd.argoproj.io/refresh":"hard"}}}'
```

- [ ] **Step 2: Commit**

```bash
git add justfile
git commit -m "$(cat <<'EOF'
just: app inspection recipes (k8s-logs, k8s-status, k8s-psql, k8s-sync)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 21: Test the WAL backup pipeline (prod)

**Files:** none

- [ ] **Step 1: Wait for the first scheduled backup, or trigger one manually**

```bash
kubectl -n todo-app-prod apply -f - <<EOF
apiVersion: postgresql.cnpg.io/v1
kind: Backup
metadata:
  name: smoke-$(date +%s)
spec:
  cluster:
    name: todo-postgres
EOF
```

- [ ] **Step 2: Watch progress**

```bash
kubectl -n todo-app-prod get backup -w
```

Expected: status transitions to `completed` within ~2 minutes.

- [ ] **Step 3: Verify objects landed in the bucket**

```bash
aws --endpoint-url=https://nbg1.your-objectstorage.com \
    s3 ls s3://todo-app-tofu-state/wal/prod/ --recursive | head
```

Expected: `base/` directory + WAL segments. If empty, inspect:

```bash
kubectl -n todo-app-prod logs -l cnpg.io/cluster=todo-postgres -c postgres --tail=200 | grep -i barman
```

Most common failure: `cnpg-s3-creds` Secret missing or wrong (re-check that the prod overlay's ESO synced — `kubectl -n todo-app-prod get externalsecret cnpg-s3-creds` should show `READY`).

No commit — runtime verification.

---

## Task 22: Smoke test — sign up + create todo in prod

**Files:** none

End-to-end UX validation.

- [ ] **Step 1: Visit and sign up**

In a private browser window, visit `https://todo.<yourdomain>/signup`. Register a new account (use a throwaway email; the app doesn't email you).

- [ ] **Step 2: Create a couple of todos**

Use the UI to add, toggle, and delete a todo. Confirm htmx interactions work (no full page reloads).

- [ ] **Step 3: Sanity-check metrics endpoint**

```bash
curl -s https://todo.<yourdomain>/metrics | head -20
```

Expected: Prometheus text format (we're not yet scraping; just confirming the endpoint is reachable). Plan 3 wires the actual scrape.

- [ ] **Step 4: Check version header**

```bash
curl -sI https://todo.<yourdomain>/ | grep -i x-app-version
```

Expected: `X-App-Version: <git-sha>` matching whatever you just deployed.

No commit — this is verification.

---

## Task 23: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Replace the "Production deployment" section with the current state**

Update the section added in Plan 1 to reflect what's live:

```markdown
## Production deployment

The production deployment runs on a self-managed 3-node k3s HA cluster on
Hetzner Cloud, managed via OpenTofu + ArgoCD. Three environments share one
cluster:

| Env | URL | Replicas | Postgres |
|---|---|---|---|
| prod    | https://todo.<yourdomain>          | 2 (HPA 2–6) | CNPG 2-instance, WAL archive |
| staging | https://staging.todo.<yourdomain>  | 1           | CNPG 1-instance |
| preview | https://pr-<N>.todo.<yourdomain>   | 1 (per PR)  | CNPG 1-instance, ephemeral |

**CI/CD:**
- `pr-validate.yml` — runs on every PR (lint, tests, docker build, manifest validation).
- `main-deploy.yml` — runs on push to `main`. Builds, pushes to GHCR, commits the new tag to the staging overlay. ArgoCD reconciles within ~30s.
- `promote-prod.yml` — manual `workflow_dispatch`. Provide a SHA; verifies the image exists, bumps the prod overlay.

**Day-2:**
```bash
just k8s-status prod              # pods + certs + PG + ExternalSecrets
just k8s-logs staging              # follow app logs
just k8s-psql staging              # psql shell into the PG primary
just k8s-sync todo-app-staging     # force an ArgoCD refresh
```

**Reference:**
- Spec: `docs/superpowers/specs/2026-05-18-k8s-deploy-design.md`
- Plans: `docs/superpowers/plans/2026-05-18-k8s-{foundation,app-and-cicd,observability,preview-envs,local-k3d}.md`
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs: README updated for Plan 2 (app + CI/CD deployed)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final verification

- [ ] `kubectl -n argocd get app` — `todo-app-staging` and `todo-app-prod` both `Synced + Healthy`
- [ ] `kubectl -n todo-app-staging get cluster todo-postgres` — `READY: True`, instances match overlay (1 staging, 2 prod)
- [ ] `kubectl -n todo-app-prod get scheduledbackup` — `todo-postgres-daily` present
- [ ] `curl -sI https://staging.todo.<yourdomain>/` returns 200 + a valid TLS cert
- [ ] `curl -sI https://todo.<yourdomain>/` returns 200 + a valid TLS cert
- [ ] Push a no-op commit to main; see `main-deploy.yml` build + push + bump staging; ArgoCD picks it up
- [ ] Manually run `promote-prod` with the same SHA; see prod overlay bump and Argo sync
- [ ] Hetzner Object Storage shows `s3://todo-app-tofu-state/wal/prod/` populated

When all of the above pass, this plan is complete. Hand off to **Plan 3 (Observability)**.
