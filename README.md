# server-forest

**forest** is the largest edition of the [brig·id](https://brig.id) server: for a company,
where sign-in must not stop. Several nodes that stand in for one another, on a PostgreSQL
database the company provides.

Its design is in the specification:
[`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md). What the three editions
are, and what separates them, is in
[`editions.md`](https://github.com/brig-id/spec/blob/main/editions.md) §2. It is being
built: what is not there yet is said [below](#what-is-not-there-yet).

## What forest is

- **The same engine as seedling and grove**, with the same functions, administration
  included: the same routes, the same passkey ceremonies, the same tokens, the same
  interface, the same operator commands. A site connected to a seedling or a grove works
  with a forest without a change. What an account, an identity, a passkey and a token are
  lives in [`core`](https://github.com/brig-id/core) and
  [`crypto`](https://github.com/brig-id/crypto): a security fix reaches every edition.
- **Nodes that hold nothing.** Any node answers any request. A passkey ceremony begun on
  one node is finished on another; a code issued by one is exchanged at another, once. A
  node can stop in the middle of a sign-in without that sign-in being lost, and an update
  replaces the nodes one by one.
- **Everything in PostgreSQL, and nothing else.** What lasts, and what the nodes share for
  a few minutes (ceremonies in progress, sign-in requests, codes), is in one database: no
  cache, no queue, no second service to run. What lives minutes is in an unlogged table,
  encrypted, under hashed identifiers.
- **Nodes create nothing.** `forest init` prepares the database once, before the nodes and
  before each update: the schema, the secret of the identifiers, the first signing key,
  the setup link of the first administrator. It connects as the role that owns the
  schema; the nodes connect as another, which cannot change it.
- **No restart for a new signing key.** Every node reads the keys again every minute,
  publishes a new one at once and signs with it five minutes later.
- **Probes.** `/health` answers while the process runs. `/ready` answers 200 when the
  database answers on a schema made for this version, and 503 with the reason otherwise,
  and from the moment the node is told to stop: it then finishes the requests it has.
  Prometheus metrics are at `/metrics` on port 9100, per node.
- **It is not a federation**: one server is one domain and one database. Accounts and
  identities are never linked across servers.

## Try it

[`deploy/compose.yaml`](deploy/compose.yaml) runs forest whole on one machine: a
PostgreSQL database, the initialisation, two nodes, and a proxy that holds the certificate
and spreads the requests. The image of a release is `ghcr.io/brig-id/forest:vX.Y.Z`; to
run the code of a branch instead, build it:

```sh
git clone https://github.com/brig-id/app ui     # the interface, embedded at build time
docker build -t brigid/forest:dev .
```

Then, in a checkout of this repository:

```sh
cp deploy/forest.env.example deploy/forest.env   # the domain, the image
deploy/prepare.sh                                # the secrets, once
docker compose -f deploy/compose.yaml --env-file deploy/forest.env up -d
docker compose -f deploy/compose.yaml --env-file deploy/forest.env logs init
```

The last command shows the setup link of the first administrator. The operator's commands
run in any node:

```sh
docker compose -f deploy/compose.yaml --env-file deploy/forest.env exec forest-1 /forest admin link
```

Keep a copy of `deploy/secrets/master_key` away from the machine: without it the database
cannot be read.

[`docs/install.md`](docs/install.md) says the rest: your own database and its two roles,
what must stand in front of the nodes, updates without interruption, and what to watch.

## Coming from seedling or grove

A seedling or a grove becomes a forest by a backup and an import: under the same domain
and the same master key, nothing changes for the people and the sites. Passkeys, sessions,
the identifiers sites hold and the tokens already issued stay valid.

```sh
grove backup /somewhere/backup.db      # on the server you leave
forest init                            # on the new one, then:
forest import /somewhere/backup.db
```

The steps, with the compose file, are in
[`docs/install.md`](docs/install.md#coming-from-seedling-or-grove).

## What is not there yet

- **A Helm chart.** The design delivers forest to a cluster with a chart; today the
  repository delivers the image and the compose file. A node is a container that needs a
  database address and a key, both as mounted files: nothing in it depends on Compose.
- **The load test of the edition** (`forest.md` §10): real nodes at the tested maximum for
  an hour. The measurements behind the design, of the database alone, are in
  [`bench/`](bench/README.md).
- **A rotation of the master key without interruption** (`forest.md` §6, open): today it
  is a maintenance window, every node stopped, as in the other editions.
- **Recommended sizes** for the nodes and the database, which come from that load test.

## What is here

| Path | Content |
| --- | --- |
| `src/` | The executable: what makes forest an edition of `brigid-server` |
| `Dockerfile`, `Dockerfile.release` | The image from source, and the released one (a static binary on distroless) |
| [`deploy/`](deploy/compose.yaml) | The compose file, the proxy's configuration, the script that writes the secrets |
| [`docs/install.md`](docs/install.md) | The integration guide |
| `tests/` | Real nodes on a real database; the deployment used in a real browser |
| [`bench/`](bench/README.md) | The measuring tool behind the design |

## Licence

[AGPL-3.0-or-later](LICENSE), like the two other editions. There is no commercial licence:
what a company can pay for is support, hosting, and modules that are separate programs.
