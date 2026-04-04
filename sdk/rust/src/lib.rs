use anyhow::{anyhow, bail, Result};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use solana_sdk::{signature::Keypair, signer::Signer, transaction::VersionedTransaction};

/// The primary client for navigating 402 gateways.
pub struct X402AgentClient {
    http: Client,
    wallet: Keypair,
}

impl X402AgentClient {
    pub fn new(wallet: Keypair) -> Self {
        Self {
            http: Client::new(),
            wallet,
        }
    }

    /// Access an API endpoint. If challenged with a 402, automatically routes to the Facilitator,
    /// builds the transaction, signs it, and retries the request fully authorized.
    pub async fn fetch_with_auto_pay(&self, url: &str, preferred_mint: &str) -> Result<reqwest::Response> {
        let res = self.http.get(url).send().await?;
        if res.status() == StatusCode::OK {
            return Ok(res);
        } else if res.status() != StatusCode::PAYMENT_REQUIRED {
            bail!("Unexpected HTTP status: {}", res.status());
        }

        let requirement: Value = res.json().await?;
        let accepts = requirement.get("accepts").and_then(|a| a.as_array())
            .ok_or_else(|| anyhow!("No payment accepts options found"))?;

        let rule = accepts.iter().find(|a| a.get("asset").and_then(|x| x.as_str()) == Some(preferred_mint))
            .ok_or_else(|| anyhow!("Resource does not accept preferred mint: {}", preferred_mint))?;

        let cap_url = rule.get("extra").and_then(|e| e.get("capabilitiesUrl")).and_then(|c| c.as_str())
            .ok_or_else(|| anyhow!("Capabilities URL missing in 402 challenge"))?;

        let fac_base_url = cap_url.replace("/capabilities", "");
        let build_url = format!("{}/build-exact-payment-tx", fac_base_url);

        let build_payload = serde_json::json!({
            "payer": self.wallet.pubkey().to_string(),
            "accepted": rule,
            "resource": requirement.get("resource"),
            "skipSourceBalanceCheck": true
        });

        let build_res = self.http.post(&build_url).json(&build_payload).send().await?;
        if !build_res.status().is_success() {
            bail!("Facilitator refused transaction build: {}", build_res.text().await?);
        }

        let build_json: Value = build_res.json().await?;
        let mut verify_body = build_json.get("verifyBodyTemplate").cloned()
            .ok_or_else(|| anyhow!("Missing verifyBodyTemplate"))?;

        let tx_b64 = build_json.get("transaction").and_then(|t| t.as_str())
            .ok_or_else(|| anyhow!("Missing unsigned transaction string"))?;

        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut vtx: VersionedTransaction = bincode::deserialize(&STANDARD.decode(tx_b64)?)?;

        let my_pubkey = self.wallet.pubkey();
        let keys = vtx.message.static_account_keys();
        let my_idx = keys.iter().position(|k| k == &my_pubkey)
            .ok_or_else(|| anyhow!("Signer wallet not found in projected transaction accounts"))?;

        vtx.signatures[my_idx] = self.wallet.sign_message(&vtx.message.serialize());
        let signed_tx_b64 = STANDARD.encode(bincode::serialize(&vtx)?);

        verify_body["paymentPayload"]["payload"]["transaction"] = Value::String(signed_tx_b64);
        let proof_b64 = STANDARD.encode(serde_json::to_string(&verify_body)?);

        let final_res = self.http.get(url)
            .header("X-PAYMENT", proof_b64)
            .send().await?;

        Ok(final_res)
    }
}
