# Agent integration (pr402 facilitator)

Runbook for two kinds of autonomous clients:

| You are… | Jump to |
|----------|---------|
| **Seller / resource provider / merchant** (you publish APIs or resources and receive payment) | [Seller agents](#seller-agents-resource-providers) |
| **Buyer / payer agent** (you call paid resources and settle via x402) | [Buyer agents](#buyer-agents-payers) |
| **I have a 402 `accepts[]` line from a seller — what now?** | [Payment pipeline](#payment-pipeline-from-accepts-to-settlement) · [pr402 vs spec](#how-pr402-differs-from-the-generic-x402-spec) |

**Canonical contract:** OpenAPI 3.1 at **`GET /openapi.json`** on your facilitator base URL.

**Important (pr402 ≠ simple wallet `payTo`):** This facilitator settles through **UniversalSettle** (`v2:solana:exact`) and/or **SLA-Escrow** (`v2:solana:sla-escrow`). Your 402 **`payTo`** (and matching proof destinations) must be the **on-chain PDA** your buyers pay into—not a bare seller wallet for settlement proofs:

- **`exact`**: use the UniversalSettle **split-vault / rail PDAs** from seller onboarding (`GET /api/v1/facilitator/onboard?wallet=…`) or your integrator’s docs.
- **`sla-escrow`**: **`payTo` must be the Escrow PDA** for the asset mint and facilitator bank (the facilitator verifies this). The **`POST /build-sla-escrow-payment-tx`** response **`verifyBodyTemplate`** sets the canonical `payTo` for you.

---

## Seller agents (resource providers)

If you receive payment for resources and want **Sovereign** status (95 bps fee tier) and correct **402 `accepts[]`** lines:

1. **Discover rules**: [Onboarding guide](/onboarding_guide.md) — Sovereign vs facilitated (JIT) paths.
2. **Protocol onboarding (on-chain provisioning)**:
   - **CLI**: `universalsettle create-vault --seller <YOUR_PUBKEY>`
   - **API (agent-native)**:
     1. **Build**: `GET /api/v1/facilitator/onboard/build-tx?wallet=<YOUR_PUBKEY>`
     2. **Sign**: Sign the returned `VersionedTransaction` (base64 bincode) with your seller key.
     3. **Send**: Broadcast to Solana.
   - **Incentive**: Proactive vault creation earns an ongoing **5 bps** protocol fee discount.
3. **Status**: Preview PDAs and fees:
   ```bash
   curl -sS "https://<facilitator-url>/api/v1/facilitator/onboard?wallet=<YOUR_PUBKEY>" | jq .
   ```
   Try the **Vault Explorer** on the facilitator `/` landing page for the same resolution.
4. **Off-chain registry (optional)**: `GET /api/v1/facilitator/onboard/challenge?wallet=…` then `POST /api/v1/facilitator/onboard` with the signed payload (requires `DATABASE_URL` + HMAC secret on the server). Persists verified vault metadata for discovery.
5. **Publishing x402**: See [Publishing a Payment Required line](#publishing-a-payment-required-line-sellers) so your `accepts[]` matches what this facilitator verifies.

<a id="publishing-a-payment-required-line-sellers"></a>

### Publishing a Payment Required line (sellers)

Your HTTP **402** body must be valid x402 **v2**, but fields must match **this facilitator’s** on-chain programs—not a generic “transfer to my wallet” mental model.

1. **Tell buyers which facilitator hosts settle**  
   Your docs or API should state the **facilitator base URL** buyers must call for `verify` / `settle` / build endpoints (same host that serves `/capabilities`). If you operate a white-label stack, document the exact origin.

2. **Bootstrap shape from discovery**  
   Call **`GET /api/v1/facilitator/supported`** (or read **`supported`** inside **`GET /capabilities`**). Copy the structure of a `kinds[]` entry for your rail (`v2:solana:exact` or `v2:solana:sla-escrow`): `network`, `scheme`, and especially **`extra`** (fee payer, program IDs, oracle lists, bank/config PDAs). Your **`accepts[]`** lines should be consistent with that shape so buyers can call builders without guessing.

3. **`v2:solana:exact` (UniversalSettle)**  
   - **`payTo`**: Must identify the **vault rail** the facilitator checks on-chain (split-vault / SOL storage / vault ATA semantics per deployment). Use PDAs from **`GET /onboard?wallet=<your_seller_pubkey>`** (`vaultPda`, `solStoragePda`, and token rail as your integration requires). Do **not** publish only your personal wallet as `payTo` unless that is explicitly the derived rail for your deployment.  
   - **`extra`**: Should align with the **`supported`** kind (e.g. `feePayer`, `programId`, `configAddress`, `merchantWallet`, `beneficiary` as your product uses them). Buyers’ proofs are checked against `paymentRequirements` and the wire transaction.

4. **`v2:solana:sla-escrow`**  
   - **`payTo`**: Must be the **Escrow PDA** for `asset` mint + facilitator **bank** (same derivation the facilitator uses). If in doubt, buyers can rely on **`POST /build-sla-escrow-payment-tx`**, which injects canonical `payTo` into **`verifyBodyTemplate`**.  
   - **`extra`**: Must include **`bankAddress`**, **`escrowProgramId`**, **`oracleAuthorities`**, and related fields consistent with **`supported`** for `sla-escrow`. **`extra.bankAddress` must match** the facilitator’s configured bank.  
   - Optional but recommended: **`merchantWallet`** in `extra` for seller identity in metadata (and for your own dashboards).

5. **Operational constraints your buyers will hit**  
   If the deployment enables a **mint allowlist** (`PR402_ALLOWED_PAYMENT_MINTS` / parameters table), unsupported mints fail verify/settle. Document which stablecoins or SPL mints you support.

6. **Minimal mental model**  
   You are not only “pasting Coinbase x402 examples”; you are **pinning** your resource to **this facilitator’s** Solana rails. When in doubt, reproduce a happy path with **`build-exact-payment-tx`** / **`build-sla-escrow-payment-tx`** locally, then mirror the `accepted` object in your live `accepts[]`.

### Seller agent checklist (automation)

1. **Discovery**: `GET /api/v1/facilitator/capabilities` — confirm `features.universalSettleExact`, `features.unsignedExactPaymentTxBuild`, and (if you sell via escrow) `features.slaEscrow` / `features.unsignedSlaEscrowPaymentTxBuild`. Onboard tx is under `httpEndpoints.buildOnboardTx`.
2. **State**: `GET /api/v1/facilitator/onboard?wallet=<PUBKEY>`. If `isSovereign: true`, you already have the discount path where applicable.
3. **Create vault tx**: `GET /api/v1/facilitator/onboard/build-tx?wallet=<PUBKEY>`.
4. **Sign & send** the unsigned shell; then re-check onboard JSON for sovereign / provisioning fields.
5. **Balances (debug)**: `GET /api/v1/facilitator/vault-snapshot?wallet=<PUBKEY>` (UniversalSettle deployments).

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

**Expiry:** Solana blockhashes expire—if verify/simulate fails with blockhash errors, **rebuild** the unsigned tx and re-sign.

---

### 1. Discover

```bash
BASE="https://preview.agent.pay402.me"
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

## Technical specs

- x402 v2: [x402-specification-v2.md](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md)
- Facilitator HTTP: **`/openapi.json`** and Markdown runbook **`/agent-integration.md`** on the deployment
