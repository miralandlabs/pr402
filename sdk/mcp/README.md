# @pr402/mcp-server

MCP adapter for the [pr402](https://github.com/miralandlabs/pr402) facilitator. Tools are thin HTTP wrappers around the REST API via `@pr402/client` — **no MCP logic lives in the Rust facilitator**.

## Install

```bash
cd pr402/sdk/mcp && npm ci && npm run build
```

## Run (stdio)

```bash
export PR402_FACILITATOR_URL=https://preview.ipay.sh
export PR402_PAYER_KEYPAIR_JSON=/path/to/keypair.json  # for pr402_pay_http_resource only
npx pr402-mcp
```

## Tools

| Tool | HTTP |
|------|------|
| `pr402_get_capabilities` | `GET /capabilities` |
| `pr402_build_exact_payment` | `POST /build-exact-payment-tx` |
| `pr402_pay_http_resource` | composite `@pr402/client.fetchWithAutoPay` |
| `pr402_seller_preview` | `GET /sellers/{wallet}/preview` |
| `pr402_seller_rail_info` | `GET /sellers/{wallet}/rails/{scheme}` |
| `pr402_seller_provision_tx` | `POST /sellers/provision-tx` |
| `pr402_enrich_payment_required` | `POST /payment-required/enrich` |

## Resources

- `pr402://capabilities`
- `pr402://openapi`
- `pr402://agent-integration`
- `pr402://payto-semantics`

## Cursor / Claude Desktop config snippet

```json
{
  "mcpServers": {
    "pr402": {
      "command": "node",
      "args": ["/absolute/path/to/pr402/sdk/mcp/dist/server.js"],
      "env": {
        "PR402_FACILITATOR_URL": "https://preview.ipay.sh"
      }
    }
  }
}
```
