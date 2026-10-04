use crate::native_block_body_v2::{NativeBlockBodyV2, VersionedSignedNativeTransaction};
use crate::native_block_execution_v3::{
    InactiveVersionedBlockTransitionV3, InactiveVersionedTransactionTransitionV3,
};
use crate::native_transaction::SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION;
use crate::native_transaction_v2::SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION_V2;
use crate::work::{keccak256, Hash32};

const RECEIPT_DOMAIN_V3: &[u8] = b"NIAHCIA/NATIVE-RECEIPT/V3";
const RECEIPTS_ROOT_DOMAIN_V3: &[u8] = b"NIAHCIA/NATIVE-RECEIPTS-ROOT/V3";
const EMPTY_RECEIPTS_ROOT_DOMAIN_V3: &[u8] = b"NIAHCIA/NATIVE-RECEIPTS-ROOT/V3/EMPTY";
const EXECUTION_ROOT_DOMAIN_V3: &[u8] = b"NIAHCIA/NATIVE-EXECUTION-ROOT/V3";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeReceiptV3 {
    pub transaction_schema_version: u64,
    pub action: u64,
    pub transaction_id: Hash32,
    pub state_root_after: Hash32,
    pub gas_used: u64,
    pub effective_fee_per_gas: u128,
    pub base_fee_burned: u128,
    pub producer_priority_fee: u128,
}

impl NativeReceiptV3 {
    pub const CANONICAL_LEN: usize = 137;

    pub fn commitment(&self) -> Hash32 {
        let mut preimage = Vec::with_capacity(RECEIPT_DOMAIN_V3.len() + Self::CANONICAL_LEN - 1);
        preimage.extend_from_slice(RECEIPT_DOMAIN_V3);
        preimage.extend_from_slice(&self.transaction_schema_version.to_be_bytes());
        preimage.extend_from_slice(&self.action.to_be_bytes());
        preimage.extend_from_slice(&self.transaction_id);
        preimage.extend_from_slice(&self.state_root_after);
        preimage.extend_from_slice(&self.gas_used.to_be_bytes());
        preimage.extend_from_slice(&self.effective_fee_per_gas.to_be_bytes());
        preimage.extend_from_slice(&self.base_fee_burned.to_be_bytes());
        preimage.extend_from_slice(&self.producer_priority_fee.to_be_bytes());
        keccak256(&preimage)
    }

    pub fn canonical_bytes(&self) -> [u8; Self::CANONICAL_LEN] {
        let mut out = [0_u8; Self::CANONICAL_LEN];
        out[0] = 3;
        out[1..9].copy_from_slice(&self.transaction_schema_version.to_be_bytes());
        out[9..17].copy_from_slice(&self.action.to_be_bytes());
        out[17..49].copy_from_slice(&self.transaction_id);
        out[49..81].copy_from_slice(&self.state_root_after);
        out[81..89].copy_from_slice(&self.gas_used.to_be_bytes());
        out[89..105].copy_from_slice(&self.effective_fee_per_gas.to_be_bytes());
        out[105..121].copy_from_slice(&self.base_fee_burned.to_be_bytes());
        out[121..137].copy_from_slice(&self.producer_priority_fee.to_be_bytes());
        out
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != Self::CANONICAL_LEN {
            return Err(format!(
                "invalid native receipt V3 length: expected {}, found {}",
                Self::CANONICAL_LEN,
                bytes.len()
            ));
        }
        if bytes[0] != 3 {
            return Err(format!(
                "unsupported native receipt V3 version: {}",
                bytes[0]
            ));
        }

        Ok(Self {
            transaction_schema_version: u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
            action: u64::from_be_bytes(bytes[9..17].try_into().unwrap()),
            transaction_id: bytes[17..49].try_into().unwrap(),
            state_root_after: bytes[49..81].try_into().unwrap(),
            gas_used: u64::from_be_bytes(bytes[81..89].try_into().unwrap()),
            effective_fee_per_gas: u128::from_be_bytes(bytes[89..105].try_into().unwrap()),
            base_fee_burned: u128::from_be_bytes(bytes[105..121].try_into().unwrap()),
            producer_priority_fee: u128::from_be_bytes(bytes[121..137].try_into().unwrap()),
        })
    }
}

pub fn native_receipts_root_v3(receipts: &[NativeReceiptV3]) -> Hash32 {
    if receipts.is_empty() {
        return keccak256(EMPTY_RECEIPTS_ROOT_DOMAIN_V3);
    }

    let mut preimage = Vec::with_capacity(RECEIPTS_ROOT_DOMAIN_V3.len() + receipts.len() * 32);
    preimage.extend_from_slice(RECEIPTS_ROOT_DOMAIN_V3);
    for receipt in receipts {
        preimage.extend_from_slice(&receipt.commitment());
    }
    keccak256(&preimage)
}

pub fn native_execution_root_v3(
    transactions_root: Hash32,
    state_root: Hash32,
    receipts_root: Hash32,
    gas_used: u64,
    base_fee_burned: u128,
    producer_priority_fee: u128,
) -> Hash32 {
    let mut preimage = Vec::with_capacity(EXECUTION_ROOT_DOMAIN_V3.len() + 136);
    preimage.extend_from_slice(EXECUTION_ROOT_DOMAIN_V3);
    preimage.extend_from_slice(&transactions_root);
    preimage.extend_from_slice(&state_root);
    preimage.extend_from_slice(&receipts_root);
    preimage.extend_from_slice(&gas_used.to_be_bytes());
    preimage.extend_from_slice(&base_fee_burned.to_be_bytes());
    preimage.extend_from_slice(&producer_priority_fee.to_be_bytes());
    keccak256(&preimage)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBlockExecutionResultV3 {
    pub transactions_root: Hash32,
    pub state_root: Hash32,
    pub receipts_root: Hash32,
    pub execution_root: Hash32,
    pub gas_used: u64,
    pub base_fee_burned: u128,
    pub producer_priority_fee: u128,
    pub receipts: Vec<NativeReceiptV3>,
}

impl NativeBlockExecutionResultV3 {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        validate_result(self)?;

        let receipt_count = u64::try_from(self.receipts.len())
            .map_err(|_| "native receipt V3 count exceeds u64".to_string())?;
        let receipts_len = self
            .receipts
            .len()
            .checked_mul(NativeReceiptV3::CANONICAL_LEN)
            .ok_or_else(|| "native execution result V3 length overflow".to_string())?;

        let mut out = Vec::with_capacity(177usize.saturating_add(receipts_len));
        out.push(3);
        out.extend_from_slice(&self.transactions_root);
        out.extend_from_slice(&self.state_root);
        out.extend_from_slice(&self.receipts_root);
        out.extend_from_slice(&self.execution_root);
        out.extend_from_slice(&self.gas_used.to_be_bytes());
        out.extend_from_slice(&self.base_fee_burned.to_be_bytes());
        out.extend_from_slice(&self.producer_priority_fee.to_be_bytes());
        out.extend_from_slice(&receipt_count.to_be_bytes());
        for receipt in &self.receipts {
            out.extend_from_slice(&receipt.canonical_bytes());
        }
        Ok(out)
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, String> {
        const HEADER_LEN: usize = 177;

        if bytes.len() < HEADER_LEN {
            return Err("native execution result V3 is truncated".into());
        }
        if bytes[0] != 3 {
            return Err(format!(
                "unsupported native execution result V3 version: {}",
                bytes[0]
            ));
        }

        let receipt_count =
            usize::try_from(u64::from_be_bytes(bytes[169..177].try_into().unwrap()))
                .map_err(|_| "native receipt V3 count exceeds platform limits".to_string())?;
        let receipts_len = receipt_count
            .checked_mul(NativeReceiptV3::CANONICAL_LEN)
            .ok_or_else(|| "native execution result V3 length overflow".to_string())?;
        let expected_len = HEADER_LEN
            .checked_add(receipts_len)
            .ok_or_else(|| "native execution result V3 length overflow".to_string())?;
        if bytes.len() != expected_len {
            return Err(format!(
                "native execution result V3 length mismatch: expected {expected_len}, found {}",
                bytes.len()
            ));
        }

        let mut receipts = Vec::with_capacity(receipt_count);
        for index in 0..receipt_count {
            let start = HEADER_LEN + index * NativeReceiptV3::CANONICAL_LEN;
            let end = start + NativeReceiptV3::CANONICAL_LEN;
            receipts.push(NativeReceiptV3::from_canonical_bytes(&bytes[start..end])?);
        }

        let result = Self {
            transactions_root: bytes[1..33].try_into().unwrap(),
            state_root: bytes[33..65].try_into().unwrap(),
            receipts_root: bytes[65..97].try_into().unwrap(),
            execution_root: bytes[97..129].try_into().unwrap(),
            gas_used: u64::from_be_bytes(bytes[129..137].try_into().unwrap()),
            base_fee_burned: u128::from_be_bytes(bytes[137..153].try_into().unwrap()),
            producer_priority_fee: u128::from_be_bytes(bytes[153..169].try_into().unwrap()),
            receipts,
        };
        validate_result(&result)?;
        Ok(result)
    }
}

fn validate_result(result: &NativeBlockExecutionResultV3) -> Result<(), String> {
    if native_receipts_root_v3(&result.receipts) != result.receipts_root {
        return Err("native execution V3 receipts root mismatch".into());
    }

    let mut gas_used = 0u64;
    let mut base_fee_burned = 0u128;
    let mut producer_priority_fee = 0u128;

    for receipt in &result.receipts {
        gas_used = gas_used
            .checked_add(receipt.gas_used)
            .ok_or_else(|| "native execution V3 receipt gas overflow".to_string())?;
        base_fee_burned = base_fee_burned
            .checked_add(receipt.base_fee_burned)
            .ok_or_else(|| "native execution V3 receipt base fee overflow".to_string())?;
        producer_priority_fee = producer_priority_fee
            .checked_add(receipt.producer_priority_fee)
            .ok_or_else(|| "native execution V3 receipt priority fee overflow".to_string())?;
    }

    if gas_used != result.gas_used {
        return Err("native execution V3 gas accounting mismatch".into());
    }
    if base_fee_burned != result.base_fee_burned {
        return Err("native execution V3 base fee accounting mismatch".into());
    }
    if producer_priority_fee != result.producer_priority_fee {
        return Err("native execution V3 priority fee accounting mismatch".into());
    }

    let execution_root = native_execution_root_v3(
        result.transactions_root,
        result.state_root,
        result.receipts_root,
        result.gas_used,
        result.base_fee_burned,
        result.producer_priority_fee,
    );
    if execution_root != result.execution_root {
        return Err("native execution V3 root mismatch".into());
    }

    if let Some(last) = result.receipts.last() {
        if last.state_root_after != result.state_root {
            return Err("native execution V3 final receipt state root mismatch".into());
        }
    }

    Ok(())
}

pub fn build_inactive_execution_result_v3(
    body: &NativeBlockBodyV2,
    transition: &InactiveVersionedBlockTransitionV3,
) -> Result<NativeBlockExecutionResultV3, String> {
    if transition.transactions_root != body.transactions_root() {
        return Err("inactive V3 transition transactions root does not match block body".into());
    }

    let decoded = body.decoded_transactions()?;
    if decoded.len() != transition.transactions.len() {
        return Err("inactive V3 transition count does not match block body".into());
    }

    let mut receipts = Vec::with_capacity(decoded.len());
    let mut gas_used = 0u64;
    let mut base_fee_burned = 0u128;
    let mut producer_priority_fee = 0u128;

    for (transaction, applied) in decoded.iter().zip(&transition.transactions) {
        let (schema_version, action, transaction_id) = match transaction {
            VersionedSignedNativeTransaction::V1(transaction) => (
                SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION,
                transaction.body.action as u64,
                transaction.tx_id()?,
            ),
            VersionedSignedNativeTransaction::V2(transaction) => (
                SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION_V2,
                transaction.body.action as u64,
                transaction.tx_id()?,
            ),
        };

        if transaction_id != applied.transaction_id() {
            return Err("inactive V3 transition transaction ID mismatch".into());
        }

        let (receipt_gas, effective_fee, receipt_burn, receipt_priority) = match applied {
            InactiveVersionedTransactionTransitionV3::V1Transfer { outcome, .. } => (
                outcome.gas_used,
                outcome.effective_fee_per_gas,
                outcome.base_fee_burned,
                outcome.producer_priority_fee,
            ),
            InactiveVersionedTransactionTransitionV3::V2Transfer { outcome, .. } => (
                outcome.gas_used,
                outcome.effective_fee_per_gas,
                outcome.base_fee_burned,
                outcome.producer_priority_fee,
            ),
            InactiveVersionedTransactionTransitionV3::Compute { result, .. } => (
                result.fee.gas_used,
                result.fee.effective_fee_per_gas,
                result.fee.base_fee_burned,
                result.fee.producer_priority_fee,
            ),
            InactiveVersionedTransactionTransitionV3::ContractCreate(result) => (
                result.gas_used,
                result.effective_fee_per_gas,
                result.base_fee_burned,
                result.producer_priority_fee,
            ),
            InactiveVersionedTransactionTransitionV3::ContractCall(result) => (
                result.gas_used,
                result.effective_fee_per_gas,
                result.base_fee_burned,
                result.producer_priority_fee,
            ),
        };

        gas_used = gas_used
            .checked_add(receipt_gas)
            .ok_or_else(|| "native execution V3 gas overflow".to_string())?;
        base_fee_burned = base_fee_burned
            .checked_add(receipt_burn)
            .ok_or_else(|| "native execution V3 base fee overflow".to_string())?;
        producer_priority_fee = producer_priority_fee
            .checked_add(receipt_priority)
            .ok_or_else(|| "native execution V3 priority fee overflow".to_string())?;

        receipts.push(NativeReceiptV3 {
            transaction_schema_version: schema_version,
            action,
            transaction_id,
            state_root_after: applied.state_root_after(),
            gas_used: receipt_gas,
            effective_fee_per_gas: effective_fee,
            base_fee_burned: receipt_burn,
            producer_priority_fee: receipt_priority,
        });
    }

    if producer_priority_fee != transition.producer_priority_fee {
        return Err("inactive V3 transition priority fee total mismatch".into());
    }

    let receipts_root = native_receipts_root_v3(&receipts);
    let execution_root = native_execution_root_v3(
        transition.transactions_root,
        transition.state_root_after,
        receipts_root,
        gas_used,
        base_fee_burned,
        producer_priority_fee,
    );

    let result = NativeBlockExecutionResultV3 {
        transactions_root: transition.transactions_root,
        state_root: transition.state_root_after,
        receipts_root,
        execution_root,
        gas_used,
        base_fee_burned,
        producer_priority_fee,
        receipts,
    };
    validate_result(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_transaction::NativeActionV1;
    use crate::native_transaction_v2::NativeActionV2;

    fn receipt(marker: u8, schema: u64, action: u64) -> NativeReceiptV3 {
        NativeReceiptV3 {
            transaction_schema_version: schema,
            action,
            transaction_id: [marker; 32],
            state_root_after: [marker.wrapping_add(1); 32],
            gas_used: 1_000,
            effective_fee_per_gas: 3,
            base_fee_burned: 2_000,
            producer_priority_fee: 1_000,
        }
    }

    #[test]
    fn receipt_v3_round_trip_and_commitment_are_deterministic() {
        let value = receipt(
            0x11,
            SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION_V2,
            NativeActionV2::Transfer as u64,
        );
        let bytes = value.canonical_bytes();
        let decoded = NativeReceiptV3::from_canonical_bytes(&bytes).unwrap();

        assert_eq!(decoded, value);
        assert_eq!(decoded.commitment(), value.commitment());
        assert_eq!(bytes[0], 3);
    }

    #[test]
    fn execution_result_v3_round_trip_recomputes_commitments() {
        let receipts = vec![
            receipt(
                0x21,
                SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION,
                NativeActionV1::Transfer as u64,
            ),
            receipt(
                0x31,
                SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION_V2,
                NativeActionV2::Transfer as u64,
            ),
        ];
        let receipts_root = native_receipts_root_v3(&receipts);
        let state_root = receipts.last().unwrap().state_root_after;
        let gas_used = 2_000;
        let base_fee_burned = 4_000;
        let producer_priority_fee = 2_000;
        let transactions_root = [0x44; 32];
        let execution_root = native_execution_root_v3(
            transactions_root,
            state_root,
            receipts_root,
            gas_used,
            base_fee_burned,
            producer_priority_fee,
        );
        let result = NativeBlockExecutionResultV3 {
            transactions_root,
            state_root,
            receipts_root,
            execution_root,
            gas_used,
            base_fee_burned,
            producer_priority_fee,
            receipts,
        };

        let bytes = result.canonical_bytes().unwrap();
        let decoded = NativeBlockExecutionResultV3::from_canonical_bytes(&bytes).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn execution_result_v3_rejects_receipt_tampering() {
        let receipts = vec![receipt(
            0x41,
            SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION_V2,
            NativeActionV2::Transfer as u64,
        )];
        let receipts_root = native_receipts_root_v3(&receipts);
        let state_root = receipts[0].state_root_after;
        let transactions_root = [0x55; 32];
        let execution_root = native_execution_root_v3(
            transactions_root,
            state_root,
            receipts_root,
            1_000,
            2_000,
            1_000,
        );
        let result = NativeBlockExecutionResultV3 {
            transactions_root,
            state_root,
            receipts_root,
            execution_root,
            gas_used: 1_000,
            base_fee_burned: 2_000,
            producer_priority_fee: 1_000,
            receipts,
        };

        let mut bytes = result.canonical_bytes().unwrap();
        let receipt_start = 177;
        bytes[receipt_start + 17] ^= 0x01;
        assert!(NativeBlockExecutionResultV3::from_canonical_bytes(&bytes).is_err());
    }

    #[test]
    fn locked_execution_v3_interoperability_vector() {
        let receipts = vec![
            receipt(
                0x21,
                SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION,
                NativeActionV1::Transfer as u64,
            ),
            receipt(
                0x31,
                SIGNED_NATIVE_TRANSACTION_SCHEMA_VERSION_V2,
                NativeActionV2::Transfer as u64,
            ),
        ];

        assert_eq!(
            hex::encode(receipts[0].canonical_bytes()),
            "03000000000000000100000000000000002121212121212121212121212121212121212121212121212121212121212121222222222222222222222222222222222222222222222222222222222222222200000000000003e800000000000000000000000000000003000000000000000000000000000007d0000000000000000000000000000003e8"
        );
        assert_eq!(
            hex::encode(receipts[0].commitment()),
            "ca4b6c34bdc6cbbad624f19e66136ba704dae82bc84771618404fcfb4c821545"
        );
        assert_eq!(
            hex::encode(receipts[1].canonical_bytes()),
            "03000000000000000200000000000000003131313131313131313131313131313131313131313131313131313131313131323232323232323232323232323232323232323232323232323232323232323200000000000003e800000000000000000000000000000003000000000000000000000000000007d0000000000000000000000000000003e8"
        );
        assert_eq!(
            hex::encode(receipts[1].commitment()),
            "d3ac77adfafe52fa93755f0a7df3c6cae63d504e4aab2bd1c80d4b91bf9c8073"
        );

        let receipts_root = native_receipts_root_v3(&receipts);
        assert_eq!(
            hex::encode(receipts_root),
            "a82b19ab8c6e367bf624744461fd0738d87fe117602e2b7a1cedb110c14918aa"
        );

        let transactions_root = [0x44; 32];
        let state_root = receipts[1].state_root_after;
        let gas_used = 2_000;
        let base_fee_burned = 4_000;
        let producer_priority_fee = 2_000;
        let execution_root = native_execution_root_v3(
            transactions_root,
            state_root,
            receipts_root,
            gas_used,
            base_fee_burned,
            producer_priority_fee,
        );
        assert_eq!(
            hex::encode(execution_root),
            "ada148e868ec43580565a5b87c6d0b129958138b1ae0f2bc0bf3b07fc2a1b6e7"
        );

        let result = NativeBlockExecutionResultV3 {
            transactions_root,
            state_root,
            receipts_root,
            execution_root,
            gas_used,
            base_fee_burned,
            producer_priority_fee,
            receipts,
        };
        let encoded = result.canonical_bytes().unwrap();
        assert_eq!(encoded.len(), 451);
        assert_eq!(
            hex::encode(encoded),
            "0344444444444444444444444444444444444444444444444444444444444444443232323232323232323232323232323232323232323232323232323232323232a82b19ab8c6e367bf624744461fd0738d87fe117602e2b7a1cedb110c14918aaada148e868ec43580565a5b87c6d0b129958138b1ae0f2bc0bf3b07fc2a1b6e700000000000007d000000000000000000000000000000fa0000000000000000000000000000007d0000000000000000203000000000000000100000000000000002121212121212121212121212121212121212121212121212121212121212121222222222222222222222222222222222222222222222222222222222222222200000000000003e800000000000000000000000000000003000000000000000000000000000007d0000000000000000000000000000003e803000000000000000200000000000000003131313131313131313131313131313131313131313131313131313131313131323232323232323232323232323232323232323232323232323232323232323200000000000003e800000000000000000000000000000003000000000000000000000000000007d0000000000000000000000000000003e8"
        );
    }

    #[test]
    fn empty_receipt_root_is_domain_separated_from_v1() {
        assert_ne!(
            native_receipts_root_v3(&[]),
            crate::native_execution::native_receipts_root_v1(&[])
        );
    }
}
