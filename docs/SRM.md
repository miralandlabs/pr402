# Seller Resource Manifest (SRM)

Machine-readable catalog of **payable HTTP resources** at a merchant origin. One manifest, many resources — not one row per SKU.

## Location

Default: `{origin}/.well-known/x402-resources.json`

Example: `https://spl-token.hashspace.me/.well-known/x402-resources.json`

## Schema version

Current: **`0.1.0`** (JSON Schema: [`public/x402-resources.schema.json`](../public/x402-resources.schema.json))

## Document shape

```json
{
  "schemaVersion": "0.1.0",
  "origin": "https://spl-token.hashspace.me",
  "merchantWallet": "…",
  "facilitatorHint": "https://ipay.sh",
  "resources": [
    {
      "id": "buy-spl-token",
      "title": "Purchase SPL tokens (sla-escrow)",
      "description": "USDC escrow → SPL delivery → oracle release",
      "useCase": "When an agent needs to buy a catalogued SPL token to a recipient wallet",
      "category": "finance",
      "method": "GET",
      "resourceUrl": "https://spl-token.hashspace.me/api/v1/buy-spl-token",
      "scheme": "sla-escrow",
      "intentContractUrl": "https://spl-token.hashspace.me/api/v1/buy-spl-token/intent-contract",
      "tags": ["spl", "escrow", "tokens"]
    }
  ]
}
```

## Multi-SKU rule

The global pr402 index lists **entry-point resource URLs** only. Parameterized offers (mints, amounts, recipients) stay in seller-local catalogs and **intent-contract** documents — see [`x402-buy-spl-token`](../../x402-buy-spl-token) `GET /api/v1/buy-spl-token/intent-contract`.

## Authoritative pricing

Search metadata is advisory. **Live HTTP 402** on `resourceUrl` (`PaymentRequired.resource.url` + `accepts[]`) is always authoritative.

## Harvest pipeline

The [discovery indexer](../scripts/discovery-indexer/index.mjs) reads public merchant origins from `GET /providers`, fetches each SRM, upserts `payable_resources` with `source = manifest_harvest`, and runs a 402 probe before public listing.

Human sellers can also register resources at [`/resources`](../public/resources/index.html) (wallet-signed API).

## Related docs

- [DISCOVERY.md](./DISCOVERY.md) — three-layer model (facilitator / merchant / payable resource)
- [SELLER_INTEGRATION.md](./SELLER_INTEGRATION.md) — seller HTTP 402 wiring
