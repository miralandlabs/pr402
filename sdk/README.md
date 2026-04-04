# pr402-client SDKs

Lightweight client libraries for **autonomous AI agents** to discover, build, sign, and settle x402 payments through the pr402 Facilitator — without any blockchain-specific knowledge.

## Design Philosophy

> **Simple is Best, yet Elegant.**

An agent only needs a wallet keypair and a target URL. The SDK handles the entire 402 lifecycle:

```
GET target → 402 Challenge → Facilitator builds tx → Agent signs → Retry with proof → 200 OK
```

## Available SDKs

| SDK | Path | Runtime |
|-----|------|---------|
| **Rust** | `sdk/rust/` | `cargo add pr402-client` |
| **TypeScript** | `sdk/ts/` | `npm install @miraland-labs/pr402-client` |

Both expose a single entry point: `fetchWithAutoPay(url, preferredMint)`.

## Rust Usage

```rust
use pr402_client::X402AgentClient;
use solana_sdk::signature::read_keypair_file;

let kp = read_keypair_file("wallet.json").unwrap();
let client = X402AgentClient::new(kp);

let res = client.fetch_with_auto_pay(
    "https://api.example.com/v1/data?key=value",
    "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU", // devnet USDC
).await?;

println!("{}", res.text().await?);
```

## TypeScript Usage

```typescript
import { Keypair } from '@solana/web3.js';
import { X402AgentClient } from '@miraland-labs/pr402-client';

const wallet = Keypair.fromSecretKey(/* ... */);
const client = new X402AgentClient(wallet);

const res = await client.fetchWithAutoPay(
  'https://api.example.com/v1/data?key=value',
  '4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU'
);

console.log(await res.json());
```

## How It Works

1. **Discovery** — The agent sends a normal `GET` request to the target URL.
2. **Challenge** — If the server responds with `HTTP 402`, the SDK parses the `accepts[]` array and selects the pricing tier matching your `preferredMint`.
3. **Delegation** — The SDK reads `extra.capabilitiesUrl` from the 402 body and sends a `POST /build-exact-payment-tx` to the pr402 Facilitator. The Facilitator handles all PDA derivation, rent computation, and instruction ordering.
4. **Signing** — The SDK deserializes the unsigned `VersionedTransaction`, locates the agent's pubkey index, and signs with Ed25519.
5. **Settlement** — The signed proof is base64-encoded into the `X-PAYMENT` header and the original request is replayed.

## Relationship to `sdk/facilitator-build-tx.ts`

The existing [`facilitator-build-tx.ts`](./facilitator-build-tx.ts) is a lower-level HTTP helper that exposes individual Facilitator endpoints (build, verify, settle, capabilities). The new `ts/` and `rust/` SDKs are higher-level **agent orchestrators** that compose those steps into a single `fetchWithAutoPay` call.
