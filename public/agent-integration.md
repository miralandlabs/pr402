# Agent integration (pr402 facilitator)

> **New here?** Start with the concise quick starts:
> - **Buyers:** [`/quickstart-buyer.md`](/quickstart-buyer.md) — SDK default + manual curl
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

> **Status.** pr402 is live on **Solana Mainnet** and **Devnet**; the `exact` rail is GA, and `sla-escrow` is available to integrators who operate or trust a production `oracle_authority` (reference: the [`oracles/`](https://github.com/miraland-labs/oracles) workspace ships three sibling oracle profiles — api-quality, onchain-transfer, file-delivery). Behavior and flags can evolve — `GET /capabilities` and `GET /openapi.json` on the host you actually call are the live contract.

| | **Recommended** | **Also available (same service)** |
|--|-----------------|----------------|
| **Production (Mainnet)** | `https://ipay.sh` | `https://agent.pay402.me` |
| **Preview (Devnet)** | `https://preview.ipay.sh` | `https://preview.agent.pay402.me` |

Confirm **`solanaNetwork`**, **`chainId`**, and feature flags with **`GET /api/v1/facilitator/health`** or **`GET /api/v1/facilitator/capabilities`** on the **host you actually call**. Match the origin your seller documents; do not assume preview.

**Wallet RPC:** Read **`solanaWalletRpcUrl`** from **`GET /health`** when you need the deployment’s wallet-facing RPC. Do not hardcode RPC URLs from documentation.

### Golden path (`exact` scheme) — HTTP 402 buyer checklist

Use this order so you do not mismatch facilitator hosts or JSON shapes:

1. **Facilitator URL** — Same origin your seller documents. **Production:** `https://ipay.sh` · **Preview:** `https://preview.ipay.sh` (also `agent.pay402.me` / `preview.agent.pay402.me`). Confirm **`solanaNetwork`** with **`GET /health`**.
2. **`GET /api/v1/facilitator/supported`** (or **`/capabilities`**) — Confirm `exact` / `v2:solana:exact` is listed.
3. **Receive HTTP 402** from the seller — Save **`paymentRequirements`** and the chosen **`accepts[]`** line.
4. **`POST /api/v1/facilitator/build-exact-payment-tx`** — Body: `{ "payer": "<buyer pubkey>", "accepted": <accepts[] line>, "resource": <from 402> }`. Response: **`transaction`**, **`verifyBodyTemplate`**, **`payerSignatureIndex`**, …
5. **Sign** — Sign `transaction` at **`payerSignatureIndex`**. Put signed base64 into **`verifyBodyTemplate.paymentPayload.payload.transaction`**.
6. **Retry seller** — Send the filled **`verifyBodyTemplate`** in header **`PAYMENT-SIGNATURE`** (raw JSON or base64). **Do not** call facilitator **`/verify`** or **`/settle`** first when using SDK, MCP, or x402-buyer-starter.
7. **Seller settles** — Seller gate calls facilitator **`/settle`** (verify runs internally). Success: **HTTP 200** + optional **`PAYMENT-RESPONSE`**.

**Advanced:** buyer-side **`POST /verify`** then **`POST /settle`** before step 6 — debugging only; see [Buyer agents](#buyer-agents-payers).

**Scheme strings (402 vs verify/settle):** Sellers often publish **`v2:solana:exact`** or **`v2:solana:sla-escrow`** in HTTP 402 **`accepts[]`**, matching **`GET /supported`** / discovery. **`POST .../build-*-payment-tx`** accepts those aliases (or wire **`exact`** / **`sla-escrow`**) on the request’s **`accepted`** object, but the returned **`verifyBodyTemplate`** **normalizes** **`scheme`** to the x402 wire values **`exact`** or **`sla-escrow`** in both **`paymentPayload.accepted`** and **`paymentRequirements`**. **`POST /verify`** and **`POST /settle`** also accept either wire or **`v2:solana:*`** on v2 bodies and normalize before verification, so older cached proofs stay valid.

**Shape reminder (after build):** The object you POST to `/verify` and `/settle` matches **`verifyBodyTemplate`** from the build response (with signed tx). It has top-level **`x402Version`**, **`paymentPayload`** (includes nested **`accepted`**, **`payload.transaction`**, **`resource`**), and **`paymentRequirements`** — see **OpenAPI** schema **`X402V2VerifySettleBody`** and its **example** on `GET /openapi.json`.

**SLA-Escrow** — Use **`POST /build-sla-escrow-payment-tx`** instead of step 4; include **`slaHash`** and **`oracleAuthority`** per OpenAPI. Do not use the exact builder for escrow lines. For the cross-actor flow (who authors the SLA bytes, when the seller uploads them, what the oracle compares against), see the **[SLA-Escrow protocol reference](https://github.com/miraland-labs/oracles/blob/main/docs/SLA_ESCROW_PROTOCOL.md)** in the oracles workspace — it's the single source of truth for the four-party interaction.

### Built-in oracle: `x402/oracles/onchain-transfer/v1`

This pr402 deployment ships and operates **one** oracle itself: an
[`oracle-onchain-transfer`](https://github.com/miraland-labs/oracles)
instance that adjudicates SPL token transfers and swaps by re-deriving
pre/post token deltas directly from `getTransaction(jsonParsed)` on the
configured cluster. When the deployment's
`PR402_SLA_ESCROW_ONCHAIN_TRANSFER_DEFAULT_PUBKEY` is set, this oracle
appears in **`GET /capabilities → slaEscrowOracleProfiles[]`** as the
default for token-transfer scenarios. Buyers MAY use it directly without
finding their own oracle operator.

The built-in oracle is **operationally distinct** from the facilitator:

- It runs on its own host (or hosts) with its own keypair, Postgres
  database, and registry storage. A regression in the oracle does NOT
  regress facilitator endpoints (`/verify`, `/settle`, `/build-*-payment-tx`,
  onboard flows). Conversely a facilitator restart does not affect the
  oracle's chain monitor or settlement queue.
- The pr402 operator MAY disable the built-in oracle at any time by
  clearing the `PR402_SLA_ESCROW_ONCHAIN_TRANSFER_DEFAULT_PUBKEY`
  parameter (DB row or env var, whichever the deployment uses). The
  profile then disappears from `slaEscrowOracleProfiles[]` per the
  existing pr402 advertise-only-when-pubkey-is-set semantics. Buyers and
  sellers fall back to ecosystem oracles with no facilitator-side change.
- When the optional health gate (`PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY`)
  is `true`, pr402 probes the oracle's `/health` endpoint and refuses to
  bind escrow to it during transient outages (HTTP 503
  `oracle_unhealthy` from `/build-sla-escrow-payment-tx`). The gate is
  off by default; flip on after the oracle has demonstrated reliable
  uptime in your deployment.

**For other delivery shapes** (`api-quality/v1`,
`file-delivery/attestation/v1`, future profiles), this pr402 deployment
does NOT operate the oracle itself. Sellers naming a non-built-in oracle
in `accepts[].extra.oracleProfiles[]` are responsible for choosing an
operator they (and their buyers) trust. pr402 reviews and lists ecosystem
oracles via the editorial registration template
([`register-oracle.md`](https://github.com/miralandlabs/pr402/issues/new?template=register-oracle.md));
listing is endorsement only of *configuration consistency* (the operator
pubkey is reachable and the profile id is canonical), not endorsement of
the operator's reliability or honesty.

**Trust extended to the built-in oracle is the trust extended to the
pr402 operator.** Trust extended to ecosystem oracles is the trust
extended to that oracle's listed operator. Buyers concerned about the
adjudication path SHOULD verify `Payment.resolution_hash` independently
after settlement; the recipe is in
[`SLA_ESCROW_PROTOCOL.md` §5](https://github.com/miraland-labs/oracles/blob/main/docs/SLA_ESCROW_PROTOCOL.md#5-trust-boundaries).

---

**Important (pr402 ≠ simple wallet `payTo`):** This facilitator settles through **UniversalSettle** (`v2:solana:exact`) and/or **SLA-Escrow** (`v2:solana:sla-escrow`). Your 402 **`payTo`** (and matching proof destinations) must be the **on-chain PDA** your buyers pay into—not a bare seller wallet for settlement proofs:

- **`exact`**: use the UniversalSettle **split-vault / rail PDAs** from seller discovery (`GET /api/v1/facilitator/sellers/…/rails/exact`) or your integrator’s docs.
- **`sla-escrow`**: **`payTo` must be the Escrow PDA** for the asset mint and facilitator bank (the facilitator verifies this). The **`POST /build-sla-escrow-payment-tx`** response **`verifyBodyTemplate`** sets the canonical `payTo` for you.

---

## Seller agents (resource providers)

### Seller endpoint decision matrix

Do **not** collapse these into one endpoint — each has a distinct side effect. Use this table (also available as **`sellerEndpointGuide`** on **`GET /capabilities`**) to pick the right call:

| Goal | Method + path | Side effect | When to use |
|------|---------------|-------------|-------------|
| Multi-rail preview + lifecycle ladder | `GET /api/v1/facilitator/sellers/{wallet}/preview` | Read-only | First seller call; inspect `lifecycle.nextStep` |
| Single-rail `payTo` lookup | `GET /api/v1/facilitator/sellers/{wallet}/rails/{scheme}` (+ `?asset=` for sla-escrow) | Read-only | Building one `accepts[]` line |
| Activate — unsigned CreateVault tx | `POST /api/v1/facilitator/sellers/provision-tx` | Returns unsigned tx | `lifecycle.nextStep === "activate"` |
| Enrich naive 402 body | `POST /api/v1/facilitator/payment-required/enrich` | Read-only JSON transform | Publishing HTTP 402 |
| Registry challenge | `GET /api/v1/facilitator/sellers/{wallet}/challenge` | Read-only | Optional public directory |
| Registry submit | `POST /api/v1/facilitator/sellers/{wallet}/register` | DB write when configured | After Activate on-chain |



### Seller lifecycle: Preview → Activate → Verify

The facilitator exposes a three-stage seller lifecycle. Each stage has a distinct side effect; the stages are surfaced machine-readably via the `lifecycle` block on `GET /api/v1/facilitator/sellers/{wallet}/preview` (`previewed` / `activated` / `verified` / `nextStep`).

| Stage | HTTP | Side effect | Required? |
|-------|------|-------------|-----------|
| **Preview** | `GET /api/v1/facilitator/sellers/{wallet}/preview` | None. Derives vault PDAs and returns on-chain state. `schemes` is keyed by wire names only (`exact`, `sla-escrow`) — at most two entries; not `v2:solana:*` aliases. | No wallet needed. |
| **Activate** | `POST /api/v1/facilitator/sellers/provision-tx` | Returns an unsigned `CreateVault` (+ optional ATA) tx. After the **seller** signs and broadcasts, the on-chain `SplitVault` exists and unlocks the sovereign 90 bps fee tier. | Required before accepting payments. |
| **Verify** (optional) | `GET /api/v1/facilitator/sellers/{wallet}/challenge` then `POST /api/v1/facilitator/sellers/{wallet}/register` | Writes a verified row in the off-chain `resource_providers` registry so the seller appears in discovery. The facilitator **refuses** this step with `409 Conflict` until Activate has landed. | Optional. Only required for verified-seller discovery listings. |

The `POST /sellers/provision-tx` response carries a `statusCode` enum so agents don't have to parse `notes[]`:

- `ALREADY_PROVISIONED` — vault (and ATA if applicable) already exist; no `transaction` returned.
- `VAULT_AND_ATA` — first-time SPL provisioning (two setup ixs).
- `VAULT_ONLY` — first-time native-SOL provisioning (single `CreateVault` ix).
- `ATA_ONLY` — vault exists, adding a new SPL mint (single ATA-create ix).

If you receive payment for resources and want **Sovereign** status (90 bps fee tier — a 10 bps discount off the standard 100 bps rate) and correct **402 `accepts[]`** lines:

1. **Discover rules**: [Onboarding guide](/onboarding_guide.md) — Sovereign vs facilitated (JIT) paths.
2. **Preview (no wallet):** `GET /api/v1/facilitator/sellers/{YOUR_PUBKEY}/preview`. The `lifecycle` block tells you what stage to act on next.
3. **Activate (on-chain):**
   - **Build**: `POST /api/v1/facilitator/sellers/provision-tx` with `{ "wallet": "<YOUR_PUBKEY>", "asset": "SOL" }` (or `USDC`, `USDT`, or a base58 SPL mint). Idempotent per `(wallet, asset)` (`statusCode: "ALREADY_PROVISIONED"` + no `transaction` when done).
   - **Sign** the base64 bincode `VersionedTransaction` with your seller key.
   - **Send** to Solana. Signing with the seller wallet itself earns the ongoing **10 bps** protocol fee discount (90 bps sovereign rate vs 100 bps standard).
4. **Discover your `payTo` (vault PDA)** and metadata:
   ```bash
   curl -sS "https://<facilitator-url>/api/v1/facilitator/sellers/{YOUR_PUBKEY}/rails/exact" | jq .
   ```
   Try the **Vault Explorer** on the facilitator `/` landing page for the same resolution.
5. **Verify identity (optional):** `GET /api/v1/facilitator/sellers/{wallet}/challenge`, sign the returned `message` with the wallet, then `POST /api/v1/facilitator/sellers/{wallet}/register` with `{ wallet, message, signature, asset }`. The signature must be base58-encoded Ed25519. The facilitator returns **`409 Conflict`** if the on-chain vault does not yet exist — run Activate first. Requires `DATABASE_URL` + HMAC secret on the server; persists verified vault metadata for discovery.
6. **Publishing x402**: See [Publishing a Payment Required line](#publishing-a-payment-required-line-sellers) so your `accepts[]` matches what this facilitator verifies.

<a id="publishing-a-payment-required-line-sellers"></a>

### Publishing a Payment Required line (sellers)

Your HTTP **402** body must be valid x402 **v2**, but fields must match **this facilitator’s** on-chain programs—not a generic “transfer to my wallet” mental model.

1. **Tell buyers which facilitator hosts settle**  
   Your docs or API should state the **facilitator base URL** buyers must call for `verify` / `settle` / build endpoints (same host that serves `/capabilities`). If you operate a white-label stack, document the exact origin.

2. **Bootstrap shape from discovery**  
   Call **`GET /api/v1/facilitator/supported`** (or read **`supported`** inside **`GET /capabilities`**). Copy the structure of a `kinds[]` entry for your rail (`v2:solana:exact` or `v2:solana:sla-escrow`): `network`, `scheme`, and especially **`extra`** (fee payer, program IDs, oracle lists, bank/config PDAs). Your **`accepts[]`** lines should be consistent with that shape so buyers can call builders without guessing.

   **Several options in one 402:** x402 **`accepts[]`** is an **array**—each entry is a full payment requirement with its own **`payTo`**, **`asset`**, and metadata; the buyer returns **one** chosen line as **`accepted`**. This is how you advertise more than one token or rail on the same resource. With this facilitator’s **one asset per merchant wallet** rule, use **distinct seller pubkeys** per rail and give each `accepts[]` row the **`payTo`** / **`extra.merchantWallet`** from discovery for **that** key (see [`onboarding_guide.md`](./onboarding_guide.md) — *Policy: one payment asset per merchant wallet*).

3. **`v2:solana:exact` (UniversalSettle)**  
   - **`payTo`**: Must identify the **vault rail** the facilitator checks on-chain (split-vault / SOL storage / vault ATA semantics per deployment). Use PDAs from **`GET /api/v1/facilitator/sellers/{your_seller_pubkey}/rails/exact`** (`vaultPda`, `solStoragePda`, etc.). Do **not** publish only your personal wallet as `payTo` unless that is explicitly the derived rail for your deployment.  
   - **`extra`**: Should align with the **`supported`** kind (e.g. `feePayer`, `programId`, `configAddress`, `merchantWallet`, `beneficiary` as your product uses them). Buyers’ proofs are checked against `paymentRequirements` and the wire transaction.

4. **`v2:solana:sla-escrow`**  
   - **`payTo`**: Must be the **Escrow PDA** for `asset` mint + facilitator **bank** (same derivation the facilitator uses). If in doubt, buyers can rely on **`POST /build-sla-escrow-payment-tx`**, which injects canonical `payTo` into **`verifyBodyTemplate`**.  
   - **`extra`**: Must include **`bankAddress`**, **`escrowProgramId`**, **`oracleAuthorities`**, and related fields consistent with **`supported`** for `sla-escrow`. **`extra.bankAddress` must match** the facilitator’s configured bank.  
   - Optional but recommended: **`merchantWallet`** in `extra` for seller identity in metadata (and for your own dashboards).

5. **Operational constraints your buyers will hit — payment mint allowlist**  
   Deployments may set **`PR402_ALLOWED_PAYMENT_MINTS`** (comma / whitespace‑separated base58 mints; env or `parameters` table — include **`11111111111111111111111111111111`** if native SOL lines must pass).  
   - **Non-empty list:** `exact` **and** `sla-escrow` **`/verify`**, **`/settle`**, **`build-exact-payment-tx`**, and **`build-sla-escrow-payment-tx`** reject any `accepted.asset` / `paymentRequirements.asset` not in the list (same error text as verify).  
   - **Empty / unset:** permissive (all mints). The facilitator logs a **one-time** warning at first check in that mode — do not rely on this in production.  
   - **`POST /payment-required/enrich`:** when an allowlist is configured, the server **warns** (non-blocking) if any **`accepts[].asset`** is missing from the list so you can fix 402 bodies before buyers hit hard failures.

6. **Minimal mental model**  
   You are not only “pasting Coinbase x402 examples”; you are **pinning** your resource to **this facilitator’s** Solana rails. When in doubt, reproduce a happy path with **`build-exact-payment-tx`** / **`build-sla-escrow-payment-tx`** locally, then mirror the `accepted` object in your live `accepts[]`.

### Seller agent checklist (automation)

1. **Capabilities**: `GET /api/v1/facilitator/capabilities` — confirm `features.universalSettleExact`, `features.unsignedExactPaymentTxBuild`, and (if you sell via escrow) `features.slaEscrow` / `features.unsignedSlaEscrowPaymentTxBuild`. Seller lifecycle endpoints are under `httpEndpoints.sellerPreview` / `sellerChallenge` / `sellerRegister` / `sellerProvisionTx` / `sellerRailInfo` / `paymentRequiredEnrich`.
2. **Preview**: `GET /api/v1/facilitator/sellers/{PUBKEY}/preview`. Inspect the `lifecycle` block — if `nextStep === "activate"`, skip to step 4. If `isSovereign: true` on `schemes.exact`, you already have the discount path.
3. **Scheme discovery**: `GET /api/v1/facilitator/sellers/{PUBKEY}/rails/exact` (or `/sellers/{PUBKEY}/rails/sla-escrow?asset=<MINT>`) for a single canonical `payTo`.
4. **Activate**: `POST /api/v1/facilitator/sellers/provision-tx` with `wallet` + **`asset`** for that seller key's single rail. Inspect `statusCode`: `ALREADY_PROVISIONED` means no tx to sign; otherwise sign the base64 bincode tx and broadcast.
5. **Verify (optional)**: `GET /api/v1/facilitator/sellers/{PUBKEY}/challenge` → sign the returned `message` → `POST /api/v1/facilitator/sellers/{PUBKEY}/register` with `{ message, signature (base58), asset }` (path wallet is canonical; body `wallet`, if present, must equal the path). Expect `409 Conflict` if Activate hasn't landed on-chain yet.
6. **Balances (debug)**: `GET /api/v1/facilitator/vault-snapshot?wallet=<PUBKEY>` (UniversalSettle deployments).

---

## Buyer agents (payers)

> **Pick your stack**

| You use… | Install | Best for |
|----------|---------|----------|
| **Cursor / Claude Desktop / MCP host** | `npx -y @pr402/mcp-server` | Tool `pr402_pay_http_resource` + seller MCP tools. Config: [`/agent-tools.json`](/agent-tools.json). |
| **Node script or embed** | `npm i @pr402/client` | `pr402-buy` CLI or `X402AgentClient.fetchWithAutoPay`. |
| **Python LangChain** | `pip install langchain-pr402` | `X402GetTool` / `X402PostTool` — [PyPI](https://pypi.org/project/langchain-pr402/). |
| **Rust** | `cargo install pr402-client` | `pr402-buy` binary + library. |

> **Fastest CLI path:** `npm i -g @pr402/client && pr402-buy --resource <URL> --payer <keypair.json> --mint <MINT>`. Flow: 402 → build → sign → **`PAYMENT-SIGNATURE`** retry; the **seller** settles. Manual curl steps below are for scratch implementations or debugging.

### MCP hosts (Cursor, Claude Desktop)

**Package:** [`@pr402/mcp-server`](https://www.npmjs.com/package/@pr402/mcp-server) (stdio MCP adapter over `@pr402/client`).

1. **Install:** `npm install -g @pr402/mcp-server` or `npx -y @pr402/mcp-server`.
2. **Configure** (project `.cursor/mcp.json` or Claude Desktop `mcpServers`):

```json
{
  "mcpServers": {
    "pr402": {
      "command": "npx",
      "args": ["-y", "@pr402/mcp-server"],
      "env": {
        "PR402_FACILITATOR_URL": "https://preview.ipay.sh",
        "PR402_PAYER_KEYPAIR_JSON": "/absolute/path/to/buyer-keypair.json"
      }
    }
  }
}
```

3. **Devnet:** use `https://preview.ipay.sh` and a funded Devnet keypair. **Mainnet:** `https://ipay.sh`.
4. **Tools:** buyer — `pr402_get_capabilities`, `pr402_build_exact_payment`, `pr402_pay_http_resource`; seller — `pr402_seller_preview`, `pr402_seller_rail_info`, `pr402_seller_provision_tx`, `pr402_enrich_payment_required`.
5. **Machine-readable catalog:** `GET /agent-tools.json` on your facilitator host.

Example Cursor config: [x402-buyer-starter `examples/mcp/cursor-mcp.json`](https://github.com/miraland-labs/x402-buyer-starter/blob/main/examples/mcp/cursor-mcp.json). Source: `pr402/sdk/mcp/README.md`.

> **Discover sellers.** `GET /api/v1/facilitator/providers` returns the public directory of verified, opted-in sellers (paginated via `?limit=&cursor=`). Each entry carries `serviceUrl`, `tags[]`, `displayName`, and the settlement rail pubkeys — enough to build an `accepts[]` line without a prior 402. Single-wallet lookup: `GET /api/v1/facilitator/providers/{wallet}`. The facilitator verifies wallet control only; it does not vet the advertised service.

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

Default path when calling an HTTP 402 seller (matches `@pr402/client`, MCP, x402-buyer-starter):

1. **Read** `accepts[]` — pick one line for your wallet, chain, and asset.
2. **Confirm facilitator** — seller-documented base URL (must match escrow **`extra`** for `sla-escrow`).
3. **`GET /capabilities`** — confirm build features for your scheme.
4. **Build** — `POST /build-exact-payment-tx` or `/build-sla-escrow-payment-tx` with `payer`, `accepted`, and scheme fields per OpenAPI.
5. **Sign** — sign `transaction` at **`payerSignatureIndex`**. **Do not** add address lookup tables.
6. **Fill template** — put signed tx into **`verifyBodyTemplate.paymentPayload.payload.transaction`**. Keep **`paymentPayload.accepted`** and **`paymentRequirements`** identical.
7. **Retry seller** — header **`PAYMENT-SIGNATURE`** (raw JSON preferred; base64 accepted).
8. **Seller settles** — seller calls facilitator **`/settle`**. Read **`PAYMENT-RESPONSE`** on **200** for on-chain proof.

**Blockhash expiry:** rebuild from step 4 and re-sign.

**Advanced — buyer-side verify/settle:** `POST /verify` then `POST /settle` with the same body before step 7. Use for debugging only; not the SDK/starter default. Reuse **`correlationId`** / **`X-Correlation-ID`** when auditing.

---

### 1. Discover

```bash
# Use the same origin your seller documents: production or preview (see table above).
BASE="https://ipay.sh"   # preview: https://preview.ipay.sh — or agent.pay402.me / preview.agent.pay402.me (same APIs)
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

Sign at **`payerSignatureIndex`**. Replace **`paymentPayload.payload.transaction`** in `verifyBodyTemplate` with the signed base64. Keep **`accepted`** identical to **`paymentRequirements`**.

---

### 4. Retry with `PAYMENT-SIGNATURE`

Retry the seller request with the filled **`verifyBodyTemplate`** in header **`PAYMENT-SIGNATURE`**. The seller gate calls facilitator **`/settle`**.

---

### Advanced: buyer-side verify and settle

Not the default for `pr402-buy`, MCP, or x402-buyer-starter. Same JSON body for both:

```bash
curl -sS -X POST "$BASE/api/v1/facilitator/verify" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .

curl -sS -X POST "$BASE/api/v1/facilitator/settle" \
  -H "Content-Type: application/json" \
  -d @verify-body.json | jq .
```

If the seller still requires proof, retry with **`PAYMENT-SIGNATURE`** as in step 4.

---

## Scheme naming

pr402 uses **short canonical names** on the wire:

| Canonical (use this) | Qualified alias (also accepted) | Description |
|--------|-----------|-------------|
| `exact` | `v2:solana:exact` | UniversalSettle instant settlement |
| `sla-escrow` | `v2:solana:sla-escrow` | SLA-Escrow time-bound settlement |

In `accepts[]`, `paymentRequirements.scheme`, and builder request `accepted.scheme`, use the **canonical** name. The qualified forms are accepted for backward compatibility. **`GET /sellers/{wallet}/preview`** (preview) and **`POST /sellers/{wallet}/register`** (registration) return the same wire keys under `schemes` — never duplicate alias entries.

---

## Design highlights (what makes pr402 different)

These are deliberate design choices that differentiate pr402 from a generic x402 facilitator:

| Feature | Benefit |
|---------|---------|
| **`verifyBodyTemplate`** | Build endpoints return a ready-to-use verify/settle body template. Buyers just sign and slot the tx in — no manual JSON construction, no mismatched fields. |
| **Idempotent `/settle`** | If the transaction is already confirmed on-chain, settle returns success. Safe for retries, agent loops, and network interruptions. |
| **`/payment-required/enrich` (Lite → Full 402)** | Sellers can post a naive 402 body with bare wallet `payTo` and receive back a fully institutional response with PDA-derived addresses and `extra` metadata. Eliminates PDA math on the seller side. |
| **Dual scheme support** | Both `exact` (instant UniversalSettle) and `sla-escrow` (time-bound escrow with oracle adjudication) are supported from a single facilitator deployment. |
| **`/sellers/{wallet}/rails/{scheme}` (lightweight)** | Single-scheme, read-only lookup of `payTo` PDA. No auth, no DB. Sellers can call this from any language with a simple HTTP GET. |
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
