# pr402 maintainer & operator docs

In-repo documentation for **operators, maintainers, and integrators** who work from the Git tree. End-user guides served by the facilitator live under [`public/`](../public/) and [`docs-site/`](../docs-site/) (VitePress).

**Live contract:** `GET /openapi.json` and `GET /api/v1/facilitator/capabilities` on the deployment you call — not static markdown alone.

## Index

| Document | Audience | Purpose |
|----------|----------|---------|
| [AGENT_INTEGRATION.md](./AGENT_INTEGRATION.md) | Buyers / agents | Pointer to served buyer runbook [`public/agent-integration.md`](../public/agent-integration.md) |
| [SELLER_INTEGRATION.md](./SELLER_INTEGRATION.md) | Sellers / RPs | Pointer to served seller runbook [`public/onboarding_guide.md`](../public/onboarding_guide.md) |
| [CRON_OPERATIONS.md](./CRON_OPERATIONS.md) | Operators | Settlement keeper crons (vault sweep, sla-escrow settle/close), auth, tuning |
| [OPS_RECOVERY_PLAYBOOK.md](./OPS_RECOVERY_PLAYBOOK.md) | Operators | Manual recovery for abnormal DB/chain states (verify-only rows, audit gaps, split-brain settle) |
| [SLA_ESCROW_FEE_PAYER_AND_SETTLE.md](./SLA_ESCROW_FEE_PAYER_AND_SETTLE.md) | Maintainers | Fee-payer layouts, `/verify` + `/settle` branching for sla-escrow vs exact |
| [SLA_ESCROW_FULLCYCLE_ROADMAP.md](./SLA_ESCROW_FULLCYCLE_ROADMAP.md) | Maintainers | What is shipped vs planned for post-fund lifecycle HTTP/SDK |

## Ops helpers (repo root)

| Tool | Purpose |
|------|---------|
| [`scripts/derive_sla_escrow_pda.py`](../scripts/derive_sla_escrow_pda.py) | Derive Payment / bank / escrow PDAs; optional RPC state check |
| [`scripts/trigger-settlement-crons.sh`](../scripts/trigger-settlement-crons.sh) | Dry-run or live settlement keeper HTTP crons |
| [`cargo run --bin record_escrow_lifecycle`](../src/bin/record_escrow_lifecycle.rs) | Backfill `escrow_details` lifecycle fields from known tx signatures |

Note: `scripts/` may be gitignored in some clones; paths are relative to the pr402 crate root.

## What is *not* in this folder

- **Buyer/seller runbooks (canonical):** [`public/agent-integration.md`](../public/agent-integration.md), [`public/onboarding_guide.md`](../public/onboarding_guide.md), [`public/quickstart-seller.md`](../public/quickstart-seller.md)
- **Marketing / start-here site:** [`docs-site/`](../docs-site/)
- **OpenAPI schema:** [`public/openapi.json`](../public/openapi.json)

Facilitator discovery for agents uses **`GET /capabilities`** and **`GET /health`** at runtime — there is no static JSON catalog in this folder.
