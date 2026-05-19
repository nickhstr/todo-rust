# K8s Preview Environments — Plan 4 of 5

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open a PR → an isolated preview environment (`todo-app-pr-<N>` namespace with its own todo-app + CNPG cluster + Valkey) spins up automatically at `pr-<N>.todo.<yourdomain>`. Close/merge the PR → environment + PVCs torn down. Driven entirely by ArgoCD's ApplicationSet PullRequestGenerator plus a small GHA workflow that builds and pushes the per-PR image.

**Architecture:** A single ArgoCD ApplicationSet polls GitHub for open PRs against `main`. For each PR, it templates the `preview/` Kustomize overlay (already written in Plan 2 Task 11), substituting the PR number into the namespace, hostname, and image tag. A GHA workflow (`preview-build.yml`) builds + pushes `ghcr.io/nickhstr/todo-app:pr-<N>-<sha>` on every PR push so the ApplicationSet has an image to point at. cert-manager issues a per-PR cert via DNS-01.

**Tech Stack:**
- ArgoCD ApplicationSet (continues Plan 1's ArgoCD install)
- GitHub PR webhook polling (the ApplicationSet does this server-side, not via webhooks)
- A GitHub token with PR-read scope, stored in 1Password

**Spec:** `docs/superpowers/specs/2026-05-18-k8s-deploy-design.md`

**Plan position:** Plan 4 of 5. Predecessors: Plans 1–3. Followup: Plan 5 (local k3d).

---

## Prerequisites

- Plan 2 complete: `preview/` overlay exists at `deploy/argocd/manifests/todo-app/overlays/preview/`; `preview/SESSION_KEY` exists in 1Password.
- The wildcard DNS record `*.todo.<yourdomain>` from Plan 1's Tofu config covers all `pr-<N>.todo.<yourdomain>` hostnames (already in place).
- Decide LE per-PR cert vs shared wildcard cert. **This plan uses per-PR certs** (cert-manager re-issues for each new preview namespace via DNS-01). LE's rate limit is 50 certs per registered domain per week, plenty for a personal project. If you ever bump against it, swap in a shared wildcard via the `reflector` Helm chart.

---

## File Structure

```
.github/workflows/
└── preview-build.yml                # NEW

deploy/argocd/
├── apps/
│   └── todo-app/
│       └── previews.yaml            # NEW — the ApplicationSet
└── manifests/
    └── platform/
        └── argocd-pr-token/         # NEW — ExternalSecret for the GitHub PR token
            ├── kustomization.yaml
            └── external-secret.yaml
```

Plus updates to `deploy/argocd/apps/platform/` (a new sub-Application to deploy the PR-token ExternalSecret).

---

## Task 1: GitHub PR token

**Files:** none (manual setup)

The ApplicationSet's PullRequestGenerator needs read-only access to PRs on the repo.

- [ ] **Step 1: Create a fine-grained PAT**

GitHub → Settings → Developer settings → Personal access tokens → Fine-grained tokens → Generate.
- Token name: `todo-rust-argocd-pr-reader`
- Expiration: 1 year (note to rotate)
- Repository access: Only select repositories → `todo-rust`
- Repository permissions:
  - **Pull requests**: Read-only
  - **Metadata**: Read-only (mandatory)
- Generate; copy.

- [ ] **Step 2: Store in 1Password**

In vault `todo-app`:
- New item, type: API Credential
- Name: `github-pr-token`
- Field `token`: paste the value

No commit.

---

## Task 2: ExternalSecret for the PR token

**Files:**
- Create: `deploy/argocd/manifests/platform/argocd-pr-token/external-secret.yaml`
- Create: `deploy/argocd/manifests/platform/argocd-pr-token/kustomization.yaml`
- Create: `deploy/argocd/apps/platform/argocd-pr-token.yaml`

- [ ] **Step 1: ExternalSecret**

Create `deploy/argocd/manifests/platform/argocd-pr-token/external-secret.yaml`:

```yaml
apiVersion: external-secrets.io/v1beta1
kind: ExternalSecret
metadata:
  name: github-pr-token
  namespace: argocd
spec:
  refreshInterval: 24h
  secretStoreRef:
    name: onepassword-connect
    kind: ClusterSecretStore
  target:
    name: github-pr-token
    creationPolicy: Owner
  data:
    - secretKey: token
      remoteRef:
        key: github-pr-token
        property: token
```

- [ ] **Step 2: kustomization.yaml**

Create `deploy/argocd/manifests/platform/argocd-pr-token/kustomization.yaml`:

```yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
namespace: argocd
resources:
  - external-secret.yaml
```

- [ ] **Step 3: Application**

Create `deploy/argocd/apps/platform/argocd-pr-token.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: argocd-pr-token
  namespace: argocd
spec:
  project: default
  source:
    repoURL: https://github.com/nickhstr/todo-rust.git
    targetRevision: HEAD
    path: deploy/argocd/manifests/platform/argocd-pr-token
  destination:
    server: https://kubernetes.default.svc
    namespace: argocd
  syncPolicy:
    automated: { prune: true, selfHeal: true }
    syncOptions: [ServerSideApply=true]
```

- [ ] **Step 4: Commit + push + verify**

```bash
git add deploy/argocd/manifests/platform/argocd-pr-token/ \
        deploy/argocd/apps/platform/argocd-pr-token.yaml
git commit -m "$(cat <<'EOF'
gitops: ExternalSecret for ArgoCD PR-generator github token

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

```bash
kubectl -n argocd get secret github-pr-token
```

Expected: secret materialized, contains key `token`.

---

## Task 3: ApplicationSet for preview environments

**Files:**
- Create: `deploy/argocd/apps/todo-app/previews.yaml`

This is the heart of the plan.

- [ ] **Step 1: ApplicationSet manifest**

Create `deploy/argocd/apps/todo-app/previews.yaml`:

```yaml
apiVersion: argoproj.io/v1alpha1
kind: ApplicationSet
metadata:
  name: todo-app-previews
  namespace: argocd
spec:
  goTemplate: true
  goTemplateOptions: ["missingkey=error"]
  generators:
    - pullRequest:
        github:
          owner: nickhstr
          repo: todo-rust
          tokenRef:
            secretName: github-pr-token
            key: token
          labels:
            - preview                # only PRs labeled 'preview' spawn an env
        requeueAfterSeconds: 60
  template:
    metadata:
      name: 'todo-pr-{{.number}}'
    spec:
      project: default
      source:
        repoURL: https://github.com/nickhstr/todo-rust.git
        targetRevision: '{{.head_sha}}'
        path: deploy/argocd/manifests/todo-app/overlays/preview
        kustomize:
          namespace: 'todo-app-pr-{{.number}}'
          images:
            - 'todo-app=ghcr.io/nickhstr/todo-app:pr-{{.number}}-{{.head_sha}}'
          patches:
            - target:
                kind: Ingress
                name: todo-app
              patch: |-
                - op: replace
                  path: /spec/rules/0/host
                  value: pr-{{.number}}.todo.<yourdomain>
                - op: replace
                  path: /spec/tls/0/hosts/0
                  value: pr-{{.number}}.todo.<yourdomain>
      destination:
        server: https://kubernetes.default.svc
        namespace: 'todo-app-pr-{{.number}}'
      syncPolicy:
        automated:
          prune: true
          selfHeal: true
        syncOptions:
          - ServerSideApply=true
          - CreateNamespace=true
```

Notes:
- `labels: [preview]` filters to PRs explicitly labeled `preview`. Remove this filter if you want every PR to spawn an env automatically (more compute, more LE certs).
- The `patches:` block under `kustomize:` runs JSON 6902 patches after Kustomize, injecting the per-PR hostname into the Ingress. This is the only place the PR number leaks into a base manifest.
- `targetRevision: '{{.head_sha}}'` pins each preview Application to the head commit of its PR. Useful so a force-push doesn't redeploy the preview against a stale base.

Substitute `<yourdomain>` in the two patch lines.

- [ ] **Step 2: Commit + push**

```bash
git add deploy/argocd/apps/todo-app/previews.yaml
git commit -m "$(cat <<'EOF'
gitops: ApplicationSet for per-PR preview envs

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

- [ ] **Step 3: Verify ApplicationSet appears**

ArgoCD UI → Settings → ApplicationSets — `todo-app-previews` should appear, with `Status: 0 applications generated` (no PRs are open yet).

---

## Task 4: `preview-build.yml` workflow

**Files:**
- Create: `.github/workflows/preview-build.yml`

- [ ] **Step 1: Workflow**

Create `.github/workflows/preview-build.yml`:

```yaml
name: preview-build

on:
  pull_request:
    types: [opened, synchronize, reopened, labeled]

permissions:
  contents: read
  packages: write

jobs:
  build-preview:
    # Only build for PRs labeled 'preview'.
    if: contains(github.event.pull_request.labels.*.name, 'preview')
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ github.event.pull_request.head.sha }}

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
          tags: 'ghcr.io/nickhstr/todo-app:pr-${{ github.event.pull_request.number }}-${{ github.event.pull_request.head.sha }}'
          cache-from: type=gha
          cache-to: type=gha,mode=max
          build-args: |
            GIT_SHA=${{ github.event.pull_request.head.sha }}

      - name: comment on PR with preview URL
        uses: actions/github-script@v7
        with:
          script: |
            const prNumber = context.payload.pull_request.number;
            const sha = context.payload.pull_request.head.sha.substring(0, 7);
            const body = [
              `📦 Preview image built: \`ghcr.io/nickhstr/todo-app:pr-${prNumber}-${context.payload.pull_request.head.sha}\``,
              `🚀 ArgoCD will sync within ~60s → https://pr-${prNumber}.todo.<yourdomain>`,
              `📍 Current commit: \`${sha}\``,
            ].join('\n');
            const { data: comments } = await github.rest.issues.listComments({
              owner: context.repo.owner,
              repo: context.repo.repo,
              issue_number: prNumber,
            });
            const existing = comments.find(c =>
              c.user.login === 'github-actions[bot]' && c.body.includes('Preview image built'));
            if (existing) {
              await github.rest.issues.updateComment({
                owner: context.repo.owner, repo: context.repo.repo,
                comment_id: existing.id, body,
              });
            } else {
              await github.rest.issues.createComment({
                owner: context.repo.owner, repo: context.repo.repo,
                issue_number: prNumber, body,
              });
            }
```

Substitute `<yourdomain>` in the comment body.

- [ ] **Step 2: Commit + push**

```bash
git add .github/workflows/preview-build.yml
git commit -m "$(cat <<'EOF'
ci: preview-build workflow — builds per-PR image, comments URL

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
git push
```

---

## Task 5: End-to-end test — open a PR and watch a preview spin up

**Files:** none

- [ ] **Step 1: Make a trivial change on a feature branch**

```bash
git checkout -b preview/smoke-test
echo "// preview smoke" >> README.md
git add README.md
git commit -m "preview: smoke-test"
git push -u origin preview/smoke-test
```

- [ ] **Step 2: Open a PR and label it `preview`**

GitHub UI → New PR from `preview/smoke-test` → `main`. After creating, add the `preview` label (create the label if it doesn't exist).

- [ ] **Step 3: Watch the pipeline**

1. `preview-build` workflow starts, builds image, comments PR URL within ~5 minutes.
2. Within ~60s of the GHA build finishing, the ArgoCD ApplicationSet generates a new Application `todo-pr-<N>`. In the ArgoCD UI: Application appears, syncs.
3. Watch the namespace come up:
   ```bash
   kubectl get ns | grep todo-app-pr
   kubectl -n todo-app-pr-<N> get all,certificate,externalsecret
   ```
4. cert-manager issues the per-PR cert (1–2 min via DNS-01).
5. Visit `https://pr-<N>.todo.<yourdomain>` — signup page renders.

- [ ] **Step 4: Push another commit; watch update**

```bash
echo "// another change" >> README.md
git commit -am "preview: tweak"
git push
```

- `preview-build` rebuilds; new tag `pr-<N>-<new-sha>` lands in GHCR
- ApplicationSet's `targetRevision` updates to `head_sha` of the new commit
- New image rolls into the existing namespace; no namespace churn

- [ ] **Step 5: Close the PR; watch teardown**

In GitHub UI: close the PR (don't merge). Within ~60s:
- ApplicationSet sees the PR is closed; removes the Application
- ArgoCD prunes the namespace; PVCs are reclaimed (`hcloud_volumes` `reclaimPolicy: Retain` means the volume is detached but not deleted — see "cleanup" note below)

- [ ] **Step 6: Verify teardown**

```bash
kubectl get ns todo-app-pr-<N>
# Expected: NotFound, or Terminating then gone.
```

- [ ] **Step 7: (Optional) Clean up retained volumes**

Because the `hcloud-volumes` StorageClass has `reclaimPolicy: Retain` (set in Plan 1's Task 9), preview env PVCs leave behind detached Hetzner Volumes after the namespace is gone. List orphans:

```bash
# Inspect Hetzner Console → Volumes — anything matching pvc-* with no attachment is a candidate to delete.
```

If preview env volumes accumulate, change preview's StorageClass to a Delete-policy variant. For now (low PR volume), manual cleanup is fine.

---

## Task 6: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a "Preview environments" section**

Append (or insert into the deployment section):

```markdown
### Preview environments

Open a PR against `main`, label it `preview`, and within ~5 minutes:
- A docker image is built and pushed to `ghcr.io/nickhstr/todo-app:pr-<N>-<sha>`
- ArgoCD spawns a `todo-app-pr-<N>` namespace with its own todo-app + CNPG + Valkey
- The preview is reachable at `https://pr-<N>.todo.<yourdomain>` with a real LE cert

The PR gets a comment with the preview URL. Closing or merging the PR
auto-tears down the namespace. Underlying Hetzner volumes have
reclaimPolicy=Retain — they linger as orphans (cheap, ~$0.50/10GB/mo)
until manually deleted via the Hetzner console.

To skip building a preview for a particular PR, simply don't add the
`preview` label.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "$(cat <<'EOF'
docs: preview environments section

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final verification

- [ ] `kubectl -n argocd get appset todo-app-previews` exists
- [ ] `kubectl -n argocd get secret github-pr-token` exists
- [ ] Open a test PR labeled `preview` → image builds, namespace spawns, preview URL serves the app
- [ ] Push a follow-up commit → image rebuilds, namespace updates without re-creation
- [ ] Close the PR → namespace gets pruned within ~60s

Hand off to **Plan 5 (Local k3d path)**.
