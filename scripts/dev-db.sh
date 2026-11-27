#!/usr/bin/env bash
# Start local PostgreSQL and MySQL for development and tests (Linux).
# PostgreSQL: 127.0.0.1:55432 user=mt password=mt  |  MySQL: 127.0.0.1:53306 user=root (no password)
set -euo pipefail
ROOT="${MT_DEV_DB_DIR:-/var/tmp/mt-dev-db}"
PGBIN="$(ls -d /usr/lib/postgresql/*/bin | sort -V | tail -1)"
mkdir -p "$ROOT"
if ! id postgres >/dev/null 2>&1; then useradd -m postgres; fi
# Each server owns only its own directory (re-runs must not take MySQL's data away).
mkdir -p "$ROOT/pg" && chown -R postgres "$ROOT/pg" 2>/dev/null || true
if [ -d "$ROOT/mysql" ]; then chown -R mysql "$ROOT/mysql" 2>/dev/null || true; fi
if [ ! -d "$ROOT/pg/base" ]; then
  mkdir -p "$ROOT/pg" && chown postgres "$ROOT/pg"
  su postgres -c "$PGBIN/initdb -D $ROOT/pg -U mt --auth=trust >/dev/null"
fi
if [ ! -f "$ROOT/pg.log" ]; then touch "$ROOT/pg.log"; fi
chown postgres "$ROOT/pg.log"
su postgres -c "$PGBIN/pg_ctl -D $ROOT/pg -o '-p 55432 -c listen_addresses=127.0.0.1 -c max_connections=300' -l $ROOT/pg.log status >/dev/null 2>&1" \
  || su postgres -c "$PGBIN/pg_ctl -D $ROOT/pg -o '-p 55432 -c listen_addresses=127.0.0.1 -c max_connections=300' -l $ROOT/pg.log -w start >/dev/null"
if [ ! -d "$ROOT/mysql/mysql" ]; then
  mkdir -p "$ROOT/mysql" && chown mysql "$ROOT/mysql"
  mysqld --initialize-insecure --user=mysql --datadir="$ROOT/mysql" >/dev/null 2>&1
fi
if ! mysqladmin --protocol=tcp -h127.0.0.1 -P53306 -uroot ping >/dev/null 2>&1; then
  mkdir -p "$ROOT/mysql-run" && rm -f "$ROOT/mysql-run/"* && chown -R mysql "$ROOT/mysql-run"
  (setsid nohup mysqld --user=mysql --datadir="$ROOT/mysql" --port=53306 --bind-address=127.0.0.1 --mysqlx=OFF \
     --socket="$ROOT/mysql-run/mysqld.sock" --pid-file="$ROOT/mysql-run/mysqld.pid" \
     --max-connections=300 >"$ROOT/mysql.log" 2>&1 &)
  for _ in $(seq 1 60); do mysqladmin --protocol=tcp -h127.0.0.1 -P53306 -uroot ping >/dev/null 2>&1 && break; sleep 1; done
fi
echo "MT_TEST_POSTGRES_URL=postgres://mt@127.0.0.1:55432/postgres"
echo "MT_TEST_MYSQL_URL=mysql://root@127.0.0.1:53306/mysql"
