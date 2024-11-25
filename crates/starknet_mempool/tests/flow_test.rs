use std::sync::Arc;

use papyrus_network::gossipsub_impl::Topic;
use papyrus_network::network_manager::{BroadcastTopicChannels, NetworkManager};
use papyrus_network_types::network_types::BroadcastedMessageMetadata;
use papyrus_test_utils::{get_rng, GetTestInstance};
use rstest::{fixture, rstest};
use starknet_api::block::GasPrice;
use starknet_api::executable_transaction::AccountTransaction;
use starknet_api::rpc_transaction::{
    RpcDeployAccountTransaction,
    RpcInvokeTransaction,
    RpcTransaction,
};
use starknet_api::{contract_address, nonce};
use starknet_mempool::add_tx_input;
use starknet_mempool::communication::MempoolCommunicationWrapper;
use starknet_mempool::mempool::Mempool;
use starknet_mempool::test_utils::{
    add_tx,
    add_tx_expect_error,
    commit_block,
    get_txs_and_assert_expected,
};
use starknet_mempool_p2p::propagator::MempoolP2pPropagator;
use starknet_mempool_p2p::MEMPOOL_TOPIC;
use starknet_mempool_p2p_types::communication::{
    LocalMempoolP2pPropagatorClient,
    MempoolP2pPropagatorRequest,
    MempoolP2pPropagatorRequestAndResponseSender,
    MempoolP2pPropagatorResponse,
    SharedMempoolP2pPropagatorClient,
};
use starknet_mempool_types::communication::AddTransactionArgsWrapper;
use starknet_mempool_types::errors::MempoolError;
use starknet_sequencer_infra::component_server::{ComponentServerStarter, LocalComponentServer};
// Fixtures.
use tokio::sync::mpsc::channel;
use tokio::task;

#[fixture]
fn mempool() -> Mempool {
    Mempool::default()
}

// Tests.

#[rstest]
fn test_add_tx_fills_nonce_gap(mut mempool: Mempool) {
    // Setup.
    let input_address_0_nonce_0 =
        add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 0, account_nonce: 0);
    let input_address_0_nonce_1 =
        add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 1, account_nonce: 0);
    let input_address_1_nonce_0 =
        add_tx_input!(tx_hash: 3, address: "0x1", tx_nonce: 0, account_nonce: 0);

    for input in [&input_address_0_nonce_1, &input_address_1_nonce_0] {
        add_tx(&mut mempool, input);
    }

    // Test and assert: only the eligible transaction is returned.
    get_txs_and_assert_expected(&mut mempool, 2, &[input_address_1_nonce_0.tx]);

    add_tx(&mut mempool, &input_address_0_nonce_0);

    // Test and assert: all remaining transactions are returned.
    get_txs_and_assert_expected(
        &mut mempool,
        2,
        &[input_address_0_nonce_0.tx, input_address_0_nonce_1.tx],
    );
}

#[rstest]
fn test_add_tx_rejection_for_txs_passed_to_batcher(mut mempool: Mempool) {
    // Setup.
    let input_tx = add_tx_input!(tx_hash: 0, tx_nonce: 0);

    // Test.
    add_tx(&mut mempool, &input_tx);
    get_txs_and_assert_expected(&mut mempool, 1, &[input_tx.tx]);

    let input_tx_duplicate_nonce = add_tx_input!(tx_hash: 1, tx_nonce: 0);
    add_tx_expect_error(
        &mut mempool,
        &input_tx_duplicate_nonce,
        MempoolError::NonceTooOld { address: contract_address!("0x0"), nonce: nonce!(0) },
    );
}

#[rstest]
fn test_add_same_nonce_tx_after_previous_not_included_in_block(mut mempool: Mempool) {
    // Setup.
    let tx_nonce_3_account_nonce_3 =
        add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 3, account_nonce: 3);
    let tx_nonce_4_account_nonce_3 =
        add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 4, account_nonce: 3);
    let tx_nonce_5_account_nonce_3 =
        add_tx_input!(tx_hash: 3, address: "0x0", tx_nonce: 5, account_nonce: 3);

    for input in
        [&tx_nonce_3_account_nonce_3, &tx_nonce_4_account_nonce_3, &tx_nonce_5_account_nonce_3]
    {
        add_tx(&mut mempool, input);
    }

    // Test.
    get_txs_and_assert_expected(
        &mut mempool,
        2,
        &[tx_nonce_3_account_nonce_3.tx, tx_nonce_4_account_nonce_3.tx.clone()],
    );

    let nonces = [("0x0", 4)]; // Transaction with nonce 3 was included, 4 was not.
    let tx_hashes = [1];
    commit_block(&mut mempool, nonces, tx_hashes);

    let tx_nonce_4_account_nonce_4 =
        add_tx_input!(tx_hash: 4, address: "0x0", tx_nonce: 4, account_nonce: 4);
    add_tx_expect_error(
        &mut mempool,
        &tx_nonce_4_account_nonce_4,
        MempoolError::DuplicateNonce { address: contract_address!("0x0"), nonce: nonce!(4) },
    );

    get_txs_and_assert_expected(
        &mut mempool,
        2,
        &[tx_nonce_4_account_nonce_3.tx, tx_nonce_5_account_nonce_3.tx],
    );
}

#[rstest]
fn test_add_tx_handles_nonces_correctly(mut mempool: Mempool) {
    // Setup.
    let input_nonce_0 = add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 0, account_nonce: 0);
    let input_nonce_1 = add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 1, account_nonce: 1);
    let input_nonce_2 = add_tx_input!(tx_hash: 3, address: "0x0", tx_nonce: 2, account_nonce: 0);

    // Test.
    // Account is registered in mempool.
    add_tx(&mut mempool, &input_nonce_0);
    // Although the input account nonce is higher, mempool looks at its internal registry.
    add_tx(&mut mempool, &input_nonce_1);
    get_txs_and_assert_expected(&mut mempool, 2, &[input_nonce_1.tx]);
    // Although the input account nonce is lower, mempool looks at internal registry.
    add_tx(&mut mempool, &input_nonce_2);
    get_txs_and_assert_expected(&mut mempool, 1, &[input_nonce_2.tx]);
}

#[rstest]
fn test_commit_block_includes_proposed_txs_subset(mut mempool: Mempool) {
    // Setup.
    let tx_address_0_nonce_1 =
        add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 1, account_nonce: 1);
    let tx_address_0_nonce_3 =
        add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 3, account_nonce: 1);
    let tx_address_0_nonce_4 =
        add_tx_input!(tx_hash: 3, address: "0x0", tx_nonce: 4, account_nonce: 1);

    let tx_address_1_nonce_2 =
        add_tx_input!(tx_hash: 4, address: "0x1", tx_nonce: 2, account_nonce: 2);
    let tx_address_1_nonce_3 =
        add_tx_input!(tx_hash: 5, address: "0x1", tx_nonce: 3, account_nonce: 2);
    let tx_address_1_nonce_4 =
        add_tx_input!(tx_hash: 6, address: "0x1", tx_nonce: 4, account_nonce: 2);

    let tx_address_2_nonce_1 =
        add_tx_input!(tx_hash: 7, address: "0x2", tx_nonce: 1, account_nonce: 1);
    let tx_address_2_nonce_2 =
        add_tx_input!(tx_hash: 8, address: "0x2", tx_nonce: 2, account_nonce: 1);

    for input in [
        &tx_address_0_nonce_3,
        &tx_address_0_nonce_4,
        &tx_address_0_nonce_1,
        &tx_address_1_nonce_4,
        &tx_address_1_nonce_3,
        &tx_address_1_nonce_2,
        &tx_address_2_nonce_1,
        &tx_address_2_nonce_2,
    ] {
        add_tx(&mut mempool, input);
    }

    // Test.
    get_txs_and_assert_expected(
        &mut mempool,
        2,
        &[tx_address_2_nonce_1.tx.clone(), tx_address_1_nonce_2.tx],
    );
    get_txs_and_assert_expected(
        &mut mempool,
        4,
        &[
            tx_address_2_nonce_2.tx,
            tx_address_1_nonce_3.tx.clone(),
            tx_address_0_nonce_1.tx,
            tx_address_1_nonce_4.tx.clone(),
        ],
    );

    // Address 0x0 stays as proposed, address 0x1 rewinds nonce 4, address 0x2 rewinds completely.
    let nonces = [("0x0", 2), ("0x1", 4)];
    let tx_hashes = [1, 4];
    commit_block(&mut mempool, nonces, tx_hashes);

    get_txs_and_assert_expected(
        &mut mempool,
        2,
        &[tx_address_2_nonce_1.tx, tx_address_1_nonce_4.tx],
    );
}

#[rstest]
fn test_commit_block_fills_nonce_gap(mut mempool: Mempool) {
    // Setup.
    let tx_nonce_3_account_nonce_3 =
        add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 3, account_nonce: 3);
    let tx_nonce_5_account_nonce_3 =
        add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 5, account_nonce: 3);

    // Test.
    for input in [&tx_nonce_3_account_nonce_3, &tx_nonce_5_account_nonce_3] {
        add_tx(&mut mempool, input);
    }

    get_txs_and_assert_expected(&mut mempool, 2, &[tx_nonce_3_account_nonce_3.tx]);

    let nonces = [("0x0", 5)];
    let tx_hashes = [1, 3];
    commit_block(&mut mempool, nonces, tx_hashes);

    // Assert: hole was indeed closed.
    let tx_nonce_4_account_nonce_4 =
        add_tx_input!(tx_hash: 3, address: "0x0", tx_nonce: 4, account_nonce: 4);
    add_tx_expect_error(
        &mut mempool,
        &tx_nonce_4_account_nonce_4,
        MempoolError::NonceTooOld { address: contract_address!("0x0"), nonce: nonce!(4) },
    );

    get_txs_and_assert_expected(&mut mempool, 2, &[tx_nonce_5_account_nonce_3.tx]);
}

#[rstest]
fn test_commit_block_rewinds_queued_nonce(mut mempool: Mempool) {
    // Setup.
    let tx_address_0_nonce_2 =
        add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 2, account_nonce: 2);
    let tx_address_0_nonce_3 =
        add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 3, account_nonce: 2);
    let tx_address_1_nonce_2 =
        add_tx_input!(tx_hash: 3, address: "0x1", tx_nonce: 2, account_nonce: 2);
    let tx_address_1_nonce_3 =
        add_tx_input!(tx_hash: 4, address: "0x1", tx_nonce: 3, account_nonce: 2);

    for input in
        [&tx_address_0_nonce_2, &tx_address_0_nonce_3, &tx_address_1_nonce_2, &tx_address_1_nonce_3]
    {
        add_tx(&mut mempool, input);
    }

    get_txs_and_assert_expected(
        &mut mempool,
        4,
        &[
            tx_address_1_nonce_2.tx.clone(),
            tx_address_0_nonce_2.tx,
            tx_address_1_nonce_3.tx,
            tx_address_0_nonce_3.tx.clone(),
        ],
    );

    // Test.
    let nonces = [("0x0", 3)];
    let tx_hashes = [1];
    // Address 0x0: nonce 2 was accepted, but 3 was not, so is rewound.
    // Address 0x1: nonce 2 was not accepted, both 2 and 3 were rewound.
    commit_block(&mut mempool, nonces, tx_hashes);

    // Nonces 3 and 4 were re-enqueued correctly.
    get_txs_and_assert_expected(
        &mut mempool,
        2,
        &[tx_address_1_nonce_2.tx, tx_address_0_nonce_3.tx],
    );
}

#[rstest]
fn test_commit_block_from_different_leader(mut mempool: Mempool) {
    // Setup.
    // TODO: set the mempool to `validate` mode once supported.

    let tx_nonce_2 = add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 2, account_nonce: 2);
    let tx_nonce_3 = add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 3, account_nonce: 2);
    let tx_nonce_4 = add_tx_input!(tx_hash: 3, address: "0x0", tx_nonce: 4, account_nonce: 2);

    for input in [&tx_nonce_2, &tx_nonce_3, &tx_nonce_4] {
        add_tx(&mut mempool, input);
    }

    // Test.
    let nonces = [("0x0", 4), ("0x1", 2)];
    let tx_hashes = [
        1,  // Address 0: known hash accepted for nonce 2.
        99, // Address 0: unknown hash accepted for nonce 3.
        4,  // Unknown Address 1 (with unknown hash) for nonce 2.
    ];
    commit_block(&mut mempool, nonces, tx_hashes);

    // Assert: two stale transactions were removed, one was added to a block by a different leader
    // and the other "lost" to a different transaction with the same nonce that was added by the
    // different leader.
    get_txs_and_assert_expected(&mut mempool, 1, &[tx_nonce_4.tx]);
}

#[rstest]
fn test_update_gas_price_threshold(mut mempool: Mempool) {
    // Setup.
    let input_gas_price_20 =
        add_tx_input!(tx_hash: 1, address: "0x0", tip: 100, max_l2_gas_price: 20);
    let input_gas_price_30 =
        add_tx_input!(tx_hash: 2, address: "0x1", tip: 50, max_l2_gas_price: 30);

    // Test: only txs with gas price above the threshold are returned.
    mempool.update_gas_price_threshold(GasPrice(30));
    for input in [&input_gas_price_20, &input_gas_price_30] {
        add_tx(&mut mempool, input);
    }
    get_txs_and_assert_expected(&mut mempool, 2, &[input_gas_price_30.tx]);

    let nonces = [("0x1", 1)];
    let tx_hashes = [2];
    commit_block(&mut mempool, nonces, tx_hashes);

    mempool.update_gas_price_threshold(GasPrice(10));
    get_txs_and_assert_expected(&mut mempool, 2, &[input_gas_price_20.tx]);
}

#[rstest]
#[tokio::test]
async fn test_tx_sent_to_p2p(mempool: Mempool) {
    let (tx_mempool_p2p_propagator, mut rx_mempool_p2p_propagator) =
        channel::<MempoolP2pPropagatorRequestAndResponseSender>(32);
    let mempool_p2p_propagator_client: SharedMempoolP2pPropagatorClient =
        Arc::new(LocalMempoolP2pPropagatorClient::new(tx_mempool_p2p_propagator));
    let mut mempool_wrapper =
        MempoolCommunicationWrapper::new(mempool, mempool_p2p_propagator_client);

    let mut placeholder_network_manager = NetworkManager::new(Default::default(), None);
    let BroadcastTopicChannels { broadcast_topic_client, .. } = placeholder_network_manager
        .register_broadcast_topic(Topic::new(MEMPOOL_TOPIC), 32)
        .expect("Failed to register broadcast topic");
    let mempool_p2p_propagator = MempoolP2pPropagator::new(broadcast_topic_client.clone());
    let mut mempool_p2p_propagator_server =
        LocalComponentServer::new(mempool_p2p_propagator, rx_mempool_p2p_propagator);

    task::spawn(async move {
        let _ = mempool_p2p_propagator_server.start().await;
    });

    println!("cp1");
    // add_tx_input! creates an Invoke Transaction
    let tx_1_args = add_tx_input!(tx_hash: 1, address: "0x0", tx_nonce: 2, account_nonce: 2);
    let new_tx_args =
        AddTransactionArgsWrapper { args: tx_1_args.clone(), p2p_message_metadata: None };

    println!("cp2");

    let tx_2_args = add_tx_input!(tx_hash: 2, address: "0x0", tx_nonce: 3, account_nonce: 2);
    let expected_message_metadata = BroadcastedMessageMetadata::get_test_instance(&mut get_rng());
    let _propagated_tx_args = AddTransactionArgsWrapper {
        args: tx_2_args.clone(),
        p2p_message_metadata: Some(expected_message_metadata.clone()),
    };

    println!("finished setup");

    mempool_wrapper.add_tx(new_tx_args).await.unwrap();

    println!("Added new tx to mempool");

    // let new_tx_response = mempool_p2p_propagator_server.recv().await.unwrap();

    // println!("Received new tx response");

    // if let MempoolP2pPropagatorRequest::AddTransaction(rpc_transaction) = new_tx_response.request
    // {     let converted_tx = match tx_1_args.tx {
    //         AccountTransaction::Declare(_declare_tx) => {
    //             panic!(
    //                 "No implementation for converting DeclareTransaction to an
    // RpcTransaction"
    //             )
    //         }
    //         AccountTransaction::DeployAccount(deploy_account_transaction) => {
    //             RpcTransaction::DeployAccount(RpcDeployAccountTransaction::V3(
    //                 deploy_account_transaction.clone().into(),
    //             ))
    //         }
    //         AccountTransaction::Invoke(invoke_transaction) => {
    //
    // RpcTransaction::Invoke(RpcInvokeTransaction::V3(invoke_transaction.clone().into()))
    //         }
    //     };
    //     assert_eq!(rpc_transaction, converted_tx);
    //     println!("New tx matches")
    // } else {
    //     panic!("Expected AddTransaction request, got {:?}", new_tx_response.request);
    // }
    // mempool_wrapper.add_tx(propagated_tx_args).await.unwrap();

    // println!("Added propagated tx to mempool");

    // let propagated_tx_response = rx_mempool_p2p_propagator.recv().await.unwrap();

    // println!("Received propagated tx response");

    // if let MempoolP2pPropagatorRequest::ContinuePropagation(message_metadata) =
    //     propagated_tx_response.request
    // {
    //     assert_eq!(message_metadata, expected_message_metadata);
    //     println!("Message metadata matches")
    // } else {
    //     panic!("Expected ContinuePropagation request, got {:?}", propagated_tx_response.request);
    // }
}
