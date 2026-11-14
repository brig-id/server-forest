# server-forest

**forest** is the largest edition of the brig·id server: for a company, where sign-in must
not stop. Several nodes that stand in for one another, inside the infrastructure the
company already runs.

It is **not built**, and in v1 it is only designed (step 3):
[`editions.md`](https://github.com/brig-id/spec/blob/main/editions.md) §2 and §4.

## What is decided

- Stateless nodes: no single point of failure, rolling updates without interruption, a
  chart and an integration guide as deliverables.
- The same model as the two other editions: what an account, an identity, a passkey and a
  token are lives in `core` and `crypto`, and a security fix reaches the three.
- Where its state goes (which database, where the short-lived state of ceremonies lives)
  is decided with its design, on measurements. PostgreSQL alone is the hypothesis to test.
- It is not a federation: accounts and identities are never linked across servers.

## Licence

[AGPL-3.0-or-later](LICENSE), like the two other editions. There is no commercial licence:
what a company can pay for is support, hosting, and modules that are separate programs.
