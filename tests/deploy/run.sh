#!/usr/bin/env bash
# forest deployed from deploy/compose.yaml and used as a company would:
# initialised, its first administrator created in a real browser with a
# passkey, then updated node by node, a node lost, and its database
# restarted, while requests keep coming through the proxy. At each step the
# passkey still signs in and the server shows the same account. Then a
# released grove is given a site, backed up, and imported into a new forest,
# as docs/install.md says.
#
#   tests/deploy/run.sh
#
# Run from a checkout with no deploy/forest.env and no deploy/secrets/: the
# test writes both, as an operator would, with secrets of its own. It needs
# sudo (deploy/prepare.sh gives the secrets to the services' users), Docker
# with Compose, curl, openssl, the image FOREST_IMAGE (by default
# brigid/forest:dev, `docker build -t brigid/forest:dev .`), ports 80 and 443
# free, and Playwright with Chromium (PLAYWRIGHT_MODULE: see
# tests/deploy/account.cjs). CI runs it: .github/workflows/ci.yml.
set -euo pipefail
cd "$(dirname "$0")/../.."

if [ -e deploy/forest.env ] || [ -e deploy/secrets ]; then
  echo "deploy/forest.env or deploy/secrets/ exists: run this test in a clean checkout" >&2
  exit 2
fi

DOMAIN=brigid.localhost
ORIGIN="https://$DOMAIN"
work=deploy/secrets/deploy-test
tls=deploy/secrets/tls

# The operator's commands are `docker compose -f deploy/compose.yaml
# --env-file deploy/forest.env …`; the test adds its certificate.
compose() {
  docker compose -f deploy/compose.yaml -f tests/deploy/compose.yaml \
    --env-file deploy/forest.env "$@"
}
forest() { compose exec -T "$1" /forest "${@:2}"; }
web() { curl -fsS --max-time 10 --cacert "$tls/ca.pem" --resolve "$DOMAIN:443:127.0.0.1" "$@"; }
step() { printf '\n== %s\n' "$*"; }

watcher=""
on_exit() {
  status=$?
  [ -n "$watcher" ] && kill "$watcher" 2> /dev/null || true
  if [ "$status" -ne 0 ]; then
    step "failed ($status): the services and their logs"
    compose ps -a || true
    compose logs --no-color --tail 200 || true
  fi
  compose down --volumes --remove-orphans > /dev/null 2>&1 || true
  docker rm -f forest-test-grove > /dev/null 2>&1 || true
  sudo rm -rf deploy/secrets deploy/forest.env
}
trap on_exit EXIT

wait_for() { # <what> <command…>: up to two minutes
  local what="$1"
  shift
  for _ in $(seq 1 120); do
    if "$@" > /dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "✗ $what: not after two minutes" >&2
  return 1
}
healthy() { [ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q "$1")")" = healthy ]; }
ready_through_proxy() { web -o /dev/null "$ORIGIN/ready"; }

# -- The deployment, as the top of deploy/compose.yaml says ------------------

step "forest.env and the secrets"
sed -e "s|^BRIGID_DOMAIN=.*|BRIGID_DOMAIN=$DOMAIN|" \
  -e "s|^FOREST_IMAGE=.*|FOREST_IMAGE=${FOREST_IMAGE:-brigid/forest:dev}|" \
  deploy/forest.env.example > deploy/forest.env
cat deploy/forest.env
deploy/prepare.sh
# A second run writes over nothing.
if deploy/prepare.sh > /dev/null 2>&1; then
  echo "✗ prepare.sh ran twice"
  exit 1
fi

step "a certificate authority for the test, and the certificate of the proxy"
mkdir -p "$tls/$DOMAIN" "$work"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
  -subj "/CN=forest deployment test" -keyout "$tls/ca-key.pem" -out "$tls/ca.pem" 2> /dev/null
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=$DOMAIN" \
  -keyout "$tls/$DOMAIN/key.pem" -out "$tls/$DOMAIN.csr" 2> /dev/null
openssl x509 -req -in "$tls/$DOMAIN.csr" -CA "$tls/ca.pem" -CAkey "$tls/ca-key.pem" \
  -CAcreateserial -days 2 -out "$tls/$DOMAIN/cert.pem" \
  -extfile <(printf 'subjectAltName=DNS:%s\nextendedKeyUsage=serverAuth\n' "$DOMAIN") 2> /dev/null
# Read by the proxy's own user: test keys, thrown away at the end.
chmod -R a+rX "$tls"
TEST_SPKI=$(openssl x509 -in "$tls/$DOMAIN/cert.pem" -pubkey -noout |
  openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | base64)
export TEST_SPKI
if [ "${CI:-}" = true ] && command -v certutil > /dev/null; then
  # The runner's browser trusts the test's authority (never on a machine of
  # one's own: this is the user's certificate store).
  mkdir -p "$HOME/.pki/nssdb"
  [ -e "$HOME/.pki/nssdb/cert9.db" ] || certutil -d "sql:$HOME/.pki/nssdb" -N --empty-password
  certutil -d "sql:$HOME/.pki/nssdb" -A -t "C,," -n "forest deployment test" -i "$tls/ca.pem"
fi

step "the database, the initialisation, two nodes and the proxy"
compose up -d
wait_for "forest through the proxy" ready_through_proxy
compose ps -a
test "$(docker inspect -f '{{.State.ExitCode}}' "$(compose ps -a -q init)")" = 0
web "$ORIGIN/auth/server" | tee "$work/server.json"
grep -q '"edition":"forest"' "$work/server.json"
echo

step "the nodes connect as a role that cannot change the schema"
compose exec -T postgres psql -v ON_ERROR_STOP=1 -U forest_owner -d forest -Atc \
  "SELECT relpersistence FROM pg_class WHERE relname = 'pending'" | tee "$work/pending"
test "$(cat "$work/pending")" = u
node_url=$(sudo cat deploy/secrets/database_url_node)
if compose exec -T postgres psql "${node_url%\?*}" -c 'CREATE TABLE more (id TEXT)' > "$work/ddl" 2>&1; then
  cat "$work/ddl"
  echo "✗ the role of the nodes created a table"
  exit 1
fi
echo "✓ refused: $(grep -m1 ERROR "$work/ddl")"

# -- A real person -------------------------------------------------------------

step "the first administrator, from the setup link \`forest init\` printed"
link=$(compose logs --no-color init | grep -o "$ORIGIN/setup/#link=[^[:space:]\"]*" | tail -n 1)
test -n "$link"
node tests/deploy/account.cjs create "$link" "$work/account.json"

step "the operator's commands, in a node"
forest forest-1 signup open
forest forest-2 client add --name "Deployment test" --redirect-uri https://app.example.org/callback
test "$(forest forest-1 signup)" = open
forest forest-1 client list | grep -q "Deployment test"
before_keys=$(web "$ORIGIN/.well-known/jwks.json")

# -- An update, node by node, while requests keep coming ------------------------

step "requests through the proxy, ten a second, until the end of the update"
: > "$work/requests"
(
  while true; do
    if web -o /dev/null "$ORIGIN/.well-known/openid-configuration" 2> /dev/null; then
      echo ok >> "$work/requests"
    else
      echo "failed at $(date +%T)" >> "$work/requests"
    fi
    sleep 0.1
  done
) &
watcher=$!

step "the update: its initialisation first, then one node after the other"
compose run --rm --no-deps init
for node in forest-1 forest-2; do
  compose up -d --no-deps --force-recreate "$node"
  wait_for "$node healthy again" healthy "$node"
  # The proxy has seen it ready before the next one goes.
  sleep 3
done
kill "$watcher"
wait "$watcher" 2> /dev/null || true
watcher=""
answered=$(grep -c '^ok$' "$work/requests" || true)
failed=$(grep -c '^failed' "$work/requests" || true)
echo "$answered requests answered, $failed failed"
if [ "$failed" -ne 0 ] || [ "$answered" -lt 50 ]; then
  cat "$work/requests" | grep failed | head -n 20
  echo "✗ the update interrupted the server"
  exit 1
fi
echo "✓ both nodes replaced, and no request failed"
node tests/deploy/account.cjs check "$work/account.json"
test "$(web "$ORIGIN/.well-known/jwks.json")" = "$before_keys"

# -- A node is lost -------------------------------------------------------------

step "a node dies (SIGKILL): the other answers alone"
compose kill -s KILL forest-1
sleep 3
node tests/deploy/account.cjs check "$work/account.json"
compose up -d --no-deps forest-1
wait_for "forest-1 healthy again" healthy forest-1

# -- The database crashes -------------------------------------------------------

step "the database crashes and comes back: nobody signs in meanwhile, nothing is lost"
compose kill -s KILL postgres
unready=false
for _ in $(seq 1 20); do
  if ! ready_through_proxy 2> /dev/null; then
    unready=true
    break
  fi
  sleep 0.5
done
if [ "$unready" != true ]; then
  echo "✗ the nodes stayed ready without their database"
  exit 1
fi
echo "✓ no node is ready without the database"
compose up -d --no-deps postgres
wait_for "forest through the proxy" ready_through_proxy
node tests/deploy/account.cjs check "$work/account.json"
forest forest-2 client list | grep -q "Deployment test"

# -- A grove becomes a forest (docs/install.md) ---------------------------------

step "another server: the released grove, with a site registered on it, and its backup"
compose down --volumes --remove-orphans
GROVE_IMAGE="${GROVE_IMAGE:-ghcr.io/brig-id/grove:latest}"
docker rm -f forest-test-grove > /dev/null 2>&1 || true
# The same domain and the same master key as the forest it becomes.
docker run -d --name forest-test-grove \
  -e "BRIGID_MASTER_KEY=$(sudo cat deploy/secrets/master_key)" \
  -e "BRIGID_SERVER__DOMAIN=$DOMAIN" -e "BRIGID_SERVER__PUBLIC_URL=$ORIGIN" \
  "$GROVE_IMAGE" > /dev/null
grove_address=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' forest-test-grove)
wait_for "grove" curl -fsS -o /dev/null "http://$grove_address:8080/ready"
docker run --rm "$GROVE_IMAGE" --version
docker exec forest-test-grove /grove signup open
docker exec forest-test-grove /grove client add --name "From grove" \
  --redirect-uri https://app.example.org/callback > /dev/null
docker exec forest-test-grove /grove client list | sort > "$work/grove-clients"
curl -fsS "http://$grove_address:8080/.well-known/jwks.json" > "$work/grove-keys.json"
docker exec forest-test-grove /grove backup /data/backup.db
docker cp forest-test-grove:/data/backup.db "$work/backup.db"
docker rm -f forest-test-grove > /dev/null
# Read by the user forest runs as.
sudo chown 65532:65532 "$work/backup.db"

step "the forest: its database, its initialisation, the import, then the nodes"
compose up -d postgres
compose run --rm init > /dev/null
compose run --rm -v "$PWD/$work/backup.db:/backup.db:ro" init import /backup.db
compose up -d
wait_for "forest through the proxy" ready_through_proxy
forest forest-1 client list | sort > "$work/forest-clients"
if ! cmp -s "$work/grove-clients" "$work/forest-clients"; then
  diff "$work/grove-clients" "$work/forest-clients" || true
  echo "✗ the sites are not the ones grove had"
  exit 1
fi
test "$(forest forest-2 signup)" = open
if ! web "$ORIGIN/.well-known/jwks.json" | cmp -s - "$work/grove-keys.json"; then
  echo "✗ the signing keys are not the ones grove published"
  exit 1
fi
echo "✓ the same sites, sign-up policy and signing keys as the grove it was"

echo
echo "✓ forest deployed, updated node by node without a failed request, the same after a lost node and a database crash, and filled from a grove"
