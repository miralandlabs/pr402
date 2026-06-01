# @pr402/mcp-server

[MCP](https://modelcontextprotocol.io/) adapter for the [pr402](https://github.com/miralandlabs/pr402) **Solana** x402 facilitator. Exposes buyer auto-pay and seller lifecycle tools to Cursor, Claude Desktop, and any MCP host over **stdio**.

Tools are thin HTTP wrappers around the facilitator REST API via [`@pr402/client`](https://www.npmjs.com/package/@pr402/client) — **no MCP logic lives in the Rust facilitator**.

Machine-readable tool list: **`GET /agent-tools.json`** on any pr402 deployment (e.g. [mainnet](https://ipay.sh/agent-tools.json), [devnet preview](https://preview.ipay.sh/agent-tools.json)).

## Install

```bash
npm install -g @pr402/mcp-server
# or run without a global install:
npx -y @pr402/mcp-server
```

**Monorepo / from source:**

```bash
cd pr402/sdk/mcp && npm ci && npm run build
node dist/server.js
```

## Run (stdio)

An MCP server — not a CLI. It speaks JSON-RPC on **stdin/stdout**; there is no `--help` flag. Configure it in Cursor or Claude Desktop (below), or run manually:

```bash
export PR402_PAYER_KEYPAIR_JSON=/path/to/keypair.json  # pr402_pay_http_resource only
npx -y @pr402/mcp-server
```

Defaults to **`https://ipay.sh`** (Mainnet). For Devnet preview:

```bash
export PR402_FACILITATOR_URL=https://preview.ipay.sh
```

## Environment

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `PR402_FACILITATOR_URL` | No | `https://ipay.sh` | Facilitator origin (Mainnet). Use `https://preview.ipay.sh` for Devnet preview. |
| `PR402_PAYER_KEYPAIR_JSON` | For `pr402_pay_http_resource` only | — | Path to Solana keypair JSON (64-byte array file). Never commit this file. |

## Cursor config

Project file **`.cursor/mcp.json`** (or user-level MCP settings):

```json
{
  "mcpServers": {
    "pr402": {
      "command": "npx",
      "args": ["-y", "@pr402/mcp-server"],
      "env": {
        "PR402_PAYER_KEYPAIR_JSON": "/absolute/path/to/buyer-keypair.json"
      }
    }
  }
}
```

Copy-paste example: [x402-buyer-starter `examples/mcp/cursor-mcp.json`](https://github.com/miraland-labs/x402-buyer-starter/blob/main/examples/mcp/cursor-mcp.json).

**Local dev (unpublished build):**

```json
{
  "mcpServers": {
    "pr402": {
      "command": "node",
      "args": ["/absolute/path/to/pr402/sdk/mcp/dist/server.js"],
      "env": {
        "PR402_PAYER_KEYPAIR_JSON": "/absolute/path/to/buyer-keypair.json"
      }
    }
  }
}
```

## Claude Desktop config

Add under `claude_desktop_config.json` → `mcpServers` (same shape as Cursor; use `npx -y @pr402/mcp-server` as the command).

## Tools

| Tool | HTTP / behavior |
|------|-----------------|
| `pr402_get_capabilities` | `GET /capabilities` |
| `pr402_build_exact_payment` | `POST /build-exact-payment-tx` |
| `pr402_pay_http_resource` | Composite `@pr402/client.fetchWithAutoPay` (needs payer keypair) |
| `pr402_seller_preview` | `GET /sellers/{wallet}/preview` |
| `pr402_seller_rail_info` | `GET /sellers/{wallet}/rails/{scheme}` |
| `pr402_seller_provision_tx` | `POST /sellers/provision-tx` |
| `pr402_enrich_payment_required` | `POST /payment-required/enrich` |

## Resources

| URI | Content |
|-----|---------|
| `pr402://capabilities` | Live facilitator capabilities JSON |
| `pr402://openapi` | OpenAPI 3.1 contract |
| `pr402://agent-integration` | Buyer/seller runbook |
| `pr402://payto-semantics` | `payTo` PDA semantics |

## Try it

After configuring a funded keypair on the target cluster:

1. Ask your MCP host to call **`pr402_pay_http_resource`** with a seller URL on the same cluster as your facilitator (Mainnet by default).
2. Or call **`pr402_get_capabilities`** to confirm `solanaNetwork` and feature flags.

For Devnet, set `PR402_FACILITATOR_URL=https://preview.ipay.sh` and use preview sellers.

Runbook: [`/agent-integration.md`](https://ipay.sh/agent-integration.md) · Buyer quick start: [`/quickstart-buyer.md`](https://ipay.sh/quickstart-buyer.md).

## Other agent stacks

| Stack | Package |
|-------|---------|
| **MCP hosts** (this package) | `npm i @pr402/mcp-server` |
| **Node scripts / embed** | `npm i @pr402/client` (`pr402-buy` CLI + `X402AgentClient`) |
| **Python LangChain** | [`langchain-pr402`](https://pypi.org/project/langchain-pr402/) on PyPI |
| **Rust** | [`pr402-client`](https://crates.io/crates/pr402-client) on crates.io |

## Maintainers — publish to npm

From a clean tree with npm registry access to the `@pr402` scope:

```bash
cd pr402/sdk/mcp
npm ci
npm run build
npm publish --access public
```

Requires `@pr402/client@^0.3.0` on npm. Bump `version` in `package.json` for each release.

## License

Apache-2.0
