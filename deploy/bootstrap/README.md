# Cluster bootstrap

Initial installs of platform components, run *once* against a fresh cluster.
After ArgoCD comes up (Task 15), it takes over reconciliation of all of these.

Order:
1. `hcloud-ccm` + `hcloud-csi` (Task 9)        — cloud integration
2. `ingress-nginx` (Task 10)                    — provisions a Hetzner LB
3. (back to tofu) DNS records pointing at LB IP (Task 11)
4. `cert-manager` (Task 12)                     — TLS automation
5. ClusterIssuers for Let's Encrypt (Task 13)
6. Cert smoke test (Task 14)
7. ArgoCD bootstrap (Task 15)
