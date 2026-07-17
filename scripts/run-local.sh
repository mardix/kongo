#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Load environment file.
# Priority:
# 1) kongodb.env.${KONGODB_ENV} if KONGODB_ENV is set
# 2) kongodb.env
export KONGODB_ENV="${KONGODB_ENV:-local}"
ENV_FILE="$ROOT_DIR/kongodb.env"
if [[ -f "$ROOT_DIR/kongodb.env.${KONGODB_ENV}" ]]; then
  ENV_FILE="$ROOT_DIR/kongodb.env.${KONGODB_ENV}"
fi

if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ENV_FILE"
  set +a
fi

export KONGODB_STORAGE_MODE="${KONGODB_STORAGE_MODE:-local}"
export KONGODB_PORT="${KONGODB_PORT:-8080}"
export KONGODB_DATA_DIR="${KONGODB_DATA_DIR:-./data_local}"
export KONGODB_BASE_PATH="${KONGODB_BASE_PATH:-}"
export KONGODB_BACKUP_PATH="${KONGODB_BACKUP_PATH:-./backups}"
export KONGODB_AUTH_MODE="none"

# Optional archive cleanup TTL. Uncomment to force __kdb_archive retention.
# export KONGODB_ARCHIVE_TTL_SECS="${KONGODB_ARCHIVE_TTL_SECS:-86400}"

echo "Starting Kongodb"
echo "  env:   $KONGODB_ENV"
echo "  mode:  $KONGODB_STORAGE_MODE"
echo "  port:  $KONGODB_PORT"
echo "  base path: ${KONGODB_BASE_PATH:-<none>}"
echo "  gateway path: ${KONGODB_BASE_PATH}/gateway"
echo "  data:  $KONGODB_DATA_DIR"
echo "  runtime profile: ${KONGODB_RUNTIME_PROFILE:-balanced}"

exec cargo run
