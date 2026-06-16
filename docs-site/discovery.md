# Agent resource discovery

**Sellers:** use the [6-step go-live](/start-here) on [ipay.sh](https://ipay.sh/#seller-lifecycle) and [/resources](https://ipay.sh/resources) — this page is for **agents**, registry integrators, and debugging listing gates.

Three layers — do not collapse them:

| Layer | Entity | API / artifact |
|-------|--------|----------------|
| 1 | Facilitator | `GET /capabilities`, verify/settle/build |
| 2 | Merchant identity | `resource_providers`, `GET /providers` (origins only) |
| 3 | Payable resources | `payable_resources`, `GET /resources`, SRM, `resource-index.json` |

**Authoritative pricing** always comes from live HTTP 402 on each `resourceUrl`. Search metadata is advisory for agent routing.

## Public registry UI

Browse listed resources and sellers at **[registry.pr402.org](https://registry.pr402.org)** (human-friendly directory; same data as the JSON APIs below).

## Merchant origins (`GET /providers`)

Returns verified, opted-in **merchant rows** (`walletPubkey`, `serviceUrl`, tags, …). `serviceUrl` is the seller **origin** used for manifest harvest and origin binding — **not** a single payable API endpoint.

```http
GET {BASE}/api/v1/facilitator/providers?limit=50&cursor=
GET {BASE}/api/v1/facilitator/providers/{wallet}
```

## Payable resources (`GET /resources`)

Public search over **probe-approved** resource rows:

| Query | Behavior |
|-------|----------|
| `q` | Search title, description, useCase, tags |
| `category` | Exact category filter |
| `scheme` | `exact` or `sla-escrow` |
| `tag` | Tag filter |
| `limit`, `cursor` | Pagination (cursor = last `updatedAt`) |

Gate for public listing:

1. Verified merchant (Layer 2)
2. Wallet-signed register
3. `listingOptIn: true`
4. `lastProbeOk: true` (automated 402 probe)

```http
GET {BASE}/api/v1/facilitator/resources?limit=50
GET {BASE}/api/v1/facilitator/resources/{id}
```

Single-resource lookup uses numeric `id` only; **404** when not publicly listed (same visibility filters as list).

## Directory stats (`GET /directory/stats`)

Aggregate counts for registry dashboards (same visibility as list endpoints):

```json
{
  "network": "mainnet",
  "providers": { "total": 42 },
  "resources": { "total": 128, "byScheme": { "exact": 100, "sla-escrow": 28 } },
  "asOf": "2026-06-15T14:00:00Z"
}
```

```http
GET {BASE}/api/v1/facilitator/directory/stats
```

## Resource registration (Layer 3)

Separate from merchant `POST /sellers/{wallet}/register`:

| Method | Path |
|--------|------|
| GET | `/api/v1/facilitator/resources/register/challenge?wallet=` |
| POST | `/api/v1/facilitator/resources/register` |
| POST | `/api/v1/facilitator/resources/retire` |
| GET/POST | `/api/v1/facilitator/sellers/{wallet}/resources` (signed) |
| POST | `/api/v1/facilitator/resources/probe` (signed) |

UI entry: [ipay.sh/resources](https://ipay.sh/resources) — step 5 of 6 in the seller go-live path.

## Seller Resource Manifest (SRM)

Seller-owned catalog at `{origin}/.well-known/x402-resources.json`.

## Static index

`{BASE}/dist/resource-index.json` — built from public `payable_resources` rows (see OpenAPI and `@pr402/discovery`).

## Agent manifest (`GET /capabilities`)

Schema `1.2.0` adds optional pointers:

- `agentManifest.resourceSearch`
- `agentManifest.resourceRegister`
- `agentManifest.resourceIndex`
- `agentManifest.merchantOrigins`
- `agentManifest.srmSpec`
- `features.publicResourceDirectory`

## Client libraries

- `@pr402/discovery` — `searchResources`, `getResource`, `probeResource`
- MCP tools: `pr402_search_resources`, `pr402_probe_resource`

See also [API reference](/api-reference) and live **`GET {BASE}/openapi.json`**.
