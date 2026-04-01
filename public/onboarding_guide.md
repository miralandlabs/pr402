# X402 Resource Provider Onboarding Guide

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
If you have no SOL and want a seamless start:
1. Simply publish your wallet address as your payment destination.
2. When the first customer pays you, the **Facilitator** automatically "Provisions" your vault on-chain.
3. The Facilitator pays the initial Solana rent for you (~0.002 SOL).
4. **Standard Model**:
   - **Standard Fee**: You are charged the standard protocol fee (**1.00%**).
   - **Provisioning Recovery**: The protocol will automatically recover the setup cost ($1.00 USD equivalent) from your first revenue streams. Once recovered, you receive 100% of future payments (minus standard protocol fees).

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
