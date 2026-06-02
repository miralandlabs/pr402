# AGENTS.md

This file is for AI agents (Cursor, Claude Code, etc.), not human developers.
Philosophy: **Simple is Best, yet Elegant.** Make the smallest change that solves
the task; do not refactor, abstract, or add features that were not asked for.

`pr402` is the x402 facilitator: a Rust HTTP service deployed on Vercel that
verifies/settles Solana payments and hosts merchant + resource discovery.

## Topology

- `src/bin/facilitator.rs` — HTTP router (one `match (method, path)` arm per route).
- `src/bin/handlers/` — one file per concern (verify, settle, build, onboard, resources…).
- `src/db/mod.rs` — all Postgres access; public response structs live here.
- `migrations/` — `init.sql` is the canonical full schema; `NNN_*.sql` are incremental.
- `public/` — static assets + UI (`/`, `/resources`); routes mirrored in `vercel.json`.
- `sdk/` — `discovery`, `mcp`, `client` (TypeScript). `scripts/discovery-indexer/` — harvester.

## Hard boundaries (do not cross without explicit human approval)

- **Never break a live wire contract.** Response shapes of `POST /verify`, `/settle`,
  `build-*-tx`, `GET /providers`, `POST /sellers/{wallet}/register` are frozen. Add, never rename/remove.
- **Authoritative payment terms = the live HTTP 402.** Manifests, capabilities, and the
  resource directory are advisory only. Never let a catalog imply it replaces 402.
- **`GET /providers` is merchant origins only.** It is not an agent-service search.
- **No new dependencies** (`Cargo.toml`, `package.json`) unless the prompt asks for it.
- **Sibling repos are separate git repos** (gitignored from the hub). Do not commit across them.
- **No secrets in code or committed files.** Use env vars / `${{ secrets.* }}`.

## Migrations (highest-risk area — treat as guilty until proven innocent)

- Any table/index/RLS added to an incremental `NNN_*.sql` **must also be in `init.sql`**, identical.
- `init.sql` keeps RLS and the correct `DROP TABLE` order (child before parent FK).
- Incremental migrations must be idempotent (`IF NOT EXISTS`, `COALESCE`, etc.).
- `ON CONFLICT` keys must match an actual unique index.

## Verification protocol (run before claiming done; fix, don't suppress)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings   # warnings fail CI
cargo test --all-targets
# if an SDK changed:
cd sdk/discovery && npm install && npm run build
cd ../mcp       && npm install && npm run build
```

## Context (the "dark knowledge")

- Three layers: 1 = facilitator payments, 2 = merchant onboarding, 3 = payable resources.
  Keep them separate — never merge Layer 3 `resources[]` into the Layer 2 register payload.
- `/capabilities` changes must be **additive**; bump `schemaVersion`, never rename existing keys.
- Owner/admin writes use the onboard HMAC challenge + wallet signature — reuse it, don't invent new auth.
- See `docs/` (e.g. `DISCOVERY.md`, `DISCOVERY_CODE_REVIEW.md`, `SRM.md`) for deeper context.
