use crate::address::AddressNetwork;
use crate::native_block_body_v2::{NativeBlockBodyV2, VersionedSignedNativeTransaction};
use crate::native_compute_fee_v1::{
    execute_inactive_compute_transaction_with_fee_v2, InactiveComputeFeeExecutionResultV1,
};
use crate::native_contract_execution_v1::{
    execute_inactive_accepted_contract_call_v1, execute_inactive_accepted_contract_create_v1,
    InactiveAcceptedContractCallV1, InactiveAcceptedContractCreateV1,
};
use crate::native_contract_runtime_registry_v1::NativeContractRuntimeRegistryV1;
use crate::native_execution::{
    execute_transfer_v1, NativeExecutionContextV1, NativeTransferOutcomeV1,
};
use crate::native_execution_v2::{execute_transfer_v2, NativeTransferOutcomeV2};
use crate::native_state_v3::NativeStateV3;
use crate::native_transaction::NativeActionV1;
use crate::native_transaction_v2::NativeActionV2;
use crate::work::Hash32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InactiveVersionedTransactionTransitionV3 {
    V1Transfer {
        transaction_id: Hash32,
        state_root_before: Hash32,
        state_root_after: Hash32,
        outcome: NativeTransferOutcomeV1,
    },
    V2Transfer {
        transaction_id: Hash32,
        state_root_before: Hash32,
        state_root_after: Hash32,
        outcome: NativeTransferOutcomeV2,
    },
    Compute {
        result: Box<InactiveComputeFeeExecutionResultV1>,
        state_root_before: Hash32,
        state_root_after: Hash32,
    },
    ContractCreate(InactiveAcceptedContractCreateV1),
    ContractCall(InactiveAcceptedContractCallV1),
}

impl InactiveVersionedTransactionTransitionV3 {
    pub fn transaction_id(&self) -> Hash32 {
        match self {
            Self::V1Transfer { transaction_id, .. } | Self::V2Transfer { transaction_id, .. } => {
                *transaction_id
            }
            Self::Compute { result, .. } => result.execution.transition.transaction_id(),
            Self::ContractCreate(result) => result.transaction_id,
            Self::ContractCall(result) => result.transaction_id,
        }
    }
    pub fn state_root_before(&self) -> Hash32 {
        match self {
            Self::V1Transfer {
                state_root_before, ..
            }
            | Self::V2Transfer {
                state_root_before, ..
            }
            | Self::Compute {
                state_root_before, ..
            } => *state_root_before,
            Self::ContractCreate(result) => result.state_root_before,
            Self::ContractCall(result) => result.state_root_before,
        }
    }
    pub fn state_root_after(&self) -> Hash32 {
        match self {
            Self::V1Transfer {
                state_root_after, ..
            }
            | Self::V2Transfer {
                state_root_after, ..
            }
            | Self::Compute {
                state_root_after, ..
            } => *state_root_after,
            Self::ContractCreate(result) => result.state_root_after,
            Self::ContractCall(result) => result.state_root_after,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InactiveVersionedBlockTransitionV3 {
    pub transactions_root: Hash32,
    pub state_root_before: Hash32,
    pub state_root_after: Hash32,
    pub producer_priority_fee: u128,
    pub transactions: Vec<InactiveVersionedTransactionTransitionV3>,
}

pub fn execute_inactive_versioned_block_v3(
    state: &mut NativeStateV3,
    registry: &NativeContractRuntimeRegistryV1,
    body: &NativeBlockBodyV2,
    network: AddressNetwork,
    current_height: u64,
    base_fee_per_gas: u128,
) -> Result<InactiveVersionedBlockTransitionV3, String> {
    let state_root_before = state.state_root()?;
    let mut next = state.clone();
    let mut transitions = Vec::with_capacity(body.transactions.len());
    let mut producer_priority_fee = 0u128;

    for transaction in body.decoded_transactions()? {
        let before = next.state_root()?;
        let context = NativeExecutionContextV1 {
            base_fee_per_gas,
            cpu_producer: body.producer_fee_recipient,
        };
        let transition = match transaction {
            VersionedSignedNativeTransaction::V1(transaction) => {
                if transaction.body.action != NativeActionV1::Transfer {
                    return Err(
                        "inactive V3 block executor does not execute V1 contract actions".into(),
                    );
                }
                let outcome = execute_transfer_v1(
                    next.base_mut().accounts_mut(),
                    &transaction,
                    network,
                    context,
                )?;
                producer_priority_fee = producer_priority_fee
                    .checked_add(outcome.producer_priority_fee)
                    .ok_or_else(|| {
                        "inactive V3 block producer priority fee overflow".to_string()
                    })?;
                InactiveVersionedTransactionTransitionV3::V1Transfer {
                    transaction_id: transaction.tx_id()?,
                    state_root_before: before,
                    state_root_after: next.state_root()?,
                    outcome,
                }
            }
            VersionedSignedNativeTransaction::V2(transaction) => match transaction.body.action {
                NativeActionV2::Transfer => {
                    let outcome =
                        execute_transfer_v2(next.base_mut(), &transaction, network, context)?;
                    producer_priority_fee = producer_priority_fee
                        .checked_add(outcome.producer_priority_fee)
                        .ok_or_else(|| {
                            "inactive V3 block producer priority fee overflow".to_string()
                        })?;
                    InactiveVersionedTransactionTransitionV3::V2Transfer {
                        transaction_id: transaction.tx_id()?,
                        state_root_before: before,
                        state_root_after: next.state_root()?,
                        outcome,
                    }
                }
                NativeActionV2::ComputeChannelOpen
                | NativeActionV2::ComputeChannelSettle
                | NativeActionV2::ComputeChannelRefund => {
                    let result = execute_inactive_compute_transaction_with_fee_v2(
                        next.base_mut(),
                        &transaction,
                        network,
                        current_height,
                        context,
                    )?;
                    producer_priority_fee = producer_priority_fee
                        .checked_add(result.fee.producer_priority_fee)
                        .ok_or_else(|| {
                            "inactive V3 block producer priority fee overflow".to_string()
                        })?;
                    let state_root_after = next.state_root()?;
                    InactiveVersionedTransactionTransitionV3::Compute {
                        result: Box::new(result),
                        state_root_before: before,
                        state_root_after,
                    }
                }
                NativeActionV2::ContractCreate => {
                    let result = execute_inactive_accepted_contract_create_v1(
                        &mut next,
                        registry,
                        &transaction,
                        network,
                        current_height,
                        context,
                    )?;
                    producer_priority_fee = producer_priority_fee
                        .checked_add(result.producer_priority_fee)
                        .ok_or_else(|| {
                            "inactive V3 block producer priority fee overflow".to_string()
                        })?;
                    InactiveVersionedTransactionTransitionV3::ContractCreate(result)
                }
                NativeActionV2::ContractCall => {
                    let result = execute_inactive_accepted_contract_call_v1(
                        &mut next,
                        registry,
                        &transaction,
                        network,
                        current_height,
                        context,
                    )?;
                    producer_priority_fee = producer_priority_fee
                        .checked_add(result.producer_priority_fee)
                        .ok_or_else(|| {
                            "inactive V3 block producer priority fee overflow".to_string()
                        })?;
                    InactiveVersionedTransactionTransitionV3::ContractCall(result)
                }
            },
        };
        if transition.state_root_before() != before {
            return Err("inactive V3 block transition root discontinuity".into());
        }
        transitions.push(transition);
    }

    body.validate_fee_recipient_canonicality(producer_priority_fee)?;
    for window in transitions.windows(2) {
        if window[0].state_root_after() != window[1].state_root_before() {
            return Err("inactive V3 block transition roots are not contiguous".into());
        }
    }
    let state_root_after = next.state_root()?;
    if let Some(last) = transitions.last() {
        if last.state_root_after() != state_root_after {
            return Err("inactive V3 block final state root mismatch".into());
        }
    } else if state_root_after != state_root_before {
        return Err("inactive empty V3 block changed state".into());
    }

    let result = InactiveVersionedBlockTransitionV3 {
        transactions_root: body.transactions_root(),
        state_root_before,
        state_root_after,
        producer_priority_fee,
        transactions: transitions,
    };
    *state = next;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_compute_fee_v1::NATIVE_COMPUTE_CHANNEL_OPEN_GAS_V1;
    use crate::native_compute_payloads::ComputeChannelOpenPayloadV1;
    use crate::native_contract_payload_v1::ContractCreatePayloadV1;
    use crate::native_contract_runtime_registry_v1::ContractRuntimeDescriptorV1;
    use crate::native_contract_vm_v1::{NVM1_CODE_FORMAT_VERSION, NVM1_RUNTIME_ID};
    use crate::native_execution::{AccountStateV1, NativeStateV1};
    use crate::native_state_v2::{ComputeChannelSettlementPolicyV1, NativeStateV2};
    use crate::native_transaction::{DEVNET_CHAIN_ID, DEVNET_NETWORK_ID};
    use crate::native_transaction_v2::{NativeTransactionBodyV2, SignedNativeTransactionV2};
    use k256::ecdsa::{signature::hazmat::PrehashSigner, Signature, SigningKey};

    fn sign(key: &SigningKey, body: NativeTransactionBodyV2) -> SignedNativeTransactionV2 {
        let mut tx = SignedNativeTransactionV2 {
            body,
            public_key: key
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec(),
            signature: vec![0; 64],
        };
        let digest = tx.signing_digest().unwrap();
        let signature: Signature = key.sign_prehash(&digest).unwrap();
        tx.signature = signature.to_bytes().to_vec();
        tx
    }

    fn stop_module() -> Vec<u8> {
        let mut code = Vec::new();
        code.extend_from_slice(b"NVM1");
        code.extend_from_slice(&1u16.to_be_bytes());
        code.extend_from_slice(&0u16.to_be_bytes());
        code.extend_from_slice(&1u32.to_be_bytes());
        code.extend_from_slice(&1u16.to_be_bytes());
        code.extend_from_slice(&0u16.to_be_bytes());
        code.push(0x00);
        code
    }

    fn registry() -> NativeContractRuntimeRegistryV1 {
        NativeContractRuntimeRegistryV1::new([ContractRuntimeDescriptorV1 {
            runtime_id: NVM1_RUNTIME_ID,
            code_format_version: NVM1_CODE_FORMAT_VERSION,
            activation_height: 10,
            retirement_height: None,
            max_code_bytes: 65_536,
            max_init_data_bytes: 65_536,
        }])
        .unwrap()
    }

    #[test]
    fn create_then_call_in_same_v3_block_observes_prior_contract_state() {
        let key = SigningKey::from_slice(&[0x58; 32]).unwrap();
        let probe = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::Transfer,
                target_payload: vec![0x01; 20],
                value: 0,
                gas_limit: 1_000,
                max_fee_per_gas: 1,
                max_priority_fee_per_gas: 0,
                data: Vec::new(),
            },
        );
        let sender = probe
            .authenticated_sender(AddressNetwork::Devnet)
            .unwrap()
            .payload;
        let contract_id = crate::address::NiahciaAddressV1::contract_from_creator(
            AddressNetwork::Devnet,
            DEVNET_CHAIN_ID,
            sender,
            0,
        )
        .payload;

        let create_payload = ContractCreatePayloadV1 {
            runtime_id: NVM1_RUNTIME_ID,
            code: stop_module(),
            init_data: Vec::new(),
        };
        let create = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::ContractCreate,
                target_payload: Vec::new(),
                value: 100,
                gas_limit: 10,
                max_fee_per_gas: 5,
                max_priority_fee_per_gas: 2,
                data: create_payload.canonical_bytes().unwrap(),
            },
        );
        let call = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 1,
                action: NativeActionV2::ContractCall,
                target_payload: contract_id.to_vec(),
                value: 50,
                gas_limit: 10,
                max_fee_per_gas: 5,
                max_priority_fee_per_gas: 2,
                data: Vec::new(),
            },
        );

        let mut accounts = NativeStateV1::default();
        accounts.set_account(
            sender,
            AccountStateV1 {
                balance: 10_000,
                nonce: 0,
            },
        );
        let mut state = NativeStateV3::from_v2(NativeStateV2::from_v1(accounts));
        let body = NativeBlockBodyV2::from_versioned_transactions(
            [0x99; 20],
            &[
                VersionedSignedNativeTransaction::V2(create),
                VersionedSignedNativeTransaction::V2(call),
            ],
        )
        .unwrap();

        let result = execute_inactive_versioned_block_v3(
            &mut state,
            &registry(),
            &body,
            AddressNetwork::Devnet,
            10,
            2,
        )
        .unwrap();

        assert_eq!(result.transactions.len(), 2);
        assert_eq!(
            result.transactions[0].state_root_after(),
            result.transactions[1].state_root_before()
        );
        assert_eq!(state.base().accounts().account(sender).nonce, 2);
        assert_eq!(state.contract_count(), 1);
        assert_eq!(state.contract(contract_id).unwrap().balance, 150);
        assert_eq!(result.state_root_after, state.state_root().unwrap());
    }

    #[test]
    fn later_invalid_call_rolls_back_prior_create_in_v3_block() {
        let key = SigningKey::from_slice(&[0x59; 32]).unwrap();
        let probe = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::Transfer,
                target_payload: vec![0x01; 20],
                value: 0,
                gas_limit: 1_000,
                max_fee_per_gas: 1,
                max_priority_fee_per_gas: 0,
                data: Vec::new(),
            },
        );
        let sender = probe
            .authenticated_sender(AddressNetwork::Devnet)
            .unwrap()
            .payload;
        let contract_id = crate::address::NiahciaAddressV1::contract_from_creator(
            AddressNetwork::Devnet,
            DEVNET_CHAIN_ID,
            sender,
            0,
        )
        .payload;
        let create_payload = ContractCreatePayloadV1 {
            runtime_id: NVM1_RUNTIME_ID,
            code: stop_module(),
            init_data: Vec::new(),
        };
        let create = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::ContractCreate,
                target_payload: Vec::new(),
                value: 100,
                gas_limit: 10,
                max_fee_per_gas: 5,
                max_priority_fee_per_gas: 2,
                data: create_payload.canonical_bytes().unwrap(),
            },
        );
        let invalid_call = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 2,
                action: NativeActionV2::ContractCall,
                target_payload: contract_id.to_vec(),
                value: 50,
                gas_limit: 10,
                max_fee_per_gas: 5,
                max_priority_fee_per_gas: 2,
                data: Vec::new(),
            },
        );

        let mut accounts = NativeStateV1::default();
        accounts.set_account(
            sender,
            AccountStateV1 {
                balance: 10_000,
                nonce: 0,
            },
        );
        let mut state = NativeStateV3::from_v2(NativeStateV2::from_v1(accounts));
        let before = state.clone();
        let before_root = state.state_root().unwrap();
        let body = NativeBlockBodyV2::from_versioned_transactions(
            [0x99; 20],
            &[
                VersionedSignedNativeTransaction::V2(create),
                VersionedSignedNativeTransaction::V2(invalid_call),
            ],
        )
        .unwrap();

        assert!(execute_inactive_versioned_block_v3(
            &mut state,
            &registry(),
            &body,
            AddressNetwork::Devnet,
            10,
            2,
        )
        .is_err());
        assert_eq!(state, before);
        assert_eq!(state.state_root().unwrap(), before_root);
        assert_eq!(state.base().accounts().account(sender).nonce, 0);
        assert_eq!(state.base().accounts().account(sender).balance, 10_000);
        assert_eq!(state.contract_count(), 0);
        assert!(state.contract(contract_id).is_none());
    }

    #[test]
    fn contract_create_before_runtime_activation_rejects_v3_block_atomically() {
        let key = SigningKey::from_slice(&[0x5a; 32]).unwrap();
        let probe = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::Transfer,
                target_payload: vec![0x01; 20],
                value: 0,
                gas_limit: 1_000,
                max_fee_per_gas: 1,
                max_priority_fee_per_gas: 0,
                data: Vec::new(),
            },
        );
        let sender = probe
            .authenticated_sender(AddressNetwork::Devnet)
            .unwrap()
            .payload;
        let create_payload = ContractCreatePayloadV1 {
            runtime_id: NVM1_RUNTIME_ID,
            code: stop_module(),
            init_data: Vec::new(),
        };
        let create = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::ContractCreate,
                target_payload: Vec::new(),
                value: 100,
                gas_limit: 10,
                max_fee_per_gas: 5,
                max_priority_fee_per_gas: 2,
                data: create_payload.canonical_bytes().unwrap(),
            },
        );

        let mut accounts = NativeStateV1::default();
        accounts.set_account(
            sender,
            AccountStateV1 {
                balance: 10_000,
                nonce: 0,
            },
        );
        let mut state = NativeStateV3::from_v2(NativeStateV2::from_v1(accounts));
        let before = state.clone();
        let before_root = state.state_root().unwrap();
        let body = NativeBlockBodyV2::from_versioned_transactions(
            [0x99; 20],
            &[VersionedSignedNativeTransaction::V2(create)],
        )
        .unwrap();

        assert!(execute_inactive_versioned_block_v3(
            &mut state,
            &registry(),
            &body,
            AddressNetwork::Devnet,
            9,
            2,
        )
        .is_err());
        assert_eq!(state, before);
        assert_eq!(state.state_root().unwrap(), before_root);
        assert_eq!(state.base().accounts().account(sender).nonce, 0);
        assert_eq!(state.base().accounts().account(sender).balance, 10_000);
        assert_eq!(state.contract_count(), 0);
    }

    #[test]
    fn compute_open_and_contract_create_coexist_in_same_v3_block() {
        let key = SigningKey::from_slice(&[0x5b; 32]).unwrap();
        let channel_key = SigningKey::from_slice(&[0x5c; 32]).unwrap();
        let probe = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::Transfer,
                target_payload: vec![0x01; 20],
                value: 0,
                gas_limit: 1_000,
                max_fee_per_gas: 1,
                max_priority_fee_per_gas: 0,
                data: Vec::new(),
            },
        );
        let sender = probe
            .authenticated_sender(AddressNetwork::Devnet)
            .unwrap()
            .payload;
        let channel_public_key: [u8; 65] = channel_key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .try_into()
            .unwrap();
        let open_payload = ComputeChannelOpenPayloadV1 {
            worker_id: [0x71; 32],
            operator_id: [0x72; 32],
            channel_public_key,
            worker_payment_account: [0x73; 20],
            authorized_amount: 1_000,
            expiry_height: 20,
            claim_deadline_height: 25,
            refund_available_height: 26,
            service_scope_commitment: [0x74; 32],
            model_scope_commitment: [0x75; 32],
            execution_profile_scope_commitment: [0x76; 32],
            settlement_policy: ComputeChannelSettlementPolicyV1::CumulativeReceipt,
        };
        let open = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 0,
                action: NativeActionV2::ComputeChannelOpen,
                target_payload: Vec::new(),
                value: 1_000,
                gas_limit: NATIVE_COMPUTE_CHANNEL_OPEN_GAS_V1,
                max_fee_per_gas: 2,
                max_priority_fee_per_gas: 0,
                data: open_payload.canonical_bytes().unwrap(),
            },
        );
        let create_payload = ContractCreatePayloadV1 {
            runtime_id: NVM1_RUNTIME_ID,
            code: stop_module(),
            init_data: Vec::new(),
        };
        let create = sign(
            &key,
            NativeTransactionBodyV2 {
                network_id: DEVNET_NETWORK_ID,
                chain_id: DEVNET_CHAIN_ID,
                nonce: 1,
                action: NativeActionV2::ContractCreate,
                target_payload: Vec::new(),
                value: 100,
                gas_limit: 10,
                max_fee_per_gas: 5,
                max_priority_fee_per_gas: 2,
                data: create_payload.canonical_bytes().unwrap(),
            },
        );

        let mut accounts = NativeStateV1::default();
        accounts.set_account(
            sender,
            AccountStateV1 {
                balance: 100_000,
                nonce: 0,
            },
        );
        let mut state = NativeStateV3::from_v2(NativeStateV2::from_v1(accounts));
        let body = NativeBlockBodyV2::from_versioned_transactions(
            [0x99; 20],
            &[
                VersionedSignedNativeTransaction::V2(open),
                VersionedSignedNativeTransaction::V2(create),
            ],
        )
        .unwrap();

        let result = execute_inactive_versioned_block_v3(
            &mut state,
            &registry(),
            &body,
            AddressNetwork::Devnet,
            10,
            2,
        )
        .unwrap();

        assert_eq!(result.transactions.len(), 2);
        assert_eq!(
            result.transactions[0].state_root_after(),
            result.transactions[1].state_root_before()
        );
        assert_eq!(state.base().accounts().account(sender).nonce, 2);
        assert_eq!(state.base().channel_count(), 1);
        assert_eq!(state.contract_count(), 1);
        assert_eq!(result.state_root_after, state.state_root().unwrap());
    }
}
