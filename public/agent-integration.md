# Agent integration (pr402 facilitator)

> **New here?** Start with the concise quick starts:
> - **Buyers:** [`/quickstart-buyer.md`](/quickstart-buyer.md) — 6 steps, copy-paste curl commands
> - **Sellers:** [`/quickstart-seller.md`](/quickstart-seller.md) — 5 steps, no PDA math needed

Runbook for two kinds of autonomous clients:

| You are… | Jump to |
|----------|---------|
| **Seller / resource provider / merchant** (you publish APIs or resources and receive payment) | [Seller agents](#seller-agents-resource-providers) |
| **Buyer / payer agent** (you call paid resources and settle via x402) | [Buyer agents](#buyer-agents-payers) |
| **I have a 402 `accepts[]` line from a seller — what now?** | [Payment pipeline](#payment-pipeline-from-accepts-to-settlement) · [pr402 vs spec](#how-pr402-differs-from-the-generic-x402-spec) |

> **x402 v2 Header Convention** (per [HTTP transport spec](https://github.com/coinbase/x402/blob/main/specs/transports-v2/http.md)):
>
> | Header | Direction | Description |
> |--------|-----------|-------------|
> | `PAYMENT-REQUIRED` | Server → Client | Base64-encoded `PaymentRequired` JSON (on HTTP 402) |
> | `PAYMENT-SIGNATURE` | Client → Server | Payment proof (raw JSON or base64) — replaces V1 `X-PAYMENT` |
> | `PAYMENT-RESPONSE` | Server → Client | Base64-encoded settlement result (on HTTP 200 or 402 after settle) |

**Canonical contract:** OpenAPI 3.1 at **`GET /openapi.json`** on your facilitator base URL.

> **Launch phase:** Facilitator APIs and this runbook are **experimental** — **use at your own risk**.

| | **Production** | **Preview** |
|--|----------------|-------------|
| **Base URL** | `https://agent.pay402.me` | `https://preview.agent.pay402.me` |
| **Typical Solana** | Mainnet | Devnet |

Confirm **`solanaNetwork`**, **`chainId`**, and feature flags with **`GET /api/v1/facilitator/health`** or **`GET /api/v1/facilitator/capabilities`** on the **host you actually call**. Match the origin your seller documents; do not assume preview.

**Wallet RPC:** Read **`solanaWalletRpcUrl`** from **`GET /health`** when you need the deployment’s wallet-facing RPC. Do not hardcode RPC URLs from documentation.

### Golden path (`exact` scheme) — standard integration checklist

Use this order so you do not mismatch facilitator hosts or JSON shapes:

1. **Facilitator URL** — Same origin your seller documents (or embedded in discovery). Official hosts: **Production** `https://agent.pay402.me`, **Preview** `https://preview.agent.pay402.me`. Confirm **`solanaNetwork`** with **`GET /health`** on that host.
2. **`GET /api/v1/facilitator/supported`** (or **`/capabilities`**) — Confirm `exact` / `v2:solana:exact` is listed.
3. **Receive HTTP 402** from the seller — Save **`paymentRequirements`** and the chosen **`accepts[]`** line.
4. **`POST /api/v1/facilitator/build-exact-payment-tx`** — Body: `{ "payer": "<buyer pubkey>", "accepted": <same object as the accepts[] line>, "resource": <from 402> }`. Response is **camelCase** JSON (`transaction`, `verifyBodyTemplate`, `payerSignatureIndex`, …).
5. **Sign** — Deserialize `transaction` (base64 → bincode → `VersionedTransaction`), sign at **`payerSignatureIndex`**, re-encode base64, put into **`verifyBodyTemplate.paymentPayload.payload.transaction`** (replace the unsigned value).
6. **`POST /verify`** then **`POST /settle`** — Send the **filled `verifyBodyTemplate`** JSON as the body (same object for both). Optional: reuse **`correlationId`** / **`X-Correlation-ID`** from a successful verify when calling settle.

**Shape reminder (after build):** The object you POST to `/verify` and `/settle` matches **`verifyBodyTemplate`** from the build response (with signed tx). It has top-level **`x402Version`**, **`paymentPayload`** (includes nested **`accepted`**, **`payload.transaction`**, **`resource`**), and **`paymentRequirements`** — see **OpenAPI** schema **`X402V2VerifySettleBody`** and its **example** on `GET /openapi.json`.

**SLA-Escrow** — Use **`POST /build-sla-escrow-payment-tx`** instead of step 4; include **`slaHash`** and **`oracleAuthority`** per OpenAPI. Do not use the exact builder for escrow lines.

---

**Important (pr402 ≠ simple wallet `payTo`):** This facilitator settles through **UniversalSettle** (`v2:solana:exact`) and/or **SLA-Escrow** (`v2:solana:sla-escrow`). Your 402 **`payTo`** (and matching proof destinations) must be the **on-chain PDA** your buyers pay into—not a bare seller wallet for settlement proofs:

- **`exact`**: use the UniversalSettle **split-vault / rail PDAs** from seller discovery (`GET /api/v1/facilitator/discovery?wallet=…&scheme=exact`) or your integrator’s docs.
- **`sla-escrow`**: **`payTo` must be the Escrow PDA** for the asset mint and facilitator bank (the facilitator verifies this). The **`POST /build-sla-escrow-payment-tx`** response **`verifyBodyTemplate`** sets the canonical `payTo` for you.

---

## Seller agents (resource providers)

If you receive payment for resources and want **Sovereign** status (95 bps fee tier) and correct **402 `accepts[]`** lines:

1. **Discover rules**: [Onboarding guide](/onboarding_guide.md) — Sovereign vs facilitated (JIT) paths.
2. **Protocol onboarding (on-chain provisioning)**:
   - **API (agent-native)**:
     1. **Build**: `POST /api/v1/facilitator/onboard/provision` with `{ "wallet": "<YOUR_PUBKEY>", "asset": "SOL" }` (or `USDC`, `WSOL`, `USDT`, or a mint). Repeat per asset; same pair is idempotent (`alreadyProvisioned` + no `transaction` when done).
     2. **Sign**: When `transaction` is present, sign the `VersionedTransaction` (base64 bincode) with your seller key.
     3. **Send**: Broadcast to Solana when applicable.
   - **Incentive**: Proactive vault creation earns an ongoing **5 bps** protocol fee discount.
3. **Status**: Discover your `payTo` (vault PDA) and metadata:
   ```bash
   curl -sS "https://<facilitator-url>/api/v1/facilitator/discovery?wallet=<YOUR_PUBKEY>&scheme=exact" | jq .
   ```
   Try the **Vault Explorer** on the facilitator `/` landing page for the same resolution. (Note: `/onboard` is for institutional status and proactive onboarding).
4. **Off-chain registry (optional)**: `GET /api/v1/facilitator/onboard/challenge?wallet=…` then `POST /api/v1/facilitator/onboard` with the signed payload plus optional **`asset`** (defaults to **USDC**) — selects the single settlement rail recorded in Postgres for that merchant wallet. **One asset per wallet:** use another seller key for a second coin. Requires `DATABASE_URL` + HMAC secret on the server; persists verified vault metadata for discovery.
5. **Publishing x402**: See [Publishing a Payment Required line](#publishing-a-payment-required-line-sellers) so your `accepts[]` matches what this facilitator verifies.

<a id="publishing-a-payment-required-line-sellers"></a>

### Publishing a Payment Required line (sellers)

Your HTTP **402** body must be valid x402 **v2**, but fields must match **this facilitator’s** on-chain programs—not a generic “transfer to my wallet” mental model.

1. **Tell buyers which facilitator hosts settle**  
   Your docs or API should state the **facilitator base URL** buyers must call for `verify` / `settle` / build endpoints (same host that serves `/capabilities`). If you operate a white-label stack, document the exact origin.

2. **Bootstrap shape from discovery**  
   Call **`GET /api/v1/facilitator/supported`** (or read **`supported`** inside **`GET /capabilities`**). Copy the structure of a `kinds[]` entry for your rail (`v2:solana:exact` or `v2:solana:sla-escrow`): `network`, `scheme`, and especially **`extra`** (fee payer, program IDs, oracle lists, bank/config PDAs). Your **`accepts[]`** lines should be consistent with that shape so buyers can call builders without guessing.

   **Several options in one 402:** x402 **`accepts[]`** is an **array**—each entry is a full payment requirement with its own **`payTo`**, **`asset`**, and metadata; the buyer returns **one** chosen line as **`accepted`**. This is how you advertise more than one token or rail on the same resource. With this facilitator’s **one asset per merchant wallet** rule, use **distinct seller pubkeys** per rail and give each `accepts[]` row the **`payTo`** / **`extra.merchantWallet`** from discovery for **that** key (see [`onboarding_guide.md`](./onboarding_guide.md) — *Launch phase: one payment asset per merchant wallet*).

3. **`v2:solana:exact` (UniversalSettle)**  
   - **`payTo`**: Must identify the **vault rail** the facilitator checks on-chain (split-vault / SOL storage / vault ATA semantics per deployment). Use PDAs from **`GET /api/v1/facilitator/discovery?wallet=<your_seller_pubkey>&scheme=exact`** (`vaultPda`, `solStoragePda`, etc.). Do **not** publish only your personal wallet as `payTo` unless that is explicitly the derived rail for your deployment.  
   - **`extra`**: Should align with the **`supported`** kind (e.g. `feePayer`, `programId`, `configAddress`, `merchantWallet`, `beneficiary` as your product uses them). Buyers’ proofs are checked against `paymentRequirements` and the wire transaction.

4. **`v2:solana:sla-escrow`**  
   - **`payTo`**: Must be the **Escrow PDA** for `asset` mint + facilitator **bank** (same derivation the facilitator uses). If in doubt, buyers can rely on **`POST /build-sla-escrow-payment-tx`**, which injects canonical `payTo` into **`verifyBodyTemplate`**.  
   - **`extra`**: Must include **`bankAddress`**, **`escrowProgramId`**, **`oracleAuthorities`**, and related fields consistent with **`supported`** for `sla-escrow`. **`extra.bankAddress` must match** the facilitator’s configured bank.  
   - Optional but recommended: **`merchantWallet`** in `extra` for seller identity in metadata (and for your own dashboards).

5. **Operational constraints your buyers will hit — payment mint allowlist**  
   Deployments may set **`PR402_ALLOWED_PAYMENT_MINTS`** (comma / whitespace‑separated base58 mints; env or `parameters` table — include **`11111111111111111111111111111111`** if native SOL lines must pass).  
   - **Non-empty list:** `exact` **and** `sla-escrow` **`/verify`**, **`/settle`**, **`build-exact-payment-tx`**, and **`build-sla-escrow-payment-tx`** reject any `accepted.asset` / `paymentRequirements.asset` not in the list (same error text as verify).  
   - **Empty / unset:** permissive (all mints). The facilitator logs a **one-time** warning at first check in that mode — do not rely on this in production.  
   - **`POST /upgrade`:** when an allowlist is configured, the server **warns** (non-blocking) if any **`accepts[].asset`** is missing from the list so you can fix 402 bodies before buyers hit hard failures.

6. **Minimal mental model**  
   You are not only “pasting Coinbase x402 examples”; you are **pinning** your resource to **this facilitator’s** Solana rails. When in doubt, reproduce a happy path with **`build-exact-payment-tx`** / **`build-sla-escrow-payment-tx`** locally, then mirror the `accepted` object in your live `accepts[]`.

### Seller agent checklist (automation)

1. **Discovery**: `GET /api/v1/facilitator/capabilities` — confirm `features.universalSettleExact`, `features.unsignedExactPaymentTxBuild`, and (if you sell via escrow) `features.slaEscrow` / `features.unsignedSlaEscrowPaymentTxBuild`. Seller provisioning is under `httpEndpoints.onboardProvision`.
2. **State**: `GET /api/v1/facilitator/discovery?wallet=<PUBKEY>&scheme=exact`. If `isSovereign: true`, you already have the discount path where applicable.
3. **Full Onboarding**: `GET /api/v1/facilitator/onboard?wallet=<PUBKEY>` for all schemes.
4. **Provision**: `POST /api/v1/facilitator/onboard/provision` with `wallet` + **`asset`** for that seller key’s single rail (see OpenAPI `SellerProvisionTxResponse`); use **another seller key** for a second coin.
5. **Sign & send** when `transaction` is returned; then re-check onboard JSON for sovereign / provisioning fields.
6. **Balances (debug)**: `GET /api/v1/facilitator/vault-snapshot?wallet=<PUBKEY>` (UniversalSettle deployments).

---

## Buyer agents (payers)

<a id="how-pr402-differs-from-the-generic-x402-spec"></a>

### How pr402 differs from the generic x402 spec

The [x402 v2 spec](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md) describes **HTTP 402**, `accepts[]`, payloads, and facilitators in the abstract. **pr402** is a **concrete Solana facilitator** with extra rules you must satisfy:

| Topic | Generic spec expectation | pr402 reality |
|--------|-------------------------|---------------|
| **`payTo`** | Often documented as “who gets paid” / recipient identity | For **`exact`** and **`sla-escrow`**, settlement is to **on-chain PDAs** (UniversalSettle vault rail or SLA Escrow account), not necessarily a simple wallet transfer layout. |
| **Transaction shape** | Wallet or app builds something that proves payment | Proofs are **specific Solana transactions**: compute-budget layout, optional ATA create, `TransferChecked` or **SLA-Escrow `FundPayment`**, fee-payer rules, **no facilitator fee payer inside instruction account metas** (exact path). |
| **Building the tx** | Implementation-defined | Optional: **`POST /build-exact-payment-tx`** or **`POST /build-sla-escrow-payment-tx`** return **bincode `VersionedTransaction`** shells + **`verifyBodyTemplate`**. |
| **Versioned tx features** | Not specified | Transactions that use **address lookup tables** (loaded addresses) are **rejected** until explicitly supported; use static-account-key shells only. |
| **Who signs** | Varies | Often **two signers** when the facilitator pays Solana fees (fee payer + payer authority); see build response **`notes`** and OpenAPI. |
| **Facilitator URL** | May be implied | You must call the **same deployment** the seller used to define rails (check seller docs or **`capabilities`** on that host). |
| **Payment mint allowlist** | Not in the abstract spec | If configured, your **`accepted.asset`** must be listed or **`build-*`** / **`/verify`** / **`/settle`** fail early with an explicit “not supported … Approved assets: …” message. |

If verification fails with **recipient / asset / amount** errors, the usual cause is **`accepts[]`** not matching the **PDA layout** this facilitator checks—not a bug in your wallet.

---

<a id="payment-pipeline-from-accepts-to-settlement"></a>

### Payment pipeline: from `accepts[]` to settlement

Walk this in order when a seller returns **402** JSON:

1. **Read** `accepts[]` and choose one line matching your payer wallet, chain, and asset.
2. **Confirm facilitator** — Use the seller-documented base URL, or the host that issued discovery for that merchant (must match **`extra.escrowProgramId` / network** for escrow).
3. **`GET /capabilities`** on that host — Confirm `features` (e.g. `unsignedExactPaymentTxBuild`, `unsignedSlaEscrowPaymentTxBuild`, `slaEscrow`).
4. **Build (recommended)** — `POST` the matching **build** endpoint with `payer`, `accepted` (the line you chose), and scheme-specific fields (`resource`, `slaHash`, `oracleAuthority`, etc. per OpenAPI).
5. **Deserialize** the returned `transaction` (base64 → bincode → `VersionedTransaction`). **Do not** add address lookup tables.
6. **Sign** all required signer slots (see response **`notes`**; partial sign first when facilitator is fee payer, then facilitator signs at settle if applicable).
7. **Fill template** — Paste the **signed** tx base64 into **`verifyBodyTemplate`**. Keep **`paymentPayload.accepted`** and **`paymentRequirements`** **byte-for-byte identical** (same JSON object).
8. **`POST /verify`** then **`POST /settle`** with the **same** body; reuse **`X-Correlation-ID`** / body `correlationId` if the seller or your agent needs audit linkage (facilitator may mint an id on successful verify when DB is enabled).
9. **Authorized Access (Resource Provider)**: Submit the finalized JSON proof to the resource provider in the **`PAYMENT-SIGNATURE`** header (x402 v2).
     - **Optimization**: You can send the raw JSON string directly (preferred) or Base64-encode it. All X402 v2-compliant servers now support both.
     - **`PAYMENT-RESPONSE`**: After settlement, x402 v2 compliant sellers return a `PAYMENT-RESPONSE` header (base64-encoded JSON) containing the settlement result (`success`, `transaction`, `network`, `payer`). Buyer agents can inspect this header to confirm on-chain finality without polling.

**Expiry:** Solana blockhashes expire—if verify/simulate fails with blockhash errors, **rebuild** the unsigned tx and re-sign.

---

### 1. Discover

```bash
# Use the same origin your seller documents: production or preview (see table above).
BASE="https://agent.pay402.me"   # or: https://preview.agent.pay402.me
curl -sS "$BASE/api/v1/facilitator/supported" | jq .
# or
curl -sS "$BASE/api/v1/facilitator/capabilities" | jq .
```

Pick one `kinds[]` entry that matches the **402 `accepts[]`** line your resource returned (`scheme`, `network`, `asset`, `amount`, `payTo`, `extra`). Align **`payTo`** with seller-published PDAs (see [Seller agents](#seller-agents-resource-providers)).

---

### 2. Build unsigned tx

Two build endpoints; **do not** confuse them.

| `accepts[].scheme` | Endpoint | Who signs before verify |
|--------------------|----------|-------------------------|
| `exact` | `POST /api/v1/facilitator/build-exact-payment-tx` | Payer signs token authority; facilitator is fee payer at settle (default). |
| `sla-escrow` | `POST /api/v1/facilitator/build-sla-escrow-payment-tx` | Buyer partial sign; facilitator completes fee payer at settle (default). |

Request bodies are in **`openapi.json`** (`BuildExactPaymentTxRequest`, `BuildSlaEscrowPaymentTxRequest`). Response includes **`verifyBodyTemplate`** and base64 **`transaction`** (bincode `VersionedTransaction`, unsigned). For **sla-escrow**, trust the template’s **`payTo`** (canonical Escrow PDA) even if your upstream 402 line still showed a legacy value.

---

### 3. Sign locally

Use your stack’s Solana signer. Replace **`paymentPayload.payload.transaction`** in `verifyBodyTemplate` with the **signed** tx base64. Keep **`accepted`** identical to **`paymentRequirements`**.

---

### 4. Verify and settle

Use the **same** JSON body for both calls.

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .

curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .
```

---

## Scheme naming

pr402 uses **short canonical names** on the wire:

| Canonical (use this) | Qualified alias (also accepted) | Description |
|--------|-----------|-------------|
| `exact` | `v2:solana:exact` | UniversalSettle instant settlement |
| `sla-escrow` | `v2:solana:sla-escrow` | SLA-Escrow time-bound settlement |

In `accepts[]`, `paymentRequirements.scheme`, and builder request `accepted.scheme`, use the **canonical** name. The qualified forms are accepted for backward compatibility.

---

## Design highlights (what makes pr402 different)

These are deliberate design choices that differentiate pr402 from a generic x402 facilitator:

| Feature | Benefit |
|---------|---------|
| **`verifyBodyTemplate`** | Build endpoints return a ready-to-use verify/settle body template. Buyers just sign and slot the tx in — no manual JSON construction, no mismatched fields. |
| **Idempotent `/settle`** | If the transaction is already confirmed on-chain, settle returns success. Safe for retries, agent loops, and network interruptions. |
| **`/upgrade` (Lite → Full 402)** | Sellers can post a naive 402 body with bare wallet `payTo` and receive back a fully institutional response with PDA-derived addresses and `extra` metadata. Eliminates PDA math on the seller side. |
| **Dual scheme support** | Both `exact` (instant UniversalSettle) and `sla-escrow` (time-bound escrow with oracle adjudication) are supported from a single facilitator deployment. |
| **`/discovery` (lightweight)** | Single-scheme, read-only lookup of `payTo` PDA. No auth, no DB. Sellers can call this from any language with a simple HTTP GET. |
| **CORS `Access-Control-Expose-Headers`** | `PAYMENT-RESPONSE`, `X-Correlation-ID`, and `X-API-Version` are exposed so browser-based agents can read settlement results. |
| **Toxic asset protection** | Configurable mint allowlist (`PR402_ALLOWED_PAYMENT_MINTS`) prevents settlement with worthless spam tokens. |
| **Seller quick start** | Language-agnostic guide at [`/seller-quick-start.md`](/seller-quick-start.md) with pseudocode and examples in Rust, Python, JS/TS, and Go. |

---

## Technical specs

- x402 v2: [x402-specification-v2.md](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md)
- Facilitator HTTP: **`/openapi.json`** and Markdown runbook **`/agent-integration.md`** on the deployment
- Seller integration: **`/seller-quick-start.md`** on the deployment

---

## Internal ops: cron sweep (private)

`POST /api/v1/facilitator/sweep` is an **internal-only** endpoint for scheduler/cron execution.

- **Auth:** `Authorization: Bearer <token>`
- **Token source:** `PR402_SWEEP_CRON_TOKEN` (`parameters` table takes precedence over env var).
- **Purpose:** Drain eligible UniversalSettle vault balances without requiring a settlement request in the same invocation.
- **Safety:** Use `{"dryRun": true}` first in cron rollout.

### Vercel Cron note

Vercel Cron sends **GET** requests. Use:

- `GET /api/v1/facilitator/sweep-cron` (private, bearer-auth)

This route runs the same sweep engine with configured defaults (equivalent to an empty POST body).

### Sweep parameters (DB `parameters` keys)

DB values override env values in this project.

- `PR402_SWEEP_CRON_TOKEN`
  - Bearer token for the private sweep endpoint.
  - Seeded in `migrations/init.sql` as bootstrap placeholder (`CHANGE_ME_BEFORE_PRODUCTION`).
- `PR402_SWEEP_CRON_COOLDOWN_SEC` (default: `300`)
  - Minimum interval between sweep attempts per provider rail.
- `PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC` (default: `86400`)
  - Candidate must have a successful settle within this recent window.
- `PR402_SWEEP_CRON_BATCH_LIMIT` (default: `50`)
  - Maximum candidate rails processed per sweep run.
- `PR402_SWEEP_MIN_SPENDABLE_LAMPORTS` (default: `30000000`)
  - SOL threshold (0.03 SOL) before attempting sweep.
- `PR402_SWEEP_MIN_SPL_RAW_DEFAULT` (default: `3000000`)
  - Default SPL raw threshold when mint has no explicit override.
- `PR402_SWEEP_MIN_SPL_RAW_BY_MINT`
  - JSON map for per-mint SPL raw thresholds.

### Suggested scheduler body

```json
{
  "dryRun": false,
  "limit": 50,
  "cooldownSeconds": 300,
  "requireRecentSettleWithinSeconds": 86400
}
```
