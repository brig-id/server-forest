# AGENTS.md — brig·id `server-forest`

This repository is the **largest edition** of the brig·id server: for a company, on
several nodes.

## Language

**All content must be in English** — code, comments, docs, issues. No exceptions.

## Status

Designed, not built. The design is in `brig-id/spec`,
[`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md): read it before writing
anything here, and change it first when the code must differ from it. Today the repository
holds the measuring tool the design stands on (`bench/`).

## What is decided

- Several **stateless nodes**: no single point of failure in brig·id, rolling updates
  without interruption, inside the infrastructure a company already runs (a cluster, its
  secret manager, its ingress, its PostgreSQL).
- **Everything shared is in PostgreSQL**: passkey ceremonies in progress, sign-in requests
  of applications and their codes, the assertions of clients already used. No cache next
  to it (`forest.md` §3, with the measurements).
- **One store, not two.** The encrypted store of `core` runs on SQLite and on PostgreSQL
  from the same statements. Nothing that encrypts, decrypts or decides is written here.
- It is **not** a federation of servers: accounts and identities are never linked across
  servers (`model.md` §8).
- A security fix reaches the three editions: what protects an account lives in `core`
  and `crypto`, never here.
- Settings under the common prefix `BRIGID_…`; licence AGPL-3.0-or-later, like the two
  other editions. There is no commercial licence.

## Layout

| Path | Content |
| --- | --- |
| `bench/` | The measuring tool: its own Cargo package, never a dependency of the server |
| `src/` | The server: what makes forest an edition of `brigid-server` (to come) |
| `chart/` | The Helm chart (to come) |
| `docs/` | The integration guide (to come) |

## The measuring tool

`bench/` plays the statements of `core`'s store at forest's peak against a database filled
to its tested maximum. **When a statement of the store changes what a request costs, change
it in `bench/src/main.rs` too**, or the published figures describe another server. Figures
are only published with the machine they were measured on (`bench/README.md`).

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
| `forest` | The server (`src/`), its image |
| `bench` | `bench/` |
| `chart` | `chart/` |
| `docs` | `README.md`, `AGENTS.md`, `docs/` |
| `ci` | `.github/workflows/` |
| `deps` | Dependency bumps |
| `ai` | Instructions for coding agents |

**Do not use a scope outside this list.** A new scope is added to this table and to
`.vscode/settings.json` in the same commit.
