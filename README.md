# server-forest

**forest** is the largest edition of the brig·id server: for a company, where sign-in must
not stop. Several nodes that stand in for one another, inside the infrastructure the
company already runs, on a PostgreSQL database the company provides.

It is **not built yet**. Its design is in the specification:
[`forest.md`](https://github.com/brig-id/spec/blob/main/forest.md). What the three
editions are, and what separates them, is in
[`editions.md`](https://github.com/brig-id/spec/blob/main/editions.md) §2.

## What is here

| Path | Content |
| --- | --- |
| [`bench/`](bench/README.md) | The measuring tool behind the design: what forest's peak costs a database, and where the state shared between nodes can live |

What will come, in the order of the design: the server (`src/`), its image, the chart
(`chart/`) and the integration guide (`docs/`).

## What is decided

- Nodes hold no state: any of them answers any request, one can stop without a sign-in in
  progress being lost, and an update replaces them one by one.
- The same model as the two other editions: what an account, an identity, a passkey and a
  token are lives in `core` and `crypto`, and a security fix reaches the three.
- Everything a node must share with the others is in PostgreSQL, and nowhere else: no
  cache, no queue, no second service to run.
- It is not a federation: accounts and identities are never linked across servers.

## Licence

[AGPL-3.0-or-later](LICENSE), like the two other editions. There is no commercial licence:
what a company can pay for is support, hosting, and modules that are separate programs.
