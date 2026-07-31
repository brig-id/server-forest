# AGENTS.md — brig·id `server-forest`

This repository is a **placeholder** for the future global federation layer.

## Language

**All content must be in English** — code, comments, docs, issues. No exceptions.

## Status

Not started. No implementation work should happen here until `server-grove` is complete
and multi-server orchestration is stable.

## Planned scope (future)

- Global identity federation across independent brig·id deployments
- Cross-server DID resolution
- OIDC Federation (OpenID Connect Federation 1.0)
- Distributed trust anchors
- Serving the `web` UI — same static-dist mechanism as `server-leaf`
  (`ui_dist_dir`/SPA fallback), unless a CDN in front of the federation
  makes more sense at this tier; not decided yet

Do not add product code to this repository at this stage.

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
