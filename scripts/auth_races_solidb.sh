#!/usr/bin/env bash
# The auth scaffold's single-use values, used by several requests in the
# same instant, against a real SoliDB.
#
# The Rust tests send requests together too, but on SQLite, and the OpenID
# Connect provider is SoliDB-only: CI has no SoliDB, so this is run by hand.
# It generates an app with this `soli`, starts a throwaway `solidb` and the
# app on four request workers, and checks that each of these goes through
# exactly once: the first account becoming admin, a password-reset link, a
# passkey challenge, an authorization code, a refresh token, a device code.
#
#   scripts/auth_races_solidb.sh [path/to/soli]
#
# Needs `solidb`, `openssl` and `python3` on PATH. Touches nothing outside a
# temporary directory; ports with AUTH_RACES_DB_PORT / AUTH_RACES_APP_PORT.
set -euo pipefail

SOLI="$(realpath "$(command -v "${1:-soli}")")"
DB_PORT="${AUTH_RACES_DB_PORT:-6850}"
APP_PORT="${AUTH_RACES_APP_PORT:-6851}"
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  done
  rm -rf "$WORK"
}
trap cleanup EXIT

mkdir "$WORK/data"
SOLIDB_ADMIN_PASSWORD=testpass123 solidb --host 127.0.0.1 --port "$DB_PORT" \
  --data-dir "$WORK/data" --no-lua >"$WORK/solidb.log" 2>&1 &
PIDS+=($!)
sleep 4

cd "$WORK"
"$SOLI" new app >/dev/null 2>&1
cd app
sed -i.bak "s#^SOLIDB_HOST=.*#SOLIDB_HOST=http://127.0.0.1:$DB_PORT#; s#^SOLIDB_PASSWORD=.*#SOLIDB_PASSWORD=testpass123#" .env .env.test
"$SOLI" generate auth --passkeys >/dev/null 2>&1
"$SOLI" generate oidc_provider >/dev/null 2>&1
"$SOLI" db:migrate up >/dev/null 2>&1

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$WORK/oidc.pem" 2>/dev/null
openssl rsa -in "$WORK/oidc.pem" -pubout -out "$WORK/oidc.pub.pem" 2>/dev/null
export APP_ENV=test AUTH_ATTEMPTS_PER_IP=1000 SOLI_MAIL_DELIVERY_METHOD=logger
export SOLI_OIDC_ISSUER="http://127.0.0.1:$APP_PORT"
SOLI_OIDC_PRIVATE_KEY="$(cat "$WORK/oidc.pem")"
SOLI_OIDC_PUBLIC_KEY="$(cat "$WORK/oidc.pub.pem")"
export SOLI_OIDC_PRIVATE_KEY SOLI_OIDC_PUBLIC_KEY

"$SOLI" serve . --port "$APP_PORT" --workers 4 >"$WORK/app.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 60); do
  curl -s -o /dev/null "http://127.0.0.1:$APP_PORT/login" && break
  sleep 0.5
done

python3 "$HERE/auth_races_solidb.py" "$SOLI" "$WORK/app" "http://127.0.0.1:$APP_PORT"
