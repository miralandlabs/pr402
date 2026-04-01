# X402 Resource Provider Onboarding Guide

Welcome to the X402 Agentic Economy. This guide explains how to onboard as a Resource Provider (Seller) using the **UniversalSettle** protocol with Institutional Neutrality.

## 1. Institutional Neutrality
X402 is designed for "Zero-Barrier" entry. You do not need SOL in your wallet to start receiving payments. The network provides two paths for onboarding:

### Path A: Sovereign Onboarding (Merchant-Paid)
If you already have SOL and wish to fully control your vault setup:
1. Use the X402 CLI: `x402-settle create-vault --seller <YOUR_WALLET_PUBKEY>`
2. This creates your `SplitVault` account immediately.
3. **Benefit**: You avoid the one-time $1.00 provisioning recovery fee.

### Path B: Facilitated Onboarding (Shadow/JIT)
If you have no SOL and want a seamless start:
1. Simply publish your wallet address as your payment destination.
2. When the first customer pays you, the **Facilitator** automatically "Provisions" your vault on-chain.
3. The Facilitator pays the initial Solana rent for you (~0.002 SOL).
4. **Recovery**: The protocol will automatically recover this setup cost ($1.00 USD equivalent) from your first revenue streams. Once recovered, you receive 100% of future payments (minus standard protocol fees).

---

## 2. Dual-Asset Recovery Model
The protocol tracks recovery independently in both SOL and SPL-tokens (e.g., USDC).

- **SOL Threshold**: 10,000,000 lamports (0.01 SOL)
- **SPL Threshold**: 1,000,000 units (e.g., $1.00 USDC)

Your vault reaches "Fully Provisioned" status as soon as **either** threshold is met.

---

## 3. Verifying Your Status
You can check your vault's recovery progress and "Provisioned" status using the CLI:

```bash
x402-settle show-vault --seller <YOUR_WALLET_PUBKEY>
```

**Fields to watch:**
- `is_provisioned`: `1` means you are now in "Zero-Fee" mode (rent recovered).
- `sol_recovered`: Progress towards the SOL threshold.
- `spl_recovered`: Progress towards the SPL threshold.

---

## 4. Technical Details
- **Program ID**: `Univ5ett1e...` (Check Facilitator Config)
- **PDA Seeds**: `["vault", seller_pubkey]`
- **SOL Storage**: `["sol_storage", vault_pda]`

For deep integration, refer to the [X402 SDK Documentation](https://sdk.miraland.dev).
