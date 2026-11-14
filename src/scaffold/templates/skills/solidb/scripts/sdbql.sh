# Source this file, then: sdbql 'FOR u IN users FILTER u.age > @a RETURN u' '{"a": 18}'
# Env: SOLIDB_URL (default http://localhost:6745), SOLIDB_DB (default _system),
#      SOLIDB_USER / SOLIDB_PASS (HTTP Basic auth) or SOLIDB_API_KEY.
# Needs curl and python3 (python3 only builds the JSON body safely).
sdbql() {
  local url="${SOLIDB_URL:-http://localhost:6745}" db="${SOLIDB_DB:-_system}"
  local binds="${2:-}" auth
  [ -z "$binds" ] && binds='{}'
  if [ -n "$SOLIDB_API_KEY" ]; then auth=(-H "X-API-Key: $SOLIDB_API_KEY")
  else auth=(-u "${SOLIDB_USER:-admin}:${SOLIDB_PASS}"); fi
  python3 -c 'import json,sys; print(json.dumps({"query": sys.argv[1], "bindVars": json.loads(sys.argv[2]), "cache": False}))' \
      "$1" "$binds" |
    curl -s -X POST "$url/_api/database/$db/cursor" "${auth[@]}" \
      -H 'Content-Type: application/json' --data-binary @-
  echo
}
# Any other endpoint: sdb_api GET /_api/databases ; sdb_api POST /_api/database/mydb/collection '{"name":"users"}'
sdb_api() {
  local url="${SOLIDB_URL:-http://localhost:6745}" method="$1" endpoint="$2" body="$3"
  local auth data=()
  [ -n "$body" ] && data=(--data-binary "$body")
  if [ -n "$SOLIDB_API_KEY" ]; then auth=(-H "X-API-Key: $SOLIDB_API_KEY")
  else auth=(-u "${SOLIDB_USER:-admin}:${SOLIDB_PASS}"); fi
  curl -s -X "$method" "$url$endpoint" "${auth[@]}" -H 'Content-Type: application/json' "${data[@]}"
  echo
}
