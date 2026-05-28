# Changelog

## 0.2.0 — 2026-05-23

### Breaking (HTTP seller paths)

Seller lifecycle endpoints were renamed under `/api/v1`. The previous paths are removed (no aliases, no sunset window).

| Removed | Replacement |
|---------|-------------|
| `GET /api/v1/facilitator/onboard?wallet=` | `GET /api/v1/facilitator/sellers/{wallet}/preview` |
| `GET /api/v1/facilitator/discovery?wallet=&scheme=` | `GET /api/v1/facilitator/sellers/{wallet}/rails/{scheme}` |
| `POST /api/v1/facilitator/onboard/provision` | `POST /api/v1/facilitator/sellers/provision-tx` |
| `POST /api/v1/facilitator/upgrade` | `POST /api/v1/facilitator/payment-required/enrich` |
| `GET /api/v1/facilitator/onboard/challenge?wallet=` | `GET /api/v1/facilitator/sellers/{wallet}/challenge` |
| `POST /api/v1/facilitator/onboard` (registry) | `POST /api/v1/facilitator/sellers/{wallet}/register` |
| `POST /api/v1/facilitator/onboard/retire` | `POST /api/v1/facilitator/sellers/{wallet}/retire` |

`POST /sellers/{wallet}/register` and `POST /sellers/{wallet}/retire` treat the URL path segment as the canonical wallet. The request body `wallet` field is now optional; when present it must equal the path wallet or the request is rejected with `400`.

### Capabilities schema 1.1.0

- `schemaVersion` / `X-Schema-Version`: **1.1.0**
- New optional `sellerEndpointGuide` (static decision matrix mirrored at `/seller-endpoint-guide.json`)
- `httpEndpoints.onboardPreview` → `httpEndpoints.sellerPreview`
- `httpEndpoints.onboardChallenge` → `httpEndpoints.sellerChallenge`
- `httpEndpoints.onboard` → `httpEndpoints.sellerRegister`
- `httpEndpoints.onboardProvision` → `httpEndpoints.sellerProvisionTx`
- `httpEndpoints.onboardRetire` → `httpEndpoints.sellerRetire`
- `httpEndpoints.discovery` → `httpEndpoints.sellerRailInfo`
- `httpEndpoints.upgrade` → `httpEndpoints.paymentRequiredEnrich`

### Unchanged

Buyer pipeline (`/verify`, `/settle`, `/build-*-payment-tx`) is unchanged.

### New packages

- `@pr402/mcp-server` 0.1.0 — MCP tools wrapping `@pr402/client` HTTP (no Rust MCP code). Publish: `cd sdk/mcp && npm publish --access public` (see `scripts/publish-mcp.sh`). Discovery: `GET /agent-tools.json`.

## @pr402/mcp-server 0.1.1 — 2026-05-28

### Fixed

- Tool `inputSchema` uses **Zod** raw shapes (required by `@modelcontextprotocol/sdk` ≥ 1.29). Fixes startup crash: `inputSchema must be a Zod schema or raw shape`.

## @pr402/mcp-server 0.1.2 — 2026-05-28

### Fixed

- `registerToolLoose` calls `(server as any).registerTool` to bypass TS2589 on all TypeScript versions while keeping Zod runtime validation.
