# Tailwind standalone binary — replace the npm toolchain

**Status:** Approved 2026-05-20
**Branch:** `chore/tailwind-standalone-binary`

## Problem

The project depends on npm for exactly one thing: invoking `tailwindcss` to build `static/css/app.css`. That pulls a full Node toolchain into the dev compose stack, the prod image build, and the host workflow — `package.json`, `package-lock.json`, `node_modules/`, `tailwind.config.js`, plus a `node:24-bookworm-slim` build stage in `docker/Dockerfile` and a `node:24` service in `docker/compose.dev.yaml` that does `npm install` on every container start.

Tailwind Labs publishes the v4 CLI as a single statically-linked binary per OS/arch on [GitHub Releases](https://github.com/tailwindlabs/tailwindcss/releases). We can drop the entire Node toolchain by switching to that binary.

## Goals

- Remove npm, Node, `package.json`, `package-lock.json`, `node_modules/`, and `tailwind.config.js` from the repo.
- Keep the build/run/dev-loop UX equivalent: `just css`, `just css-build`, `docker compose up --build`, the dev polling loop.
- Pin Tailwind to a specific version with SHA256 verification on download.
- One acquisition path used by host, prod Dockerfile, and dev compose alike.

## Non-goals

- Switching to a different CSS framework or build tool.
- Eliminating the polling watch loop (podman macOS bind-mount limitation hasn't changed).
- Publishing a Tailwind container image upstream (none exists officially; we won't make one).
- CI changes — CI doesn't run npm today.

## Architecture

One install script, called from three places, flowing through a single `(version, sha256-table)` source of truth.

```
scripts/install-tailwind.sh   ← version + per-platform SHA256s
        ├─ justfile css / css-build   → caches to ./bin/tailwindcss   (host)
        ├─ Dockerfile css-builder stage → installs to /usr/local/bin   (prod image)
        └─ Dev compose tailwind service (via `target: css-builder`)    (dev image)
```

The script is idempotent: if the cached binary's `--help` output already reports the pinned version, it skips the download. That keeps `just css` startup near-instant after the first invocation.

## Components

### 1. `scripts/install-tailwind.sh`

POSIX shell, ~40 lines. Holds two declarations at the top:

```sh
TAILWIND_VERSION=v4.X.Y         # pin at implementation: pick latest v4 stable
INSTALL_DIR=${INSTALL_DIR:-./bin}

# SHA256 per asset, captured at implementation time from
# https://github.com/tailwindlabs/tailwindcss/releases/download/$TAILWIND_VERSION/sha256sums.txt
SHA256_linux_x64="<fill-in>"
SHA256_linux_arm64="<fill-in>"
SHA256_macos_x64="<fill-in>"
SHA256_macos_arm64="<fill-in>"
```

The `v4.X.Y` and `<fill-in>` placeholders are filled in at implementation time — the design intentionally doesn't pick a specific release tag, since a current one should be chosen when the work lands rather than embedded in a brainstorm doc.

Detection logic:

```sh
case "$(uname -s)" in
  Linux)  os=linux ;;
  Darwin) os=macos ;;
  *)      die "unsupported OS: $(uname -s)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64)   arch=x64 ;;
  arm64|aarch64)  arch=arm64 ;;
  *)              die "unsupported arch: $(uname -m)" ;;
esac
asset="tailwindcss-${os}-${arch}"
```

Cache check, then download → verify → install:

```sh
if [ -x "$INSTALL_DIR/tailwindcss" ] && "$INSTALL_DIR/tailwindcss" --help 2>&1 | grep -q "$TAILWIND_VERSION"; then
  exit 0
fi
mkdir -p "$INSTALL_DIR"
url="https://github.com/tailwindlabs/tailwindcss/releases/download/${TAILWIND_VERSION}/${asset}"
tmp=$(mktemp)
curl -fL --retry 3 -o "$tmp" "$url"
echo "$expected_sha256  $tmp" | sha256sum -c -    # macOS: shasum -a 256 -c
chmod +x "$tmp"
mv "$tmp" "$INSTALL_DIR/tailwindcss"
```

Notes:

- The version-check command is whatever the pinned release exposes (`--help`, `--version`, or `-v`); verify before writing the script. The cache-check exits 0 only on a match.
- macOS ships `shasum` not `sha256sum`. The script detects which is on PATH (`command -v sha256sum >/dev/null && hash=sha256sum || hash="shasum -a 256"`) so the same script works on darwin host and debian-bookworm-slim container alike.
- Exit non-zero on any failure. No silent fallback — the whole point of pinning is to fail loudly on drift.

### 2. `static/css/app.src.css` — `@source` directives

```css
@import "tailwindcss";

@source "../../templates";
@source "../../crates";

/* existing @theme block unchanged */
@theme {
  --color-paper: #fbf7ee;
  ...
}
```

This replaces the content-scan role that `tailwind.config.js` was filling. Without `@source`, v4 defaults to scanning `cwd` minus `.gitignore`'d paths — works in principle (Rust's `target/` is gitignored), but explicit is safer and survives any future config drift.

### 3. `docker/Dockerfile` — replace stage 4

Current stage 4 uses `node:24-bookworm-slim` and runs `npm install` + `npm run build`. Replace with two stages built on `debian:bookworm-slim`:

```dockerfile
# -----------------------------------------------------------------------------
# 4a. Tailwind binary (cacheable layer; rebuilt only when version changes)
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim AS css-builder
WORKDIR /app
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
COPY scripts/install-tailwind.sh /tmp/install-tailwind.sh
RUN INSTALL_DIR=/usr/local/bin sh /tmp/install-tailwind.sh

# -----------------------------------------------------------------------------
# 4b. Tailwind CSS build
# -----------------------------------------------------------------------------
FROM css-builder AS css
COPY templates ./templates
COPY static ./static
RUN tailwindcss -i ./static/css/app.src.css -o ./static/css/app.css --minify
```

The two-stage split (`css-builder` vs `css`) exists so dev compose can `target: css-builder` (binary only, no source copy) and mount sources at runtime.

The runtime stage's `COPY --from=css /app/static /app/static` line stays unchanged.

### 4. `docker/compose.dev.yaml` — `tailwind` service rebuilt

Drop the `node:24-bookworm-slim` image and the `npm install` on container startup. Build from the project's Dockerfile, stopping at `css-builder`:

```yaml
  tailwind:
    build:
      context: ..
      dockerfile: docker/Dockerfile
      target: css-builder
    working_dir: /app
    volumes:
      - ../templates:/app/templates:ro
      - ../static:/app/static
    command:
      - sh
      - -c
      - |
        set -eu
        build() { tailwindcss -i ./static/css/app.src.css -o ./static/css/app.css; }
        build
        echo "[tailwind] initial build done; polling for changes every 1s."
        snapshot() { find ./static/css/app.src.css ./templates -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1; }
        LAST=$$(snapshot)
        while true; do
          sleep 1
          NOW=$$(snapshot)
          if [ "$$NOW" != "$$LAST" ]; then
            LAST=$$NOW
            build || true
          fi
        done
```

The polling loop is preserved verbatim — native FS events still don't cross podman's macOS bind-mount, independent of the build tool. Only the binary call path changed (`tailwindcss` instead of `node_modules/.bin/tailwindcss`), and the `tailwind.config.js` bind-mount is gone.

### 5. `justfile`

```
css:
    ./scripts/install-tailwind.sh
    ./bin/tailwindcss -i ./static/css/app.src.css -o ./static/css/app.css --watch=always

css-build:
    ./scripts/install-tailwind.sh
    ./bin/tailwindcss -i ./static/css/app.src.css -o ./static/css/app.css --minify
```

`--watch=always` is still required for the same reason the README documents: Tailwind v4's `--watch` exits when stdin closes; `=always` keeps it alive.

The `update` recipe loses its `npm update` line.

### 6. Deletions

| File / dir | Why |
|---|---|
| `package.json` | Only declared the two `tailwindcss` scripts. |
| `package-lock.json` | Companion to package.json. |
| `node_modules/` | Generated; was already gitignored. |
| `tailwind.config.js` | v4 is CSS-first; `@source` directives in `app.src.css` replace its only purpose (content paths). |

### 7. `.gitignore`

Add: `/bin/` (downloaded binary cache).

Remove the now-obsolete block (lines 8–9, 94–100 of the current file):
```
# Tailwind / npm output (regenerated by `npm run build`)
node_modules/
...
npm-debug.log*
pnpm-debug.log*
.npm/
.pnpm-store/
```

### 8. `.dockerignore`

Remove the `node_modules/` line. Harmless to leave it, but it refers to a path that won't exist.

### 9. `README.md`

Update the quickstart section (currently mentions `npm install` / `npm run build`):

- Drop the `npm install` line.
- Replace `npm run build` with `just css-build` (and note it fetches the binary on first run).
- Update the dev-loop description: "The `tailwind` container runs an mtime-polling loop calling the standalone `tailwindcss` binary" (was "`tailwindcss` CLI" via npm).
- Update the CI-build note: "use `just css-build` instead" (was `npm run build`).

### 10. `CLAUDE.md`

- Replace the npm-flavored quickstart in the surrounding context.
- Keep the existing "Tailwind v4 `--watch` exits when stdin closes" sharp-edge entry — the `--watch=always` reason is identical regardless of how the binary is invoked.
- Add one sentence under "Sharp edges to know about": *"`scripts/install-tailwind.sh` pins the Tailwind version and SHA256 — bumping the binary is a deliberate edit to that script, not a `cargo update`-style sweep."*

## Error handling

| Failure | Behavior |
|---|---|
| Unsupported host OS/arch | Print supported matrix, exit 1. |
| Checksum mismatch | Print expected vs actual, leave the partial download in `/tmp` for inspection, exit 1. |
| GitHub releases unreachable | `curl -fL --retry 3` returns non-zero; the script propagates the exit code. Dockerfile build fails loudly; host `just css` prints the curl error. |
| Cached binary doesn't match pinned version | Re-download (don't silently use stale). |
| Tailwind binary fails at build time (CSS error) | Same behavior as today — non-zero exit, build fails. |

## Verification

Before merge:

1. **Byte-level CSS parity:** Generate `app.css` from `main` (`npm run build`) and from the change branch (`just css-build`). Diff the two outputs. Modulo trivial whitespace/ordering, identical. Document any diff in the PR.
2. **Prod image builds:** `docker compose -f docker/compose.yaml --env-file .env up --build`. App comes up, `/static/css/app.css` serves at 200 with `text/css`, page renders styled.
3. **Dev stack works:** `just up`. Initial CSS build completes, edit a class in a template, observe `app.css` regenerate within ~1s.
4. **Cargo tests untouched:** `cargo test --workspace` passes (no Rust changes; this is a sanity check).
5. **Cold checkout:** On a fresh `git clone` (or after `rm -rf bin/`), `just css-build` downloads, verifies, and produces CSS.
6. **Bad-checksum guard:** Temporarily corrupt one SHA in the script, run install — confirm it exits non-zero with a useful error.

## Migration / rollback

This is a single PR. Rollback is `git revert` of the merge commit. There is no runtime data tied to this change, no schema, no API surface.

## Out of scope (documented to prevent scope creep)

- Switching the polling watcher to a different mechanism.
- Removing the `--watch=always` flag (still required upstream).
- Trimming the runtime image (no change to runtime stage).
- A Renovate/Dependabot config for the pinned Tailwind version — a manual bump step is acceptable for now; revisit if version drift becomes a recurring chore.

## Open questions

None. All clarifying questions resolved during brainstorming (binary source, version pinning, host workflow, config file fate, container install location).
