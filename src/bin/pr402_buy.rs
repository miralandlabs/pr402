//! `pr402-buy` — one-shot buyer CLI for the x402 lifecycle.
//!
//! What it does, end-to-end, against any seller URL:
//!
//! ```text
//!   GET  <seller>                         → expect HTTP 402 + accepts[]
//!   POST <facilitator>/build-exact-payment-tx
//!   sign(transaction) locally
//!   POST <facilitator>/verify   (optional; for correlationId linkage)
//!   POST <facilitator>/settle
//!   GET  <seller>                         (retry, with PAYMENT-SIGNATURE)
//! ```
//!
//! The CLI is deliberately **seller-agnostic**: it understands HTTP 402 + x402 v2 JSON, not
//! specific seller APIs. Drop-in for any deployment that implements the protocol; use
//! `--facilitator` to point at a different host than the one the seller documents (only
//! recommended for mirror endpoints — do not silently swap preview vs mainnet).
//!
//! Supported scheme: **exact** (UniversalSettle) on v2. Escrow support is next.
//!
//! Build:
//! ```bash
//! cargo build --bin pr402-buy --features facilitator-http --release
//! ```

use std::{fs, path::PathBuf, process::ExitCode, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64_STD, Engine};
use clap::Parser;
use pr402::proto::v2::BuildPaymentTxResponse;
use reqwest::{redirect::Policy, StatusCode};
use serde_json::Value;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;
use url::Url;

/// One-shot buyer for an x402 v2 resource. Walks 402 → build → sign → settle → retry and
/// prints the final response. Works against any Solana facilitator that speaks the pr402
/// HTTP surface (facilitator host auto-discovered from the 402's `paymentRequirements` when
/// not passed explicitly).
#[derive(Parser, Debug)]
#[command(name = "pr402-buy", version, about, long_about = None)]
struct Cli {
    /// Seller resource URL. `pr402-buy` issues a GET first; a 200 is served directly,
    /// a 402 kicks off the payment lifecycle.
    #[arg(long)]
    resource: Url,

    /// Path to the payer Solana keypair (same shape as `solana-keygen` output: JSON array
    /// of 64 bytes).
    #[arg(long)]
    payer: PathBuf,

    /// Facilitator base URL. When absent, `pr402-buy` reads `paymentRequirements.extra`
    /// and tries `facilitator`, `facilitatorUrl`, or `origin` from the 402 body; failing
    /// that, it falls back to `https://preview.ipay.sh`.
    #[arg(long)]
    facilitator: Option<Url>,

    /// Which `accepts[]` line to settle when the 402 returns several. Default 0 = first
    /// line. Out-of-range is a hard error (no silent fallback).
    #[arg(long, default_value_t = 0)]
    accept_index: usize,

    /// Skip the optional `POST /verify` pre-flight. `/settle` verifies internally so this
    /// is safe for one-shot buyers; keep the default (enabled) if you want a server-minted
    /// `correlationId` in your ledger.
    #[arg(long, default_value_t = false)]
    skip_verify: bool,

    /// HTTP request timeout, in seconds.
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    /// Print JSON bodies at each step instead of just the final response.
    #[arg(long, default_value_t = false)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    // Surface the underlying error chain on failure; `?` in main would only show the top.
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pr402-buy: {e:#}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cli.timeout))
        .redirect(Policy::limited(5))
        .build()
        .context("build reqwest client")?;

    // 1. Load payer. Solana keypair JSON is `[u8; 64]`.
    let payer = load_keypair(&cli.payer)
        .with_context(|| format!("load payer keypair from {}", cli.payer.display()))?;
    if cli.verbose {
        eprintln!("payer: {}", payer.pubkey());
    }

    // 2. Probe the resource. If it's already 200, we're done.
    eprintln!("-> GET {}", cli.resource);
    let probe = client
        .get(cli.resource.clone())
        .send()
        .await
        .context("initial GET")?;
    if probe.status().is_success() {
        let body = probe.text().await.context("read 200 body")?;
        println!("{body}");
        return Ok(());
    }
    if probe.status() != StatusCode::PAYMENT_REQUIRED {
        bail!(
            "seller returned unexpected status {}: expected 402 or 200",
            probe.status()
        );
    }

    let payment_required: Value = probe
        .json()
        .await
        .context("parse 402 body as x402 v2 JSON")?;
    if cli.verbose {
        eprintln!("<- 402\n{}", pretty(&payment_required));
    }

    // 3. Extract the chosen accepts[] line + resource metadata.
    let accepts = payment_required
        .get("accepts")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("402 body missing `accepts[]` array"))?;
    if accepts.is_empty() {
        bail!("402 body has empty `accepts[]`");
    }
    let accepted = accepts.get(cli.accept_index).ok_or_else(|| {
        anyhow!(
            "--accept-index {} out of range (0..{})",
            cli.accept_index,
            accepts.len()
        )
    })?;
    let scheme = accepted
        .get("scheme")
        .and_then(Value::as_str)
        .unwrap_or("exact");
    if !(scheme == "exact" || scheme == "v2:solana:exact") {
        bail!(
            "pr402-buy today only supports the `exact` scheme; accepts[{}].scheme = {scheme}",
            cli.accept_index
        );
    }

    let resource = payment_required
        .get("resource")
        .cloned()
        .unwrap_or_else(|| Value::String(cli.resource.to_string()));

    // 4. Resolve facilitator base URL: CLI override → 402 hints → conservative default.
    let facilitator = resolve_facilitator(&cli, &payment_required)?;
    eprintln!("facilitator: {facilitator}");

    // 5. Build unsigned tx.
    let build_url = join_path(&facilitator, "api/v1/facilitator/build-exact-payment-tx")?;
    eprintln!("-> POST {build_url}");
    let build_body = serde_json::json!({
        "payer": payer.pubkey().to_string(),
        "accepted": accepted,
        "resource": resource,
    });
    let build_resp = client
        .post(build_url)
        .json(&build_body)
        .send()
        .await
        .context("facilitator build-exact-payment-tx")?;
    let build_status = build_resp.status();
    let build_text = build_resp.text().await.unwrap_or_default();
    if !build_status.is_success() {
        bail!("build-exact-payment-tx failed ({build_status}): {build_text}");
    }
    let build: BuildPaymentTxResponse = serde_json::from_str(&build_text)
        .with_context(|| format!("parse build response: {build_text}"))?;
    if cli.verbose {
        eprintln!("<- build response:\n{}", pretty_str(&build_text));
    }

    // 6. Sign the unsigned tx at `payer_signature_index`.
    let tx_bytes = B64_STD
        .decode(&build.transaction)
        .context("decode unsigned transaction base64")?;
    let mut tx: VersionedTransaction = bincode::deserialize(&tx_bytes)
        .context("bincode-deserialize unsigned VersionedTransaction")?;
    let message_bytes = tx.message.serialize();
    let signature = payer.sign_message(&message_bytes);
    let idx = build.payer_signature_index;
    if idx >= tx.signatures.len() {
        bail!(
            "payer_signature_index {idx} out of bounds (signatures.len={})",
            tx.signatures.len()
        );
    }
    tx.signatures[idx] = signature;

    let signed = bincode::serialize(&tx).context("bincode-serialize signed tx")?;
    let signed_b64 = B64_STD.encode(&signed);

    // 7. Fill the verify body template with the signed tx and dispatch /verify + /settle.
    let mut verify_body = build.verify_body_template.clone();
    inject_signed_tx(&mut verify_body, &signed_b64)?;

    if !cli.skip_verify {
        let verify_url = join_path(&facilitator, "api/v1/facilitator/verify")?;
        eprintln!("-> POST {verify_url}");
        let verify_resp = client
            .post(verify_url)
            .json(&verify_body)
            .send()
            .await
            .context("facilitator verify")?;
        let status = verify_resp.status();
        let text = verify_resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("verify failed ({status}): {text}");
        }
        if cli.verbose {
            eprintln!("<- verify OK:\n{}", pretty_str(&text));
        }
        // Reuse correlationId when the facilitator minted one.
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(cid) = v.get("correlationId").and_then(Value::as_str) {
                if let Some(obj) = verify_body.as_object_mut() {
                    obj.insert("correlationId".into(), Value::String(cid.to_string()));
                }
            }
        }
    }

    let settle_url = join_path(&facilitator, "api/v1/facilitator/settle")?;
    eprintln!("-> POST {settle_url}");
    let settle_resp = client
        .post(settle_url)
        .json(&verify_body)
        .send()
        .await
        .context("facilitator settle")?;
    let status = settle_resp.status();
    let settle_text = settle_resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("settle failed ({status}): {settle_text}");
    }
    eprintln!("<- settle OK");
    if cli.verbose {
        eprintln!("{}", pretty_str(&settle_text));
    }

    // 8. Retry the resource with the PAYMENT-SIGNATURE header. Sellers accept the raw
    //    JSON body or a base64-encoded form; we send the raw JSON since it's smaller.
    let proof = serde_json::to_string(&verify_body).context("serialize PAYMENT-SIGNATURE")?;
    eprintln!("-> GET {} (PAYMENT-SIGNATURE)", cli.resource);
    let final_resp = client
        .get(cli.resource.clone())
        .header("PAYMENT-SIGNATURE", proof)
        .send()
        .await
        .context("resource retry with PAYMENT-SIGNATURE")?;
    let final_status = final_resp.status();
    let payment_response = final_resp
        .headers()
        .get("PAYMENT-RESPONSE")
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let final_text = final_resp.text().await.unwrap_or_default();

    if !final_status.is_success() {
        bail!("resource retry failed ({final_status}): {final_text}");
    }

    if cli.verbose {
        if let Some(pr) = payment_response.as_deref() {
            eprintln!("<- PAYMENT-RESPONSE header (base64): {pr}");
        }
    }

    // Final payload on stdout so pipelines can consume it directly.
    println!("{final_text}");
    Ok(())
}

// ---- helpers ----------------------------------------------------------------

fn load_keypair(path: &PathBuf) -> Result<Keypair> {
    let raw = fs::read_to_string(path).context("read keypair file")?;
    let bytes: Vec<u8> =
        serde_json::from_str(&raw).context("parse keypair JSON (expected array of 64 u8 bytes)")?;
    if bytes.len() != 64 {
        bail!("keypair file must be 64 bytes, got {}", bytes.len());
    }
    Keypair::try_from(bytes.as_slice()).map_err(|e| anyhow!("invalid keypair bytes: {e}"))
}

/// Mint the final facilitator base URL. Order: explicit `--facilitator` → 402 hints
/// (`paymentRequirements.extra.facilitator`) → preview default. Emits a warning when the
/// seller didn't document a facilitator and we had to guess.
fn resolve_facilitator(cli: &Cli, payment_required: &Value) -> Result<Url> {
    if let Some(u) = &cli.facilitator {
        return Ok(u.clone());
    }
    let candidate = payment_required
        .pointer("/accepts/0/extra/facilitator")
        .or_else(|| payment_required.pointer("/accepts/0/extra/facilitatorUrl"))
        .and_then(Value::as_str);
    if let Some(hint) = candidate {
        return Url::parse(hint).with_context(|| format!("parse facilitator hint: {hint}"));
    }
    eprintln!(
        "warning: 402 body did not document a facilitator; falling back to https://preview.ipay.sh"
    );
    Url::parse("https://preview.ipay.sh").map_err(Into::into)
}

fn join_path(base: &Url, path: &str) -> Result<Url> {
    // `Url::join` against a path without a leading '/' resolves relative to the base's
    // directory; force absolute so callers can pass either shape.
    let p = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let mut out = base.clone();
    out.set_path(&p);
    Ok(out)
}

fn inject_signed_tx(verify_body: &mut Value, signed_b64: &str) -> Result<()> {
    let payload = verify_body
        .pointer_mut("/paymentPayload/payload")
        .ok_or_else(|| anyhow!("verifyBodyTemplate missing paymentPayload.payload"))?;
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| anyhow!("paymentPayload.payload is not an object"))?;
    obj.insert("transaction".into(), Value::String(signed_b64.to_string()));
    Ok(())
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn pretty_str(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(v) => pretty(&v),
        Err(_) => s.to_string(),
    }
}
