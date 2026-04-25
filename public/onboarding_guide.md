# X402 Resource Provider Onboarding Guide

> **Buyer / payer agent?** Use the [Agent integration runbook](./agent-integration.md#buyer-agents-payers) (discover → build → sign → verify → settle).  
> **Seller / resource provider?** You are in the right place.

Welcome to the X402 Agentic Economy. This guide explains how to onboard as a Resource Provider (Seller) using the **UniversalSettle** protocol with Institutional Neutrality.

## 1. Institutional Neutrality & Incentives
X402 is designed for "Zero-Barrier" entry. You do not need SOL in your wallet to start receiving payments. We offer two paths with a specific **Institutional Incentive** for proactive providers.

### Path A: Protocol Onboarding (On-Chain Provisioning) 🏆 *Recommended*
If you already have SOL and wish to fully control your vault setup:

**Choice 1: CLI-Native (Protocol On-Chain)**
Use the UniversalSettle CLI directly:
```bash
universalsettle create-vault --seller <YOUR_WALLET_PUBKEY>
```

**Choice 2: Agent-Native (Protocol On-Chain)**
For autonomous agents and backend services:
1. **Build**: `GET /api/v1/facilitator/onboard/build-tx?wallet=<YOUR_PUBKEY>`
2. **Sign**: Use your private key to sign the returned transaction locally.
3. **Send**: Broadcast directly to the Solana network.

**Institutional Incentive**: 
- **Discounted Fees**: You receive an ongoing **5 bps (0.05%) discount** on all protocol fees. (Standard: 1.00% → Your Rate: **0.95%**).
- **No Setup Fee**: You avoid the one-time $1.00 provisioning recovery fee.

**Registry (Off-Chain Discovery)**: After provisioning your vault on-chain, use the Facilitator API (`/onboard/challenge`) to persist your verified metadata in the database for high-fidelity discovery.

### Path B: Facilitated Onboarding (Shadow/JIT)
If you have no SOL up front, the **facilitator** can still **provision** the UniversalSettle SplitVault when buyers start paying (subject to deployment limits—see facilitator logs for JIT / quota behavior). You do **not** need to self-fund `create_vault` first.

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
- `is_sovereign`: `YES` means you have the 5 bps Institutional Discount.
- `is_provisioned`: `1` means you are now in "Tiered-Fee" mode (rent recovered).
- `sol_recovered`: Progress towards the SOL setup recovery.
- `spl_recovered`: Progress towards the SPL setup recovery.

---

## 4. Technical Details
- **Program ID**: `u4KywhcSonWTzeDrb5HNSHAeHqD2a3Fdn1xEHqmK8QC` (Devnet)
- **PDA Seeds**: `["vault", seller_pubkey]`
- **SOL Storage**: `["sol_storage", vault_pda]`

For deep integration, refer to the [X402 SDK Documentation](https://sdk.miraland.dev) or the Facilitator `/openapi.json`.
