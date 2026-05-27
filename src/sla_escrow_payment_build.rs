//! Build **unsigned** legacy [`VersionedTransaction`] shells for `v2:solana:sla-escrow` [`FundPayment`]
//! (same instruction layout as [`sla_escrow_api::sdk::EscrowSdk::fund_payment`], PDAs resolved from
//! `SLAEscrowConfig.program_id` — not the crate-wired `sla_escrow_api::ID` alone).
//!
//! **Solana network fee payer:** Default **`facilitator_pays_transaction_fees: false`** — the **buyer**
//! pays transaction fees and is the **sole** signer (CLI-shaped shell). The x402 v2 spec emphasizes
//! facilitator-sponsored **landing** fees for the standard **`exact`** rail; **SLA-Escrow** is an
//! extension and does not assume the facilitator subsidizes gas (operators may bill via plans, etc.).
//! Set **`facilitator_pays_transaction_fees: true`** for a **facilitator fee payer** shell (two
//! signers, same pattern as [`crate::exact_payment_build`]): buyer signs `FundPayment` authority;
//! slot 0 is reserved for the facilitator at `/settle`. The facilitator pubkey must not appear in
//! instruction account metas.
//!
//! **FundPayment principal** (tokens debited from the buyer’s source ATA into escrow) is always paid
//! by the buyer; only **who pays SOL for the transaction** is toggled above.
//!
//! **Instruction layout:** `[SetComputeUnitLimit, SetComputeUnitPrice, …optional ATA…, FundPayment]`
//! (buyer-paid default). When **`facilitator_pays_transaction_fees: true`**, append trailing
//! compute-budget ixs after `FundPayment` so verify still sees the facilitator ceiling after
//! wallets prepend their own budget instructions. **Verify** enforces CU limits and
//! FundPayment layout (last non-compute-budget instruction) only on the facilitator-sponsored
//! path; buyer-paid txs leave compute budget and trailing instructions to the signing wallet.

use std::str::FromStr;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use solana_transaction::{
    versioned::VersionedTransaction, AccountMeta, Address, Instruction, Message, Transaction,
};

use crate::chain::solana::{SolanaChainProvider, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID};
use crate::chain::solana_sla_escrow::{
    build_fund_payment_instruction_from_uid_bytes, parse_payment_uid_hex, sanitize_uid,
};
use crate::chain::TxBudget;
use crate::scheme::v2_solana_escrow::types::SLAEscrowScheme;
use crate::util::tx_builder::{
    associated_token_address, compute_budget_ix_set_limit, compute_budget_ix_set_price,
    create_associated_token_account_idempotent_ix, estimate_blockhash_expiry_unix,
    parse_u64_from_json,
};
use sla_escrow_api::consts::{MAX_TTL_SECONDS, MIN_TTL_SECONDS};

use spl_token::solana_program::program_pack::Pack;

/// Request body for `POST /api/v1/facilitator/build-sla-escrow-payment-tx`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSlaEscrowPaymentTxRequest {
    /// Buyer pubkey; signs `FundPayment` (second signer when the facilitator is fee payer).
    pub payer: String,
    /// One element from `402 accepts[]` (`scheme: "sla-escrow"`).
    pub accepted:
        crate::proto::v2::PaymentRequirements<String, serde_json::Value, String, serde_json::Value>,
    /// From the `402` body `resource` field.
    pub resource: serde_json::Value,
    /// 32-byte SLA terms hash as 64 hex chars (may be all zeros for testing).
    pub sla_hash: String,
    /// Oracle pubkey; must be listed in `accepted.extra.oracleAuthorities`.
    pub oracle_authority: String,
    /// Unique id for this payment (PDA seed). Generated if omitted.
    ///
    /// Legacy "string" path: pr402 ASCII-encodes the value with
    /// `sanitize_uid` (strip `-`, take the first 32 bytes, zero-pad)
    /// and uses those bytes as the PDA seed and on-chain
    /// `Payment.payment_uid`. This is the historical default; new
    /// callers should prefer `payment_uid_hex` so the on-chain bytes
    /// equal the hex they own — no implicit text encoding step.
    #[serde(default)]
    pub payment_uid: Option<String>,
    /// Buyer-controlled 32-byte `payment_uid` as exactly 64 lowercase
    /// hex characters. When set, pr402 uses these bytes verbatim — no
    /// `sanitize_uid` mangling — for both the PDA seed and the
    /// on-chain `Payment.payment_uid` field. The same hex string MUST
    /// appear in the SLA's `payment_uid` field; the oracle binds them
    /// at evaluation time.
    ///
    /// If both `payment_uid` (string) and `payment_uid_hex` (hex) are
    /// set, the request is rejected with 400 to prevent ambiguity.
    #[serde(default)]
    pub payment_uid_hex: Option<String>,
    /// If `false` (default), require payer source ATA to exist and hold enough tokens.
    #[serde(default)]
    pub skip_source_balance_check: bool,
    /// If `true`, build a **facilitator fee payer** shell (two signers), aligned with `build-exact-payment-tx`.
    /// Default `false` — **buyer** pays fees (one signer, matching sla-escrow CLI).
    /// **HTTP:** `POST /build-sla-escrow-payment-tx` returns 400 when this is `true` unless the deployment sets `PR402_SLA_ESCROW_ALLOW_FACILITATOR_FEE_SPONSORSHIP`. Direct library calls to `build_sla_escrow_fund_payment_tx` are not gated.
    #[serde(default)]
    pub facilitator_pays_transaction_fees: bool,
    /// If `true`, the builder will inject wrap instructions if the payment mint is wrapped SOL.
    pub auto_wrap_sol: Option<bool>,
}

// Response struct was unified into `crate::proto::v2::BuildPaymentTxResponse`.

#[derive(Debug, thiserror::Error)]
pub enum SlaEscrowPaymentBuildError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("network mismatch (expected {expected}, got {got})")]
    NetworkMismatch { expected: String, got: String },
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("SLA escrow not configured for this facilitator")]
    NotConfigured,
    #[error("RPC error: {0}")]
    Rpc(String),
    /// Wave A §3.2 — the facilitator's health gate is enabled and the chosen
    /// oracle's `/health` probe failed within the last 30s. Caller surfaces
    /// HTTP 503 so the buyer SDK retries another profile.
    #[error("oracle unhealthy: {0}")]
    OracleUnhealthy(String),
}

// compute_budget and associated_token_address helpers moved to crate::util::tx_builder

// create_associated_token_account_idempotent_ix moved to crate::util::tx_builder

fn parse_sla_hash_hex(s: &str) -> Result<[u8; 32], SlaEscrowPaymentBuildError> {
    if s.len() != 64 {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(
            "slaHash must be 64 hex characters".into(),
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let pair = &s[i * 2..i * 2 + 2];
        out[i] = u8::from_str_radix(pair, 16).map_err(|_| {
            SlaEscrowPaymentBuildError::InvalidRequest(format!("slaHash: invalid hex at {}", i))
        })?;
    }
    Ok(out)
}

/// `FundPayment.seller` / on-chain `payment.seller`: merchant payout wallet (release/refund destination).
/// Prefers `extra.beneficiary`, then `extra.merchantWallet`. Must not be the escrow PDA.
fn resolve_fund_payment_seller_pk(
    extra: &serde_json::Value,
    escrow_pda: &Pubkey,
) -> Result<Pubkey, SlaEscrowPaymentBuildError> {
    let beneficiary = extra.get("beneficiary").and_then(|v| v.as_str());
    let merchant_wallet = extra.get("merchantWallet").and_then(|v| v.as_str());
    let s = beneficiary.or(merchant_wallet).ok_or_else(|| {
        SlaEscrowPaymentBuildError::InvalidRequest(
            "accepted.extra.beneficiary or accepted.extra.merchantWallet is required (seller pubkey for FundPayment; must match verify/settle paymentRequirements.extra)"
                .into(),
        )
    })?;
    let pk = Pubkey::from_str(s).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "beneficiary/merchantWallet (seller): {}",
            e
        ))
    })?;
    if pk == *escrow_pda {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(
            "beneficiary/merchantWallet must be the merchant's wallet, not the SLA escrow PDA"
                .into(),
        ));
    }
    Ok(pk)
}

// parse_u64_from_json moved to crate::util::tx_builder

/// Build an unsigned SLA-Escrow fund-payment transaction and verify-body template.
///
/// [`crate::parameters::PR402_ALLOWED_PAYMENT_MINTS`] matches `/verify` / `/settle`: resolved from the
/// **`parameters`** table when `db` is `Some`, otherwise from env only. `db: None` does **not** disable
/// an env-configured allowlist.
pub async fn build_sla_escrow_fund_payment_tx(
    provider: &SolanaChainProvider,
    db: Option<&crate::db::Pr402Db>,
    req: BuildSlaEscrowPaymentTxRequest,
) -> Result<crate::proto::v2::BuildPaymentTxResponse, SlaEscrowPaymentBuildError> {
    let escrow_cfg = provider
        .sla_escrow()
        .ok_or(SlaEscrowPaymentBuildError::NotConfigured)?;
    let program_id = escrow_cfg.program_id;

    let payer_pk = Pubkey::from_str(&req.payer)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(format!("payer: {}", e)))?;

    let scheme = &req.accepted.scheme;
    if scheme != SLAEscrowScheme.as_ref() && scheme != "v2:solana:sla-escrow" {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "scheme must be {:?} or \"v2:solana:sla-escrow\", got {:?}",
            SLAEscrowScheme.as_ref(),
            scheme
        )));
    }

    let expected_network = provider.chain_id().to_string();
    let got_net = req.accepted.network.to_string();
    if got_net != expected_network {
        return Err(SlaEscrowPaymentBuildError::NetworkMismatch {
            expected: expected_network,
            got: got_net,
        });
    }

    let asset_str = &req.accepted.asset;
    let mint = Pubkey::from_str(asset_str)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(e.to_string()))?;

    if let Err(msg) = crate::parameters::ensure_allowed_payment_mint(db, &mint).await {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(msg));
    }

    const NATIVE_SOL_MINT: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
    if mint == Pubkey::default() || mint == NATIVE_SOL_MINT {
        return Err(SlaEscrowPaymentBuildError::Unsupported(
            "SLA-Escrow native SOL fund layout is not supported by this builder; use SPL USDC or build locally"
                .into(),
        ));
    }

    let amount = parse_u64_from_json(&req.accepted.amount, "accepted.amount")
        .map_err(SlaEscrowPaymentBuildError::InvalidRequest)?;

    let ttl_seconds_i64 = req.accepted.max_timeout_seconds as i64;

    if ttl_seconds_i64 < MIN_TTL_SECONDS {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "maxTimeoutSeconds must be >= {} (facilitator verify enforces TTL)",
            MIN_TTL_SECONDS
        )));
    }
    if ttl_seconds_i64 > MAX_TTL_SECONDS {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "maxTimeoutSeconds must be <= {}",
            MAX_TTL_SECONDS
        )));
    }
    let cutoff = crate::sla_escrow_ttl::resolve_delivery_cutoff_seconds();
    let budget = crate::sla_escrow_ttl::resolve_delivery_budget_seconds();
    if let Err(e) = crate::sla_escrow_ttl::validate_fund_payment_ttl(
        ttl_seconds_i64 as u64,
        ttl_seconds_i64 as u64,
        cutoff,
        budget,
    ) {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(e.to_string()));
    }

    let extra = req.accepted.extra.as_ref().ok_or_else(|| {
        SlaEscrowPaymentBuildError::InvalidRequest("accepted.extra missing".into())
    })?;

    let escrow_prog_str = extra
        .get("escrowProgramId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.escrowProgramId missing".into(),
            )
        })?;
    let extra_program_id = Pubkey::from_str(escrow_prog_str).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("escrowProgramId: {}", e))
    })?;
    if extra_program_id != program_id {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "accepted.extra.escrowProgramId ({}) does not match facilitator ESCROW_PROGRAM_ID ({})",
            extra_program_id, program_id
        )));
    }

    let (expected_bank_pda, _) = provider.get_bank_pda(&program_id);
    let bank_str = extra
        .get("bankAddress")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest("accepted.extra.bankAddress missing".into())
        })?;
    let extra_bank = Pubkey::from_str(bank_str)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(format!("bankAddress: {e}")))?;
    let loaded_bank = escrow_cfg.bank_address.unwrap_or(expected_bank_pda);
    if extra_bank != loaded_bank {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "accepted.extra.bankAddress ({extra_bank}) does not match facilitator escrow bank ({loaded_bank})"
        )));
    }

    let (expected_config_pda, _) = provider.get_config_pda(&program_id);
    let config_str = extra
        .get("configAddress")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.configAddress missing".into(),
            )
        })?;
    let extra_config = Pubkey::from_str(config_str)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(format!("configAddress: {e}")))?;
    if extra_config != expected_config_pda {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "accepted.extra.configAddress ({extra_config}) does not match facilitator escrow config ({expected_config_pda})"
        )));
    }

    let oracle_pk = Pubkey::from_str(&req.oracle_authority).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("oracleAuthority: {}", e))
    })?;
    let authorities = extra
        .get("oracleAuthorities")
        .and_then(|x| x.as_array())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.oracleAuthorities missing or not an array".into(),
            )
        })?;
    let mut oracle_ok = false;
    for v in authorities {
        let s = v.as_str().ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.oracleAuthorities entries must be strings".into(),
            )
        })?;
        let p = Pubkey::from_str(s).map_err(|e| {
            SlaEscrowPaymentBuildError::InvalidRequest(format!("oracleAuthorities: {}", e))
        })?;
        if p == oracle_pk {
            oracle_ok = true;
            break;
        }
    }
    if !oracle_ok {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(
            "oracleAuthority is not listed in accepted.extra.oracleAuthorities".into(),
        ));
    }

    // OPTIONAL: cross-check `accepted.extra.oracleProfiles[]` when the seller
    // advertises the richer per-profile shape. The chosen `oracleAuthority`
    // MUST match an entry's `operatorPubkey`. This catches the common error
    // of an authority pubkey that's allow-listed in the flat array but does
    // not correspond to any advertised profile (e.g. seller listed two
    // pubkeys but only one runs the right oracle binary).
    //
    // Strict mode (`PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH=true` via the
    // parameters table or env) further requires the matched entry's
    // `profileId` to be one of the profiles this facilitator advertises on
    // `/capabilities`. Off by default to avoid breaking sellers who haven't
    // migrated to the richer shape yet.
    if let Some(profiles) = extra.get("oracleProfiles").and_then(|x| x.as_array()) {
        let mut matched_profile_id: Option<String> = None;
        let mut seen_operator_pubkeys = std::collections::HashSet::new();
        for entry in profiles {
            let op = entry
                .get("operatorPubkey")
                .and_then(|x| x.as_str())
                .ok_or_else(|| {
                    SlaEscrowPaymentBuildError::InvalidRequest(
                        "accepted.extra.oracleProfiles[].operatorPubkey missing or not a string"
                            .into(),
                    )
                })?;
            if !seen_operator_pubkeys.insert(op.to_string()) {
                return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
                    "duplicate oracleProfiles[].operatorPubkey: {op}"
                )));
            }
            let entry_pk = Pubkey::from_str(op).map_err(|e| {
                SlaEscrowPaymentBuildError::InvalidRequest(format!(
                    "oracleProfiles[].operatorPubkey: {}",
                    e
                ))
            })?;
            if !authorities.iter().any(|v| {
                v.as_str()
                    .and_then(|s| Pubkey::from_str(s).ok())
                    .is_some_and(|p| p == entry_pk)
            }) {
                return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
                    "oracleProfiles[].operatorPubkey ({op}) is not listed in accepted.extra.oracleAuthorities"
                )));
            }
            if entry_pk == oracle_pk {
                matched_profile_id = entry
                    .get("profileId")
                    .and_then(|x| x.as_str())
                    .map(str::to_string);
                break;
            }
        }
        if matched_profile_id.is_none() {
            return Err(SlaEscrowPaymentBuildError::InvalidRequest(
                "oracleAuthority is not listed in accepted.extra.oracleProfiles[].operatorPubkey"
                    .into(),
            ));
        }
        // Strict mode (opt-in).
        let strict = crate::parameters::resolve_string_sync(
            crate::parameters::PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH,
            crate::parameters::PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH,
        )
        .map(|s| {
            let v = s.trim().to_ascii_lowercase();
            v == "true" || v == "1" || v == "yes" || v == "on"
        })
        .unwrap_or(false);
        if strict {
            if let Some(profile_id) = &matched_profile_id {
                let advertised_json = crate::parameters::resolve_string_sync(
                    crate::parameters::PR402_SLA_ESCROW_ORACLE_PROFILES_JSON,
                    crate::parameters::PR402_SLA_ESCROW_ORACLE_PROFILES_JSON,
                );
                let mut found = false;
                if let Some(raw) = advertised_json.as_deref() {
                    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
                        found = arr
                            .iter()
                            .filter_map(|v| v.get("profileId").and_then(|p| p.as_str()))
                            .any(|p| p == profile_id);
                    }
                }
                if !found {
                    // Fallback: each profile family has a per-profile-key
                    // `*_DEFAULT_PUBKEY` entry. If any of the three is set,
                    // the corresponding canonical profile id is considered
                    // advertised (matches discovery handler logic).
                    let canonical_for = [
                        (
                            crate::parameters::PR402_SLA_ESCROW_API_QUALITY_DEFAULT_PUBKEY,
                            "x402/oracles/api-quality/v1",
                        ),
                        (
                            crate::parameters::PR402_SLA_ESCROW_ONCHAIN_TRANSFER_DEFAULT_PUBKEY,
                            "x402/oracles/onchain-transfer/v1",
                        ),
                        (
                            crate::parameters::PR402_SLA_ESCROW_FILE_DELIVERY_DEFAULT_PUBKEY,
                            "x402/oracles/file-delivery/attestation/v1",
                        ),
                    ];
                    found = canonical_for.iter().any(|(key, canonical_id)| {
                        crate::parameters::resolve_string_sync(key, key).is_some()
                            && profile_id == *canonical_id
                    });
                }
                if !found {
                    return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
                        "oracleProfiles[].profileId ({}) is not advertised by this facilitator \
                         (PR402_SLA_ESCROW_REQUIRE_PROFILE_MATCH=true)",
                        profile_id
                    )));
                }
            }
        }

        // Wave A §3.2 — health gate (default OFF). When
        // `PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY` is truthy, probe the
        // matched profile's oracle and refuse to bind escrow when the probe
        // fails. The probe is cached for 30s and short-circuits when the gate
        // is disabled, so this is no-op for existing deployments.
        if let Some(profile_id) = matched_profile_id.as_deref() {
            if let Some(registry_url) = registry_url_for_profile(profile_id) {
                if let Some(err) = crate::oracle_health::probe_unhealthy(Some(&registry_url)).await
                {
                    return Err(SlaEscrowPaymentBuildError::OracleUnhealthy(format!(
                        "{profile_id}: {err}"
                    )));
                }
            }
        }
    }

    // Resolve the payment uid using strict precedence:
    //   - both `payment_uid` and `payment_uid_hex` set → 400 (ambiguous)
    //   - `payment_uid_hex` set → use those raw 32 bytes verbatim
    //   - `payment_uid` (string) set → legacy `sanitize_uid` path
    //   - neither set → mint a fresh ULID and run it through `sanitize_uid`
    //
    // The string form is what we surface to back-compat callers in the
    // response (`payment_uid` field). The bytes form is what we actually
    // use to derive PDAs and write into FundPayment data — so a hex
    // caller's on-chain bytes equal the hex they own, no implicit text
    // encoding step.
    let str_uid_set = req
        .payment_uid
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let hex_uid_set = req
        .payment_uid_hex
        .as_ref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let (payment_uid_str, payment_uid_bytes) = match (str_uid_set, hex_uid_set) {
        (true, true) => {
            return Err(SlaEscrowPaymentBuildError::InvalidRequest(
                "payment_uid and payment_uid_hex are mutually exclusive; set at most one".into(),
            ));
        }
        (false, true) => {
            // SAFETY: hex_uid_set guarantees the field is Some + non-empty.
            let hex = req.payment_uid_hex.as_deref().unwrap();
            let bytes =
                parse_payment_uid_hex(hex).map_err(SlaEscrowPaymentBuildError::InvalidRequest)?;
            (hex.to_string(), bytes)
        }
        (true, false) => {
            let s = req.payment_uid.as_deref().unwrap();
            (s.to_string(), sanitize_uid(s))
        }
        (false, false) => {
            let s = ulid::Ulid::new().to_string();
            let bytes = sanitize_uid(&s);
            (s, bytes)
        }
    };
    // Lower-hex of the on-chain 32-byte payment_uid — surfaced in the
    // response so clients that did NOT pass `payment_uid_hex` can read
    // back the canonical bytes without re-running `sanitize_uid`.
    let payment_uid_canonical_hex: String = {
        let mut s = String::with_capacity(64);
        use std::fmt::Write;
        for b in payment_uid_bytes {
            let _ = write!(&mut s, "{b:02x}");
        }
        s
    };

    let sla_hash = parse_sla_hash_hex(&req.sla_hash)?;

    let bank_pda = escrow_cfg.bank_address.ok_or_else(|| {
        SlaEscrowPaymentBuildError::InvalidRequest(
            "facilitator: bank_address not loaded (SLA escrow bank account missing in config)"
                .into(),
        )
    })?;
    let (escrow_pda, _) = provider.get_escrow_pda(mint, bank_pda);

    let seller_pk = resolve_fund_payment_seller_pk(extra, &escrow_pda)?;

    if let Some(pool) = db {
        let (sm, om) = crate::proto::settlement_rail_from_x402_asset(asset_str);
        if let Err(e) = pool
            .assert_merchant_single_rail_policy(&seller_pk.to_string(), sm.as_str(), om.as_deref())
            .await
        {
            return Err(SlaEscrowPaymentBuildError::InvalidRequest(e.to_string()));
        }
    }

    let mint_acc = provider
        .rpc_client()
        .get_account(&mint)
        .await
        .map_err(|e| SlaEscrowPaymentBuildError::Rpc(e.to_string()))?;

    let token_program = if mint_acc.owner == spl_token::ID {
        spl_token::ID
    } else if mint_acc.owner == TOKEN_2022_PROGRAM_ID {
        // Plain Token-2022 mint only (extended layouts rejected on-chain).
        if mint_acc.data.len() > spl_token::state::Mint::LEN {
            return Err(SlaEscrowPaymentBuildError::Unsupported(
                "Token-2022 mint uses extensions not supported by sla-escrow (plain 82-byte mint only)"
                    .into(),
            ));
        }
        TOKEN_2022_PROGRAM_ID
    } else {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "mint owner {} is not Token or Token-2022",
            mint_acc.owner
        )));
    };

    let _decimals = spl_token::state::Mint::unpack(&mint_acc.data)
        .map_err(|_| {
            SlaEscrowPaymentBuildError::InvalidRequest("mint account: unpack failed".into())
        })?
        .decimals;

    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| SlaEscrowPaymentBuildError::Rpc(e.to_string()))?;

    let recent_blockhash_expires_at = estimate_blockhash_expiry_unix();

    let source_ata = associated_token_address(&payer_pk, &mint, &token_program);

    let budget = TxBudget::FundPayment;
    let cu_limit = compute_budget_ix_set_limit(budget.cu_limit());
    let cu_price = compute_budget_ix_set_price(budget.cu_price());

    let auto_wrap = req.auto_wrap_sol.unwrap_or(false);
    const WSOL_MINT: Pubkey = solana_pubkey::pubkey!("So11111111111111111111111111111111111111112");
    let mut ixs: Vec<Instruction> = vec![cu_limit, cu_price];

    if mint == WSOL_MINT && auto_wrap {
        ixs.push(create_associated_token_account_idempotent_ix(
            &payer_pk,
            &payer_pk,
            &WSOL_MINT,
            &spl_token::ID,
        ));
        let mut data = Vec::with_capacity(12);
        data.extend_from_slice(&2u32.to_le_bytes()); // Transfer discriminator
        data.extend_from_slice(&amount.to_le_bytes());
        ixs.push(Instruction {
            program_id: SYSTEM_PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(payer_pk, true),
                AccountMeta::new(source_ata, false),
            ],
            data,
        });
        ixs.push(
            spl_token::instruction::sync_native(&spl_token::ID, &source_ata).map_err(|e| {
                SlaEscrowPaymentBuildError::InvalidRequest(format!("sync_native: {}", e))
            })?,
        );
    } else if !req.skip_source_balance_check {
        let bal = provider
            .rpc_client()
            .get_token_account_balance(&source_ata)
            .await
            .map_err(|e| {
                SlaEscrowPaymentBuildError::InvalidRequest(format!(
                    "payer source ATA {} (mint {}): {}",
                    source_ata, mint, e
                ))
            })?;
        let raw: u64 = bal
            .amount
            .parse()
            .map_err(|_| SlaEscrowPaymentBuildError::Rpc("could not parse token balance".into()))?;
        if raw < amount {
            return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
                "payer balance {} raw < required {} (ATA {})",
                raw, amount, source_ata
            )));
        }
    }

    let parsed = Pubkey::from_str(&req.accepted.pay_to).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("accepted.payTo: {}", e))
    })?;
    if parsed != escrow_pda {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "accepted.payTo must be the SLA Escrow PDA for this asset (expected {})",
            escrow_pda
        )));
    }

    let escrow_token_ata = associated_token_address(&escrow_pda, &mint, &token_program);

    let need_create_escrow_ata = provider
        .rpc_client()
        .get_account(&escrow_token_ata)
        .await
        .is_err();
    if need_create_escrow_ata {
        ixs.push(create_associated_token_account_idempotent_ix(
            &payer_pk,
            &escrow_pda,
            &mint,
            &token_program,
        ));
    }

    let fund_ix = build_fund_payment_instruction_from_uid_bytes(
        program_id,
        payer_pk,
        seller_pk,
        mint,
        amount,
        ttl_seconds_i64,
        &payment_uid_bytes,
        sla_hash,
        oracle_pk,
        token_program,
    );
    ixs.push(fund_ix);
    if req.facilitator_pays_transaction_fees {
        // Wallets often replace leading budget ixs with ~400k CU. Solana uses the *last*
        // SetComputeUnitLimit — append facilitator ceiling after FundPayment so verify
        // (facilitator-sponsored path only) still sees our budget after wallet sign.
        ixs.push(compute_budget_ix_set_limit(budget.cu_limit()));
        ixs.push(compute_budget_ix_set_price(budget.cu_price()));
    }

    let fee_payer_pk = if req.facilitator_pays_transaction_fees {
        provider.fee_payer()
    } else {
        payer_pk
    };
    let fee_payer_addr = Address::new_from_array(fee_payer_pk.to_bytes());
    let message = Message::new_with_blockhash(&ixs, Some(&fee_payer_addr), &blockhash);
    let tx = Transaction::new_unsigned(message);
    let vtx = VersionedTransaction::from(tx);

    // Payer signature index: matches exact builder pattern for SDK parity.
    let payer_signature_index = vtx
        .message
        .static_account_keys()
        .iter()
        .position(|k| *k == payer_pk)
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "internal: payer pubkey missing from transaction signers".into(),
            )
        })?;

    // Full ordered signer list (see `BuildPaymentTxResponse::signer_pubkeys`). Covers
    // both the sponsored path (fee_payer + buyer) and the buyer-as-fee-payer path.
    let num_signers = vtx.message.header().num_required_signatures as usize;
    let signer_pubkeys: Vec<String> = vtx
        .message
        .static_account_keys()
        .iter()
        .take(num_signers)
        .map(|k| k.to_string())
        .collect();

    let wire = bincode::serialize(&vtx).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("bincode serialize: {}", e))
    })?;
    let tx_b64 = STANDARD.encode(wire);

    let mut accepted_norm = serde_json::to_value(&req.accepted).map_err(|_| {
        SlaEscrowPaymentBuildError::InvalidRequest("failed to serialize accepted".into())
    })?;
    let accepted_obj = accepted_norm.as_object_mut().ok_or_else(|| {
        SlaEscrowPaymentBuildError::InvalidRequest("accepted must be a JSON object".into())
    })?;
    accepted_obj.insert(
        "payTo".to_string(),
        serde_json::json!(escrow_pda.to_string()),
    );
    crate::util::normalize_scheme_field_in_map(&mut accepted_norm);

    let verify_body_template = serde_json::json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": accepted_norm.clone(),
            "payload": { "transaction": tx_b64 },
            "resource": req.resource,
            "extensions": {}
        },
        "paymentRequirements": accepted_norm,
    });

    let notes = if req.facilitator_pays_transaction_fees {
        vec![
            "SLA-Escrow (sponsored): facilitator pays Solana fees; buyer signs FundPayment authority (second signer, same pattern as build-exact-payment-tx).".into(),
            "Sign with the buyer keypair only (partial sign); POST /verify then /settle so the facilitator fills fee-payer signature slot 0.".into(),
            "accepted.extra.beneficiary (preferred) or merchantWallet must be the seller payout wallet; it is encoded as FundPayment.seller and must match paymentRequirements.extra on verify.".into(),
            "Blockhashes expire; rebuild if verification fails with BlockhashNotFound.".into(),
        ]
    } else {
        vec![
            "SLA-Escrow (default): buyer pays Solana fees and is the sole signer — facilitator does not appear in instruction accounts.".into(),
            "Sign with the buyer keypair; you may broadcast before verify or use /settle as today.".into(),
            "accepted.extra.beneficiary (preferred) or merchantWallet must be the seller payout wallet; it is encoded as FundPayment.seller and must match paymentRequirements.extra on verify.".into(),
            "Blockhashes expire; rebuild if verification fails with BlockhashNotFound.".into(),
        ]
    };

    Ok(crate::proto::v2::BuildPaymentTxResponse {
        x402_version: 2,
        transaction: tx_b64,
        recent_blockhash: blockhash.to_string(),
        recent_blockhash_expires_at,
        fee_payer: fee_payer_pk.to_string(),
        payer: payer_pk.to_string(),
        payment_uid: Some(payment_uid_str),
        payment_uid_hex: Some(payment_uid_canonical_hex),
        payer_signature_index,
        signer_pubkeys,
        verify_body_template,
        notes,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOracleConfirmTxRequest {
    pub oracle_authority: String,
    pub mint: String,
    pub payment_uid: String,
    pub delivery_hash: String,
    #[serde(default)]
    pub resolution_hash: Option<String>,
    pub resolution_state: u8,
    pub resolution_reason: u16,
    /// If `true`, the transaction will require the oracle to pay the gas fee. Default `false` for now, meaning it just builds a shell. Usually oracles pay for confirm.
    #[serde(default)]
    pub fee_payer: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOracleConfirmTxResponse {
    pub transaction: String,
    pub program_id: String,
    pub payment_pda: String,
}

pub async fn build_oracle_confirm_tx(
    provider: &SolanaChainProvider,
    req: BuildOracleConfirmTxRequest,
) -> Result<BuildOracleConfirmTxResponse, SlaEscrowPaymentBuildError> {
    let escrow_cfg = provider
        .sla_escrow()
        .ok_or(SlaEscrowPaymentBuildError::NotConfigured)?;
    let program_id = escrow_cfg.program_id;

    let oracle_pk = Pubkey::from_str(&req.oracle_authority).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("oracleAuthority: {}", e))
    })?;
    let mint_pk = Pubkey::from_str(&req.mint)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(format!("mint: {}", e)))?;

    let delivery_hash = parse_sla_hash_hex(&req.delivery_hash)?;
    let resolution_hash = match &req.resolution_hash {
        Some(s) if !s.is_empty() => parse_sla_hash_hex(s)?,
        _ => [0u8; 32],
    };

    let ix = crate::chain::solana_sla_escrow::build_confirm_oracle_instruction(
        program_id,
        oracle_pk,
        mint_pk,
        &req.payment_uid,
        delivery_hash,
        resolution_hash,
        req.resolution_state,
        req.resolution_reason,
    );

    let fee_payer = req.fee_payer.as_deref().unwrap_or(&req.oracle_authority);
    let fee_payer_pk = Pubkey::from_str(fee_payer)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(format!("feePayer: {}", e)))?;

    let recent_blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| SlaEscrowPaymentBuildError::Rpc(e.to_string()))?;

    let budget = TxBudget::OracleConfirm;
    let cu_limit_ix = compute_budget_ix_set_limit(budget.cu_limit());
    let cu_price_ix = compute_budget_ix_set_price(budget.cu_price());
    let ixs = vec![cu_limit_ix, cu_price_ix, ix];

    let message = Message::new_with_blockhash(&ixs, Some(&fee_payer_pk), &recent_blockhash);
    let tx = VersionedTransaction::from(solana_transaction::Transaction::new_unsigned(message));

    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(bincode::serialize(&tx).unwrap());

    let (bank_pda, _) = crate::chain::solana_sla_escrow::derive_bank_pda(&program_id);
    let (payment_pda, _) = crate::chain::solana_sla_escrow::derive_payment_pda(
        &program_id,
        &bank_pda,
        &req.payment_uid,
    );

    Ok(BuildOracleConfirmTxResponse {
        transaction: tx_b64,
        program_id: program_id.to_string(),
        payment_pda: payment_pda.to_string(),
    })
}

/// Wave A §3.2 helper — look up the advertised `registry_url` for a given
/// canonical `profile_id`. Reads the same per-profile parameter keys that
/// `discovery::build_sla_escrow_oracle_profiles` consults so the gate sees
/// exactly the URL we publish on `/capabilities`. Returns `None` when the
/// profile id is unknown or no registry URL is configured (in which case the
/// gate cannot probe and the build proceeds).
fn registry_url_for_profile(profile_id: &str) -> Option<String> {
    let key = match profile_id {
        "x402/oracles/api-quality/v1" => {
            crate::parameters::PR402_SLA_ESCROW_API_QUALITY_REGISTRY_URL
        }
        "x402/oracles/onchain-transfer/v1" => {
            crate::parameters::PR402_SLA_ESCROW_ONCHAIN_TRANSFER_REGISTRY_URL
        }
        "x402/oracles/file-delivery/attestation/v1" => {
            crate::parameters::PR402_SLA_ESCROW_FILE_DELIVERY_REGISTRY_URL
        }
        _ => return None,
    };
    crate::parameters::resolve_string_sync(key, key).filter(|s| !s.is_empty())
}
