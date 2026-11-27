# Installing forest

forest is several identical nodes on one PostgreSQL database
([`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md)). This guide says what
you provide, how the server starts, how it is updated without interruption, and how a
seedling or a grove becomes a forest.

The quickest way to see it whole is [`deploy/compose.yaml`](../deploy/compose.yaml), on one
machine: the [README](../README.md#try-it) has the four commands. What follows holds for
any platform: a node is a container that needs two files and a few settings.

## 1. What you provide

| What | Detail |
| --- | --- |
| A domain | `id.example.com`. It cannot change later: passkeys are bound to it |
| A PostgreSQL database | A version its project still supports (15 to 19 are tested), a primary that accepts writes, reached over TLS. forest does not read from standbys |
| Two database roles | One that **owns** the schema, for `forest init`. One for the **nodes**, which reads and writes rows and cannot change the schema |
| A master key | 32 random bytes in hexadecimal (`openssl rand -hex 32`), as a file mounted in every node. Keep a copy offline: without it the database cannot be read |
| Something in front | A proxy, a load balancer or an ingress that holds the certificate, spreads requests over the nodes that are ready, and **replaces** `X-Forwarded-For` |
| Synchronised clocks | Nodes compare expiry times: within a few seconds of each other |

### The database

```sql
CREATE ROLE forest_owner LOGIN PASSWORD '…';
CREATE ROLE forest_node  LOGIN PASSWORD '…';
CREATE DATABASE forest OWNER forest_owner;
```

`forest init --grant forest_node`, run as `forest_owner`, gives the nodes' role what it
needs and nothing more, each time it runs.

The address of the database is a file, as a secret manager mounts it:

```text
postgres://forest_node:<password>@db.internal:5432/forest
```

- **The connection verifies the database's certificate.** An address that names no
  `sslmode` gets `verify-full`. For a certificate authority of your own, add
  `?sslrootcert=/path/to/ca.pem`.
- An address that asks for less (`sslmode=require`, `disable`…) is refused unless
  `BRIGID_DATABASE__ALLOW_UNVERIFIED=true`: the connection carries every row. The compose
  file sets it, because its database is on the private network of the same machine.
- Each node opens at most 10 connections (`BRIGID_DATABASE__MAX_CONNECTIONS`). The
  database must allow that many per node, and a few more for the commands.
- What lives minutes is in an **unlogged** table: not written to the database's log, empty
  again after a crash or a failover, so that nothing already used can come back. On a
  service without unlogged tables: `forest init --logged-pending`.

### The settings of a node

| Variable | Value |
| --- | --- |
| `BRIGID_MASTER_KEY_FILE` | The file holding the master key |
| `BRIGID_DATABASE__URL_FILE` | The file holding the address of the database (the nodes' role; the owner's for `init`) |
| `BRIGID_SERVER__DOMAIN` | `id.example.com` |
| `BRIGID_SERVER__PUBLIC_URL` | `https://id.example.com` |
| `BRIGID_SERVER__HOST`, `BRIGID_SERVER__PORT` | `0.0.0.0`, `8080`: plain HTTP, inside your network only |
| `BRIGID_SERVER__METRICS_PORT` | `9100`: Prometheus metrics, never published |
| `BRIGID_SECURITY__TRUST_FORWARDED_FOR` | `true`: the nodes are reached through your proxy only |
| `BRIGID_SERVER__SHUTDOWN_DELAY_SECONDS` | How long a node told to stop keeps answering, not ready (default 5): at least the time your proxy takes to notice |

The image runs as an unprivileged user (65532), has no shell, and writes nothing to disk:
its root file system can be read-only.

### What stands in front

- **TLS** ends there. A node has no certificate and obtains none.
- **Readiness**: send requests only to the nodes whose `GET /ready` answers 200. A node told
  to stop answers 503 at once and closes a few seconds later; a node whose database does
  not answer, or whose schema is not the one its version serves on, answers 503 with the
  reason. `GET /health` only says that the process runs: use it to restart a node, never
  to route to it.
- **`X-Forwarded-For` replaced**, never appended to: the nodes key their rate limits on it.
- **A global rate limit.** Each node limits each address in its own memory: behind
  something that spreads requests, an address can reach each node's limit. The limit that
  counts is yours, in front.

[`deploy/Caddyfile`](../deploy/Caddyfile) is all of this in a dozen lines.

## 2. First start

```sh
forest init --grant forest_node     # once, as the role that owns the schema
```

It creates the schema, the secret of the identifiers and the first signing key, and prints
the **setup link** of the first administrator on standard output. Then start the nodes.

- Run it as a job that ends before the nodes start. Two at once is harmless: one waits for
  the other. A second run changes nothing.
- **A node creates nothing.** Started before `init`, it waits a minute for it, saying what
  is missing, then ends.
- The setup link works once, for an hour. `forest admin link`, in any node, makes a new
  one.

The other commands are those of every edition, run in a node (`forest signup`, `forest
client …`, `forest limits`, `forest admin link`, `forest rotate-oidc-key`). `backup` and
`restore` are for a SQLite file: forest refuses them and names PostgreSQL's own tools.

## 3. Updates, without interruption

1. `forest init` of the new version (the same command, the new image).
2. Replace the nodes one by one, waiting for each new one to be ready.

For a while nodes of two versions answer on one database. A release whose schema change
does not allow that says so in its notes; the nodes of the earlier version then stop being
ready as soon as `init` has run, instead of writing wrongly.

With the compose file:

```sh
compose="docker compose -f deploy/compose.yaml --env-file deploy/forest.env"
$compose pull
$compose run --rm --no-deps init
$compose up -d --no-deps forest-1     # wait for it to be healthy
$compose up -d --no-deps forest-2
```

## 4. Keys

- **Signing keys**: `forest rotate-oidc-key`, in any node. No restart: every node publishes
  the new key within a minute and signs with it five minutes after its creation. The key
  before keeps verifying tokens for two hours.
- **The master key**: `forest rotate-key --old <file> --new <file>` re-encrypts every row.
  Stop every node first: it is a maintenance window.

## 5. What fails, and what happens

| Failure | Effect |
| --- | --- |
| A node stops or crashes | Its requests in flight fail and are sent again by the browser or the site; ceremonies and sign-ins in progress continue on another node |
| The database does not answer | No node is ready: nobody signs in until it answers. Sessions survive |
| The database crashes or fails over | Sign-ins in progress start again (the unlogged table comes back empty). Nothing else is lost |
| The database is restored from a backup | Sessions opened and passkeys added since the backup are gone, as in any edition |

Back the database up with PostgreSQL's own tools, and the master key apart from it.

## 6. What to watch

- `GET /ready` of each node, and why it refuses when it does.
- `GET /metrics` on port 9100 of each node: requests and their latency, per route.
- The database: connections, the size of `pending` (it holds minutes of sign-ins: a table
  that grows is a flood), replication lag if you keep a standby.

## Coming from seedling or grove

Under the **same domain** and the **same master key**, a backup of a seedling or a grove
is imported into a forest that has no account yet. Passkeys, sessions, the identifiers
sites hold and the tokens already issued stay valid; sign-ins in progress start again.

1. On the server you leave: `seedling backup /somewhere/backup.db` (or `grove backup`).
2. Prepare forest with the same domain and the same master key, and run `forest init`.
3. `forest import /somewhere/backup.db`, as the role that owns the schema. One
   transaction: an import that fails changes nothing. A backup made by an earlier version
   is brought up to date on a private copy, in `/tmp`.
4. Stop the old server, start the nodes, and point the domain at what stands in front of
   them. Two servers must never answer for one domain.

With the compose file: put the old server's key in `deploy/secrets/master_key` **before**
`deploy/prepare.sh`, which keeps it; give the backup to the user the image runs as; then:

```sh
compose="docker compose -f deploy/compose.yaml --env-file deploy/forest.env"
sudo chown 65532 /somewhere/backup.db
$compose up -d postgres
$compose run --rm init
$compose run --rm -v /somewhere/backup.db:/backup.db:ro init import /backup.db
$compose up -d
```

A server imported before its first account has no administrator: the next `forest init`
prints a new setup link, and so does `forest admin link`.

The certificate a seedling obtained for itself is not imported: the proxy holds forest's.
To change the master key, rotate it after the import.
