//! v2:solana:escrow payment scheme implementation.

pub mod types;

use std::collections::HashMap;
use std::error::Error;
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
use crate::util::Base64Bytes;
use sla_escrow_api::instruction::{EscrowInstruction, FundPayment};

use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

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

        // Use the shared settle_transaction logic
        let tx_sig = settle_transaction(&self.provider, verification).await?;

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
                    ttl_seconds: 3600.into(), // Default 1 hour
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
        let (escrow_pda, _) = self.provider.get_escrow_pda(Pubkey::default(), bank_pda);
        let (sol_storage_pda, _) =
            self.provider
                .get_sla_escrow_sol_storage_pda(Pubkey::default(), bank_pda, escrow_pda);

        Ok(crate::facilitator::SchemeOnboardInfo {
            vault_pda: escrow_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            fee_bps: fee_bps.into(),
            status: "Active".to_string(),
        })
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
    let transaction = bincode::deserialize::<VersionedTransaction>(bytes.as_slice())
        .map_err(|e| PaymentVerificationError::InvalidFormat(e.to_string()))?;

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
    let escrow_config = provider.sla_escrow().expect("SLAEscrow config missing");
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

    let fund_payment = FundPayment::try_from_bytes(data).map_err(|e| {
        PaymentVerificationError::TransactionSimulation(format!("Invalid FundPayment Data: {}", e))
    })?;

    if Address::new(Pubkey::from(fund_payment.seller.to_bytes())) != requirements.pay_to {
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

    let extra = requirements
        .extra
        .as_ref()
        .ok_or(PaymentVerificationError::InvalidFormat(
            "Missing extra requirements".into(),
        ))?;

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

    let signed_tx = TransactionInt::new(transaction.clone())
        .sign(provider)
        .map_err(|e| PaymentVerificationError::TransactionSimulation(e.to_string()))?;

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
    let beneficiary = Address::new(Pubkey::from(fund_payment.seller.to_bytes()));
    Ok(VerifyTransferResult {
        payer,
        beneficiary,
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
    let transaction = bincode::deserialize::<VersionedTransaction>(bytes.as_slice())?;

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

    let instr_data = &instructions[fund_idx].data;
    let fund_payment = FundPayment::try_from_bytes(instr_data)?;

    Ok(EscrowAuditMetadata {
        escrow_pda: escrow_pda.to_string(),
        bank_pda: bank_pda.to_string(),
        oracle: Pubkey::from(fund_payment.oracle_authority.to_bytes()).to_string(),
        sla_hash: None, // Could be extracted if present in data, currently placeholder
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
