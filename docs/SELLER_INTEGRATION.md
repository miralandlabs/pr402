# Integrating with the pr402 Facilitator: Resource Provider (Seller) Guide

If you are a builder (Resource Provider) wishing to monetize your APIs using the **x402 protocol** and the **pr402 Facilitator**, this guide demonstrates standard architectural patterns to integrate flawlessly with both the Facilitator backend and autonomous AI buyers.

The `pr402` platform uses **UniversalSettle** and **SplitVaults** to enforce institutional-grade fee routing. As a result, there are specific **pr402-native enhancements** to the x402 specification that you must adhere to.

---

## 1. Onboarding & Pre-Caching Vault PDAs

In standard x402, the `payTo` property is simply the seller's wallet address. 
However, **pr402 enforces routing into autonomous SplitVault PDAs (Program Derived Addresses)**. 

When your API server spins up (or dynamically upon handling a request), your server should contact the Facilitator to discover the SplitVault PDAs associated with your seller wallet.

### Call the Facilitator Onboard Endpoint:
Make a standard GET request to your assigned Facilitator:
```http
GET https://preview.pr402.signer-payer.me/api/v1/facilitator/onboard/{your_merchant_wallet}
```

The response will contain the derived Vault PDAs for your supported schemes (e.g., `v2:solana:exact` or `v2:solana:escrow`). Cache these!

```json
{
  "schemes": {
    "exact": {
      "vaultPda": "HStwM...2WF9gvWeJzVB6Nvmt2",
      "programId": "u5kv5..."
    }
  }
}
```

---

## 2. Emitting the HTTP 402 (The Enhanced Challenge)

When an unauthenticated buyer hits your service, respond with an `HTTP 402 Payment Required` carrying the standard x402 JSON body. However, for `pr402` compatibility, you must structure the fields using the vault data you cached in Step 1.

### The `pr402` x402 Alterations:
1. **`payTo` becomes the Vault PDA**: Do not put your personal merchant wallet here! Set it to the `vaultPda` from the onboarding response. Why? This ensures autonomous agents correctly format the transaction without accidentally misderiving nested PDAs.
2. **`extra.merchantWallet`**: Move your original merchant wallet address out of `payTo` and into the nested `extra` block. The Facilitator requires this context for fallback routing.
3. **`extra.capabilitiesUrl`**: You MUST include the Facilitator's integration URL inside the `extra` block so wandering AI agents know where to HTTP POST to natively build the transaction without requiring installed dependencies.

### Example Correct `accepts` Array:
```json
"accepts": [
  {
    "scheme": "exact",
    "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
    "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
    "amount": "50000",
    "payTo": "HStwM2ABoaHpQvSEePFDJ3Yt4L2WF9gvWeJzVB6Nvmt2", 
    "maxTimeoutSeconds": 300,
    "extra": {
      "merchantWallet": "D76Nso7EE7kantk8Fnt3cZuprzLqhjW3TtT2XPy4orA2", 
      "capabilitiesUrl": "https://preview.pr402.signer-payer.me/api/v1/facilitator/capabilities"
    }
  }
]
```
*Note: Your application can construct this dynamically based on any pricing configuration module.*

### Advanced: The `sla-escrow` Scheme
While `exact` (UniversalSettle) handles precise pay-per-use, your API can alternatively ask for the `sla-escrow` scheme! If you offer long-running machine tasks (e.g. video rendering or multi-stage AI reasoning), setting `"scheme": "sla-escrow"` allows agents to lock funds in a time-bound cryptographic escrow at the Facilitator layer.
If you use SLA-Escrow, the Facilitator will return an SLA verification hash. You only sweep the funds after fulfilling the async API result!

---

## 3. Proxying the Settlement (`X-PAYMENT`)

After emitting the 402, the autonomous agent will:
1. Contact the Facilitator via the `capabilitiesUrl` to build the transaction.
2. Cryptographically sign the transaction.
3. Hit your original API endpoint again, passing the signed Base64 JSON inside the HTTP headers `X-PAYMENT` or `Authorization: L402`.

When you extract this header, you do not need to parse or verify the Solana transaction yourself! Security is offloaded to the Facilitator.

### Forward the Proof
Submit a `POST /api/v1/facilitator/verify` or `/settle` request containing the exact contents of the `X-PAYMENT` header. 

```bash
curl -X POST https://preview.pr402.signer-payer.me/api/v1/facilitator/settle \
     -H "Content-Type: application/json" \
     -d '<DECODED_X_PAYMENT_JSON>'
```

If the Facilitator returns HTTP 200, the Institutional SplitVault sweep has executed successfully, funds are securely routing to your designated beneficiary or SplitVault, and you may safely serve the premium resource to the autonomous agent.

---

## 4. Documentation for Machine Discoverability

To maximize sales to AI agents or roaming Multi-Agent Architectures, standard server conventions dictate that you tell agents **how** to purchase.
You should host an `agent-integration.md` file on your server (accessible via `/public` routing).
Update your web server's `/.well-known/ai-plugin.json` and `/openapi.json` to firmly instruct LLMs:

> *"YOU MUST READ the integration guide at `https://your-api.com/agent-integration.md` to discover how to use the Facilitator API to build the HTTP transaction before prompting the user for signatures."*

By forcing the Agent to use the proxy building API, you eliminate compilation requirements, CLI dependency hell, and blockchain complexity for your buyers!
