#!/bin/sh
# Download the pinned Tailwind v4 standalone CLI binary for the current host
# OS/arch, verify its SHA256, and install it into $INSTALL_DIR (default ./bin).
# Idempotent: if the cached binary already matches the pinned version, exits 0
# without touching the network.
#
# Bump the version by editing TAILWIND_VERSION + the four SHA256_* values
# below. Source of truth for hashes is:
#   https://github.com/tailwindlabs/tailwindcss/releases/download/<VERSION>/sha256sums.txt

set -eu

TAILWIND_VERSION=v4.3.0
INSTALL_DIR=${INSTALL_DIR:-./bin}

SHA256_linux_x64="73f0e5459054e5cfaa8ab6f3b940f3fbe0f13cc7fd83bc24e7c655033c203400"
SHA256_linux_arm64="8f48dcb72be3b351c10563c5329b4638ba8516820dc3b3a1609625a166e87cbd"
SHA256_macos_x64="2ba252f770817091e6d0d12a84e0dd531bcc29aad1bfd9d976a3aff1a071b67a"
SHA256_macos_arm64="56b4bbc62dbdc4614a78930d9c6986423a2ec63e4e640144a59a5d95c914322e"

die() { printf 'install-tailwind: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux)  os=linux ;;
  Darwin) os=macos ;;
  *)      die "unsupported OS: $(uname -s) (supported: Linux, Darwin)" ;;
esac

case "$(uname -m)" in
  x86_64|amd64)  arch=x64 ;;
  arm64|aarch64) arch=arm64 ;;
  *)             die "unsupported arch: $(uname -m) (supported: x86_64, arm64)" ;;
esac

asset="tailwindcss-${os}-${arch}"
eval "expected_sha=\${SHA256_${os}_${arch}}"
[ -n "${expected_sha:-}" ] || die "no SHA256 pinned for ${os}-${arch}"

# Cache check: skip download if binary exists and reports the pinned version.
# Tailwind v4 prints the version on its --help banner.
if [ -x "$INSTALL_DIR/tailwindcss" ] \
   && "$INSTALL_DIR/tailwindcss" --help 2>&1 | grep -q "$TAILWIND_VERSION"; then
  exit 0
fi

# Pick a sha256 verifier: sha256sum on Linux, shasum on macOS.
if command -v sha256sum >/dev/null 2>&1; then
  sha_cmd="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  sha_cmd="shasum -a 256"
else
  die "no sha256 tool found (need sha256sum or shasum)"
fi

mkdir -p "$INSTALL_DIR"
url="https://github.com/tailwindlabs/tailwindcss/releases/download/${TAILWIND_VERSION}/${asset}"
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT

printf 'install-tailwind: downloading %s\n' "$url" >&2
curl -fL --retry 3 -o "$tmp" "$url" || die "curl failed for $url"

actual_sha=$($sha_cmd "$tmp" | awk '{print $1}')
if [ "$actual_sha" != "$expected_sha" ]; then
  die "SHA256 mismatch for ${asset}: expected ${expected_sha}, got ${actual_sha}"
fi

chmod +x "$tmp"
mv "$tmp" "$INSTALL_DIR/tailwindcss"
trap - EXIT

printf 'install-tailwind: installed %s/tailwindcss (%s)\n' "$INSTALL_DIR" "$TAILWIND_VERSION" >&2
