#!/usr/bin/env bash
# Project-local Postgres for development: its own cluster in dev/.pgdata on port
# 54329, so it never touches a system Postgres. Needs Homebrew postgresql@18.
#
#   dev/pg.sh start   init (first run), start, load Chinook
#   dev/pg.sh stop
#   dev/pg.sh psql    open psql on the chinook database
#   dev/pg.sh reset   stop and delete the cluster
#   dev/pg.sh load    load Chinook + perf.big into a server that's already
#                     running (CI: PGHOST/PGPORT/PGUSER, psql on PATH)
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
PGDATA="$DIR/.pgdata"
PORT=54329
export PGHOST="${PGHOST:-localhost}" PGPORT="${PGPORT:-$PORT}" PGUSER="${PGUSER:-fabio}"

brew_bin() { echo "$(brew --prefix postgresql@18)/bin"; }

# Idempotent: skips what's already there. $1 is the directory holding psql
# (empty = PATH).
load() {
  local psql="${1:+$1/}psql"
  if ! "$psql" -d postgres -tAc "select 1 from pg_database where datname='chinook'" | grep -q 1; then
    "$psql" -d postgres -q -v ON_ERROR_STOP=1 -f "$DIR/data/Chinook_PostgreSql.sql" >/dev/null
    "$psql" -d chinook -qc "create extension if not exists pg_stat_statements"
  fi
  # Tests expect track analyzed and genre not. Autovacuum gets to track only
  # eventually (a fresh CI server often hadn't), and never to genre: too small.
  "$psql" -d chinook -qc "analyze track"
  # A table big enough that counting it is slow: tests the count timeout,
  # and gives Phase 3 something worth explaining.
  if ! "$psql" -d chinook -tAc "select to_regclass('perf.big')" | grep -q big; then
    "$psql" -d chinook -q -v ON_ERROR_STOP=1 <<'SQL'
create schema perf;
create table perf.big (id bigint primary key, bucket int not null, label text not null);
insert into perf.big select g, g % 1000, md5(g::text) from generate_series(1, 5000000) g;
analyze perf.big;
SQL
  fi
  echo "postgres://$PGUSER@$PGHOST:$PGPORT/chinook"
}

# TLS with a private CA, so verify-full can be tested: dev/.pgdata/ssl/ca.crt
# signs a certificate for "localhost" only (not 127.0.0.1). Plain connections
# keep working; the server offers TLS, it doesn't require it.
ssl() {
  local dir="$PGDATA/ssl"
  if [ ! -f "$dir/server.crt" ]; then
    mkdir -p "$dir"
    openssl req -x509 -new -nodes -newkey rsa:2048 -days 800 -subj "/CN=Fabio dev CA" \
      -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
      -keyout "$dir/ca.key" -out "$dir/ca.crt" 2>/dev/null
    openssl req -new -nodes -newkey rsa:2048 -subj "/CN=localhost" \
      -keyout "$dir/server.key" -out "$dir/server.csr" 2>/dev/null
    printf 'subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature,keyEncipherment\nbasicConstraints=CA:FALSE\n' > "$dir/server.ext"
    openssl x509 -req -in "$dir/server.csr" -CA "$dir/ca.crt" -CAkey "$dir/ca.key" -CAcreateserial \
      -days 800 -extfile "$dir/server.ext" -out "$dir/server.crt" 2>/dev/null
    chmod 600 "$dir/server.key"
  fi
  if ! grep -q "^ssl = on" "$PGDATA/postgresql.conf"; then
    printf "ssl = on\nssl_cert_file = '%s'\nssl_key_file = '%s'\n" "$dir/server.crt" "$dir/server.key" >> "$PGDATA/postgresql.conf"
    "$1/pg_ctl" -D "$PGDATA" reload >/dev/null
    sleep 1
  fi
}

case "${1:-start}" in
  start)
    BIN="$(brew_bin)"
    if [ ! -d "$PGDATA" ]; then
      "$BIN/initdb" -D "$PGDATA" -U fabio --auth=trust --encoding=UTF8 >/dev/null
      echo "shared_preload_libraries = 'pg_stat_statements'" >> "$PGDATA/postgresql.conf"
    fi
    "$BIN/pg_ctl" -D "$PGDATA" -o "-p $PORT -k /tmp" -l "$PGDATA/server.log" -w status >/dev/null 2>&1 \
      || "$BIN/pg_ctl" -D "$PGDATA" -o "-p $PORT -k /tmp" -l "$PGDATA/server.log" -w start >/dev/null
    ssl "$BIN"
    load "$BIN"
    ;;
  load)  load "" ;;
  stop)  "$(brew_bin)/pg_ctl" -D "$PGDATA" -w stop ;;
  psql)  exec "$(brew_bin)/psql" -d chinook ;;
  reset) "$(brew_bin)/pg_ctl" -D "$PGDATA" -w stop 2>/dev/null || true; rm -rf "$PGDATA" ;;
  *) echo "usage: $0 start|stop|psql|reset|load" >&2; exit 1 ;;
esac
