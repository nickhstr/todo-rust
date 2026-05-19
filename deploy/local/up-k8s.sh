#!/usr/bin/env bash
# Bring up todo-app on a local k3d cluster, using the same Kustomize
# manifests as production. Idempotent: re-run safely.
#
# Differences from prod:
#   - no ArgoCD; we `kubectl apply -k` directly
#   - no 1Password / ESO; plain Secret carries a throwaway session key
#   - no ingress-nginx / cert-manager / LB; use `kubectl port-forward`
#   - no Hetzner CSI; use k3s local-path provisioner
#   - 1 app replica, 1-instance CNPG, smaller resources

set -euo pipefail

CLUSTER="${CLUSTER:-todo}"
NAMESPACE="todo-app-local"
CNPG_CHART_VERSION="${CNPG_CHART_VERSION:-0.22.0}"
IMAGE="ghcr.io/nickhstr/todo-app:bootstrap"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# 1. Cluster
if ! k3d cluster list 2>/dev/null | awk '{print $1}' | grep -qx "$CLUSTER"; then
  echo "[1/6] Creating k3d cluster '${CLUSTER}'..."
  k3d cluster create "$CLUSTER" \
    --servers 1 --agents 0 \
    --wait
else
  echo "[1/6] Cluster '${CLUSTER}' already exists."
fi

export KUBECONFIG="$(k3d kubeconfig write "$CLUSTER")"
echo "  KUBECONFIG=${KUBECONFIG}"

# 2. Build the app image and import it into the k3d cluster.
#    We tag with the same value the local overlay expects so we don't need
#    to edit the manifests at apply time.
echo "[2/6] Building app image '${IMAGE}'..."
GIT_SHA="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
docker build \
  --tag "$IMAGE" \
  --build-arg "GIT_SHA=${GIT_SHA}" \
  --file "${REPO_ROOT}/docker/Dockerfile" \
  "$REPO_ROOT"

echo "[3/6] Importing image into k3d..."
k3d image import "$IMAGE" --cluster "$CLUSTER"

# 4. CNPG operator
echo "[4/6] Installing CloudNativePG operator..."
helm repo add cnpg https://cloudnative-pg.github.io/charts >/dev/null 2>&1 || true
helm repo update >/dev/null
helm upgrade --install cloudnative-pg cnpg/cloudnative-pg \
  --namespace cnpg-system \
  --create-namespace \
  --version "$CNPG_CHART_VERSION" \
  --wait --timeout 5m

# 5. Wait for CRDs to register, then apply
echo "[5/6] Waiting for CNPG CRDs..."
kubectl wait --for=condition=Established crd/clusters.postgresql.cnpg.io --timeout=60s

echo "[5/6] Applying todo-app local overlay..."
kubectl apply -k "${REPO_ROOT}/deploy/argocd/manifests/todo-app/overlays/local"

# 6. Wait for app readiness (Postgres needs to be up first; this can take a few minutes on first run)
echo "[6/6] Waiting for app pod readiness (up to 5m)..."
kubectl -n "$NAMESPACE" wait --for=condition=Available deployment/todo-app --timeout=5m || \
  echo "  (deployment not ready yet — check 'kubectl -n $NAMESPACE get all' for diagnostics)"

cat <<MSG

✓ todo-app local stack is up.

  Port-forward to reach it:
    kubectl --kubeconfig "$KUBECONFIG" -n $NAMESPACE port-forward svc/todo-app 8080:80

  Then open http://localhost:8080

  Tear down:
    k3d cluster delete $CLUSTER

  Rebuild app image only (no cluster churn):
    docker build --tag $IMAGE --build-arg GIT_SHA=\$(git rev-parse --short HEAD) --file docker/Dockerfile .
    k3d image import $IMAGE --cluster $CLUSTER
    kubectl --kubeconfig "$KUBECONFIG" -n $NAMESPACE rollout restart deployment/todo-app
MSG
