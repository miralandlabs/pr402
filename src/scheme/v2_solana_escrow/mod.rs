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

use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

use crate::chain::solana_sla_escrow::SLAEscrowInstruction;

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
        let request = types::VerifyRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request).await?;
        Ok(proto::v2::VerifyResponse::valid(verification.payer.to_string()).into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let request = types::SettleRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request).await?;
        let payer = verification.payer.to_string();

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
            let oracle_authority = self.provider.fee_payer();

            let extra = Some(
                serde_json::to_value(types::SLAEscrowPaymentRequirementsExtra {
                    fee_payer: fee_payer.into(),
                    oracle_authority: oracle_authority.into(),
                    escrow_program_id: escrow_config.program_id.into(),
                    ttl_seconds: 3600, // Default 1 hour
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
            fee_bps,
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
    if data.is_empty() || data[0] != SLAEscrowInstruction::FundPayment as u8 {
        return Err(PaymentVerificationError::TransactionSimulation(
            "Invalid SLAEscrow Instruction".into(),
        ));
    }
    if data.len() < 209 {
        // Minimal length for FundPayment
        return Err(PaymentVerificationError::TransactionSimulation(
            "Invalid FundPayment Data Length".into(),
        ));
    }

    let extra = requirements
        .extra
        .as_ref()
        .ok_or(PaymentVerificationError::InvalidFormat(
            "Missing extra requirements".into(),
        ))?;

    let seller = Pubkey::new_from_array(data[1..33].try_into().unwrap());
    let mint = Pubkey::new_from_array(data[33..65].try_into().unwrap());
    let amount = read_u64_le(&data[65..73]);
    let ttl_seconds = read_u64_le(&data[73..81]);
    let oracle_authority = Pubkey::new_from_array(data[177..209].try_into().unwrap());

    if Address::new(seller) != requirements.pay_to {
        return Err(PaymentVerificationError::RecipientMismatch);
    }
    if Address::new(mint) != requirements.asset {
        return Err(PaymentVerificationError::AssetMismatch);
    }
    if amount != requirements.amount.inner() {
        return Err(PaymentVerificationError::InvalidPaymentAmount);
    }
    if ttl_seconds < 60 {
        return Err(PaymentVerificationError::TransactionSimulation(
            "TTL too short".into(),
        ));
    }
    if Address::new(oracle_authority) != extra.oracle_authority {
        return Err(PaymentVerificationError::TransactionSimulation(
            "Oracle authority mismatch".into(),
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
    let beneficiary = Address::new(seller);
    Ok(VerifyTransferResult {
        payer,
        beneficiary,
        transaction,
    })
}

fn read_u64_le(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[..8]);
    u64::from_le_bytes(buf)
}
