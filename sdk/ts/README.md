# @pr402/client

Lightweight [x402 v2](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md) client for buyer agents on Solana. Ships as one package with two faces:

- **Library** (`X402AgentClient`) — drop into any Node project that needs to call paid HTTP endpoints.
- **CLI** (`pr402-buy`) — single binary for quick tests, scripted pipelines, and one-off agents.

Both share one code path, so behavior is identical.

**Related packages:** [`@pr402/mcp-server`](https://www.npmjs.com/package/@pr402/mcp-server) for Cursor / Claude Desktop (MCP stdio) · [`langchain-pr402`](https://pypi.org/project/langchain-pr402/) for Python LangChain agents · machine-readable MCP catalog: `GET /agent-tools.json` on your facilitator host.

## Install

Library and CLI both install from the same package:

```bash
# Project dependency
npm install @pr402/client

# Global CLI
npm install -g @pr402/client

# Or run without installing
npx @pr402/client pr402-buy --help
```

## Use the CLI

```bash
pr402-buy \
  --resource https://some.seller.com/api/thing \
  --payer ~/.config/solana/buyer.json \
  --mint 4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU
```

On success the paid body is printed to stdout. Pipeline-ready:

```bash
pr402-buy -r … -p … -m … | jq .
```

### Flags

| Flag | Purpose |
|---|---|
| `--resource, -r <URL>` | **Required.** Paid URL. `pr402-buy` GETs first; 200 returns directly, 402 kicks off the payment loop. |
| `--payer, -p <PATH>` | **Required.** Solana keypair JSON (array of 64 bytes, same shape as `solana-keygen new`). |
| `--mint, -m <PUBKEY>` | **Required.** Base58 mint to pay with — must match one of `accepts[].asset` from the 402. |
| `--auto-wrap-sol` | Advanced: inject WSOL wrap instructions. Off by default. |
| `--verbose, -v` | Print bodies at each step on stderr. |
| `--help, -h` | Usage info. |

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Resource fetched successfully. |
| `1` | Usage / flag error. |
| `2` | Network or HTTP transport failure. |
| `3` | Protocol-level rejection (facilitator or seller returned a definitive error). |

## Use the library

```ts
import { X402AgentClient, X402Error } from "@miraland-labs/pr402-client";
import { Keypair } from "@solana/web3.js";
import * as fs from "node:fs";

const bytes = JSON.parse(fs.readFileSync("/path/to/keypair.json", "utf8"));
const wallet = Keypair.fromSecretKey(new Uint8Array(bytes));
const client = new X402AgentClient(wallet);

try {
  const res = await client.fetchWithAutoPay(
    "https://some.seller.com/api/thing",
    "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU", // devnet USDC
  );
  const data = await res.json();
  console.log(data);
} catch (e) {
  if (e instanceof X402Error && e.code === "MINT_NOT_ACCEPTED") {
    console.error("Available mints:", e.availableMints);
  } else {
    throw e;
  }
}
```

The method returns a standard `Response` so you can inspect headers (`PAYMENT-RESPONSE`, correlation ids, caches) before reading the body.

## What this does under the hood

1. `GET <resource>` — expect 402.
2. `POST <facilitator>/build-exact-payment-tx` — receive an unsigned `VersionedTransaction` plus a `verifyBodyTemplate` with everything except the signed tx pre-filled.
3. Sign the transaction locally at `payerSignatureIndex`.
4. `GET <resource>` with `PAYMENT-SIGNATURE` header — the seller forwards the proof to the facilitator, which verifies and settles in one hop, and returns the paid response.

The client never builds a Solana transaction from scratch — the facilitator does. CU limits, token-program branches, PDA derivations, and fee-payer details come from the facilitator's build response, so buyer code stays forward-compatible across facilitator policy changes.

## Error codes

All protocol-level errors come as `X402Error` with a typed `code` field:

| Code | Meaning |
|---|---|
| `MINT_NOT_ACCEPTED` | Preferred mint isn't in the seller's `accepts[]`. `e.availableMints` lists the options. |
| `BLOCKHASH_EXPIRED` | Build response is too old; request a fresh build. `e.expiresAt` is the Unix timestamp. |
| `RATE_LIMITED` | Facilitator returned 429. `e.retryAfterSecs` indicates the wait. |
| `BUILD_FAILED` | Facilitator rejected the build request. `e.httpStatus` + the message carry the reason. |
| `MISSING_CAPABILITIES_URL` | Seller didn't integrate with a facilitator; contact them. |
| `MISSING_ACCEPTS` / `MISSING_VERIFY_TEMPLATE` / `MISSING_TRANSACTION` | Seller or facilitator configuration issue. |
| `UNEXPECTED_STATUS` | Seller returned something other than 200 or 402. |
| `TRANSPORT` | Network or serialization error. |

## Supported schemes

- `v2:solana:exact` (UniversalSettle) — today.
- `v2:solana:sla-escrow` — not yet.

## License

Apache-2.0
