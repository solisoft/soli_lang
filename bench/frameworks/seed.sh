#!/usr/bin/env bash
# Create and seed both datasets in both databases:
#   posts   50 rows      — the read workloads
#   wposts  800,000 rows — the write workloads (reset before every write cell)
#
# PostgreSQL serves Rails, Express, Laravel and Django; SoliDB serves Soli.
set -u
PGURL="${PGURL:-postgres://bench:bench@127.0.0.1:5433/bench}"
SDB="${SDB:-http://localhost:6745/_api/database/default}"
SDB_AUTH="${SDB_AUTH:-admin:admin}"

pg_reset_reads() {
  psql "$PGURL" -qc "DROP TABLE IF EXISTS posts;" \
    -c "CREATE TABLE posts (id serial PRIMARY KEY, title text, views int);" \
    -c "INSERT INTO posts (id,title,views) SELECT g,'Post title '||g,g*7 FROM generate_series(1,50) g;" >/dev/null
}
pg_reset_writes() {
  psql "$PGURL" -qc "DROP TABLE IF EXISTS wposts;" \
    -c "CREATE TABLE wposts (id serial PRIMARY KEY, title text, views int);" \
    -c "INSERT INTO wposts (id,title,views) SELECT g,'Post title '||g,g*7 FROM generate_series(1,800000) g;" \
    -c "SELECT setval(pg_get_serial_sequence('wposts','id'),900000);" >/dev/null
}
sdb_reset() {  # $1 = collection, $2 = row count
  curl -s -u "$SDB_AUTH" -X DELETE "$SDB/collection/$1" >/dev/null
  curl -s -u "$SDB_AUTH" -X POST "$SDB/collection" -H 'Content-Type: application/json' \
       -d "{\"name\":\"$1\"}" >/dev/null
  local batch=100000 lo hi
  for (( lo=1; lo<=$2; lo+=batch )); do
    hi=$(( lo + batch - 1 )); [ "$hi" -gt "$2" ] && hi=$2
    # Generated in the database, the way generate_series does for PostgreSQL —
    # inserting 800k documents over HTTP one at a time would take minutes.
    curl -s -u "$SDB_AUTH" -X POST "$SDB/cursor" -H 'Content-Type: application/json' \
      -d "{\"query\":\"FOR i IN $lo..$hi INSERT { _key: TO_STRING(i), title: CONCAT(\\\"Post title \\\", i), views: i * 7 } INTO $1 RETURN 1\"}" \
      -o /dev/null
  done
}

case "${1:-all}" in
  reads)  pg_reset_reads; sdb_reset posts 50 ;;
  writes) pg_reset_writes; sdb_reset wposts 800000 ;;
  all)    pg_reset_reads; sdb_reset posts 50; pg_reset_writes; sdb_reset wposts 800000 ;;
  *) echo "usage: $0 [reads|writes|all]"; exit 1 ;;
esac
echo "seeded: $(psql "$PGURL" -tAc 'SELECT count(*) FROM posts')/50 posts, $(psql "$PGURL" -tAc 'SELECT count(*) FROM wposts') wposts (PostgreSQL)"
