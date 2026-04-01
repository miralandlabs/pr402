# Agent integration (pr402 facilitator)

Short runbook for **buyer-side** agents (wallets, OpenClaw-style orchestrators, or custom HTTP clients) and **resource providers** (sellers).

**Canonical contract:** OpenAPI 3.1 at **`GET /openapi.json`** on your facilitator base URL (e.g. `https://preview.pr402.signer-payer.me/openapi.json`).

---

## 0. Resource Provider Onboarding (Sellers)

If you are a Resource Provider (Seller) wanting to join the X402 ecosystem and achieve **Sovereign** status (95 bps fee tier):

1.  **Discover Rules**: Read the [Onboarding Guide](/onboarding_guide.md) to understand the **Two Paths** (Sovereign vs. Facilitated).
2.  **Step 2: Protocol Onboarding (On-Chain Provisioning)**:
    - **CLI**: `universalsettle create-vault --seller <YOUR_PUBKEY>`
    - **API (Agent-Native)**:
        1. **Build**: `GET /api/v1/facilitator/onboard/build-tx?wallet=<YOUR_PUBKEY>`
        2. **Sign**: Use your private key to sign the returned `VersionedTransaction` (base64 bincode).
        3. **Send**: Broadcast to the Solana network.
    - **Institutional Incentive**: Proactively creating your vault earns you a **5 bps (0.05%) fee discount** for life at the protocol level.
3.  **Step 3: Verification**: Use the Onboard API to check your current registration and provisioning status:
    ```bash
    curl -sS "https://<facilitator-url>/api/v1/facilitator/onboard?wallet=<YOUR_PUBKEY>" | jq .
    ```
4.  **Step 4: Facilitator Registry (Off-Chain Discovery)**: For persisted database registration (highly recommended for high-volume traders and discovery hubs), use the Challenge flow in `/openapi.json` (`/onboard/challenge` + `POST /onboard`). This persists your metadata in the facilitator's database.

---

## 1. Discover (Buyers)

```bash
BASE="https://preview.pr402.signer-payer.me"
curl -sS "$BASE/api/v1/facilitator/supported" | jq .
# or
curl -sS "$BASE/api/v1/facilitator/capabilities" | jq .
```

Pick one `kinds[]` entry that matches the **402 `accepts[]`** line your resource returned (`scheme`, `network`, `asset`, `amount`, `payTo`, `extra`).

---

## 2. Build unsigned tx

Two build endpoints; **do not** confuse them.

| `accepts[].scheme` | Endpoint | Who signs before verify |
|--------------------|----------|-------------------------|
| `exact` | `POST /api/v1/facilitator/build-exact-payment-tx` | Payer signs token authority; facilitator is fee payer at settle (default). |
| `sla-escrow` | `POST /api/v1/facilitator/build-sla-escrow-payment-tx` | Buyer partial sign; facilitator completes fee payer at settle (default). |

Request bodies are in **`openapi.json`** (`BuildExactPaymentTxRequest`, `BuildSlaEscrowPaymentTxRequest`). Response includes **`verifyBodyTemplate`** and base64 **`transaction`** (bincode `VersionedTransaction`, unsigned).

---

## 3. Sign locally

Use your stack’s Solana signer. Replace **`paymentPayload.payload.transaction`** in `verifyBodyTemplate` with the **signed** tx base64. Keep **`accepted`** identical to **`paymentRequirements`**.

---

## 4. Verify and settle

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

---

## 5. Agentic Interaction Checklist (Sellers)

For autonomous agents performing proactive onboarding, ensure your logic follows this integration pattern:

1. **Discovery**: Call `GET /api/v1/facilitator/capabilities`. Confirm `features.buildOnboardTx` is `true`.
2. **Current State**: Call `GET /api/v1/facilitator/onboard?wallet=<PUBKEY>`. 
   - If `isSovereign: true`, skip onboarding (already at 95 bps).
3. **Construction**: Call `GET /api/v1/facilitator/onboard/build-tx?wallet=<PUBKEY>`.
4. **Local Signing**:
   - Deserialize the `transaction` (base64 bincode).
   - Sign it with your `seller` private key.
5. **Broadcasting**: Send the signed transaction to a Solana RPC (use `recentBlockhash` from the response).
6. **Validation**: Re-call `GET /api/v1/facilitator/onboard?wallet=<PUBKEY>`. Verify `isSovereign: true` and `feeBps: "95"`.

---

## 6. Technical Specs

- x402 v2: [x402-specification-v2.md](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md)
- Facilitator HTTP: **`/openapi.json`** and Markdown runbook **`/agent-integration.md`** on the deployment
