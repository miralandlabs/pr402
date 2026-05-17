# X402 Resource Provider Onboarding Guide

> **Buyer / payer agent?** Use the [Agent integration runbook](/agent-integration#buyer-agents-payers) (discover → build → sign → verify → settle).  
> **Seller / resource provider?** You are in the right place.

Welcome to the X402 Agentic Economy. This guide explains how to onboard as a Resource Provider (Seller) using the **UniversalSettle** protocol with Institutional Neutrality.

> **Status.** pr402 and the `exact` rail are live on **Solana Mainnet** and **Devnet**. The policy described below (one payment asset per merchant wallet) is an **operational policy in the facilitator's application layer**, not a hard on-chain constraint, and may be relaxed as the network matures.

**Official facilitator hosts:** **Recommended:** **Production** `https://ipay.sh` (Solana Mainnet) · **Preview** `https://preview.ipay.sh` (Solana Devnet). **Also available** (same service; not deprecated): **`https://agent.pay402.me`**, **`https://preview.agent.pay402.me`**. Use the deployment that matches your programs and the facilitator URL you give buyers; verify with **`GET /api/v1/facilitator/health`** or **`/capabilities`**.

### SLA-Escrow: oracle profile and default operator hints

For **`sla-escrow`**, the on-chain payment includes an **`oracle_authority`** that resolves delivery via **`ConfirmOracle`**. The x402 ecosystem ships three reference oracle profiles in the [`oracles/`](https://github.com/miraland-labs/oracles) workspace:

- **`x402/oracles/api-quality/v1`** — JSON HTTP API delivery quality (status, latency, schema, required fields, body length). Hash-bound SLA + delivery JSON fetched from an HTTP evidence registry.
- **`x402/oracles/onchain-transfer/v1`** — SPL token transfer / swap delivery, re-derived from `getTransaction(jsonParsed)` against the on-chain tx signature.
- **`x402/oracles/file-delivery/attestation/v1`** — Large-file streaming delivery (video / dataset / artifact); SHA-256 + MIME sniff verification.

Set **`oracle_authority`** to your chosen operator's pubkey (the keypair that runs the matching oracle binary). Deployments may advertise default operators per profile via `GET /api/v1/facilitator/capabilities` under **`slaEscrowOracleProfiles[]`**. Each profile is configured via two interchangeable mechanisms (DB `parameters` row wins, env var fallback so Vercel deployments avoid the env-size limit): the JSON override `PR402_SLA_ESCROW_ORACLE_PROFILES_JSON`, or per-profile keys `PR402_SLA_ESCROW_API_QUALITY_DEFAULT_PUBKEY`, `PR402_SLA_ESCROW_ONCHAIN_TRANSFER_DEFAULT_PUBKEY`, `PR402_SLA_ESCROW_FILE_DELIVERY_DEFAULT_PUBKEY`. Integrators should still confirm the pubkey matches their trust model.

### Policy: one payment asset per merchant wallet

This is a **product and operations policy** enforced in the facilitator's application layer — not a hard on-chain constraint. We keep the **operator and integrator experience intentionally small**: **each merchant wallet is expected to register and settle on a single payment asset (one coin / one settlement rail)**—for example, USDC *or* native SOL, not both under the same wallet in our off-chain registry. This follows a simple design principle we care about: **favor simplicity first, implemented with care so the result still feels elegant**—minimal surface area for discovery, reconciliation, and automation.

That rule is enforced in the facilitator’s **application layer** (not by tightening on-chain or database uniqueness beyond what already exists). **We may relax or refine it as the network matures; that is a product and operations decision, not a guarantee.** If you already know you will accept **multiple tokens**, plan for **separate seller wallets** (one wallet per asset/rail you want to treat as a first-class merchant identity). That pattern stays compatible with UniversalSettle and keeps future policy changes straightforward.

**How this fits x402:** A Payment Required response uses an **`accepts[]` array**—each entry is a full payment option with its own **`payTo`**, **`asset`**, **`scheme`**, **`network`**, and optional **`extra`** (including **`merchantWallet`**). The protocol does not limit you to a single option: you advertise **one row per acceptable method**; the buyer then sends back **one** chosen line as **`accepted`**. To offer, say, USDC and native SOL under this facilitator’s policy, publish **two `accepts[]` rows**—each with the correct **`payTo`** (vault PDA from discovery for **that** seller key) and **`extra.merchantWallet`** set to the **wallet for that rail**—not two assets squeezed under the same merchant pubkey.

## 1. Institutional Neutrality & Incentives
X402 is designed for "Zero-Barrier" entry. You do not need SOL in your wallet to start receiving payments. We offer two paths with a specific **Institutional Incentive** for proactive providers.

### Path A: Protocol Onboarding (On-Chain Provisioning) 🏆 *Recommended*
If you already have SOL and wish to fully control your vault setup:

### Agentic Provisioning (Protocol On-Chain)
For autonomous agents and backend services:
1. **Build**: `POST /api/v1/facilitator/onboard/provision` with JSON body `{ "wallet": "<YOUR_PUBKEY>", "asset": "SOL" }` (or `USDC`, `USDT`, or a base58 mint). Under the facilitator's application-layer policy, use **one asset per merchant wallet**; repeats for the same `(wallet, asset)` are idempotent. For another token, use **another seller wallet** (see above).
2. **Sign**: If the response includes `transaction`, sign the base64 bincode `VersionedTransaction` locally. If `alreadyProvisioned` is true, there is nothing to send for that asset.
3. **Send**: Broadcast to Solana when a transaction is present.

**Institutional Incentive**: 
- **Discounted Fees**: You receive an ongoing **10 bps (0.10%) discount** on all protocol fees. (Standard: **1.00%** → Your Rate: **0.90%**).
- **No Setup Fee**: You avoid the one-time $1.00 provisioning recovery fee.

**Registry (Off-Chain Discovery)**: After provisioning your vault on-chain, use the Facilitator API (`/onboard/challenge`) to persist your verified metadata in the database for high-fidelity discovery.

### Path B: Facilitated Onboarding (Shadow/JIT)
If you have no SOL up front, the **facilitator** can still **provision** the UniversalSettle SplitVault when buyers start paying (subject to deployment limits—see facilitator logs for JIT / quota behavior). You do **not** need to self-fund `create_vault` first.

**Explicit self-provision (Path A) is what we recommend**—it is predictable, gives you the institutional fee discount when the seller pays creation, and avoids leaning on facilitator-paid setup. **It is not mandatory.** You may still **discover** the correct vault rail via the facilitator, set that **`payTo`** in your HTTP 402 `Payment Required` response, and let the **first payment (or pre-build) path** create or complete vault accounts when the facilitator acts as payer. On-chain, UniversalSettle records **who paid** `CreateVault`: if the **seller** signs and pays, the vault is treated as **sovereign / provision-complete** for fee-tier purposes; if a **different payer** (e.g. the facilitator) fronts rent, the program leaves **`is_sovereign` unset** and **`is_provisioned` false** until **provisioning recovery** catches up via sweeps—exactly why **recovery accounting** and the **discounted fee tier** for proactive sellers exist in the program (see `universalsettle` `SplitVault` state and `CreateVault` handling, and this repo’s README for fee parameters).

The facilitator’s **one-asset-per-wallet registry policy** (above) governs **off-chain** `resource_providers` consistency when a **merchant wallet** is declared in metadata; it does **not** replace or contradict JIT on-chain creation. First contact for a wallet with **no registry row yet** is still allowed for the rail you choose; conflicts are rejected when the same merchant wallet is **already** registered for a **different** asset.

**Important for your HTTP 402 body:** Buyers must pay into the **SplitVault rail PDAs** this facilitator verifies—not a bare wallet address in `payTo`. Before publishing `accepts[]`:

1. Resolve canonical addresses with **`GET /api/v1/facilitator/discovery?wallet=<YOUR_PUBKEY>&scheme=exact`**, **or**
2. Post a minimal draft body to **`POST /api/v1/facilitator/upgrade`** so the response injects the correct **`payTo`** vault PDA and `extra` metadata.

Put your **merchant identity** in **`extra.merchantWallet`**; keep **`payTo`** as the vault PDA from discovery/upgrade. The buyer’s payment flow may call **`build-exact-payment-tx`**, which runs **vault setup** (`ensure_vault_setup`) when UniversalSettle is configured—see pr402 `exact_payment_build`.

**Standard Model (fees / recovery):**
   - **Standard Fee**: You are charged the standard protocol fee (**1.00%**) until sovereign discount applies.
   - **Provisioning Recovery**: The protocol recovers facilitator-paid setup costs from your revenue per on-chain `SplitVault` / config rules. See on-chain state and facilitator discovery for live numbers.

---

## 2. Status & Tracking
The protocol tracks recovery independently in both SOL and SPL-tokens (e.g., USDC).

- **SOL Recovery Target**: 10,000,000 lamports (0.01 SOL)
- **SPL Recovery Target**: 1,000,000 units (e.g., $1.00 USDC)

Your vault reaches "Fully Provisioned" status as soon as **either** threshold is met.

---

## 3. Verifying Your Status
You can check your vault's recovery progress, fee rate, and "Sovereign" status using the CLI:

```bash
universalsettle vault-status --seller <YOUR_WALLET_PUBKEY>
```

**Fields to watch:**
- `is_sovereign`: `YES` means you have the 10 bps Institutional Discount (pay 90 bps instead of the standard 100 bps).
- `is_provisioned`: `1` means you are now in "Tiered-Fee" mode (rent recovered).
- `sol_recovered`: Progress towards the SOL setup recovery.
- `spl_recovered`: Progress towards the SPL setup recovery.

---

## 4. Technical details

**Program IDs, vault PDAs, and `extra` fields are deployment-specific** (Mainnet vs Devnet, program upgrades, facilitator config). Do **not** copy static addresses from documentation into production.

Authoritative sources on the facilitator host you call:

- **`GET /api/v1/facilitator/supported`** — copy **`programId`**, **`configAddress`**, and other **`extra`** keys from the `kinds[]` entry that matches your scheme (`v2:solana:exact`, `v2:solana:sla-escrow`).
- **`GET /api/v1/facilitator/discovery?wallet=…&scheme=exact`** — canonical **`payTo`** / rail PDAs for your seller wallet.
- **`POST /api/v1/facilitator/upgrade`** — injects the same institutional **`payTo`** / **`extra`** into a naive 402 body.

**Conceptually**, UniversalSettle-style deployments derive vault-related PDAs from the seller identity on-chain; the exact seeds and program IDs must match what **`supported`** advertises for that cluster.

For deep integration, refer to the [X402 SDK Documentation](https://sdk.miraland.dev), **`GET /openapi.json`** on your facilitator, and [API reference](/api-reference) in this site.
