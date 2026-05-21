#!/usr/bin/env bash
# One-shot: create the Hetzner Object Storage bucket that holds OpenTofu state
# and (later) Postgres WAL backups. Run this ONCE per environment.
#
# Prereqs: HCLOUD_TOKEN env var or `op` access to "hetzner-api-token".
# Requires `aws` CLI (Hetzner Object Storage is S3-compatible).

set -euo pipefail

BUCKET="${BUCKET:-todo-app-tofu-state}"
REGION="${REGION:-nbg1}"
ENDPOINT="https://${REGION}.your-objectstorage.com"

if ! command -v aws >/dev/null; then
    echo "aws CLI required. brew install awscli" >&2
    exit 1
fi

if [[ -z "${AWS_ACCESS_KEY_ID:-}" ]]; then
    echo "Set AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY from a Hetzner Object Storage credential pair."
    echo "Create one at: Console → Security → S3 credentials."
    exit 1
fi

aws --endpoint-url="$ENDPOINT" s3 mb "s3://${BUCKET}" --region "$REGION"
aws --endpoint-url="$ENDPOINT" s3api put-bucket-versioning \
--bucket "$BUCKET" \
--versioning-configuration Status=Enabled

echo "Bucket s3://${BUCKET} ready at ${ENDPOINT}"
