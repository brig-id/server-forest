#!/bin/sh
# Run by the postgres image when it creates the database: the role the
# nodes of forest connect as. It owns nothing; `forest init --grant
# forest_node` lets it read and write rows, never change the schema.
set -eu
psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
  -v password="$(cat /run/secrets/node_password)" <<'SQL'
CREATE ROLE forest_node LOGIN PASSWORD :'password';
SQL
