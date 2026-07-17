#!/bin/sh
set -eu

# Env-file loading order:
# 1) KONGODB_ENV_FILE (explicit path)
# 2) /app/kongodb.env.$KONGODB_ENV (when KONGODB_ENV is set)
# 3) /app/kongodb.env (default)
load_env_file() {
  f="$1"
  if [ -n "$f" ] && [ -f "$f" ]; then
    # Load env-file values as defaults only; Dockerfile ENV and `docker run -e`
    # values must keep precedence over baked-in profiles.
    while IFS= read -r line || [ -n "$line" ]; do
      case "$line" in
        "" | "#"*) continue ;;
        export\ *) line=${line#export } ;;
      esac

      key=${line%%=*}
      if [ "$key" = "$line" ]; then
        continue
      fi
      case "$key" in
        "" | *[!A-Za-z0-9_]* | [0-9]*) continue ;;
      esac

      if printenv "$key" >/dev/null 2>&1; then
        continue
      fi
      export "$line"
    done < "$f"
    echo "loaded env file: $f"
    return 0
  fi
  return 1
}

if [ "${KONGODB_ENV_FILE:-}" != "" ]; then
  load_env_file "${KONGODB_ENV_FILE}" || {
    echo "KONGODB_ENV_FILE not found: ${KONGODB_ENV_FILE}" >&2
    exit 1
  }
elif [ "${KONGODB_ENV:-}" != "" ]; then
  load_env_file "/app/kongodb.env.${KONGODB_ENV}" || {
    echo "env profile file not found: /app/kongodb.env.${KONGODB_ENV}" >&2
    exit 1
  }
else
  load_env_file "/app/kongodb.env" || true
fi

exec sh -c 'KONGODB_PORT=${PORT:-${KONGODB_PORT:-8080}} kongodb'
