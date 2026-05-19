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
