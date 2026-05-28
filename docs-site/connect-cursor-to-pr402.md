---
title: "Connect Cursor to pr402"
---

# Connect Cursor to pr402

> **Audience:** You use **Cursor** (or another MCP host) and want pr402 buyer/seller tools without writing HTTP glue.

**Package:** [`@pr402/mcp-server@0.1.2`](https://www.npmjs.com/package/@pr402/mcp-server) — stdio MCP adapter over [`@pr402/client`](https://www.npmjs.com/package/@pr402/client).

> **Version pin:** Use **`@pr402/mcp-server@0.1.2`**. Versions **0.1.0** and **0.1.1** are broken — do not use them.

## Install

```bash
npx -y @pr402/mcp-server@0.1.2
```

## Configure Cursor

Create or edit **`.cursor/mcp.json`** in your project (or user-level MCP settings):

```json
{
  "mcpServers": {
    "pr402": {
      "command": "npx",
      "args": ["-y", "@pr402/mcp-server@0.1.2"],
      "env": {
        "PR402_FACILITATOR_URL": "https://preview.ipay.sh",
        "PR402_PAYER_KEYPAIR_JSON": "/absolute/path/to/buyer-keypair.json"
      }
    }
  }
}
```

Restart Cursor after saving. **`PR402_PAYER_KEYPAIR_JSON`** is required only for **`pr402_pay_http_resource`** (auto-pay). Other tools work without it.

Copy-paste example: [x402-buyer-starter `examples/mcp/cursor-mcp.json`](https://github.com/miraland-labs/x402-buyer-starter/blob/main/examples/mcp/cursor-mcp.json).

## Devnet vs Mainnet

Set **`PR402_FACILITATOR_URL`** to the facilitator origin that matches your keypair and the seller you pay:

| Environment | `PR402_FACILITATOR_URL` |
|-------------|-------------------------|
| **Preview (Devnet)** | `https://preview.ipay.sh` |
| **Production (Mainnet)** | `https://ipay.sh` |

Same service is also served at `https://preview.agent.pay402.me` (Devnet) and `https://agent.pay402.me` (Mainnet). Confirm **`solanaNetwork`** with **`GET /api/v1/facilitator/health`** on the host you use.

Use a **funded Devnet keypair** with preview; **Mainnet** keypair and real funds with `ipay.sh`.

## Tools

| Tool | Role |
|------|------|
| `pr402_get_capabilities` | Buyer — facilitator features and `solanaNetwork` |
| `pr402_build_exact_payment` | Buyer — unsigned exact payment tx + `verifyBodyTemplate` |
| `pr402_pay_http_resource` | Buyer — auto-pay an HTTP resource (needs payer keypair) |
| `pr402_seller_preview` | Seller — lifecycle and rail preview |
| `pr402_seller_rail_info` | Seller — `payTo` PDA for a scheme |
| `pr402_seller_provision_tx` | Seller — unsigned Activate tx |
| `pr402_enrich_payment_required` | Seller — enrich naive 402 → institutional shape |

Machine-readable catalog: **`GET /agent-tools.json`** on your facilitator host — e.g. [preview.ipay.sh/agent-tools.json](https://preview.ipay.sh/agent-tools.json).

## Further reading

- [Agent integration runbook](/agent-integration) — full buyer/seller narrative; live copy at [`/agent-integration.md`](https://preview.ipay.sh/agent-integration.md)
- [@pr402/mcp-server on npm](https://www.npmjs.com/package/@pr402/mcp-server)
- [Buyer Quickstart](/quickstart-buyer) — protocol steps under the MCP tools

## Try it (Devnet)

After configuring a funded Devnet keypair, ask Cursor to call **`pr402_get_capabilities`** or **`pr402_pay_http_resource`** against a preview seller URL.
