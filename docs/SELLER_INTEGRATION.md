# Integrating with the pr402 Facilitator: Resource Provider (Seller) Guide

If you are a builder (Resource Provider) wishing to monetize your APIs using the **x402 protocol** and the **pr402 Facilitator**, this guide demonstrates standard architectural patterns to integrate flawlessly with both the Facilitator backend and autonomous AI buyers.

The `pr402` platform uses **UniversalSettle** and **SplitVaults** to enforce institutional-grade fee routing. As a result, there are specific **pr402-native enhancements** to the x402 specification that you must adhere to.

> **Launch phase:** **Experimental** — integrate **at your own risk**.

> **x402 v2 Header Convention** (per [HTTP transport spec](https://github.com/coinbase/x402/blob/main/specs/transports-v2/http.md)):
>
> | Header | Direction | Description |
> |--------|-----------|-------------|
> | `PAYMENT-REQUIRED` | Server → Client | Base64-encoded `PaymentRequired` JSON (on HTTP 402) |
> | `PAYMENT-SIGNATURE` | Client → Server | Payment proof (raw JSON or base64) — replaces legacy `X-PAYMENT` |
> | `PAYMENT-RESPONSE` | Server → Client | Base64-encoded settlement result (on HTTP 200 or 402 after settle) |

---

## 1. Discover Your Vault PDAs

In standard x402, the `payTo` property is simply the seller's wallet address.
However, **pr402 enforces routing into autonomous SplitVault PDAs (Program Derived Addresses)**.

When your API server spins up (or dynamically upon handling a request), your server should contact the Facilitator to discover the SplitVault PDAs associated with your seller wallet.

Replace **`$BASE`** with your facilitator origin (the same URL buyers use for `verify` / `settle`). Official deployments: **Production** `https://agent.pay402.me` (Mainnet), **Preview** `https://preview.agent.pay402.me` (Devnet). Confirm cluster with **`GET $BASE/api/v1/facilitator/health`**.

### Quick lookup (single scheme):
```http
GET $BASE/api/v1/facilitator/discovery?wallet=YOUR_PUBKEY&scheme=exact
```

### Full onboard preview (all schemes):
```http
GET $BASE/api/v1/facilitator/onboard?wallet=YOUR_PUBKEY
```

The response will contain the derived Vault PDAs for your supported schemes. Cache these!

```json
{
  "wallet": "D76Nso7...",
  "facilitator": "pr402",
  "schemes": {
    "exact": {
      "label": "SplitVault (Provider State)",
      "role": "Resource Provider (Seller)",
      "vaultPda": "HStwM...2WF9gvWeJzVB6Nvmt2",
      "solStoragePda": "7jK2...",
      "feeBps": "100",
      "status": "Active",
      "isSovereign": false
    }
  }
}
```

> **`discovery` vs `onboard`**: Use `discovery` for a lightweight, single-scheme lookup of your `payTo` PDA. Use `onboard` to see all schemes at once and to access the full registration lifecycle (challenge-response proof-of-control, DB persistence, sovereign vault creation).

---

## 2. Emitting the HTTP 402 (The Enhanced Challenge)

When an unauthenticated buyer hits your service, respond with an `HTTP 402 Payment Required` carrying the standard x402 JSON body. For `pr402` compatibility, structure the fields using the vault data you cached in Step 1.

### The `pr402` x402 Alterations:
1. **`payTo` becomes the Vault PDA**: Do not put your personal merchant wallet here! Set it to the `vaultPda` from the discovery/onboard response. This ensures autonomous agents correctly format the transaction without accidentally misderiving nested PDAs.
2. **`extra.merchantWallet`**: Move your original merchant wallet address into the nested `extra` block. The Facilitator requires this context for fallback routing.
3. **`extra` from `/supported`**: Merge the `extra` metadata from the matching `kinds[]` entry in `GET /api/v1/facilitator/supported` into your `accepts[]` line.

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
      "feePayer": "...",
      "programId": "...",
      "configAddress": "..."
    }
  }
]
```
*Note: Your application can construct this dynamically based on any pricing configuration module.*

### Alternative: Use the `/upgrade` Endpoint

If you want to skip PDA derivation entirely, you can post a "naive" 402 body (with your bare wallet as `payTo`) to **`POST /api/v1/facilitator/upgrade`**. The facilitator will return a fully institutional 402 with correct PDAs and `extra` metadata injected. See [Upgrade Endpoint](#upgrade-endpoint) below.

### Advanced: The `sla-escrow` Scheme
While `exact` (UniversalSettle) handles precise pay-per-use, your API can alternatively use the `sla-escrow` scheme. If you offer long-running machine tasks (e.g., video rendering or multi-stage AI reasoning), setting `"scheme": "sla-escrow"` allows agents to lock funds in a time-bound cryptographic escrow at the Facilitator layer.

---

## 3. Proxying the Settlement (`PAYMENT-SIGNATURE`)

After emitting the 402, the autonomous agent will:
1. Contact the Facilitator to build the unsigned transaction.
2. Cryptographically sign the transaction.
3. Hit your original API endpoint again, passing the signed proof in the **`PAYMENT-SIGNATURE`** header (x402 v2).

> **Backward compatibility**: Legacy `X-PAYMENT` is still accepted by most servers. New implementations should read `PAYMENT-SIGNATURE` first, falling back to `X-PAYMENT`.

When you extract this header, you do not need to parse or verify the Solana transaction yourself! Security is offloaded to the Facilitator.

### Forward the Proof
Submit a `POST /api/v1/facilitator/verify` or `/settle` request containing the decoded contents of the `PAYMENT-SIGNATURE` header.

```bash
curl -X POST "$BASE/api/v1/facilitator/settle" \
     -H "Content-Type: application/json" \
     -d '<DECODED_PAYMENT_SIGNATURE_JSON>'
```

If the Facilitator returns HTTP 200, the Institutional SplitVault sweep has executed successfully, funds are securely routing to your designated beneficiary or SplitVault, and you may safely serve the premium resource to the autonomous agent.

### Returning `PAYMENT-RESPONSE` (v2)

After successful settlement, x402 v2 sellers should return a **`PAYMENT-RESPONSE`** header to the buyer. This is a base64-encoded JSON object containing the settlement result (`success`, `transaction`, `network`, `payer`). Buyer agents can inspect this header to confirm on-chain finality without polling.

---

<a id="upgrade-endpoint"></a>

## 4. The Upgrade Endpoint

**`POST /api/v1/facilitator/upgrade`** is a convenience endpoint that transforms a "Lite" 402 response into a fully institutional one.

**What it does:**
- Takes a `PaymentRequired` JSON body where `payTo` is a bare seller wallet address
- Returns a new `PaymentRequired` with:
  - `payTo` replaced by the correct SplitVault PDA (for `exact`) or Escrow PDA (for `sla-escrow`)
  - Full `extra` metadata injected (feePayer, programId, configAddress, oracleAuthorities, etc.)

**When to use it:**
- You are a seller who does not want to derive PDAs locally
- You want the facilitator to populate institutional metadata for you
- You are migrating from a bare wallet `payTo` to the pr402 vault model

**Example:**
```bash
curl -X POST "$BASE/api/v1/facilitator/upgrade" \
  -H "Content-Type: application/json" \
  -d '{"x402Version":2,"accepts":[{"scheme":"exact","network":"solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1","payTo":"YOUR_BARE_WALLET","amount":"50000","asset":"USDC_MINT"}]}'
```

---

## 5. Documentation for Machine Discoverability

To maximize sales to AI agents or roaming Multi-Agent Architectures, standard server conventions dictate that you tell agents **how** to purchase.
You should host an `agent-integration.md` file on your server (accessible via `/public` routing).
Update your web server's `/.well-known/ai-plugin.json` and `/openapi.json` to firmly instruct LLMs:

> *"YOU MUST READ the integration guide at `https://your-api.com/agent-integration.md` to discover how to use the Facilitator API to build the HTTP transaction before prompting the user for signatures."*

By forcing the Agent to use the proxy building API, you eliminate compilation requirements, CLI dependency hell, and blockchain complexity for your buyers!
