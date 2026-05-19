# K8s Observability — Plan 3 of 5

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the dev-compose observability stack (Prometheus, Grafana, Loki, Tempo, OTel collector, dashboards) into the cluster, fronted by Grafana on an ingress, with Alertmanager wired to send email via Gmail SMTP. Flip the app's OTEL_ENABLED on so traces/logs/metrics flow end-to-end.

**Architecture:** Single `observability` namespace running kube-prometheus-stack (operator + Prometheus + Alertmanager + Grafana + node-exporter + kube-state-metrics), Loki (single-binary, filesystem PVC), Tempo (monolithic, filesystem PVC), and the OpenTelemetry Collector. Grafana Alloy ships pod logs to Loki. The app's existing OTLP export wires into the in-cluster collector; the existing Grafana dashboard JSON is loaded via the dashboards sidecar. Alertmanager routes critical+warning alerts to a Gmail address.

**Tech Stack:**
- kube-prometheus-stack (~62.x)
- Loki (3.x via official Helm chart)
- Tempo (2.x via official Helm chart)
- OpenTelemetry Collector (0.108+)
- Grafana Alloy (1.x) for log shipping
- Alertmanager v0.27+

**Spec:** `docs/superpowers/specs/2026-05-18-k8s-deploy-design.md`

**Plan position:** Plan 3 of 5. Predecessors: Plans 1+2. Followups: Plan 4 (preview envs), Plan 5 (local k3d).

---

## Prerequisites

- Plan 2 complete: staging and prod app environments are up and serving traffic.
- A 1Password item `smtp-gmail` exists with fields:
  - `from` — your Gmail address (e.g., `you@gmail.com`)
  - `password` — a Google App Password (https://myaccount.google.com/apppasswords)
- DNS: `grafana.<yourdomain>` A-record exists pointing at the LB IP. Add via Tofu in `deploy/tofu/modules/dns/main.tf` (add to `local.records`) then re-apply.

---

## File Structure

Additions only — no file removals.

```
deploy/argocd/
├── apps/
│   ├── platform/
│   │   └── observability/             # one Application file per chart
│   │       ├── kube-prometheus-stack.yaml
│   │       ├── loki.yaml
│   │       ├── tempo.yaml
│   │       ├── otel-collector.yaml
│   │       ├── alloy.yaml
│   │       └── alert-rules.yaml       # raw PrometheusRule manifests
│   └── ...
└── manifests/
    └── platform/
        ├── kube-prometheus-stack/
        │   ├── values.yaml
        │   └── dashboards/            # ConfigMaps with grafana_dashboard label
        │       └── todo-app.yaml
        ├── loki/values.yaml
        ├── tempo/values.yaml
        ├── otel-collector/
        │   ├── values.yaml
        │   └── config.yaml            # ConfigMap with collector pipeline
        ├── alloy/values.yaml
        └── alert-rules/
            ├── kustomization.yaml
            └── rules.yaml
```

Plus updates to:
- `deploy/argocd/manifests/todo-app/base/configmap.yaml` (flip `APP__OBSERVABILITY__OTEL_ENABLED=true`, set endpoint)
- `deploy/argocd/manifests/todo-app/base/deployment.yaml` (add `prometheus.io/scrape` annotations or a ServiceMonitor)

---

## Task 1: DNS for grafana subdomain

**Files:**
- Modify: `deploy/tofu/modules/dns/main.tf`

- [ ] **Step 1: Add grafana subdomain to the record list**

In `deploy/tofu/modules/dns/main.tf`, change `local.records` to include grafana:

```hcl
locals {
  records = var.lb_ipv4 == "" ? [] : [
    var.domain_prefix,
    "staging.${var.domain_prefix}",
    "*.${var.domain_prefix}",
    "grafana",                            # grafana.<zone> — apex grafana, not nested under todo.
  ]
}
```

(Or keep grafana nested as `grafana.${var.domain_prefix}` — choose whichever DNS shape you prefer; the wildcard `*.${var.domain_prefix}` would also cover the nested form.)

- [ ] **Step 2: Re-apply**

```bash
cd deploy/tofu
tofu apply         # type yes
```

Expected: 1 record added.

- [ ] **Step 3: Verify**

```bash
dig +short grafana.<yourdomain> @1.1.1.1
```

Returns `${LB_IPV4}`.

- [ ] **Step 4: Commit**

```bash
git add deploy/tofu/modules/dns/main.tf
git commit -m "$(cat <<'EOF'
infra: add grafana DNS record

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: kube-prometheus-stack — Helm values

**Files:**
- Create: `deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml`

- [ ] **Step 1: Values**

Create `deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml`:

```yaml
fullnameOverride: prom

# Prometheus
prometheus:
  prometheusSpec:
    retention: 7d
    retentionSize: "9GB"
    storageSpec:
      volumeClaimTemplate:
        spec:
          storageClassName: hcloud-volumes
          accessModes: [ReadWriteOnce]
          resources:
            requests: { storage: 10Gi }
    resources:
      requests: { cpu: 100m, memory: 512Mi }
      limits:   { cpu: 1000m, memory: 1.5Gi }
    podMonitorSelectorNilUsesHelmValues: false
    serviceMonitorSelectorNilUsesHelmValues: false
    ruleSelectorNilUsesHelmValues: false
    enableRemoteWriteReceiver: true   # so k6 / OTel etc. can push
    additionalScrapeConfigs:
      - job_name: kubernetes-pods
        kubernetes_sd_configs:
          - role: pod
        relabel_configs:
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_scrape]
            action: keep
            regex: "true"
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_path]
            action: replace
            target_label: __metrics_path__
            regex: (.+)
          - source_labels: [__address__, __meta_kubernetes_pod_annotation_prometheus_io_port]
            action: replace
            target_label: __address__
            regex: ([^:]+)(?::\d+)?;(\d+)
            replacement: $1:$2

# Alertmanager
alertmanager:
  alertmanagerSpec:
    storage:
      volumeClaimTemplate:
        spec:
          storageClassName: hcloud-volumes
          accessModes: [ReadWriteOnce]
          resources: { requests: { storage: 2Gi } }
    resources:
      requests: { cpu: 25m, memory: 64Mi }
      limits:   { cpu: 250m, memory: 256Mi }
  config:
    global:
      smtp_smarthost: 'smtp.gmail.com:587'
      smtp_from: '__SMTP_FROM__'           # patched in by the alertmanager-config secret (Task 4)
      smtp_auth_username: '__SMTP_FROM__'
      smtp_auth_password: '__SMTP_PASSWORD__'
      smtp_require_tls: true
    route:
      receiver: email
      group_by: [alertname, namespace, severity]
      group_wait: 30s
      group_interval: 5m
      repeat_interval: 4h
      routes:
        - matchers: [severity = "critical"]
          continue: false
          receiver: email
    receivers:
      - name: email
        email_configs:
          - to: '__SMTP_FROM__'

# Grafana
grafana:
  fullnameOverride: grafana
  adminPassword: "REPLACE_AT_FIRST_LOGIN"
  ingress:
    enabled: true
    ingressClassName: nginx
    annotations:
      cert-manager.io/cluster-issuer: letsencrypt-prod
    hosts: ["grafana.<yourdomain>"]
    tls:
      - hosts: ["grafana.<yourdomain>"]
        secretName: grafana-tls
  persistence:
    enabled: true
    storageClassName: hcloud-volumes
    size: 5Gi
  sidecar:
    dashboards:
      enabled: true
      label: grafana_dashboard
      labelValue: "1"
      searchNamespace: ALL
    datasources:
      enabled: true
      label: grafana_datasource
      labelValue: "1"
      searchNamespace: ALL
  resources:
    requests: { cpu: 25m, memory: 128Mi }
    limits:   { cpu: 500m, memory: 512Mi }

# node-exporter and kube-state-metrics defaults are fine
```

Substitute `<yourdomain>` in the two hostnames.

- [ ] **Step 2: Commit**

```bash
git add deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml
git commit -m "$(cat <<'EOF'
gitops: kube-prometheus-stack helm values

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: kube-prometheus-stack — Application

**Files:**
- Create: `deploy/argocd/apps/platform/observability/kube-prometheus-stack.yaml`

- [ ] **Step 1: Application**

Create `deploy/argocd/apps/platform/observability/kube-prometheus-stack.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: kube-prometheus-stack
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://prometheus-community.github.io/helm-charts
      chart: kube-prometheus-stack
      targetRevision: 62.6.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true, CreateNamespace=true, ServerSideDiff=true]
  # The chart ships large CRDs; bigger generated YAML can exceed the default
  # apply timeout. Bump ApplicationSpec's resource fields if syncs flap.
```

- [ ] **Step 2: Commit + push + verify**

```bash
git add deploy/argocd/apps/platform/observability/kube-prometheus-stack.yaml
git commit -m "$(cat <<'EOF'
gitops: install kube-prometheus-stack via argocd

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD UI: app `Synced + Healthy` (this one is the slowest of the platform installs — give it 5+ minutes for first sync because of the CRDs and storage provisioning).

```bash
kubectl -n observability get pods
```

Expected: alertmanager-prom-alertmanager-0, prom-grafana-..., prom-kube-state-metrics-..., prom-prometheus-node-exporter-... (per node), prom-prometheus-prom-prometheus-0, prom-operator-... — all Running.

---

## Task 4: Alertmanager SMTP credentials via ESO

**Files:**
- Create: `deploy/argocd/manifests/platform/kube-prometheus-stack/alertmanager-config.yaml`
- Modify: `deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml`

The placeholder `__SMTP_*__` values in the chart values aren't substituted — we need to inject real SMTP creds via a Secret. kube-prometheus-stack supports overriding the Alertmanager config entirely via a Secret named `alertmanager-<alertmanager-name>` (e.g., `alertmanager-prom-alertmanager`).

- [ ] **Step 1: ExternalSecret + alertmanager.yaml content**

Create `deploy/argocd/manifests/platform/kube-prometheus-stack/alertmanager-config.yaml`:

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: alertmanager-prom-alertmanager
  namespace: observability
spec:
  refreshInterval: 1h
  secretStoreRef:
    name: onepassword-connect
    kind: ClusterSecretStore
  target:
    name: alertmanager-prom-alertmanager
    template:
      type: Opaque
      data:
        alertmanager.yaml: |
          global:
            smtp_smarthost: 'smtp.gmail.com:587'
            smtp_from: '{{ .from }}'
            smtp_auth_username: '{{ .from }}'
            smtp_auth_password: '{{ .password }}'
            smtp_require_tls: true
          route:
            receiver: email
            group_by: [alertname, namespace, severity]
            group_wait: 30s
            group_interval: 5m
            repeat_interval: 4h
          receivers:
            - name: email
              email_configs:
                - to: '{{ .from }}'
                  send_resolved: true
  data:
    - secretKey: from
      remoteRef: { key: smtp-gmail, property: from }
    - secretKey: password
      remoteRef: { key: smtp-gmail, property: password }
```

- [ ] **Step 2: Tell kube-prometheus-stack not to manage its own AM config**

In `deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml`, replace the `alertmanager.config` block with:

```yaml
alertmanager:
  alertmanagerSpec:
    storage: ...   # keep as before
    resources: ... # keep as before
  config: {}       # leave empty — config comes from the ESO-managed Secret above
  configSecret: alertmanager-prom-alertmanager
```

- [ ] **Step 3: Add ESO manifest as a sub-Application**

Append the file under the existing Application or create a new sibling. Simplest: create a separate one.

Create `deploy/argocd/apps/platform/observability/alertmanager-config.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: alertmanager-config
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/platform/kube-prometheus-stack
    directory:
      recurse: false
      include: 'alertmanager-config.yaml'
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 4: Commit + push**

```bash
git add deploy/argocd/manifests/platform/kube-prometheus-stack/values.yaml \
        deploy/argocd/manifests/platform/kube-prometheus-stack/alertmanager-config.yaml \
        deploy/argocd/apps/platform/observability/alertmanager-config.yaml
git commit -m "$(cat <<'EOF'
gitops: alertmanager smtp config via ESO

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 5: Verify alert delivery**

```bash
# Fire a test alert via Alertmanager's API
kubectl -n observability port-forward svc/prom-alertmanager 9093:9093 &
PF_PID=$!

curl -X POST http://localhost:9093/api/v2/alerts -H 'Content-Type: application/json' -d '[{
  "labels": {"alertname":"SmokeTest","severity":"critical","instance":"hand-rolled"},
  "annotations":{"summary":"smoke from kubectl"},
  "startsAt":"'$(date -u +%Y-%m-%dT%H:%M:%S.000Z)'"
}]'

kill $PF_PID
```

Expected: an email arrives at your Gmail within ~30s. (Check spam folder.)

---

## Task 5: Starter alert rules

**Files:**
- Create: `deploy/argocd/manifests/platform/alert-rules/rules.yaml`
- Create: `deploy/argocd/manifests/platform/alert-rules/kustomization.yaml`
- Create: `deploy/argocd/apps/platform/observability/alert-rules.yaml`

- [ ] **Step 1: PrometheusRule manifest**

Create `deploy/argocd/manifests/platform/alert-rules/rules.yaml`:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: todo-app-baseline
  labels:
    release: prom         # discovered by kube-prometheus-stack's PromOperator selector
spec:
  groups:
    - name: app
      rules:
        - alert: AppPodCrashloop
          expr: rate(kube_pod_container_status_restarts_total{namespace=~"todo-app-.*"}[10m]) > 0.005
          for: 10m
          labels: { severity: warning }
          annotations:
            summary: "{{ $labels.namespace }}/{{ $labels.pod }} crashing"
            description: "Container in {{ $labels.pod }} is restarting frequently."

        - alert: App5xxBurnRate
          expr: |
            sum(rate(http_requests_total{namespace=~"todo-app-.*",status=~"5.."}[5m]))
              / sum(rate(http_requests_total{namespace=~"todo-app-.*"}[5m])) > 0.05
          for: 10m
          labels: { severity: critical }
          annotations:
            summary: "App 5xx rate above 5% for 10m"

    - name: data
      rules:
        - alert: PostgresDown
          expr: up{job=~"prom-.*postgres-exporter.*"} == 0
          for: 5m
          labels: { severity: critical }
          annotations:
            summary: "Postgres pod {{ $labels.pod }} appears down"

    - name: nodes
      rules:
        - alert: DiskPressure
          expr: |
            (1 - node_filesystem_free_bytes{mountpoint="/"} / node_filesystem_size_bytes{mountpoint="/"}) > 0.85
          for: 15m
          labels: { severity: warning }
          annotations:
            summary: "Node {{ $labels.instance }} disk > 85% full"

    - name: certs
      rules:
        - alert: CertExpiringSoon
          expr: certmanager_certificate_expiration_timestamp_seconds - time() < 14*86400
          for: 1h
          labels: { severity: warning }
          annotations:
            summary: "{{ $labels.name }} expires in < 14 days"
```

- [ ] **Step 2: kustomization.yaml**

Create `deploy/argocd/manifests/platform/alert-rules/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: observability
resources:
  - rules.yaml
```

- [ ] **Step 3: Application**

Create `deploy/argocd/apps/platform/observability/alert-rules.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: alert-rules
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/platform/alert-rules
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 4: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/alert-rules/ \
        deploy/argocd/apps/platform/observability/alert-rules.yaml
git commit -m "$(cat <<'EOF'
gitops: starter PrometheusRule set (app, data, nodes, certs)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In Prometheus UI (port-forward `kubectl -n observability port-forward svc/prom-prometheus 9090`) → Status → Rules: all four rule groups should appear.

---

## Task 6: Loki + Tempo

**Files:**
- Create: `deploy/argocd/manifests/platform/loki/values.yaml`
- Create: `deploy/argocd/manifests/platform/tempo/values.yaml`
- Create: `deploy/argocd/apps/platform/observability/loki.yaml`
- Create: `deploy/argocd/apps/platform/observability/tempo.yaml`

- [ ] **Step 1: Loki values**

Create `deploy/argocd/manifests/platform/loki/values.yaml`:

```yaml
deploymentMode: SingleBinary

loki:
  auth_enabled: false
  commonConfig:
    replication_factor: 1
  storage:
    type: filesystem
  schemaConfig:
    configs:
      - from: 2025-01-01
        store: tsdb
        object_store: filesystem
        schema: v13
        index:
          prefix: index_
          period: 24h
  pattern_ingester:
    enabled: false
  limits_config:
    retention_period: 168h     # 7d
    max_query_series: 5000
  compactor:
    retention_enabled: true
    delete_request_store: filesystem

singleBinary:
  replicas: 1
  persistence:
    enabled: true
    storageClass: hcloud-volumes
    size: 10Gi
  resources:
    requests: { cpu: 50m, memory: 256Mi }
    limits:   { cpu: 1, memory: 1Gi }

write: { replicas: 0 }
read: { replicas: 0 }
backend: { replicas: 0 }

chunksCache:
  enabled: false
resultsCache:
  enabled: false
gateway:
  enabled: false
test:
  enabled: false
```

- [ ] **Step 2: Tempo values**

Create `deploy/argocd/manifests/platform/tempo/values.yaml`:

```yaml
tempo:
  retention: 72h
  storage:
    trace:
      backend: local
      local:
        path: /var/tempo/traces
  receivers:
    otlp:
      protocols:
        grpc:
          endpoint: 0.0.0.0:4317
        http:
          endpoint: 0.0.0.0:4318

persistence:
  enabled: true
  storageClassName: hcloud-volumes
  size: 10Gi

resources:
  requests: { cpu: 50m, memory: 256Mi }
  limits:   { cpu: 1, memory: 1Gi }
```

- [ ] **Step 3: Applications**

Create `deploy/argocd/apps/platform/observability/loki.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: loki
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://grafana.github.io/helm-charts
      chart: loki
      targetRevision: 6.16.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/loki/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

Create `deploy/argocd/apps/platform/observability/tempo.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: tempo
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://grafana.github.io/helm-charts
      chart: tempo
      targetRevision: 1.10.3
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/tempo/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 4: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/loki/ \
        deploy/argocd/manifests/platform/tempo/ \
        deploy/argocd/apps/platform/observability/loki.yaml \
        deploy/argocd/apps/platform/observability/tempo.yaml
git commit -m "$(cat <<'EOF'
gitops: install loki + tempo

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD: both apps `Synced + Healthy`. `kubectl -n observability get pods` shows `loki-0` and `tempo-0` Running.

---

## Task 7: OpenTelemetry Collector

**Files:**
- Create: `deploy/argocd/manifests/platform/otel-collector/values.yaml`
- Create: `deploy/argocd/apps/platform/observability/otel-collector.yaml`

- [ ] **Step 1: Values**

Create `deploy/argocd/manifests/platform/otel-collector/values.yaml`:

```yaml
mode: deployment

image:
  repository: otel/opentelemetry-collector-contrib

replicaCount: 1

resources:
  requests: { cpu: 50m, memory: 128Mi }
  limits:   { cpu: 500m, memory: 512Mi }

config:
  receivers:
    otlp:
      protocols:
        grpc: { endpoint: 0.0.0.0:4317 }
        http: { endpoint: 0.0.0.0:4318 }

  processors:
    batch:
      timeout: 5s
      send_batch_size: 1024
    memory_limiter:
      check_interval: 5s
      limit_percentage: 80
      spike_limit_percentage: 25

  exporters:
    otlp/tempo:
      endpoint: tempo.observability.svc.cluster.local:4317
      tls: { insecure: true }
    otlphttp/loki:
      endpoint: http://loki.observability.svc.cluster.local:3100/otlp
    prometheusremotewrite:
      endpoint: http://prom-prometheus.observability.svc.cluster.local:9090/api/v1/write
      tls: { insecure: true }
      resource_to_telemetry_conversion: { enabled: true }

  service:
    pipelines:
      traces:
        receivers: [otlp]
        processors: [memory_limiter, batch]
        exporters: [otlp/tempo]
      logs:
        receivers: [otlp]
        processors: [memory_limiter, batch]
        exporters: [otlphttp/loki]
      metrics:
        receivers: [otlp]
        processors: [memory_limiter, batch]
        exporters: [prometheusremotewrite]

ports:
  otlp:      { enabled: true, containerPort: 4317, servicePort: 4317, protocol: TCP }
  otlp-http: { enabled: true, containerPort: 4318, servicePort: 4318, protocol: TCP }
```

- [ ] **Step 2: Application**

Create `deploy/argocd/apps/platform/observability/otel-collector.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: otel-collector
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://open-telemetry.github.io/opentelemetry-helm-charts
      chart: opentelemetry-collector
      targetRevision: 0.108.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/otel-collector/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/otel-collector/ \
        deploy/argocd/apps/platform/observability/otel-collector.yaml
git commit -m "$(cat <<'EOF'
gitops: install OpenTelemetry Collector

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In ArgoCD: app `Synced + Healthy`. Service `otel-collector-opentelemetry-collector` in `observability` namespace exposes OTLP gRPC on :4317 and HTTP on :4318.

---

## Task 8: Flip app OTEL_ENABLED on

**Files:**
- Modify: `deploy/argocd/manifests/todo-app/base/configmap.yaml`

- [ ] **Step 1: Update the ConfigMap**

Replace these two lines in `deploy/argocd/manifests/todo-app/base/configmap.yaml`:

```yaml
  APP__OBSERVABILITY__OTEL_ENABLED: "false"
```

with:

```yaml
  APP__OBSERVABILITY__OTEL_ENABLED: "true"
  APP__OBSERVABILITY__OTEL_ENDPOINT: "http://otel-collector-opentelemetry-collector.observability.svc.cluster.local:4317"
```

- [ ] **Step 2: Commit + push + verify**

```bash
git add deploy/argocd/manifests/todo-app/base/configmap.yaml
git commit -m "$(cat <<'EOF'
gitops: flip app OTEL_ENABLED on; point at in-cluster collector

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

ArgoCD reconciles staging + prod; app pods roll. Then:

```bash
# Trigger some requests
for _ in $(seq 1 30); do curl -s https://staging.todo.<yourdomain>/healthz > /dev/null; done

# Port-forward Tempo and query for spans
kubectl -n observability port-forward svc/tempo 3200:3200 &
sleep 2
curl -s "http://localhost:3200/api/search?tags=service.name=todo-app&start=$(($(date +%s)-300))&end=$(date +%s)" | head
```

Expected: spans for `todo-app`. Stop port-forward.

---

## Task 9: Scrape app `/metrics`

**Files:**
- Create: `deploy/argocd/manifests/todo-app/base/servicemonitor.yaml`
- Modify: `deploy/argocd/manifests/todo-app/base/kustomization.yaml`

- [ ] **Step 1: ServiceMonitor**

Create `deploy/argocd/manifests/todo-app/base/servicemonitor.yaml`:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: todo-app
  labels:
    release: prom    # discovered by prom-operator
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: todo-app
  endpoints:
    - port: http
      path: /metrics
      interval: 30s
```

- [ ] **Step 2: Add to kustomization**

In `deploy/argocd/manifests/todo-app/base/kustomization.yaml`, append to `resources:`:

```yaml
  - servicemonitor.yaml
```

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/manifests/todo-app/base/servicemonitor.yaml \
        deploy/argocd/manifests/todo-app/base/kustomization.yaml
git commit -m "$(cat <<'EOF'
gitops: ServiceMonitor for todo-app /metrics

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

Wait for sync, then check Prometheus (port-forward `prom-prometheus:9090`) → Status → Targets. `serviceMonitor/todo-app-*/todo-app/0` targets across staging+prod should be `UP`.

---

## Task 10: Port the dev Grafana dashboard

**Files:**
- Create: `deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards/todo-app.yaml`

The dashboards sidecar (Task 2 values) auto-loads ConfigMaps labeled `grafana_dashboard: "1"`.

- [ ] **Step 1: Wrap the existing dashboard JSON in a ConfigMap**

```bash
mkdir -p deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards
```

Create `deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards/todo-app.yaml`:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: dashboard-todo-app
  namespace: observability
  labels:
    grafana_dashboard: "1"
data:
  todo-app.json: |
    PASTE_CONTENTS_OF_docker_grafana_dashboards_app.json
```

Replace `PASTE_CONTENTS_OF_...` with the literal contents of `docker/grafana/dashboards/app.json`. (You can use `kubectl create configmap --from-file --dry-run=client -o yaml` to generate this if the file is long; commit the resulting YAML.)

A more maintainable alternative: keep the JSON file as-is and use a Kustomize `configMapGenerator`:

```yaml
# In deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards/kustomization.yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: observability
generatorOptions:
  disableNameSuffixHash: true
  labels:
    grafana_dashboard: "1"
configMapGenerator:
  - name: dashboard-todo-app
    files:
      - todo-app.json=../../../../../../docker/grafana/dashboards/app.json
```

And register this as an Argo Application (or add to the kube-prometheus-stack Application sources). The relative `..` chain is ugly; the simpler path is to copy the JSON file into this folder.

- [ ] **Step 2: Decide on approach and execute**

Pick whichever shape works for you; the result is one ConfigMap labeled `grafana_dashboard: "1"` in `observability` namespace containing the dashboard JSON.

- [ ] **Step 3: Application wrapper (if needed)**

If you went with the configMapGenerator approach, add another sub-Application:

Create `deploy/argocd/apps/platform/observability/dashboards.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: dashboards
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 4: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards/ \
        deploy/argocd/apps/platform/observability/dashboards.yaml
git commit -m "$(cat <<'EOF'
gitops: port todo-app grafana dashboard into the cluster

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

Visit `https://grafana.<yourdomain>` (log in `admin` / the password from `values.yaml`'s `adminPassword`, then change it). Dashboards → "todo-app" should appear with live data.

---

## Task 11: Pod log shipping with Grafana Alloy

**Files:**
- Create: `deploy/argocd/manifests/platform/alloy/values.yaml`
- Create: `deploy/argocd/apps/platform/observability/alloy.yaml`

- [ ] **Step 1: Values**

Create `deploy/argocd/manifests/platform/alloy/values.yaml`:

```yaml
alloy:
  configMap:
    create: true
    content: |
      logging {
        level = "info"
      }

      discovery.kubernetes "pods" {
        role = "pod"
      }

      discovery.relabel "pod_logs" {
        targets = discovery.kubernetes.pods.targets
        rule {
          source_labels = ["__meta_kubernetes_pod_node_name"]
          target_label  = "node"
        }
        rule {
          source_labels = ["__meta_kubernetes_namespace"]
          target_label  = "namespace"
        }
        rule {
          source_labels = ["__meta_kubernetes_pod_name"]
          target_label  = "pod"
        }
        rule {
          source_labels = ["__meta_kubernetes_pod_container_name"]
          target_label  = "container"
        }
      }

      loki.source.kubernetes "pods" {
        targets    = discovery.relabel.pod_logs.output
        forward_to = [loki.write.default.receiver]
      }

      loki.write "default" {
        endpoint {
          url = "http://loki.observability.svc.cluster.local:3100/loki/api/v1/push"
        }
      }

controller:
  type: daemonset
```

- [ ] **Step 2: Application**

Create `deploy/argocd/apps/platform/observability/alloy.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: alloy
  namespace: argocd
  finalizers: [resources-finalizer.argocd.argoproj.io]
spec:
  project: default
  sources:
    - repoURL: https://grafana.github.io/helm-charts
      chart: alloy
      targetRevision: 0.10.0
      helm:
        valueFiles:
          - $values/deploy/argocd/manifests/platform/alloy/values.yaml
    - repoURL: https://github.com/nickhstr/todo-rust.git
      targetRevision: HEAD
      ref: values
  destination:
    server: https://kubernetes.default.svc
    namespace: observability
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/alloy/ \
        deploy/argocd/apps/platform/observability/alloy.yaml
git commit -m "$(cat <<'EOF'
gitops: alloy daemonset for pod log shipping into loki

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In Grafana → Explore → Loki datasource, query `{namespace="todo-app-staging"} | json`. Logs should appear within ~30s of running `curl https://staging.todo.<yourdomain>/`.

---

## Task 12: Datasource autoprovisioning (Loki + Tempo into Grafana)

**Files:**
- Create: `deploy/argocd/manifests/platform/kube-prometheus-stack/datasources.yaml`

The Grafana sidecar (Task 2) also auto-loads datasources from ConfigMaps labeled `grafana_datasource: "1"`. Prometheus is wired by kube-prometheus-stack itself; we add Loki and Tempo.

- [ ] **Step 1: ConfigMap**

Create `deploy/argocd/manifests/platform/kube-prometheus-stack/datasources.yaml`:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: datasource-loki-tempo
  namespace: observability
  labels:
    grafana_datasource: "1"
data:
  loki.yaml: |
    apiVersion: 1
    datasources:
      - name: Loki
        type: loki
        access: proxy
        url: http://loki.observability.svc.cluster.local:3100
        isDefault: false
      - name: Tempo
        type: tempo
        access: proxy
        url: http://tempo.observability.svc.cluster.local:3200
        jsonData:
          tracesToLogsV2:
            datasourceUid: loki
            tags: [{ key: 'service.name', value: 'service_name' }]
```

- [ ] **Step 2: Add to the dashboards Application (or create a new app)**

The simplest is to put the file in the dashboards folder so it ships alongside dashboards (same Application).

```bash
mv deploy/argocd/manifests/platform/kube-prometheus-stack/datasources.yaml \
   deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards/datasources.yaml
```

If the dashboards folder has a `kustomization.yaml`, append `datasources.yaml` to its `resources:` (or `generators:` if you used configMapGenerator). Otherwise the directory-mode Application will pick it up automatically.

- [ ] **Step 3: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/kube-prometheus-stack/dashboards/datasources.yaml
git commit -m "$(cat <<'EOF'
gitops: autoprovision loki + tempo grafana datasources

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

In Grafana → Configuration → Data Sources: Prometheus, Loki, Tempo all present. Visit Explore and confirm queries work against each.

---

## Final verification

- [ ] All observability Applications `Synced + Healthy` in ArgoCD
- [ ] `kubectl -n observability get pods` — all `Running`
- [ ] `https://grafana.<yourdomain>` loads (valid LE cert); the `todo-app` dashboard shows live data
- [ ] Prometheus → Status → Targets — `todo-app` ServiceMonitor targets are `UP`
- [ ] Tempo query for `service.name=todo-app` returns spans
- [ ] Loki query for `{namespace="todo-app-prod"}` returns logs
- [ ] Fire a test Alertmanager alert via the API (Task 4 Step 5) and confirm email arrives
- [ ] Stop one app pod (`kubectl -n todo-app-staging delete pod -l app.kubernetes.io/component=web`); within minutes the `AppPodCrashloop` rule should not fire (since k8s self-heals). Use it as a sanity check that rules are loading.

Hand off to **Plan 4 (Preview environments)**.
