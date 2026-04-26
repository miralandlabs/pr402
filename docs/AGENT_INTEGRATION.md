# Agent integration (pr402 facilitator)

The buyer runbook is **[`public/agent-integration.md`](../public/agent-integration.md)** — static asset served at **`GET /agent-integration.md`** (same pattern as **`GET /openapi.json`**). Optional redirect: **`GET /api/v1/facilitator/agent-integration.md`**.

**Launch phase:** **Experimental** — **use at your own risk**.

| | **Production** | **Preview** |
|--|----------------|-------------|
| Base URL | `https://agent.pay402.me` | `https://preview.agent.pay402.me` |

Edit the Markdown under **`public/`** only. Confirm **`solanaNetwork`** on the host you integrate against: **`GET /api/v1/facilitator/health`**. For wallet RPC, use **`solanaWalletRpcUrl`** from that response — do not copy URLs from docs.
