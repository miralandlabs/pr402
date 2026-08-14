# Changelog

## 0.1.4

- Package discovery support directly so published installs no longer depend on an unpublished local `file:` package.
- Keep the discovery MCP tools and their public behavior unchanged.

## 0.1.3

- Default `PR402_FACILITATOR_URL` is **`https://ipay.sh`** (Mainnet). Set `https://preview.ipay.sh` for Devnet.
- MCP server `version` reads from `package.json` (no drift).
- `pr402_pay_http_resource`: validate keypair shape; return `isError` on failures instead of crashing the process.

## 0.1.2

- Prior release.
