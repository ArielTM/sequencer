use std::collections::HashMap;

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::cached_state::CachedState;
use blockifier::test_utils::dict_state_reader::DictStateReader;
use blockifier::test_utils::struct_impls::LoadFile;
use starknet_api::class_hash;
use starknet_api::contract_class::ContractClass as RawContractClass;
use starknet_api::core::ClassHash;
use starknet_api::deprecated_contract_class::ContractClass as DeprecatedContractClass;

pub const TOKEN_FOR_TESTING_CLASS_HASH: &str = "0x30";
// This package is run within the StarkWare repository build directory.
pub const TOKEN_FOR_TESTING_CONTRACT_PATH: &str =
    "./src/starkware/starknet/core/test_contract/starknet_compiled_contracts_lib/starkware/\
     starknet/core/test_contract/token_for_testing.json";

pub fn create_py_test_state() -> CachedState<DictStateReader> {
    let noa: ContractClass =
        RawContractClass::V0(DeprecatedContractClass::from_file(TOKEN_FOR_TESTING_CONTRACT_PATH))
            .try_into()
            .unwrap();
    let class_hash_to_class = HashMap::from([(class_hash!(TOKEN_FOR_TESTING_CLASS_HASH), noa)]);
    CachedState::from(DictStateReader { class_hash_to_class, ..Default::default() })
}
