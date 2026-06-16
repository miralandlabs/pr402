# Agent resource discovery (pr402)

**Sellers:** use the **6-step go-live** on [ipay.sh](https://ipay.sh/#seller-lifecycle) and [/resources](https://ipay.sh/resources) — do not read this doc unless you are integrating agents or debugging listing gates.

Three layers — do not collapse them (developer reference):

| Layer | Entity | API / artifact |
|-------|--------|----------------|
| 1 | Facilitator | `GET /capabilities`, verify/settle/build |
| 2 | Merchant identity | `resource_providers`, `GET /providers` (origins only) |
| 3 | Payable resources | `payable_resources`, `GET /resources`, SRM, `resource-index.json` |

**Authoritative pricing** always comes from live HTTP 402 on each `resourceUrl`. Search metadata is advisory for agent routing.

## Merchant origins (`GET /providers`)

Returns verified, opted-in **merchant rows** (`walletPubkey`, `serviceUrl`, tags, …). `serviceUrl` is the seller **origin** used for manifest harvest and origin binding — **not** a single payable API endpoint.

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

Single-resource lookup: `GET /api/v1/facilitator/resources/{id}` (same visibility filters; 404 when not public).

## Directory stats (`GET /directory/stats`)

Aggregate counts for registry dashboards (same visibility as list endpoints — no client-side pagination):

```json
{
  "network": "mainnet",
  "providers": { "total": 42 },
  "resources": { "total": 128, "byScheme": { "exact": 100, "sla-escrow": 28 } },
  "asOf": "2026-06-15T14:00:00Z"
}
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

UI entry: [`/resources`](../public/resources/index.html) — **step 5 of 6** in the seller go-live path (steps 2–3 on the home page).

## Seller Resource Manifest (SRM)

Seller-owned catalog at `{origin}/.well-known/x402-resources.json`. See [SRM.md](./SRM.md).

## Static index

[`public/dist/resource-index.json`](../public/dist/resource-index.json) — built by [`discovery-indexer/index.mjs`](../discovery-indexer/index.mjs) from public `payable_resources` rows.

## Scheduling

[`.github/workflows/discovery-indexer-cron.yml`](../.github/workflows/discovery-indexer-cron.yml) runs the indexer hourly against preview then production (gated behind preview to stagger load). The `--harvest` pass re-probes every listed `resourceUrl` and writes `last_probe_ok` to Postgres, so liveness in `GET /resources` stays fresh between deploys. Requires `PR402_PREVIEW_DATABASE_URL` / `PR402_DATABASE_URL` secrets (alongside the existing `PR402_PREVIEW_BASE_URL` / `PR402_BASE_URL`); jobs skip cleanly until those are set.

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
