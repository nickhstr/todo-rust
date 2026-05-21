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
