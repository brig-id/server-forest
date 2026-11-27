#!/usr/bin/env bash
# Writes the secrets deploy/compose.yaml mounts: the master key, the
# passwords of the two database roles, and the two addresses that hold them.
# A file that is already there is kept: to move a seedling or a grove here,
# put its master key in deploy/secrets/master_key first. Needs openssl, and
# sudo to give each file to the user that reads it in its container.
#
#   deploy/prepare.sh
#
# Keep a copy of deploy/secrets/master_key away from this machine: without
# it the database cannot be read.
set -euo pipefail
cd "$(dirname "$0")"

if sudo test -e secrets/database_url_node; then
  echo "deploy/secrets/database_url_node exists: the secrets are already there, nothing was changed" >&2
  exit 1
fi

mkdir -p secrets
(
  umask 077
  if sudo test -e secrets/master_key; then
    echo "deploy/secrets/master_key is kept as it is"
  else
    openssl rand -hex 32 > secrets/master_key
  fi
  openssl rand -hex 24 > secrets/owner_password
  openssl rand -hex 24 > secrets/node_password
  address() { printf 'postgres://%s:%s@postgres:5432/forest?sslmode=disable\n' "$1" "$(cat "secrets/$2")"; }
  address forest_owner owner_password > secrets/database_url_owner
  address forest_node node_password > secrets/database_url_node
)
# forest runs as 65532 (nonroot), postgres as 999: each reads its own files.
sudo chown 65532:65532 secrets/master_key secrets/database_url_owner secrets/database_url_node
sudo chmod 0400 secrets/master_key
sudo chown 999:999 secrets/owner_password secrets/node_password
ls -l secrets
echo "✓ The secrets are in deploy/secrets/. Keep a copy of master_key elsewhere."
