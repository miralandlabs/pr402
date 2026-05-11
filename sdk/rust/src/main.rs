//! `pr402-buy` — one-shot buyer CLI for the x402 lifecycle.
//!
//! Thin wrapper around [`pr402_client::X402AgentClient::fetch_with_auto_pay`]:
//! loads a Solana keypair from disk, asks the library to navigate the 402 gate,
//! and prints the paid response body to stdout.
//!
//! ```text
//!   GET  <resource>                              → HTTP 402
//!   POST <facilitator>/build-exact-payment-tx
//!   sign(transaction) locally
//!   GET  <resource>  (retry, PAYMENT-SIGNATURE)  → HTTP 200
//! ```
//!
//! The CLI is seller-agnostic — it understands the x402 v2 protocol, not
//! specific seller APIs. Forward-compatible by construction: all
//! facilitator-specific tx shape (CU budgets, token-program branches, PDA
//! math) comes from the build endpoint, so updates to facilitator policy
//! flow through automatically.

use std::{fs, path::PathBuf, process::ExitCode, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use pr402_client::X402AgentClient;
use solana_sdk::signature::Keypair;
use tokio::time::timeout;

/// One-shot x402 v2 buyer. Walks 402 → build → sign → settle → retry and
/// prints the final response body. Works against any Solana facilitator that
/// speaks the pr402 HTTP surface.
#[derive(Parser, Debug)]
#[command(name = "pr402-buy", version, about, long_about = None)]
struct Cli {
    /// The paid URL. `pr402-buy` issues a GET first; a 200 is returned directly,
    /// a 402 kicks off the payment lifecycle.
    #[arg(long)]
    resource: String,

    /// Path to the payer Solana keypair (same shape as `solana-keygen new` output:
    /// JSON array of 64 u8 bytes).
    #[arg(long)]
    payer: PathBuf,

    /// Payment mint (base58) that the buyer wants to settle in. Must match one of
    /// the `accepts[].asset` values in the seller's 402 response. Common devnet
    /// USDC: `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`; native SOL:
    /// `11111111111111111111111111111111`. See the seller's 402 body for the
    /// authoritative list.
    #[arg(long)]
    mint: String,

    /// Whether the facilitator should inject a `syncNative` wrap when paying in
    /// WSOL (advanced; most buyers leave this off).
    #[arg(long, default_value_t = false)]
    auto_wrap_sol: bool,

    /// HTTP request timeout, in seconds. Bounds the full lifecycle, so keep it
    /// generous — Solana settle can take several seconds at peak load.
    #[arg(long, default_value_t = 45)]
    timeout: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pr402-buy: {e:#}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let wallet = load_keypair(&cli.payer)
        .with_context(|| format!("load payer keypair from {}", cli.payer.display()))?;
    let client = X402AgentClient::new(wallet).with_auto_wrap_sol(cli.auto_wrap_sol);

    let fetch = client.fetch_with_auto_pay(&cli.resource, &cli.mint);
    let resp = timeout(Duration::from_secs(cli.timeout), fetch)
        .await
        .map_err(|_| anyhow!("timed out after {}s waiting for paid response", cli.timeout))?
        .map_err(|e| anyhow!("{e}"))?;

    let status = resp.status();
    let body = resp.text().await.context("read paid response body")?;
    if !status.is_success() {
        bail!("resource returned {status} after payment:\n{body}");
    }
    // Final payload to stdout so pipelines can consume it directly.
    println!("{body}");
    Ok(())
}

fn load_keypair(path: &PathBuf) -> Result<Keypair> {
    let raw = fs::read_to_string(path).context("read keypair file")?;
    let bytes: Vec<u8> =
        serde_json::from_str(&raw).context("parse keypair JSON (expected array of 64 u8 bytes)")?;
    if bytes.len() != 64 {
        bail!("keypair file must be 64 bytes, got {}", bytes.len());
    }
    Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow!("invalid keypair bytes: {e}"))
}
