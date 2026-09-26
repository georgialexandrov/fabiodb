#!/usr/bin/env bash
# Project-local Postgres for development: its own cluster in dev/.pgdata on port
# 54329, so it never touches a system Postgres. Needs Homebrew postgresql@18.
#
#   dev/pg.sh start   init (first run), start, load Chinook
#   dev/pg.sh stop
#   dev/pg.sh psql    open psql on the chinook database
#   dev/pg.sh reset   stop and delete the cluster
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
PGDATA="$DIR/.pgdata"
PORT=54329
BIN="$(brew --prefix postgresql@18)/bin"
export PGHOST=localhost PGPORT=$PORT PGUSER=fabio

case "${1:-start}" in
  start)
    if [ ! -d "$PGDATA" ]; then
      "$BIN/initdb" -D "$PGDATA" -U fabio --auth=trust --encoding=UTF8 >/dev/null
      echo "shared_preload_libraries = 'pg_stat_statements'" >> "$PGDATA/postgresql.conf"
    fi
    "$BIN/pg_ctl" -D "$PGDATA" -o "-p $PORT -k /tmp" -l "$PGDATA/server.log" -w status >/dev/null 2>&1 \
      || "$BIN/pg_ctl" -D "$PGDATA" -o "-p $PORT -k /tmp" -l "$PGDATA/server.log" -w start >/dev/null
    if ! "$BIN/psql" -d postgres -tAc "select 1 from pg_database where datname='chinook'" | grep -q 1; then
      "$BIN/psql" -d postgres -q -v ON_ERROR_STOP=1 -f "$DIR/data/Chinook_PostgreSql.sql" >/dev/null
      "$BIN/psql" -d chinook -qc "create extension if not exists pg_stat_statements"
    fi
    echo "postgres://fabio@localhost:$PORT/chinook"
    ;;
  stop)  "$BIN/pg_ctl" -D "$PGDATA" -w stop ;;
  psql)  exec "$BIN/psql" -d chinook ;;
  reset) "$BIN/pg_ctl" -D "$PGDATA" -w stop 2>/dev/null || true; rm -rf "$PGDATA" ;;
  *) echo "usage: $0 start|stop|psql|reset" >&2; exit 1 ;;
esac
