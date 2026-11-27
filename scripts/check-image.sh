#!/usr/bin/env bash
# Checks an image of forest as a platform team would find it: with a
# PostgreSQL database, a key and a domain, `init` prepares the database and
# prints the setup link, two nodes become ready on it, each serves the
# interface from the binary under a policy made of script hashes, and a node
# told to stop ends cleanly. Each check says what it found when it fails.
#
#   scripts/check-image.sh <image>
#
# Needs Docker and curl, and to reach the addresses of containers (Linux).
set -euo pipefail

image="$1"
name="forest-check-$$"
key=0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0

cleanup() {
  docker rm -f "$name-pg" "$name-1" "$name-2" > /dev/null 2>&1 || true
}
trap cleanup EXIT
address_of() { docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$1"; }

# Building is not enough: a missing library only shows when the binary starts.
docker run --rm "$image" --help > /dev/null
test "$(docker run --rm "$image" --version | cut -d' ' -f1)" = forest

docker run -d --name "$name-pg" -e POSTGRES_PASSWORD=check -e POSTGRES_DB=forest \
  postgres:18@sha256:64c8b0d31e4ab6ea665e8426ea406a94bf52de3c05b3a4a8bf713704eefa9328 > /dev/null
# Over the network, not the local socket: while the image creates the
# database it runs a first server that listens on the socket only.
for _ in $(seq 1 60); do
  docker exec "$name-pg" pg_isready -q -h 127.0.0.1 -U postgres -d forest && break
  sleep 1
done
docker exec "$name-pg" pg_isready -q -h 127.0.0.1 -U postgres -d forest

settings=(
  -e "BRIGID_MASTER_KEY=$key"
  -e BRIGID_SERVER__DOMAIN=localhost
  -e BRIGID_SERVER__HOST=0.0.0.0
  -e BRIGID_SERVER__PUBLIC_URL=http://localhost:8080
  # A database without TLS, on the machine's own bridge: written here, as
  # an operator must write it.
  -e "BRIGID_DATABASE__URL=postgres://postgres:check@$(address_of "$name-pg"):5432/forest?sslmode=disable"
  -e BRIGID_DATABASE__ALLOW_UNVERIFIED=true
)

dir="$(mktemp -d)"
docker run --rm "${settings[@]}" "$image" init > "$dir/link" 2> "$dir/init"
grep -q '^http://localhost:8080/setup/#link=' "$dir/link" || { cat "$dir/init" "$dir/link"; echo "no setup link"; exit 1; }
# A second initialisation changes nothing and succeeds.
docker run --rm "${settings[@]}" "$image" init > /dev/null 2>&1

for n in 1 2; do
  docker run -d --name "$name-$n" --read-only "${settings[@]}" "$image" > /dev/null
done
for n in 1 2; do
  node="http://$(address_of "$name-$n"):8080"
  for _ in $(seq 1 30); do
    curl -fsS "$node/ready" > /dev/null 2>&1 && break
    sleep 1
  done
  curl -fsS "$node/ready" > /dev/null || { docker logs "$name-$n"; exit 1; }
  curl -fsS "$node/auth/server" | grep -q '"edition":"forest"' || { curl -s "$node/auth/server"; exit 1; }

  curl -fsS -D "$dir/headers" -o "$dir/page" "$node/signup/"
  grep -qi "^content-type: text/html" "$dir/headers" || { cat "$dir/headers"; exit 1; }
  grep -qi "^content-security-policy: .*script-src 'self' 'sha256-" "$dir/headers" || { cat "$dir/headers"; exit 1; }
  if grep -qi "unsafe-inline" "$dir/headers"; then cat "$dir/headers"; exit 1; fi
  # (sed reads the whole page: `grep | head` would end with a broken pipe.)
  script=$(sed -n 's/.*\(build\/q-[A-Za-z0-9_-]*\.js\).*/\1/p' "$dir/page" | sed -n 1p)
  test -n "$script" || { echo "no script named in the page"; exit 1; }
  curl -fsS -o /dev/null "$node/$script"
  unknown=$(curl -s -o /dev/null -w '%{http_code}' "$node/nothing/")
  test "$unknown" = 404 || { echo "an unknown path answered $unknown"; exit 1; }
done

# Both nodes publish the one key `init` made.
one=$(curl -fsS "http://$(address_of "$name-1"):8080/.well-known/jwks.json")
two=$(curl -fsS "http://$(address_of "$name-2"):8080/.well-known/jwks.json")
test "$one" = "$two" || { echo "the nodes publish different keys"; exit 1; }

# The image's own health check, and a clean stop.
docker exec "$name-1" /forest healthcheck
docker stop -t 30 "$name-1" > /dev/null
status=$(docker inspect -f '{{.State.ExitCode}}' "$name-1")
test "$status" = 0 || { docker logs "$name-1"; echo "a node told to stop ended with $status"; exit 1; }
curl -fsS "http://$(address_of "$name-2"):8080/ready" > /dev/null

echo "✓ $image: init, two nodes ready on one database, the interface, a clean stop"
