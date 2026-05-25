//! On-chain `Payment` account decoding and settlement decision matrix.

use solana_pubkey::Pubkey;

/// Minimal projection of the on-chain `Payment` struct.
///
/// Field offsets per `oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md` §4.4
/// (body offsets, after the 8-byte account header per §1.4).
#[derive(Debug, Clone)]
pub struct PaymentView {
    pub payment_uid: [u8; 32],
    pub buyer: Pubkey,
    pub seller: Pubkey,
    pub mint: Pubkey,
    pub oracle_authority: Pubkey,
    pub expires_at: i64,
    pub delivery_timestamp: i64,
    pub closed_at: i64,
    pub oracle_fee_bps: u16,
    pub state: u8,
    pub resolution_state: u8,
}

impl PaymentView {
    pub fn payment_uid_hex(&self) -> String {
        self.payment_uid
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementAction {
    Release,
    Refund,
    SkipPreOutcome,
    #[allow(dead_code)]
    SkipBuyerOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    Close,
    SkipNotTerminal,
    SkipClosureDelay,
}

/// Decide which permissionless settlement path applies given on-chain state.
pub fn decide_settlement_action(payment: &PaymentView, is_expired: bool) -> SettlementAction {
    match payment.resolution_state {
        1 => SettlementAction::Release,
        2 => SettlementAction::Refund,
        _ => {
            if is_expired {
                if payment.delivery_timestamp != 0 {
                    SettlementAction::Release
                } else {
                    SettlementAction::Refund
                }
            } else {
                SettlementAction::SkipPreOutcome
            }
        }
    }
}

/// Decide whether `ClosePayment` is allowed (terminal state + closure delay elapsed).
pub fn decide_close_action(payment: &PaymentView, now_unix: i64) -> CloseAction {
    if payment.state == 0 {
        return CloseAction::SkipNotTerminal;
    }
    if now_unix <= payment.closed_at {
        return CloseAction::SkipClosureDelay;
    }
    CloseAction::Close
}

/// Decode a `Payment` account's raw bytes per ABI §4.4.
pub fn decode_payment_view(data: &[u8]) -> Result<PaymentView, String> {
    const HEADER_LEN: usize = 8;
    const BODY_LEN: usize = 376;
    if data.len() < HEADER_LEN + BODY_LEN {
        return Err(format!(
            "account data too small: expected >= {} bytes, got {}",
            HEADER_LEN + BODY_LEN,
            data.len()
        ));
    }

    if data[0] != 103 {
        return Err(format!(
            "account discriminator mismatch: expected 103 (Payment), got {}",
            data[0]
        ));
    }

    let body = &data[HEADER_LEN..HEADER_LEN + BODY_LEN];

    let mut payment_uid = [0u8; 32];
    payment_uid.copy_from_slice(&body[0..32]);
    let mut buyer = [0u8; 32];
    buyer.copy_from_slice(&body[64..96]);
    let mut seller = [0u8; 32];
    seller.copy_from_slice(&body[96..128]);
    let mut mint = [0u8; 32];
    mint.copy_from_slice(&body[128..160]);
    let mut oracle = [0u8; 32];
    oracle.copy_from_slice(&body[160..192]);

    let expires_at = i64::from_le_bytes(
        body[312..320]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let delivery_timestamp = i64::from_le_bytes(
        body[328..336]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let closed_at = i64::from_le_bytes(
        body[320..328]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let oracle_fee_bps = u16::from_le_bytes(
        body[372..374]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let state = body[374];
    let resolution_state = body[375];

    Ok(PaymentView {
        payment_uid,
        buyer: Pubkey::from(buyer),
        seller: Pubkey::from(seller),
        mint: Pubkey::from(mint),
        oracle_authority: Pubkey::from(oracle),
        expires_at,
        delivery_timestamp,
        closed_at,
        oracle_fee_bps,
        state,
        resolution_state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_not_expired_skips() {
        let p = PaymentView {
            payment_uid: [0; 32],
            buyer: Pubkey::new_unique(),
            seller: Pubkey::new_unique(),
            mint: Pubkey::default(),
            oracle_authority: Pubkey::new_unique(),
            expires_at: i64::MAX,
            delivery_timestamp: 0,
            closed_at: 0,
            oracle_fee_bps: 0,
            state: 0,
            resolution_state: 0,
        };
        assert_eq!(
            decide_settlement_action(&p, false),
            SettlementAction::SkipPreOutcome
        );
    }

    #[test]
    fn approved_releases() {
        let p = PaymentView {
            payment_uid: [1; 32],
            buyer: Pubkey::new_unique(),
            seller: Pubkey::new_unique(),
            mint: Pubkey::default(),
            oracle_authority: Pubkey::new_unique(),
            expires_at: 0,
            delivery_timestamp: 0,
            closed_at: 0,
            oracle_fee_bps: 0,
            state: 0,
            resolution_state: 1,
        };
        assert_eq!(
            decide_settlement_action(&p, false),
            SettlementAction::Release
        );
    }
}
