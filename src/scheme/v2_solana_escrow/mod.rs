//! v2:solana:escrow payment scheme implementation.

pub mod types;

use base64::{engine::general_purpose::STANDARD, Engine};
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
        _db: Option<crate::db::Pr402Db>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn Error>> {
        // SLAEscrow requires SLAEscrowConfig to be present
        if provider.solana.sla_escrow().is_none() {
            return Err("SLAEscrowConfig is missing but escrow scheme is requested".into());
        }
        Ok(Box::new(V2SolanaSLAEscrowFacilitator {
            provider: provider.solana,
        }))
    }
}

pub struct V2SolanaSLAEscrowFacilitator {
    provider: Arc<SolanaChainProvider>,
}

#[async_trait::async_trait]
impl X402SchemeFacilitator for V2SolanaSLAEscrowFacilitator {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeFacilitatorError> {
        let request_v2 = types::VerifyRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request_v2).await?;

        // Escrow audit rows (`escrow_details`) are persisted in `bin/facilitator.rs` *after*
        // `record_payment_verify` creates the parent `payment_attempts` row — this scheme runs
        // before that insert, so upserting here always failed with "Parent payment attempt not found".

        Ok(proto::v2::VerifyResponse::valid(verification.payer.to_string()).into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let request_v2 = types::SettleRequest::from_proto(request.clone())?;
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

        Ok(proto::v2::SettleResponse::Success {
            payer,
            transaction: tx_sig.to_string(),
            network: self.provider.chain_id().to_string(),
        }
        .into())
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

        // In SLA-Escrow, the "Vault" is the Escrow account for a specific mint.
        // Onboarding returns the info for the native SOL escrow by default.
        // Note: This is an idempotent singleton account per Mint/Bank, not per wallet.
        let (escrow_pda, _) = self.provider.get_escrow_pda(Pubkey::default(), bank_pda);
        let (sol_storage_pda, _) =
            self.provider
                .get_sla_escrow_sol_storage_pda(Pubkey::default(), bank_pda, escrow_pda);

        Ok(crate::facilitator::SchemeOnboardInfo {
            label: "SLA Escrow Bank".to_string(),
            role: "Institutional Escrow".to_string(),
            vault_pda: bank_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            token_pda: None,
            fee_bps: fee_bps.into(),
            status: "Active".to_string(),
            is_sovereign: false,
            provisioning_status: None,
        })
    }

    async fn build_onboard_tx(
        &self,
        wallet: &str,
    ) -> Result<proto::v2::BuildPaymentTxResponse, X402SchemeFacilitatorError> {
        // Institutional Consistency: Onboarding always targets the core UniversalSettle SplitVault
        // so that the seller qualifies for the 95 bps rate regardless of payment scheme.
        let seller = solana_pubkey::Pubkey::from_str(wallet)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;
        let us_config = self.provider.universalsettle().ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("UniversalSettle not enabled".to_string())
        })?;

        let ix = crate::chain::solana_universalsettle::build_create_vault_instruction(
            us_config.program_id,
            seller,
            seller,
        );

        let blockhash = self
            .provider
            .rpc_client()
            .get_latest_blockhash()
            .await
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;

        // Manual construction of VersionedTransaction (unsigned shell)
        let message = solana_message::v0::Message::try_compile(&seller, &[ix], &[], blockhash)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;

        let tx = solana_transaction::versioned::VersionedTransaction {
            signatures: vec![solana_signature::Signature::default()],
            message: solana_message::VersionedMessage::V0(message),
        };

        let tx_b64 = STANDARD.encode(bincode::serialize(&tx).unwrap());

        Ok(proto::v2::BuildPaymentTxResponse {
            x402_version: 2,
            transaction: tx_b64,
            recent_blockhash: blockhash.to_string(),
            fee_payer: seller.to_string(),
            payer: seller.to_string(),
            payment_uid: None,
            verify_body_template: serde_json::Value::Null,
            notes: vec!["Sovereign onboarding transaction created. Sign and send to receive the 95 bps institutional discount.".to_string()],
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

                    let extra = types::SLAEscrowPaymentRequirementsExtra {
                        fee_payer: self.provider.fee_payer().into(),
                        oracle_authorities,
                        escrow_program_id: escrow_config.program_id.into(),
                        bank_address: bank_address.into(),
                        config_address: config_address.into(),
                        fee_bps: fee_bps.into(),
                        oracle_fee_bps: oracle_fee_bps.into(),
                        ttl_seconds: 3600.into(), // Default 1 hour
                        sla_fund_tx_network_fee_payer: Some("buyer".to_string()),
                        merchant_wallet,
                        beneficiary,
                    };
                    obj.insert("extra".to_string(), serde_json::to_value(extra).unwrap());

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
            if msg.contains("Blockhash not found") || msg.contains("blockhash not found") || msg.contains("BlockhashNotFound") {
                return Err(crate::chain::solana::SolanaChainProviderError::Transport(
                    "retry build: transaction blockhash has expired or is invalid".to_string(),
                ).into());
            }
            if msg.contains("already been processed") || msg.contains("AlreadyProcessed") {
                if matches!(
                    provider
                        .rpc_client()
                        .get_signature_status_with_commitment(&primary, CommitmentConfig::confirmed())
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
    let compute_units = verify_compute_limit_instruction(&transaction, 0)?;
    if compute_units > provider.max_compute_unit_limit() {
        return Err(PaymentVerificationError::TransactionSimulation(
            "MaxComputeUnitLimitExceeded".into(),
        ));
    }
    verify_compute_price_instruction(provider.max_compute_unit_price(), &transaction, 1)?;

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

    // Canonical Escrow PDA for this mint + bank (matches onboard / builder); `payTo` must be this PDA.
    let (expected_escrow_pda, _) = provider.get_escrow_pda(*requirements.asset.pubkey(), bank_pda);

    if requirements.pay_to.pubkey() != &expected_escrow_pda {
        return Err(PaymentVerificationError::RecipientMismatch);
    }

    if Pubkey::from(fund_payment.seller.to_bytes()) != expected_escrow_pda {
        return Err(PaymentVerificationError::RecipientMismatch);
    }
    if Address::new(Pubkey::from(fund_payment.mint.to_bytes())) != requirements.asset {
        return Err(PaymentVerificationError::AssetMismatch);
    }
    if u64::from_le_bytes(fund_payment.amount) != requirements.amount.inner() {
        return Err(PaymentVerificationError::InvalidPaymentAmount);
    }
    if u64::from_le_bytes(fund_payment.ttl_seconds) < 60 {
        return Err(PaymentVerificationError::TransactionSimulation(
            "TTL too short".into(),
        ));
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
    provider
        .simulate_transaction_with_config(&signed_tx.inner, cfg)
        .await
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

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

    Ok(EscrowAuditMetadata {
        escrow_pda: escrow_pda.to_string(),
        bank_pda: bank_pda.to_string(),
        oracle: Pubkey::from(fund_payment.oracle_authority.to_bytes()).to_string(),
        sla_hash: Some(sla_hex),
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
