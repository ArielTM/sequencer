use blockifier_reexecution::state_reader::test_state_reader::{
    ConsecutiveTestStateReaders,
    OfflineConsecutiveStateReaders,
    SerializableDataPrevBlock,
    SerializableOfflineReexecutionData,
};
use blockifier_reexecution::state_reader::utils::{
    reexecute_and_verify_correctness,
    JSON_RPC_VERSION,
};
use clap::{Args, Parser, Subcommand};
use starknet_api::block::BlockNumber;
use starknet_gateway::config::RpcStateReaderConfig;

/// BlockifierReexecution CLI.
#[derive(Debug, Parser)]
#[clap(name = "blockifier-reexecution-cli", version)]
pub struct BlockifierReexecutionCliArgs {
    #[clap(flatten)]
    global_options: GlobalOptions,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Args, Debug)]
struct SharedArgs {
    /// Node url.
    /// Default: https://free-rpc.nethermind.io/mainnet-juno/. Won't work for big tests.
    #[clap(long, short = 'n', default_value = "https://free-rpc.nethermind.io/mainnet-juno/")]
    node_url: String,

    /// Block number.
    #[clap(long, short = 'b')]
    block_number: u64,

    // Directory path to json files. Default:
    // "./crates/blockifier_reexecution/resources/block_{block_number}".
    #[clap(long, short = 'd', default_value = None)]
    directory_path: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Runs the RPC test.
    RpcTest {
        #[clap(flatten)]
        shared_args: SharedArgs,
    },

    /// Writes the RPC queries to json files.
    WriteRpcRepliesToJson {
        #[clap(flatten)]
        shared_args: SharedArgs,
    },

    // Reexecutes the block from JSON files.
    ReexecuteBlock {
        #[clap(flatten)]
        shared_args: SharedArgs,
    },
}

#[derive(Debug, Args)]
struct GlobalOptions {}

/// Main entry point of the blockifier reexecution CLI.
fn main() {
    let args = BlockifierReexecutionCliArgs::parse();

    match args.command {
        Command::RpcTest { shared_args: SharedArgs { node_url, block_number, .. } } => {
            println!("Running RPC test for block number {block_number} using node url {node_url}.",);

            let config = RpcStateReaderConfig {
                url: node_url,
                json_rpc_version: JSON_RPC_VERSION.to_string(),
            };

            reexecute_and_verify_correctness(ConsecutiveTestStateReaders::new(
                BlockNumber(block_number - 1),
                Some(config),
                false,
            ));

            // Compare the expected and actual state differences
            // by avoiding discrepancies caused by insertion order
            println!("RPC test passed successfully.");
        }

        Command::WriteRpcRepliesToJson {
            shared_args: SharedArgs { node_url, block_number, directory_path },
        } => {
            let directory_path = directory_path.unwrap_or(format!(
                "./crates/blockifier_reexecution/resources/block_{block_number}/"
            ));

            // TODO(Aner): refactor to reduce code duplication.
            let config = RpcStateReaderConfig {
                url: node_url,
                json_rpc_version: JSON_RPC_VERSION.to_string(),
            };

            let consecutive_state_readers =
                ConsecutiveTestStateReaders::new(BlockNumber(block_number - 1), Some(config), true);

            let serializable_data_next_block =
                consecutive_state_readers.get_serializable_data_next_block().unwrap();

            // Run the reexecution test and get the state maps and contract class mapping.
            let block_state = reexecute_and_verify_correctness(consecutive_state_readers).unwrap();
            let serializable_data_prev_block = SerializableDataPrevBlock {
                state_maps: block_state.get_initial_reads().unwrap().into(),
                contract_class_mapping: block_state
                    .state
                    .get_contract_class_mapping_dumper()
                    .unwrap(),
            };

            // Write the reexecution data to a json file.
            SerializableOfflineReexecutionData {
                serializable_data_prev_block,
                serializable_data_next_block,
            }
            .write_to_file(&directory_path, "reexecution_data.json")
            .unwrap();

            println!(
                "RPC replies required for reexecuting block {block_number} written to json file."
            );
        }

        Command::ReexecuteBlock {
            shared_args: SharedArgs { block_number, directory_path, .. },
        } => {
            let full_file_path = directory_path.unwrap_or(format!(
                "./crates/blockifier_reexecution/resources/block_{block_number}"
            )) + "/reexecution_data.json";

            reexecute_and_verify_correctness(
                OfflineConsecutiveStateReaders::new_from_file(&full_file_path).unwrap(),
            );

            println!("Reexecution test for block {block_number} passed successfully.");
        }
    }
}
