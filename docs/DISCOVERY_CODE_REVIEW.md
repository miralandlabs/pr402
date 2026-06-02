# Code review guideline: Agent discovery + sla-escrow starter

Give this to a reviewer who **did not write the diff**. Their job is to prove or disprove that the implementation matches the plan and is safe to ship — not to re-design it.

Related: [DISCOVERY.md](./DISCOVERY.md) · [SRM.md](./SRM.md)

---

## 1. Review charter

**Question:** Does this change add Layer 3 (payable resources) without breaking Layer 1 (facilitator payments) or Layer 2 (merchant onboarding)?

**Out of scope for this review:** New product ideas, refactors, style bikeshedding.

**Reviewer mindset:** Assume the author missed bootstrap paths, route wiring, and backward-compat edges. Your job is to find the next `init.sql` gap.

---

## 2. Non-negotiables (must all pass)

| Rule | How to verify |
|------|----------------|
| **No wire break** on `POST /verify`, `/settle`, build-*-tx, `GET /providers`, `POST /sellers/{wallet}/register` | Diff those handlers/structs; response shapes unchanged |
| **`GET /providers` stays merchant origins only** | No resource rows; no new filters that change semantics |
| **No static Miraland catalog** | No `agent-services.json`, no curated service list in repo |
| **Layer 3 is separate** | Resource register is **not** merged into merchant `discovery` payload |
| **Authoritative pricing = live 402** | Public search is advisory; docs/code must not imply catalog replaces 402 |
| **Fresh DB bootstrap works** | `init.sql` contains full `payable_resources` DDL + indexes + DROP order |
| **Existing DB upgrade path works** | `008_payable_resources.sql` is idempotent and matches `init.sql` |

---

## 3. Review phases (in order)

### Phase A — Schema & migrations (highest risk)

**Files:** `migrations/init.sql`, `migrations/008_payable_resources.sql`

1. **Parity check** — DDL in `008` must match `init.sql` (table, constraints, indexes, RLS):

```bash
diff <(sed -n '/CREATE TABLE IF NOT EXISTS payable_resources/,/);/p' migrations/init.sql) \
     <(sed -n '/CREATE TABLE IF NOT EXISTS payable_resources/,/);/p' migrations/008_payable_resources.sql)
```

2. **DROP order** — `payable_resources` dropped **before** `resource_providers` (FK).

3. **Partial indexes** — Public listing index requires `last_probe_ok = TRUE`; confirm app queries use the same predicates as `list_public_resources` in `src/db/mod.rs`.

4. **Cascade retire** — `retire_resource_provider()` must also retire all `payable_resources` for that wallet.

5. **Greenfield test** (local Postgres):

```bash
psql "$EMPTY_DATABASE_URL" -f migrations/init.sql
psql "$EMPTY_DATABASE_URL" -c "\d payable_resources"
psql "$EMPTY_DATABASE_URL" -c "\di idx_payable_resources*"
```

6. **Brownfield test**:

```bash
# On a DB that already had pre-discovery schema (no payable_resources):
psql "$EXISTING_DATABASE_URL" -f migrations/008_payable_resources.sql
```

**Red flags:** Table only in incremental migration; index mismatch; missing RLS; `ON CONFLICT` upsert keys don't match unique indexes.

---

### Phase B — API surface & routing

**Files:** `src/bin/facilitator.rs`, `src/bin/handlers/resources.rs`, `vercel.json`

Confirm every planned route exists in **both** Rust router **and** Vercel routes:

| Method | Path |
|--------|------|
| GET | `/api/v1/facilitator/resources/register/challenge?wallet=` |
| POST | `/api/v1/facilitator/resources/register` |
| POST | `/api/v1/facilitator/resources/retire` |
| GET | `/api/v1/facilitator/resources` |
| POST | `/api/v1/facilitator/resources/probe` |
| GET/POST | `/api/v1/facilitator/sellers/{wallet}/resources` (signed) |

**Route ordering:** In `facilitator.rs`, `/resources/register/challenge` must match **before** bare `/resources` (prefix trap).

**Capabilities:** `schemaVersion` = `1.2.0`; new `agentManifest` keys are **additive** only; `features.publicResourceDirectory` gated on DB.

**Known gap to check:** `public/openapi.json` — plan mentioned OpenAPI paths; verify whether `/resources` routes are documented or explicitly deferred.

---

### Phase C — Business rules & validation

**Files:** `src/payable_resource.rs`, `src/bin/handlers/resources.rs`, `src/bin/handlers/onboard.rs` (`validate_discovery`)

| Rule | Expected behavior |
|------|-------------------|
| Merchant verified | Register rejected if `resource_provider_verified(wallet)` is false |
| Origin binding | `resource_url` host must match merchant `service_url` host |
| Metadata limits | Same caps as `validate_discovery()` (title 64, description 280, tags ≤5, etc.) |
| Scheme | Only `exact` \| `sla-escrow` |
| Public search | Only rows with `listing_opt_in`, verified, not retired, **`last_probe_ok = true`** |
| Probe | Unpaid request → HTTP 402 + valid JSON + `accepts[0].scheme` + `resource.url` match |

**Manual API smoke** (after migration + `PR402_ONBOARD_HMAC_SECRET`):

1. Complete Layer 2 onboard for a test wallet with `serviceUrl` set.
2. `GET .../resources/register/challenge?wallet=...` → sign → `POST .../resources/register`.
3. `POST .../resources/probe` with `{ "id": ... }` → `lastProbeOk` updated.
4. `GET .../resources?q=...` → row appears only after probe ok + listing opt-in.
5. `POST .../sellers/{wallet}/retire` → payable rows retired too.

---

### Phase D — Security & auth

1. **Register/retire/owner list** reuse `onboard_auth` HMAC + wallet signature (same as merchant register).
2. **Probe endpoint** — check who can call it:
   - Reviewer should confirm whether optional bearer (currently tied to sweep cron token) is intentional or a mistake.
   - UI “Test probe” must work without bearer if that’s the product intent.
3. **Public endpoints** — `GET /resources` must not leak `last_probe_error` or internal fields (compare `PublicResourceEntry` vs `OwnerResourceEntry` in `src/db/mod.rs`).
4. **No auth bypass** — wallet A cannot register URLs on wallet B’s origin (host binding + signature wallet match).

---

### Phase E — UI & static assets

**Files:** `public/resources/index.html`, `public/index.html`, `vercel.json`

- Root nav link to `/resources` only (not a 4th onboarding step).
- Unverified merchant → message + link back to `/`, not inline wizard.
- `/resources` loads `wallet.js`; register + list flows hit correct API paths.
- Static: `/dist/resource-index.json`, `/x402-resources.schema.json` served via Vercel.

**UX bug to watch:** Owner list UI uses `POST` for `/sellers/{wallet}/resources`; plan text says `GET` with query auth — confirm handler supports both and Vercel allows the method used.

---

### Phase F — Discovery pipeline & clients

| Component | Files | Check |
|-----------|-------|-------|
| Indexer | `discovery-indexer/index.mjs` | `--harvest` needs `DATABASE_URL` + `pg`; without DB only rebuilds from API |
| Static index | `public/dist/resource-index.json` | Placeholder vs generated; not hand-curated services |
| `@pr402/discovery` | `sdk/discovery/` | Builds; `searchResources` / `probeResource` match API |
| MCP | `sdk/mcp/src/tools/discovery.ts` | Tools registered; `@pr402/discovery` dependency resolves |
| Buyer demo | `x402-buyer-starter/typescript/examples/discover-and-pay.ts` | Documents required env vars |

**Indexer red flags:** Harvest upserts without origin check; probe not run after harvest; writes that mutate seller 402 behavior.

---

### Phase G — SRM & reference sellers

**Files:** `docs/SRM.md`, `public/x402-resources.schema.json`,  
`solrisk/public/.well-known/x402-resources.json`,  
`x402-buy-spl-token/public/.well-known/x402-resources.json`

- Schema version `0.1.0` consistent.
- Reference manifests use **`REPLACE_WITH_LIVE_MERCHANT_WALLET`** — flag if deployed as-is.
- buy-spl-token entry point URL only (not per-SKU index).

---

### Phase H — sla-escrow seller starter (wire-only)

**Files:** `x402-seller-starter/` (rust / typescript / python)

Confirm **in scope:** 402 + verify/settle FundPayment, SRM at `/.well-known/x402-resources.json`, `find_escrow_payto`.

Confirm **out of scope (must not appear):** SubmitDelivery, registry upload, delivery hash paths.

- `X402_SCHEME=sla-escrow` path builds `oracleProfiles[]` invariant (Rust test in `sla_escrow.rs`).
- README states delivery is out of scope; points to `x402-buy-spl-token` for full cycle.

---

### Phase I — Docs & hub

**Files:** `docs/DISCOVERY.md`, hub `README.md`, `ARCHITECTURE_OVERVIEW.md`, `X402_ECOSYSTEM_PITCH.md`

- Three-layer model documented.
- `/providers` vs `/resources` distinction clear.
- No claim that `/providers` is agent service search.

---

## 4. Automated checks (run before human sign-off)

```bash
cd pr402 && cargo test
cd ../x402-seller-starter/rust && cargo test
cd ../../pr402/sdk/discovery && npm install && npm run build
# If MCP changed:
cd ../mcp && npm install && npm run build
```

Reviewer should record pass/fail for each command.

---

## 5. Red-flag checklist (quick “stop ship”)

- [ ] `payable_resources` missing from `init.sql` or differs from `008`
- [ ] Any change to `PublicProviderEntry` or `/providers` handler filters
- [ ] `resources[]` added to merchant register payload
- [ ] Public search rows without probe gate
- [ ] Origin binding not enforced on register/harvest
- [ ] Merchant retire does not cascade to payable resources
- [ ] Vercel route missing for a Rust handler
- [ ] Capabilities removed/renamed existing `agentManifest` fields
- [ ] Curated service JSON checked into `public/`
- [ ] Reference SRM shipped with placeholder wallet on production hosts

---

## 6. Sign-off template

Reviewer fills this in PR or review doc:

```
## Discovery + escrow starter review

Reviewer: ___
Date: ___

Bootstrap: init.sql greenfield [ PASS | FAIL ] — notes: ___
Brownfield: 008 migration [ PASS | FAIL ] — notes: ___
Backward compat: existing APIs unchanged [ PASS | FAIL ]
Layer separation: /providers vs /resources [ PASS | FAIL ]
Security: auth + origin binding [ PASS | FAIL ]
Probe gate + public listing [ PASS | FAIL ]
UI /resources + vercel routes [ PASS | FAIL ]
Clients (discovery SDK, MCP, indexer) [ PASS | FAIL | N/A ]
Seller starter wire-only sla-escrow [ PASS | FAIL ]
Docs accurate [ PASS | FAIL ]

Blockers: ___
Non-blocking follow-ups: ___

Recommendation: [ APPROVE | APPROVE WITH FIXES | REJECT ]
```

---

## 7. Note for reviewers

The original implementation pass missed putting `payable_resources` in `init.sql` (fixed afterward). Treat **migration parity** and **route wiring** as guilty until proven innocent. Do not approve on “looks fine” — run the greenfield `init.sql` test and at least one full register → probe → search flow.
