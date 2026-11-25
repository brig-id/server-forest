# bench — what forest's peak costs a database

forest runs several nodes on one PostgreSQL database. Its design
([`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md) §3) had one question
to answer on measurements: where does the state live that a single node keeps in memory
today (passkey ceremonies in progress, sign-in requests of applications, their codes)?

- in PostgreSQL, in an ordinary table;
- in PostgreSQL, in an unlogged table;
- in a cache next to PostgreSQL.

This tool plays the statements a request costs in `core`'s store, at forest's peak, against
a database filled to forest's tested maximum, and reports how long each request waited for
the database.

It is a measuring tool, not a part of the server, and not the load test of the edition:
no node, no passkey, no token. What a node computes is the same in every option and scales
with the number of nodes; the database does not.

## What it plays

The peak to hold is 2,900 requests per second for 1,000,000 accounts
([`editions.md`](https://github.com/brig-id/spec/blob/main/editions.md) §2). One second
of it:

| Per second | What | Requests each |
| --- | --- | --- |
| 150 | Sign-ins to an application with a passkey ceremony | 6: authorize, ceremony begin, ceremony finish, linking page, approve, token |
| 350 | Sign-ins to an application from an open session | 4: authorize, linking page, approve, token |
| 120 | Visits to the account's pages | 5 on one session |

One sign-in in ten is abandoned before the person answers: its rows stay until they
expire, and it sends fewer requests, which is why a run answers a little under 2,900.

Requests start on time whether or not the earlier ones were answered: when the database
falls behind, the wait is counted. A request's time runs from its arrival to its last
statement's answer.

`--touch` chooses what a request does to its session: `always` pushes its idle deadline
back at every request, as the store does today; `coarse` writes it only when it moves by a
minute or more.

## Running it

It needs PostgreSQL, and a cache (Valkey or Redis) for `--ephemeral cache`.

```sh
# A database with stated means, and a cache.
docker run -d --name forest-bench-pg --cpus 4 --memory 8g --shm-size 512m \
  -e POSTGRES_PASSWORD=bench -e POSTGRES_DB=forest postgres:18 \
  -c shared_buffers=1GB -c max_connections=200 \
  -c max_wal_size=8GB -c checkpoint_timeout=15min
docker run -d --name forest-bench-cache --cpus 1 --memory 1g valkey/valkey:8

# The schema of `core` as PostgreSQL holds it, then a million accounts
# (about a quarter of an hour, 8.8 GB).
docker cp schema.sql forest-bench-pg:/tmp/ && docker cp fill.sql forest-bench-pg:/tmp/
docker exec forest-bench-pg psql -q -U postgres -d forest -v ON_ERROR_STOP=1 -f /tmp/schema.sql
docker exec forest-bench-pg psql -q -U postgres -d forest -v ON_ERROR_STOP=1 \
  -v accounts=1000000 -f /tmp/fill.sql

export BENCH_DATABASE_URL=postgres://postgres:bench@<address of the database>/forest
export BENCH_CACHE_URL=redis://<address of the cache>/
cargo run --release -- --ephemeral unlogged --touch always
cargo run --release -- --help
```

A run exits with an error when a request failed or a piece of state was not found when
asked for: such a run is not a measurement.

## Files

| File | Content |
| --- | --- |
| `schema.sql` | The tables of `core` a sign-in reads or writes, as PostgreSQL holds them: the same shapes as in SQLite |
| `pending.sql` | The two tables for the state nodes share; created again before each run |
| `fill.sql` | Fills a database: `accounts` accounts, ten identities each, three applications each, three sessions for ten accounts |
| `src/main.rs` | The requests, the load, the report |

**When a statement of `core`'s store changes what a request costs, change it here too.**

## What was measured

On 2026-11-25. PostgreSQL 18.7 in a container held to 4 processors and 8 GB of memory, with
the settings of "Running it"; on a laptop (AMD Ryzen 5 6600H, NVMe disk), the tool on the
same machine, 64 connections; Valkey 8.1 without persistence. 1,000,000 accounts,
10,000,000 identities, 300,000 open sessions: 8.8 GB. Each run measures 120 seconds after
30 seconds of warm-up; three runs for each option, `--touch always`.

Time a whole sign-in waited for the database, all its requests added, in milliseconds:

| `--ephemeral` | With a passkey (6 requests): median | 95th percentile, best and worst run | From a session (4 requests): median | 95th percentile, best and worst run |
| --- | --- | --- | --- | --- |
| `logged` | 52 to 56 | 62 to 463 | 35 to 38 | 43 to 358 |
| `unlogged` | 30 to 36 | 35 to 81 | 20 to 24 | 24 to 58 |
| `cache` | 29 to 31 | 73 to 529 | 18 to 20 | 51 to 406 |

One run with the unlogged table, request by request:

```text
request                    count    p50 ms    p95 ms    p99 ms    max ms
authorize                  60000       1.0       1.9       2.2      21.7
ceremony begin             18000       0.2       0.3       0.3       2.8
ceremony finish            16234       7.8      10.4      15.9     111.1
linking page               58236       4.7       7.3       9.2     114.8
approve                    54057      11.2      14.0      21.2     118.0
token                      54063       3.7       5.3       7.3     106.0
account page               72004       3.9       5.7       7.7     106.9
sign-in, passkey (6)       16234      29.9      34.7      59.7     144.5
sign-in, session (4)       37819      20.0      23.8      39.8     134.9
errors: 0   state not found when asked: 0   log written: 687.1 MB (5724 kB/s)
pending at the end: 9689 rows, 22.1 MB on disk
```

Other runs, one each:

| Run | Result |
| --- | --- |
| `--scale 2` (5,800 requests per second) | Medians unchanged: 56 ms (`logged`), 30 ms (`unlogged`), 25 ms (`cache`) for a sign-in with a passkey; the database uses under three of its four processors |
| `--touch coarse` | Median 52 ms (`logged`) and 23 ms (`unlogged`) for a sign-in with a passkey: 4 to 7 ms saved |
| Database held to 2 processors and 4 GB | Does not hold the peak in any option: its processors are the limit, sign-ins wait seconds |
| Same, `--scale 0.5`, `unlogged` | Holds: median 31 ms, 95th percentile 35 ms for a sign-in with a passkey |

What these figures are worth:

- The medians and the order of the options are stable from run to run.
- The 95th percentiles are not: this machine's disk stalls at times, and a stall shows in
  some runs of every option. They are given as the best and the worst of three runs, not
  as a promise.
- The accounts are drawn evenly, so almost every write touches a page nobody touched
  since the last checkpoint, and the database logs the whole page: 5 to 7 MB of log per
  second at the peak. Real traffic returns to the same accounts and writes less.
- The tool and the database share one machine: the database had its processors to
  itself only as far as the container's limit holds.
- Each write that must last waits once for the disk, about 4 ms on this machine, and a
  sign-in makes several: the medians are mostly that, and are the same at a tenth of the
  load. The statements are played one by one, as `core` issues them today; grouping a
  request's writes in one transaction is not measured.

What the design concludes from them is in
[`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md) §3.
