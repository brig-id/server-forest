# AGENTS.md — brig·id `server-forest`

This repository is the **largest edition** of the brig·id server: for a company, on
several nodes. It holds no code yet, and it is private for now.

## Language

**All content must be in English** — code, comments, docs, issues. No exceptions.

## Status

Not started. In v1 forest is **designed, not built** (step 3): what it is is in
`brig-id/spec`, [`editions.md`](https://github.com/brig-id/spec/blob/main/editions.md) §2
and §4; read it before writing anything here.

## What is decided

- Several **stateless nodes**: no single point of failure, rolling updates without
  interruption, inside the infrastructure a company already runs (a cluster, its secret
  manager, its ingress).
- What must leave a node's memory to get there is listed in `editions.md` §4: passkey
  ceremonies in progress, sign-in requests of applications and their codes, rate limits.
- **Where the state goes is open** until the design: PostgreSQL alone is the hypothesis to
  test, on measurements. State in an encrypted cookie is already rejected.
- It is **not** a federation of servers: accounts and identities are never linked across
  servers (`model.md` §8).
- A security fix reaches the three editions: what protects an account lives in `core`
  and `crypto`, never here.
- Settings under the common prefix `BRIGID_…`; licence AGPL-3.0-or-later, like the two
  other editions. There is no commercial licence.

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
| `forest` | Top-level binary or federation logic |
| `ci` | `.github/workflows/` |
| `deps` | Dependency bumps |

**Do not use a scope outside this list.** Update this table and `.vscode/settings.json`
when implementation begins.
