# pr402-client (Rust)

Lightweight [x402 v2](https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md) client for buyer agents on Solana. Ships as one crate with two faces:

- **Library** (`pr402_client::X402AgentClient`) — embed in your own agent, Discord bot, scraper, or RAG loop.
- **CLI** (`pr402-buy`) — single binary for quick tests and scripted pipelines.

The library handles discovery, tx building, local signing, and retry with `PAYMENT-SIGNATURE` automatically. The CLI is a thin wrapper; both share one code path so behavior stays identical.

## Install the CLI

```bash
# From source during pre-release
cargo install --git https://github.com/miralandlabs/pr402 --branch main pr402-client

# After crates.io publication
cargo install pr402-client
```

Installs a binary called `pr402-buy`.

## Use the CLI

```bash
pr402-buy \
  --resource https://some.seller.com/api/thing \
  --payer ~/.config/solana/buyer.json \
  --mint 4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU
```

On success, the paid 200 body is printed to stdout. Pipeline-ready:

```bash
pr402-buy --resource … --payer … --mint … | jq .
```

### Flags

| Flag | Purpose |
|---|---|
| `--resource <url>` | **Required.** The paid URL. `pr402-buy` issues a GET first; 200 returns directly, 402 kicks off the payment lifecycle. |
| `--payer <path>` | **Required.** Solana keypair JSON (same shape as `solana-keygen new` output). |
| `--mint <base58>` | **Required.** Which payment mint the buyer wants to settle in (must match one of the seller's `accepts[].asset` values). |
| `--auto-wrap-sol` | Advanced: have the facilitator inject a `syncNative` wrap when paying in WSOL. Off by default. |
| `--timeout <sec>` | Total lifecycle timeout. Default `45`. |

## Use the library

```toml
[dependencies]
pr402-client = "0.1"
solana-sdk = "2.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use pr402_client::X402AgentClient;
use solana_sdk::{signature::read_keypair_file, signer::Signer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let wallet = read_keypair_file("~/.config/solana/buyer.json")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let client = X402AgentClient::new(wallet);

    let resp = client
        .fetch_with_auto_pay(
            "https://some.seller.com/api/thing",
            "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU", // devnet USDC
        )
        .await?;

    println!("{}", resp.text().await?);
    Ok(())
}
```

The library returns a `reqwest::Response` on success so your code can inspect headers (`PAYMENT-RESPONSE`, correlation IDs, cache hints) before reading the body.

## What this does under the hood

1. `GET <resource>` — expect 402.
2. `POST <facilitator>/build-exact-payment-tx` — the facilitator returns an unsigned `VersionedTransaction` plus a `verifyBodyTemplate` with everything except the signed tx already filled in.
3. Sign the transaction locally at `payerSignatureIndex`.
4. `GET <resource>` with `PAYMENT-SIGNATURE` header — the seller forwards the proof to the facilitator, which verifies + settles in one hop and returns the paid response.

The client never builds a Solana transaction from scratch — it asks the facilitator to build one. That means CU limits, token-program branches, PDA derivations, and fee-payer details all come from the facilitator and stay forward-compatible across facilitator policy changes.

## Supported schemes

- `v2:solana:exact` (UniversalSettle) — today.
- `v2:solana:sla-escrow` — not yet.

## Error handling

The library surfaces actionable errors as `X402Error` variants:

| Variant | What it means | What to do |
|---|---|---|
| `MintNotAccepted` | Your `--mint` isn't in the seller's `accepts[]` | Pick one from the listed `available_mints` |
| `BlockhashExpired` | Build response is too old | Retry; the next build gets a fresh blockhash |
| `RateLimited` | Facilitator 429 | Wait `retry_after_secs` seconds |
| `BuildFailed` | Facilitator 4xx/5xx on build | Inspect `status` + `detail`; usually a config mismatch |
| `UnexpectedStatus` | Seller returned something other than 200 or 402 | Seller error; not a payment issue |

The CLI exits with code 1 and prints the chained error to stderr on any of these.

## License

Apache-2.0
