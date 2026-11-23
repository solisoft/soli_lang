#!/usr/bin/env bash
# The auth scaffold's single-use values, used by several requests in the
# same instant, against a real SoliDB.
#
# The Rust tests send requests together too, but on SQLite, and the OpenID
# Connect provider is SoliDB-only. CI runs this after the specs (the `test`
# job), and it runs by hand the same way. It generates an app with this `soli`, starts a throwaway `solidb` and the
# app on four request workers, and checks that each of these goes through
# exactly once: the first account becoming admin, a password-reset link, a
# passkey challenge, an authorization code, a refresh token, a device code.
#
#   scripts/auth_races_solidb.sh [path/to/soli]
#
# Needs `solidb`, `openssl`, `curl` and `python3` on PATH. Everything lives in
# a temporary directory; the two servers take ports that were free (or
# AUTH_RACES_DB_PORT / AUTH_RACES_APP_PORT, refused when something answers).
set -euo pipefail

# What the shell exports wins over an app's `.env`: nothing of the caller's
# database may reach the commands below.
unset SOLI_DB_ADAPTER DATABASE_URL SOLIDB_HOST SOLIDB_DATABASE SOLIDB_USERNAME \
  SOLIDB_PASSWORD SOLIDB_API_KEY SOLIDB_JWT SOLI_PROTECT_ENV APP_ENV \
  SOLI_SESSION_DRIVER SOLI_ENGINE AUTH_SIGNUP AUTH_SETUP_TOKEN

die() {
  echo "auth_races_solidb: $*" >&2
  exit 1
}
free_port() {
  python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}
answers() {
  curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$1/"
}
# Wait until the process `$1` answers on port `$2`, or die: a server that did
# not start must not leave the checks talking to someone else's.
ready() {
  for _ in $(seq 1 80); do
    kill -0 "$1" 2>/dev/null || die "$3 exited — see $4"
    answers "$2" && return 0
    sleep 0.5
  done
  die "$3 did not answer on port $2"
}

SOLI="$(realpath "$(command -v "${1:-soli}")")"
DB_PORT="${AUTH_RACES_DB_PORT:-$(free_port)}"
APP_PORT="${AUTH_RACES_APP_PORT:-$(free_port)}"
answers "$DB_PORT" && die "something already answers on port $DB_PORT"
answers "$APP_PORT" && die "something already answers on port $APP_PORT"
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
# (`--no-lua` where this solidb has it: nothing here runs a script.)
NO_LUA=()
if solidb --help 2>&1 | grep -q -- "--no-lua"; then NO_LUA=(--no-lua); fi
SOLIDB_ADMIN_PASSWORD=testpass123 JWT_SECRET=throwaway-secret-of-the-auth-race-script \
  solidb --host 127.0.0.1 --port "$DB_PORT" --data-dir "$WORK/data" ${NO_LUA[@]+"${NO_LUA[@]}"} \
  >"$WORK/solidb.log" 2>&1 &
PIDS+=($!)
ready "$!" "$DB_PORT" solidb "$WORK/solidb.log"

# The environment every command below runs in, migrations included: under
# APP_ENV=test the app reads `.env.test`, and so must `db:migrate`.
export APP_ENV=test AUTH_ATTEMPTS_PER_IP=1000 SOLI_MAIL_DELIVERY_METHOD=logger

cd "$WORK"
"$SOLI" new app >/dev/null 2>&1
cd app
sed -i.bak "s#^SOLIDB_HOST=.*#SOLIDB_HOST=http://127.0.0.1:$DB_PORT#; s#^SOLIDB_PASSWORD=.*#SOLIDB_PASSWORD=testpass123#" .env .env.test
"$SOLI" generate auth --passkeys >/dev/null 2>&1
"$SOLI" generate oidc_provider >/dev/null 2>&1
"$SOLI" db:migrate up >"$WORK/migrate.log" 2>&1 || { cat "$WORK/migrate.log" >&2; die "db:migrate up failed"; }
# The unique indexes are part of what is checked: the migrations must be on
# the database the app is about to use.
# (Colours are stripped with an escape every sed reads, not GNU's `\x1b`.)
"$SOLI" db:migrate status >"$WORK/status.log" 2>&1 \
  || { cat "$WORK/status.log" >&2; die "db:migrate status failed"; }
sed $'s/\033\\[[0-9;]*m//g' "$WORK/status.log" | grep "applied, 0 pending" >/dev/null \
  || { cat "$WORK/status.log" >&2; die "migrations are pending on the database the app uses"; }

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$WORK/oidc.pem" 2>/dev/null
openssl rsa -in "$WORK/oidc.pem" -pubout -out "$WORK/oidc.pub.pem" 2>/dev/null
export SOLI_OIDC_ISSUER="http://127.0.0.1:$APP_PORT"
SOLI_OIDC_PRIVATE_KEY="$(cat "$WORK/oidc.pem")"
SOLI_OIDC_PUBLIC_KEY="$(cat "$WORK/oidc.pub.pem")"
export SOLI_OIDC_PRIVATE_KEY SOLI_OIDC_PUBLIC_KEY

"$SOLI" serve . --port "$APP_PORT" --workers 4 >"$WORK/app.log" 2>&1 &
PIDS+=($!)
ready "$!" "$APP_PORT" "soli serve" "$WORK/app.log"

python3 "$HERE/auth_races_solidb.py" "$SOLI" "$WORK/app" "http://127.0.0.1:$APP_PORT"
