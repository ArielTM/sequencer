use assert_matches::assert_matches;
use blockifier::blockifier::block::BlockInfo;
use blockifier::versioned_constants::StarknetVersion;
use pretty_assertions::assert_eq;
use rstest::{fixture, rstest};
use starknet_api::block::BlockNumber;
use starknet_api::core::ClassHash;
use starknet_api::{class_hash, felt};
use starknet_core::types::ContractClass::{Legacy, Sierra};

use crate::state_reader::compile::legacy_to_contract_class_v0;
use crate::state_reader::test_state_reader::TestStateReader;

#[fixture]
pub fn test_block_number() -> BlockNumber {
    BlockNumber(700000)
}

#[fixture]
pub fn test_state_reader(test_block_number: BlockNumber) -> TestStateReader {
    TestStateReader::new_for_testing(test_block_number)
}

#[rstest]
pub fn test_get_block_info(test_state_reader: TestStateReader, test_block_number: BlockNumber) {
    assert_matches!(
        test_state_reader.get_block_info(),
        Ok(BlockInfo { block_number, .. }) if block_number == test_block_number
    );
}

#[rstest]
pub fn test_get_starknet_version(test_state_reader: TestStateReader) {
    assert_eq!(test_state_reader.get_starknet_version().unwrap(), StarknetVersion::V0_13_2_1)
}

#[rstest]
pub fn test_get_contract_class(test_state_reader: TestStateReader, test_block_number: BlockNumber) {
    // An example of existing class hash in Mainnet.
    let class_hash =
        class_hash!("0x3131fa018d520a037686ce3efddeab8f28895662f019ca3ca18a626650f7d1e");

    // Test getting the contract class using RPC request.
    let deprecated_contract_class =
        test_state_reader.get_contract_class(&class_hash).unwrap_or_else(|err| {
            panic!(
                "Error retrieving deprecated contract class for class hash {}: {}
            This class hash exist in Mainnet Block Number: {}",
                class_hash, test_block_number, err
            );
        });

    // Test compiling the contract class.
    match deprecated_contract_class {
        Legacy(legacy) => {
            // Test compiling the contract class.
            assert!(legacy_to_contract_class_v0(legacy).is_ok());
        }
        // This contract class is deprecated.
        Sierra(_) => panic!("Expected a legacy contract class"),
    }
}