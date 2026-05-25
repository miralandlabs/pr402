//! v2:solana:escrow payment scheme implementation.

pub mod types;

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Write as _;
use std::mem;
use std::str::FromStr;
use std::sync::Arc;

use crate::chain::solana::{Address, SolanaChainProvider};
use crate::chain::ChainProvider;
use crate::proto;
use crate::proto::PaymentVerificationError;
use crate::scheme::v2_solana_escrow::types::SLAEscrowScheme;
use crate::scheme::{
    X402SchemeFacilitator, X402SchemeFacilitatorBuilder, X402SchemeFacilitatorError, X402SchemeId,
};

// We reuse some shared Solana verification logic, but we need custom verify for escrow
use crate::scheme::v2_solana_exact::shared::{
    settle_transaction, verify_compute_limit_instruction, verify_compute_price_instruction,
    TransactionInt, VerifyTransferResult,
};
use crate::util::{
    decode_versioned_transaction_from_bincode, reject_versioned_tx_with_address_lookup_tables,
    Base64Bytes,
};
use sla_escrow_api::instruction::{EscrowInstruction, FundPayment};

use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_signature::Signature;

pub struct V2SolanaSLAEscrow;

impl X402SchemeId for V2SolanaSLAEscrow {
    fn namespace(&self) -> &str {
        "solana"
    }

    fn scheme(&self) -> &str {
        SLAEscrowScheme.as_ref()
    }
}

impl X402SchemeFacilitatorBuilder for V2SolanaSLAEscrow {
    fn build(
        &self,
        provider: ChainProvider,
        _config: Option<serde_json::Value>,
        db: Option<crate::db::Pr402Db>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn Error>> {
        // SLAEscrow requires SLAEscrowConfig to be present
        if provider.solana.sla_escrow().is_none() {
            return Err("SLAEscrowConfig is missing but escrow scheme is requested".into());
        }
        Ok(Box::new(V2SolanaSLAEscrowFacilitator {
            provider: provider.solana,
            db,
        }))
    }
}

pub struct V2SolanaSLAEscrowFacilitator {
    provider: Arc<SolanaChainProvider>,
    db: Option<crate::db::Pr402Db>,
}

#[async_trait::async_trait]
impl X402SchemeFacilitator for V2SolanaSLAEscrowFacilitator {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeFacilitatorError> {
        let request_v2 = types::VerifyRequest::from_proto(request.clone())?;
        crate::parameters::ensure_allowed_payment_mint(
            self.db.as_ref(),
            request_v2.payment_requirements.asset.pubkey(),
        )
        .await
        .map_err(X402SchemeFacilitatorError::InvalidPayload)?;
        let verification = verify_transfer(&self.provider, &request_v2).await?;

        // Escrow audit rows (`escrow_details`) are persisted in `bin/facilitator.rs` *after*
        // `record_payment_verify` creates the parent `payment_attempts` row — this scheme runs
        // before that insert, so upserting here always failed with "Parent payment attempt not found".

        Ok(proto::VerifyResponse::valid(verification.payer.to_string()))
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let request_v2 = types::SettleRequest::from_proto(request.clone())?;
        crate::parameters::ensure_allowed_payment_mint(
            self.db.as_ref(),
            request_v2.payment_requirements.asset.pubkey(),
        )
        .await
        .map_err(X402SchemeFacilitatorError::InvalidPayload)?;
        let verification = verify_transfer(&self.provider, &request_v2).await?;
        let payer = verification.payer.to_string();

        // `escrow_details` upsert runs after `record_payment_settle` in `bin/facilitator.rs`
        // so the parent `payment_attempts` row exists (same ordering bug as verify).

        // Facilitator-sponsored fund txs (fee payer = facilitator, buyer = 2nd signer): reuse the same
        // `sign` + `send_and_confirm` path as `exact`. Legacy buyer-paid shells keep idempotent
        // confirm/submit in [`settle_sla_escrow_fund_payment`].
        let facilitator = self.provider.pubkey();
        let sponsor_is_facilitator = verification
            .transaction
            .message
            .static_account_keys()
            .first()
            .copied()
            == Some(facilitator);

        // PRIORITY: Use explicit beneficiary for collection/audit if provided, otherwise default to merchant identity
        let final_beneficiary = request_v2
            .payment_requirements
            .extra
            .as_ref()
            .and_then(|e| e.beneficiary.as_ref())
            .map(|a| *a.pubkey())
            .unwrap_or_else(|| {
                // For escrow, identity is often the merchant wallet if present in extra
                request_v2
                    .payment_requirements
                    .extra
                    .as_ref()
                    .and_then(|e| e.merchant_wallet.as_ref())
                    .map(|a| *a.pubkey())
                    .unwrap_or_else(|| *verification.merchant_identity.pubkey())
            });

        let tx_sig = if sponsor_is_facilitator {
            settle_transaction(&self.provider, verification, Some(final_beneficiary), None).await?
        } else {
            settle_sla_escrow_fund_payment(&self.provider, verification).await?
        };

        Ok(proto::SettleResponse::success(
            payer,
            tx_sig.to_string(),
            self.provider.chain_id().to_string(),
        ))
    }

    async fn supported(&self) -> Result<proto::SupportedResponse, X402SchemeFacilitatorError> {
        let chain_id = self.provider.chain_id();
        let kinds: Vec<proto::SupportedPaymentKind> = {
            let fee_payer = self.provider.fee_payer();
            let escrow_config = self
                .provider
                .sla_escrow()
                .expect("SLAEscrow config missing");

            let (bank_address, _) = self.provider.get_bank_pda(&escrow_config.program_id);
            let (config_address, _) = self.provider.get_config_pda(&escrow_config.program_id);
            let fee_bps = escrow_config.fee_bps.unwrap_or(0);
            let oracle_fee_bps = escrow_config.oracle_fee_bps.unwrap_or(0);

            let oracle_authorities = escrow_config
                .oracle_authorities
                .iter()
                .map(|p| (*p).into())
                .collect::<Vec<Address>>();

            let extra = Some(
                serde_json::to_value(types::SLAEscrowPaymentRequirementsExtra {
                    fee_payer: fee_payer.into(),
                    oracle_authorities,
                    escrow_program_id: escrow_config.program_id.into(),
                    bank_address: bank_address.into(),
                    config_address: config_address.into(),
                    fee_bps: fee_bps.into(),
                    oracle_fee_bps: oracle_fee_bps.into(),
                    ttl_seconds: 3600.into(), // Default 1 hour
                    delivery_cutoff_seconds:
                        (crate::sla_escrow_ttl::resolve_delivery_cutoff_seconds().max(0) as u64)
                            .into(),
                    delivery_budget_seconds:
                        (crate::sla_escrow_ttl::resolve_delivery_budget_seconds().max(0) as u64)
                            .into(),
                    // Advertise the CU envelope the facilitator actually enforces on
                    // verify for the buyer-signed FundPayment tx.
                    max_compute_unit_limit: (crate::chain::TxBudget::FundPayment.cu_limit() as u64)
                        .into(),
                    recommended_compute_unit_price: crate::chain::TxBudget::FundPayment
                        .cu_price()
                        .into(),
                    sla_fund_tx_network_fee_payer: Some("buyer".to_string()),
                    merchant_wallet: None,
                    beneficiary: None,
                })
                .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?,
            );

            vec![proto::SupportedPaymentKind {
                x402_version: 2,
                scheme: SLAEscrowScheme.to_string(),
                network: chain_id.to_string(),
                extra,
            }]
        };

        let signers = {
            let mut signers = HashMap::with_capacity(1);
            signers.insert(
                chain_id,
                self.provider
                    .signer_addresses()
                    .iter()
                    .map(|a| a.to_string())
                    .collect(),
            );
            signers
        };

        Ok(proto::SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        })
    }

    async fn onboard(
        &self,
        wallet: &str,
    ) -> Result<crate::facilitator::SchemeOnboardInfo, X402SchemeFacilitatorError> {
        let _seller = solana_pubkey::Pubkey::from_str(wallet)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;
        let escrow_config = self.provider.sla_escrow().ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("SLAEscrow not enabled".to_string())
        })?;
        let bank_pda = escrow_config.bank_address.ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("SLAEscrow bank not loaded".to_string())
        })?;
        let fee_bps = escrow_config.fee_bps.unwrap_or(0);

        // Pre-compute the per-asset escrow PDAs we expect dashboards
        // and `payTo` resolvers to want most: native SOL plus the
        // cluster's canonical USDC mint. The legacy single-mint
        // `vault_pda` field continues to point at the SOL preview for
        // back-compat; new clients should iterate `vault_pda_previews`.
        let sol_mint = Pubkey::default();
        let (sol_escrow_pda, _) = self.provider.get_escrow_pda(sol_mint, bank_pda);
        let (sol_storage_pda, _) =
            self.provider
                .get_sla_escrow_sol_storage_pda(sol_mint, bank_pda, sol_escrow_pda);

        let mut previews = vec![crate::facilitator::VaultPdaPreview {
            label: "SOL".to_string(),
            mint: sol_mint.to_string(),
            vault_pda: sol_escrow_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
        }];

        if let Some(usdc_mint) =
            crate::seller_provision::canonical_usdc_mint(self.provider.as_ref())
        {
            let (usdc_escrow_pda, _) = self.provider.get_escrow_pda(usdc_mint, bank_pda);
            let (usdc_sol_storage_pda, _) =
                self.provider
                    .get_sla_escrow_sol_storage_pda(usdc_mint, bank_pda, usdc_escrow_pda);
            previews.push(crate::facilitator::VaultPdaPreview {
                label: crate::seller_provision::canonical_usdc_label(self.provider.as_ref())
                    .to_string(),
                mint: usdc_mint.to_string(),
                vault_pda: usdc_escrow_pda.to_string(),
                sol_storage_pda: usdc_sol_storage_pda.to_string(),
            });
        }

        Ok(crate::facilitator::SchemeOnboardInfo {
            // The "(preview)" suffix used to confuse operators (it
            // suggested the staging/production split implied by
            // preview.ipay.sh, not the "pre-computed PDA" meaning of
            // preview math). Clearer: just "SLA Escrow", with each
            // entry in `vault_pda_previews` carrying its own per-asset
            // label.
            label: "SLA Escrow".to_string(),
            role: "Institutional Escrow".to_string(),
            // Legacy single-mint surface — points at the SOL preview.
            // New callers should iterate `vault_pda_previews`.
            vault_pda: sol_escrow_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            token_pda: None,
            fee_bps: fee_bps.into(),
            status: "Active".to_string(),
            is_sovereign: false,
            provisioning_status: None,
            bank_pda: Some(bank_pda.to_string()),
            pay_to_kind: Some("escrowPda".to_string()),
            pay_to_resolve: Some("discovery.vaultPda".to_string()),
            vault_pda_preview_mint: Some(sol_mint.to_string()),
            vault_pda_previews: Some(previews),
        })
    }

    async fn build_onboard_provision_tx(
        &self,
        _wallet: &str,
        _asset: &str,
    ) -> Result<crate::seller_provision::SellerProvisionTxResponse, X402SchemeFacilitatorError>
    {
        Err(X402SchemeFacilitatorError::InvalidPayload(
            "Seller SplitVault provisioning is only supported on v2:solana:exact (UniversalSettle); use POST /api/v1/facilitator/onboard/provision with the exact scheme deployment."
                .into(),
        ))
    }

    async fn discovery(
        &self,
        wallet: &str,
        asset: Option<&str>,
    ) -> Result<crate::facilitator::SchemeOnboardInfo, X402SchemeFacilitatorError> {
        let _seller = solana_pubkey::Pubkey::from_str(wallet)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;
        let escrow_config = self.provider.sla_escrow().ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("SLAEscrow not enabled".to_string())
        })?;
        let bank_pda = escrow_config.bank_address.ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("SLAEscrow bank not loaded".to_string())
        })?;

        // In SLA-Escrow, the "Vault" (payTo) is the Escrow account for a specific mint.
        let mint = if let Some(a) = asset {
            Pubkey::from_str(a)
                .map_err(|e| X402SchemeFacilitatorError::InvalidPayload(e.to_string()))?
        } else {
            Pubkey::default() // Default to Native SOL discovery
        };

        let (escrow_pda, _) = self.provider.get_escrow_pda(mint, bank_pda);
        let (sol_storage_pda, _) = self
            .provider
            .get_sla_escrow_sol_storage_pda(mint, bank_pda, escrow_pda);

        Ok(crate::facilitator::SchemeOnboardInfo {
            label: format!(
                "SLA Escrow ({})",
                if mint == Pubkey::default() {
                    "SOL"
                } else {
                    "Asset"
                }
            ),
            role: "Institutional Escrow".to_string(),
            vault_pda: escrow_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            token_pda: None,
            fee_bps: escrow_config.fee_bps.unwrap_or(0).into(),
            status: "Active".to_string(),
            is_sovereign: false,
            provisioning_status: None,
            bank_pda: Some(bank_pda.to_string()),
            pay_to_kind: Some("escrowPda".to_string()),
            pay_to_resolve: Some("this.vaultPda".to_string()),
            vault_pda_preview_mint: Some(mint.to_string()),
            // `discovery` answers a single mint per call; the `previews`
            // surface is only meaningful on the multi-asset `onboard`
            // path. Leave `None` so clients reading this response know
            // the lone `vault_pda` field is the authoritative answer.
            vault_pda_previews: None,
        })
    }

    async fn upgrade(
        &self,
        request: &proto::PaymentRequired,
    ) -> Result<proto::PaymentRequired, X402SchemeFacilitatorError> {
        let mut pr = match request {
            proto::PaymentRequired::V2(v2) => v2.clone(),
        };

        for (i, accept) in pr.accepts.iter_mut().enumerate() {
            let scheme = accept.get("scheme").and_then(|s| s.as_str());
            let network = accept.get("network").and_then(|n| n.as_str());

            if scheme == Some(SLAEscrowScheme.as_ref())
                && network == Some(&self.provider.chain_id().to_string())
            {
                // SE-CRIT-12: Escrow Elevation.
                // In SLA-Escrow, we must provide the trusted oracles and program metadata.
                let escrow_config = self
                    .provider
                    .sla_escrow()
                    .expect("SLAEscrow config missing");

                let (bank_address, _) = self.provider.get_bank_pda(&escrow_config.program_id);
                let (config_address, _) = self.provider.get_config_pda(&escrow_config.program_id);
                let fee_bps = escrow_config.fee_bps.unwrap_or(0);
                let oracle_fee_bps = escrow_config.oracle_fee_bps.unwrap_or(0);

                let oracle_authorities = escrow_config
                    .oracle_authorities
                    .iter()
                    .map(|p| (*p).into())
                    .collect::<Vec<Address>>();

                if let Some(obj) = accept.as_object_mut() {
                    let merchant_wallet = obj
                        .get("extra")
                        .and_then(|e| e.get("merchantWallet"))
                        .and_then(|w| w.as_str())
                        .and_then(|w| Pubkey::from_str(w).ok())
                        .map(Address::new);

                    let beneficiary = obj
                        .get("extra")
                        .and_then(|e| e.get("beneficiary"))
                        .and_then(|w| w.as_str())
                        .and_then(|w| Pubkey::from_str(w).ok())
                        .map(Address::new);

                    let institutional = types::SLAEscrowPaymentRequirementsExtra {
                        fee_payer: self.provider.fee_payer().into(),
                        oracle_authorities,
                        escrow_program_id: escrow_config.program_id.into(),
                        bank_address: bank_address.into(),
                        config_address: config_address.into(),
                        fee_bps: fee_bps.into(),
                        oracle_fee_bps: oracle_fee_bps.into(),
                        ttl_seconds: 3600.into(), // Default 1 hour
                        delivery_cutoff_seconds:
                            (crate::sla_escrow_ttl::resolve_delivery_cutoff_seconds().max(0) as u64)
                                .into(),
                        delivery_budget_seconds:
                            (crate::sla_escrow_ttl::resolve_delivery_budget_seconds().max(0) as u64)
                                .into(),
                        max_compute_unit_limit: (crate::chain::TxBudget::FundPayment.cu_limit()
                            as u64)
                            .into(),
                        recommended_compute_unit_price: crate::chain::TxBudget::FundPayment
                            .cu_price()
                            .into(),
                        sla_fund_tx_network_fee_payer: Some("buyer".to_string()),
                        merchant_wallet,
                        beneficiary,
                    };

                    // Merge facilitator institutional fields into the seller's
                    // existing `extra` so delegated-authoring keys (`commitMaterial`,
                    // `intentContractUrl`, `oracleProfiles`, …) survive elevation.
                    let mut merged = obj
                        .get("extra")
                        .and_then(|e| e.as_object())
                        .cloned()
                        .unwrap_or_default();
                    if let Some(inst) = serde_json::to_value(institutional)
                        .ok()
                        .and_then(|v| v.as_object().cloned())
                    {
                        for (k, v) in inst {
                            merged.insert(k, v);
                        }
                    }
                    obj.insert("extra".to_string(), serde_json::Value::Object(merged));

                    tracing::info!(
                        index = i,
                        scheme = %SLAEscrowScheme.as_ref(),
                        "Upgraded Lite challenge with Escrow institutional metadata"
                    );
                }
            }
        }

        Ok(proto::PaymentRequired::V2(pr))
    }
}

/// Submit or acknowledge an SLA-Escrow **FundPayment** transaction for x402 `/settle` (**buyer-paid**
/// message layout only — facilitator-sponsored layouts use [`settle_transaction`] from [`settle`]).
///
/// The tx must be **fully signed** by the buyer (`!is_fully_signed` → error; never call
/// [`settle_transaction`] here — it would overwrite slot 0 with the facilitator key). Then: confirm
/// existing signature on-chain, submit unchanged if pending, or accept “already processed”.
async fn settle_sla_escrow_fund_payment(
    provider: &SolanaChainProvider,
    verification: VerifyTransferResult,
) -> Result<Signature, X402SchemeFacilitatorError> {
    let tx_int = TransactionInt::new(verification.transaction.clone());
    if !tx_int.is_fully_signed() {
        // Only reached when message fee payer is **not** the facilitator (buyer-paid / CLI). Never
        // call [`settle_transaction`] here — it overwrites signature slot 0 with the facilitator key.
        return Err(X402SchemeFacilitatorError::OnchainFailure(
            "escrow settle (buyer-paid): fund transaction must be fully signed by the buyer"
                .to_string(),
        ));
    }

    let primary = *verification.transaction.signatures.first().ok_or_else(|| {
        X402SchemeFacilitatorError::OnchainFailure(
            "escrow settle: transaction has no signatures".to_string(),
        )
    })?;

    if primary == Signature::default() {
        return Err(X402SchemeFacilitatorError::OnchainFailure(
            "escrow settle: primary signature is default".to_string(),
        ));
    }

    match provider
        .rpc_client()
        .get_signature_status_with_commitment(&primary, CommitmentConfig::confirmed())
        .await
    {
        Ok(Some(Ok(()))) => return Ok(primary),
        Ok(Some(Err(e))) => {
            return Err(X402SchemeFacilitatorError::OnchainFailure(format!(
                "fund transaction failed on-chain: {:?}",
                e
            )));
        }
        Ok(None) | Err(_) => {}
    }

    match tx_int
        .send_and_confirm(provider, CommitmentConfig::confirmed())
        .await
    {
        Ok(sig) => Ok(sig),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Blockhash not found")
                || msg.contains("blockhash not found")
                || msg.contains("BlockhashNotFound")
            {
                return Err(crate::chain::solana::SolanaChainProviderError::Transport(
                    "retry build: transaction blockhash has expired or is invalid".to_string(),
                )
                .into());
            }
            if msg.contains("already been processed") || msg.contains("AlreadyProcessed") {
                if matches!(
                    provider
                        .rpc_client()
                        .get_signature_status_with_commitment(
                            &primary,
                            CommitmentConfig::confirmed()
                        )
                        .await,
                    Ok(Some(Ok(())))
                ) {
                    Ok(primary)
                } else {
                    Err(e.into())
                }
            } else {
                Err(e.into())
            }
        }
    }
}

/// Verify a v2 escrow transfer request.
pub async fn verify_transfer(
    provider: &SolanaChainProvider,
    request: &types::VerifyRequest,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    let accepted = &payload.accepted;
    if accepted != requirements {
        return Err(PaymentVerificationError::AcceptedRequirementsMismatch);
    }

    let chain_id = provider.chain_id();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(PaymentVerificationError::UnsupportedChain);
    }

    let transaction_b64_string = &payload.payload.transaction;

    let bytes = Base64Bytes::from(transaction_b64_string.as_bytes())
        .decode()
        .map_err(|e| PaymentVerificationError::InvalidFormat(e.to_string()))?;
    let transaction = decode_versioned_transaction_from_bincode(bytes.as_slice())
        .map_err(PaymentVerificationError::InvalidFormat)?;
    reject_versioned_tx_with_address_lookup_tables(&transaction)
        .map_err(PaymentVerificationError::InvalidFormat)?;

    let instructions = transaction.message.instructions();
    let budget = crate::chain::TxBudget::FundPayment;
    let _compute_units = verify_compute_limit_instruction(budget.cu_limit(), &transaction, 0)?;
    verify_compute_price_instruction(budget.cu_price(), &transaction, 1)?;

    let is_spl_token = requirements.asset.pubkey() != &Pubkey::default();

    let fund_payment_idx = if instructions.len() == 3 {
        2
    } else if instructions.len() == 4 && is_spl_token {
        3
    } else {
        return Err(PaymentVerificationError::TransactionSimulation(
            "InvalidTransactionInstructionsCount".into(),
        ));
    };

    let tx = TransactionInt::new(transaction.clone());
    let fund_instruction = tx
        .instruction(fund_payment_idx)
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

    fund_instruction
        .assert_not_empty()
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

    let program_id = fund_instruction.program_id();
    let escrow_config = provider.sla_escrow().ok_or_else(|| {
        PaymentVerificationError::TransactionSimulation("SLA Escrow not configured".into())
    })?;
    if program_id != escrow_config.program_id {
        return Err(PaymentVerificationError::TransactionSimulation(
            "Invalid SLAEscrow Program ID".into(),
        ));
    }

    let data = fund_instruction.data_slice();
    if data.is_empty() || data[0] != EscrowInstruction::FundPayment as u8 {
        return Err(PaymentVerificationError::TransactionSimulation(
            "Invalid SLAEscrow Instruction".into(),
        ));
    }

    // Instruction data is [discriminator || FundPayment]; on-chain `parse_instruction` strips the
    // byte before `FundPayment::try_from_bytes`; match that here.
    let body_len = mem::size_of::<FundPayment>();
    if data.len() != 1 + body_len {
        return Err(PaymentVerificationError::TransactionSimulation(format!(
            "Invalid FundPayment Data length: expected {}, got {}",
            1 + body_len,
            data.len()
        )));
    }

    let fund_payment = FundPayment::try_from_bytes(&data[1..]).map_err(|e| {
        PaymentVerificationError::TransactionSimulation(format!("Invalid FundPayment Data: {}", e))
    })?;

    let extra = requirements
        .extra
        .as_ref()
        .ok_or(PaymentVerificationError::InvalidFormat(
            "Missing extra requirements".into(),
        ))?;

    let bank_pda = escrow_config.bank_address.ok_or_else(|| {
        PaymentVerificationError::TransactionSimulation(
            "SLA Escrow bank_address not loaded for facilitator".into(),
        )
    })?;

    if extra.bank_address.pubkey() != &bank_pda {
        return Err(PaymentVerificationError::InvalidFormat(
            "paymentRequirements.extra.bankAddress does not match facilitator escrow bank".into(),
        ));
    }

    let (expected_config_pda, _) = provider.get_config_pda(&escrow_config.program_id);
    if extra.config_address.pubkey() != &expected_config_pda {
        return Err(PaymentVerificationError::InvalidFormat(
            "paymentRequirements.extra.configAddress does not match facilitator escrow config"
                .into(),
        ));
    }

    // Canonical Escrow PDA for this mint + bank (matches onboard / builder); `payTo` must be this PDA.
    let (expected_escrow_pda, _) = provider.get_escrow_pda(*requirements.asset.pubkey(), bank_pda);

    if requirements.pay_to.pubkey() != &expected_escrow_pda {
        return Err(PaymentVerificationError::RecipientMismatch);
    }

    // `FundPayment.seller` is the on-chain payout wallet (`payment.seller`), not the escrow PDA.
    // It must match explicit `beneficiary` or `merchantWallet` from requirements (same precedence as post-verify identity).
    let expected_seller = extra
        .beneficiary
        .map(|a| *a.pubkey())
        .or_else(|| extra.merchant_wallet.map(|a| *a.pubkey()))
        .ok_or_else(|| {
            PaymentVerificationError::InvalidFormat(
                "paymentRequirements.extra must include merchantWallet or beneficiary (FundPayment seller / release payout)"
                    .into(),
            )
        })?;
    if expected_seller == expected_escrow_pda {
        return Err(PaymentVerificationError::InvalidFormat(
            "merchantWallet/beneficiary must not be the escrow PDA; use the seller's wallet pubkey"
                .into(),
        ));
    }
    if Pubkey::from(fund_payment.seller.to_bytes()) != expected_seller {
        return Err(PaymentVerificationError::RecipientMismatch);
    }
    if Address::new(Pubkey::from(fund_payment.mint.to_bytes())) != requirements.asset {
        return Err(PaymentVerificationError::AssetMismatch);
    }
    if u64::from_le_bytes(fund_payment.amount) != requirements.amount.inner() {
        return Err(PaymentVerificationError::InvalidPaymentAmount);
    }
    let tx_ttl = u64::from_le_bytes(fund_payment.ttl_seconds);
    let cutoff = i64::try_from(extra.delivery_cutoff_seconds.inner()).unwrap_or(i64::MAX);
    let budget = i64::try_from(extra.delivery_budget_seconds.inner()).unwrap_or(i64::MAX);
    if let Err(e) = crate::sla_escrow_ttl::validate_fund_payment_ttl(
        tx_ttl,
        requirements.max_timeout_seconds,
        cutoff,
        budget,
    ) {
        return Err(PaymentVerificationError::InvalidFormat(e.to_string()));
    }

    // extra already defined above

    let selected_oracle = Address::new(Pubkey::from(fund_payment.oracle_authority.to_bytes()));
    if !extra.oracle_authorities.contains(&selected_oracle) {
        return Err(PaymentVerificationError::TransactionSimulation(
            "Untrusted Oracle authority selected".into(),
        ));
    }

    let buyer_pubkey = fund_instruction
        .account(0)
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

    let fee_payer_pubkey = provider.pubkey();
    let message_fee_payer = transaction
        .message
        .static_account_keys()
        .first()
        .copied()
        .ok_or_else(|| {
            PaymentVerificationError::TransactionSimulation("missing fee payer key".into())
        })?;
    let facilitator_sponsors_fees = message_fee_payer == fee_payer_pubkey;

    for instruction in transaction.message.instructions().iter() {
        for account_idx in instruction.accounts.iter() {
            let account = transaction
                .message
                .static_account_keys()
                .get(*account_idx as usize)
                .ok_or(PaymentVerificationError::TransactionSimulation(
                    "Account not found".into(),
                ))?;

            if *account == fee_payer_pubkey {
                return Err(PaymentVerificationError::TransactionSimulation(
                    "FeePayerIncludedInInstructionAccounts".into(),
                ));
            }
        }
    }

    if facilitator_sponsors_fees && buyer_pubkey != message_fee_payer {
        let need_sigs = transaction.message.header().num_required_signatures as usize;
        if need_sigs < 2 {
            return Err(PaymentVerificationError::TransactionSimulation(
                "Facilitator-sponsored fund tx must list two signers (facilitator fee payer + buyer)"
                    .into(),
            ));
        }
    }

    // Simulation: add facilitator signature only when the message fee payer is the facilitator
    // (`exact`-aligned two-signer shell). Buyer-paid / CLI layouts must keep client signatures intact.
    let signed_tx = if facilitator_sponsors_fees {
        TransactionInt::new(transaction.clone())
            .sign(provider)
            .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?
    } else {
        TransactionInt::new(transaction.clone())
    };

    let cfg = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: false,
        commitment: Some(CommitmentConfig::confirmed()),
        encoding: None,
        accounts: None,
        inner_instructions: false,
        min_context_slot: None,
    };
    let sim_result = provider
        .simulate_transaction_with_config(&signed_tx.inner, cfg)
        .await
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

    if let Some(err) = sim_result.value.err {
        return Err(PaymentVerificationError::TransactionSimulation(format!(
            "Simulation failed on-chain: {:?}",
            err
        )));
    }

    let payer = Address::new(buyer_pubkey);
    let identity = extra.merchant_wallet.unwrap_or(requirements.pay_to);
    let beneficiary = extra.beneficiary.unwrap_or(identity);

    Ok(VerifyTransferResult {
        payer,
        merchant_identity: identity,
        final_beneficiary: beneficiary,
        vault_pda: expected_escrow_pda,
        transaction,
    })
}

/// Helper struct for DB auditing of escrows.
struct EscrowAuditMetadata {
    escrow_pda: String,
    bank_pda: String,
    oracle: String,
    sla_hash: Option<String>,
    /// 64-char lowercase hex of the on-chain `Payment.payment_uid`. The
    /// settlement cron uses this to derive the Payment PDA without
    /// re-decoding the FundPayment instruction.
    payment_uid_hex: Option<String>,
}

/// Re-extract on-chain derivations and instruction details specifically for the audit log.
fn extract_escrow_audit_metadata(
    provider: &SolanaChainProvider,
    request: &types::VerifyRequest,
) -> Result<EscrowAuditMetadata, Box<dyn Error + Send + Sync>> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    // We assume the caller confirmed it's an SLAEscrow request
    let escrow_config = provider.sla_escrow().ok_or("escrow config missing")?;
    let bank_pda = escrow_config.bank_address.ok_or("bank not loaded")?;

    // Derive Escrow PDA
    let (escrow_pda, _) = provider.get_escrow_pda(*requirements.asset.pubkey(), bank_pda);

    // Decode instruction to get Oracle
    let bytes = Base64Bytes::from(payload.payload.transaction.as_bytes()).decode()?;
    let transaction = decode_versioned_transaction_from_bincode(bytes.as_slice())
        .map_err(|e| -> Box<dyn Error + Send + Sync> { e.into() })?;

    // Find FundPayment instruction
    let is_spl_token = requirements.asset.pubkey() != &Pubkey::default();
    let num_instr = transaction.message.instructions().len();
    let fund_idx = if num_instr == 3 {
        2
    } else if num_instr == 4 && is_spl_token {
        3
    } else {
        0
    };

    let instructions = transaction.message.instructions();
    if fund_idx >= instructions.len() {
        return Err("FundPayment index out of bounds".into());
    }

    let instr_data = instructions[fund_idx].data.as_slice();
    if instr_data.is_empty() || instr_data[0] != EscrowInstruction::FundPayment as u8 {
        return Err("invalid or missing FundPayment discriminator".into());
    }
    let body_len = mem::size_of::<FundPayment>();
    if instr_data.len() != 1 + body_len {
        return Err(format!(
            "invalid FundPayment data length: expected {}, got {}",
            1 + body_len,
            instr_data.len()
        )
        .into());
    }
    let fund_payment = FundPayment::try_from_bytes(&instr_data[1..])?;

    let mut sla_hex = String::with_capacity(64);
    for b in fund_payment.sla_hash {
        write!(&mut sla_hex, "{b:02x}").unwrap();
    }

    let mut uid_hex = String::with_capacity(64);
    for b in fund_payment.payment_uid {
        write!(&mut uid_hex, "{b:02x}").unwrap();
    }

    Ok(EscrowAuditMetadata {
        escrow_pda: escrow_pda.to_string(),
        bank_pda: bank_pda.to_string(),
        oracle: Pubkey::from(fund_payment.oracle_authority.to_bytes()).to_string(),
        sla_hash: Some(sla_hex),
        payment_uid_hex: Some(uid_hex),
    })
}

/// Persist [`crate::db::Pr402Db::upsert_escrow_detail`] **after** `record_payment_verify` created `payment_attempts`.
pub async fn persist_escrow_audit_after_verify(
    db: &crate::db::Pr402Db,
    provider: &SolanaChainProvider,
    request: &proto::VerifyRequest,
    correlation_id: &str,
) {
    let request_v2 = match types::VerifyRequest::from_proto(request.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "server_log",
                error = %e,
                correlation_id = %correlation_id,
                "escrow audit after verify: from_proto failed"
            );
            return;
        }
    };
    let metadata = match extract_escrow_audit_metadata(provider, &request_v2) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                target: "server_log",
                error = %e,
                correlation_id = %correlation_id,
                "extract_escrow_audit_metadata failed"
            );
            return;
        }
    };
    if let Err(e) = db
        .upsert_escrow_detail(
            correlation_id,
            &metadata.escrow_pda,
            &metadata.bank_pda,
            &metadata.oracle,
            metadata.sla_hash.as_deref(),
            None,
            metadata.payment_uid_hex.as_deref(),
        )
        .await
    {
        tracing::warn!(
            target: "server_log",
            error = %e,
            correlation_id = %correlation_id,
            "upsert_escrow_detail failed after verify"
        );
    }
}

/// Persist escrow audit row **after** `record_payment_settle` (optionally stores settlement tx id).
pub async fn persist_escrow_audit_after_settle(
    db: &crate::db::Pr402Db,
    provider: &SolanaChainProvider,
    request: &proto::SettleRequest,
    correlation_id: &str,
    fund_signature: Option<&str>,
) {
    let request_v2 = match types::SettleRequest::from_proto(request.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "server_log",
                error = %e,
                correlation_id = %correlation_id,
                "escrow audit after settle: from_proto failed"
            );
            return;
        }
    };
    let metadata = match extract_escrow_audit_metadata(provider, &request_v2) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                target: "server_log",
                error = %e,
                correlation_id = %correlation_id,
                "extract_escrow_audit_metadata failed (settle)"
            );
            return;
        }
    };
    if let Err(e) = db
        .upsert_escrow_detail(
            correlation_id,
            &metadata.escrow_pda,
            &metadata.bank_pda,
            &metadata.oracle,
            metadata.sla_hash.as_deref(),
            fund_signature,
            metadata.payment_uid_hex.as_deref(),
        )
        .await
    {
        tracing::warn!(
            target: "server_log",
            error = %e,
            correlation_id = %correlation_id,
            "upsert_escrow_detail failed after settle"
        );
    }
}
