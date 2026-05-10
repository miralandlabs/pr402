# Changelog

All notable changes to `@miraland-labs/pr402-client` are documented here.

## 0.3.0

### Fixed (behavior change)

- **`X402AgentClient.fetchWithAutoPay` now sends the settlement proof on the
  retry request as `PAYMENT-SIGNATURE` (x402 v2) instead of `X-PAYMENT` (x402 v1).**
  Every seller in the pr402 ecosystem — `x402-seller-starter`, `aethervane`,
  `spl-token-balance-serverless` — reads only `PAYMENT-SIGNATURE`, so the old
  `X-PAYMENT` emit silently failed against every real seller and the client
  would retry forever with repeated 402s. This is a behavior fix: any code
  paths that were working already kept working (sellers here never read
  `X-PAYMENT`), and any code path that was silently failing now succeeds.

  The value shape is unchanged (base64(UTF-8 JSON) of the signed
  `verifyBodyTemplate`). Sellers accept both raw-JSON and base64 in that
  header per the x402 HTTP transport-v2 spec; this client continues to emit
  base64 for URL-safety.

### Added

- **`pr402-buy` CLI.** `npx @miraland-labs/pr402-client pr402-buy …` walks the
  full x402 lifecycle (fetch 402 → build → sign → settle → retry) against
  any seller URL. Seller-agnostic; uses the same `X402AgentClient` the
  library exposes so the CLI and importable API evolve together.

## 0.2.0

- Published with `X402AgentClient` for autonomous agent use.
