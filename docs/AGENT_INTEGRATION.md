# Agent integration (pr402 facilitator)

The buyer runbook is **[`public/agent-integration.md`](../public/agent-integration.md)** — static asset served at **`GET /agent-integration.md`** (same pattern as **`GET /openapi.json`**). Optional redirect: **`GET /api/v1/facilitator/agent-integration.md`**.

**All in-repo docs:** [`docs/README.md`](./README.md)

**Status:** pr402 is live on Solana Mainnet and Devnet; behavior and feature flags can evolve — `GET /capabilities` and `GET /openapi.json` on the host you call are the live contract.

| | **Production (Mainnet)** | **Preview (Devnet)** |
|--|--------------------------|----------------------|
| **Recommended** | `https://ipay.sh` | `https://preview.ipay.sh` |
| **Also (same APIs; not deprecated)** | `https://agent.pay402.me` | `https://preview.agent.pay402.me` |

Edit the Markdown under **`public/`** only. Confirm **`solanaNetwork`** on the host you integrate against: **`GET /api/v1/facilitator/health`**. For wallet RPC, use **`solanaWalletRpcUrl`** from that response — do not copy URLs from docs.
