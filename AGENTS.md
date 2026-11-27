# AGENTS.md — brig·id `server-forest`

This repository is the **largest edition** of the brig·id server: for a company, on
several nodes. The server itself is **`brigid-server` in `brig-id/core`**, the same code
seedling and grove run; this repository holds only what makes it forest.

## Language

**All content must be in English** — code, comments, docs, issues. No exceptions.

## Status

Being built. The design is in `brig-id/spec`,
[`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md): read it before writing
anything here, and change it first when the code must differ from it.

| Path | What |
| --- | --- |
| `src/main.rs` | The executable: `EDITION` (name, version, embedded interface) and `brigid_server::run_node`, with the commands of an edition made of several nodes (`init`, `import`) |
| `build.rs` | Embeds the interface (`brigid-ui-embed`, `BRIGID_UI_DIST`; a release build requires it) |
| `Dockerfile` | The image built from source (the interface from `ui/`, a checkout of brig-id/app) |
| `Dockerfile.release` | The released image: the static binaries of the release, on distroless/static |
| `scripts/check-image.sh` | With a PostgreSQL container: `init`, two nodes ready, the interface served, a clean stop |
| `deploy/` | `compose.yaml` (PostgreSQL, `init`, two nodes, a proxy), `Caddyfile`, `prepare.sh` (the secrets), `forest.env.example` |
| `docs/install.md` | The integration guide: what to provide, first start, updates, keys, coming from seedling or grove |
| `tests/edition.rs` | What forest says and refuses, without a database |
| `tests/nodes.rs`, `tests/import.rs` | Real nodes as processes on PostgreSQL (`BRIGID_TEST_POSTGRES`): a passkey sign-in spread over two nodes, a node waiting for `init`, a key rotated without restart, a single-node server imported |
| `tests/deploy/` | `run.sh`: deploys `deploy/compose.yaml`, creates the administrator in Chromium (`account.cjs`), updates node by node under requests, kills a node, crashes the database |
| `.github/workflows/ci.yml` | fmt, clippy, tests, audits; the nodes on PostgreSQL 15 and 19; the image from source and the deployment used; the static binaries (x86_64, aarch64); the bench |
| `.github/workflows/release.yml` | The release, by hand (see "Releases") |
| `bench/` | The measuring tool: its own Cargo package, never a dependency of the server |

Not there yet: the Helm chart (`chart/`), the load test of the edition, a rotation of the
master key without interruption.

## What is decided

- Several **stateless nodes**: no single point of failure in brig·id, rolling updates
  without interruption, inside the infrastructure a company already runs (its secret
  manager, its ingress, its PostgreSQL).
- **Everything shared is in PostgreSQL**: passkey ceremonies in progress, sign-in requests
  of applications and their codes, the assertions of clients already used. No cache next
  to it (`forest.md` §3, with the measurements).
- **Nodes create nothing**: `forest init` does, once, under a lock. A node that finds no
  schema, a schema not made for its version, no secret or no key is not ready and says
  why. It never repairs.
- **One store, not two.** The encrypted store of `core` runs on SQLite and on PostgreSQL
  from the same statements. Nothing that encrypts, decrypts or decides is written here:
  what a node does differently from a single-node server is in `core`
  (`brigid-server/src/nodes.rs`, `brigid-store/src/nodes.rs`).
- It is **not** a federation of servers: accounts and identities are never linked across
  servers (`model.md` §8).
- A security fix reaches the three editions: what protects an account lives in `core`
  and `crypto`, never here.
- Settings under the common prefix `BRIGID_…`; licence AGPL-3.0-or-later, like the two
  other editions. There is no commercial licence.

## Tests

`cargo nextest run` without a database runs `tests/edition.rs`; the others say that
`BRIGID_TEST_POSTGRES` is not set and pass. With a PostgreSQL that answers:

```sh
docker run -d --name forest-pg -e POSTGRES_PASSWORD=test -e POSTGRES_DB=forest -p 5432:5432 postgres:19
BRIGID_TEST_POSTGRES=postgres://postgres:test@localhost:5432/forest cargo nextest run
```

Each test makes a schema of its own in that database, and roles when it needs them. A test
of the nodes must spread its requests (`common::Spread`): what it shows is that no node
holds what another needs.

## The measuring tool

`bench/` plays the statements of `core`'s store at forest's peak against a database filled
to its tested maximum. **When a statement of the store changes what a request costs, change
it in `bench/src/main.rs` too**, or the published figures describe another server. Figures
are only published with the machine they were measured on (`bench/README.md`).

## Releases

Nothing releases on a merge, and the agent never starts a release. A release is asked for
by hand: `brigid release cut forest` from the workspace, or Actions → Release → Run
workflow (`.github/workflows/release.yml`). It plans (version and notes), builds `ci.yml`
on that commit of `main` as a release build, pushes `ghcr.io/brig-id/forest`, then
creates the GitHub Release with the static binaries, **and the tag with it, last**: a
release that fails on the way leaves no tag and uses no number. So:

- **the version is the tag.** `Cargo.toml` says `0.0.0`; the release build writes the
  version in before compiling (`scripts/set-version.sh`). There is no `CHANGELOG.md`:
  the notes are on the GitHub Release, written from the commit subjects of this
  repository **and of crypto, the core and the interface** between the commits the two
  releases follow;
- **the core, crypto and the interface are followed at a commit of their `main`**, not
  at a release (they have none): `branch = "main"` in `Cargo.toml` with the commit in
  `Cargo.lock`, and `APP_REF: <commit>` in `ci.yml`. The pull request that needs a newer
  one moves it: `cargo update -p brigid-server`, or the new `APP_REF` (a commit of the
  interface's `main`, never of a branch: the `integration` job checks). Alone, that is
  a `build(deps)` commit;
- **one pull request per change.** A change that needs the core, crypto or the interface
  too is a pull request there as well, **on a branch of the same name**: the
  `integration` job (brig-id/.github, `integration.yml`) runs this server's tests with
  those branches, on PostgreSQL, before anything is merged. Merge from crypto down to
  here; this repository's own CI stays red until the others are merged and `Cargo.lock`
  / `APP_REF` follow the commits they got on `main`;
- the commit type says whether a person running the server would notice: `feat`, `fix`,
  `perf` and `revert` are listed in the notes and are a reason for a release; `build`,
  `ci`, `chore`, `docs`, `test` and `refactor` are not, and with only those there is no
  release (unless forced). Before 1.0 a release is a patch, and a `!` or a
  `BREAKING CHANGE:` footer a minor;
- the commit subject is the line people read in the notes: write it for them.

## Commit conventions

Format: `type(scope): <emoji> description`

| Type | Emoji | When |
| --- | --- | --- |
| `feat` | ✨ | New feature |
| `fix` | 🐛 | Bug fix |
| `docs` | 📝 | Documentation only |
| `chore` | 🔧 | Maintenance, config |
| `ci` | 👷 | CI/CD |
| `revert` | ⏪ | Reverts a previous commit |

### Allowed scopes

| Scope | Maps to |
| --- | --- |
| `forest` | The server: `src/`, `build.rs`, `tests/` |
| `docker` | `Dockerfile`, `Dockerfile.release`, `deploy/`, `scripts/` |
| `bench` | `bench/` |
| `chart` | `chart/` |
| `docs` | `README.md`, `AGENTS.md`, `docs/` |
| `ci` | `.github/workflows/` |
| `deps` | Dependency bumps |
| `ai` | Instructions for coding agents |

**Do not use a scope outside this list.** A new scope is added to this table and to
`.vscode/settings.json` in the same commit.
