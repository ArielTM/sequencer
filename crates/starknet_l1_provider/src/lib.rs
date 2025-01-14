pub mod errors;

#[cfg(test)]
pub mod test_utils;

use indexmap::{IndexMap, IndexSet};
use starknet_api::executable_transaction::L1HandlerTransaction;
use starknet_api::transaction::TransactionHash;

use crate::errors::L1ProviderError;

type L1ProviderResult<T> = Result<T, L1ProviderError>;

#[cfg(test)]
#[path = "l1_provider_tests.rs"]
pub mod l1_provider_tests;

// TODO: optimistic proposer support, will add later to keep things simple, but the design here
// is compatible with it.
#[derive(Debug, Default)]
pub struct L1Provider {
    tx_manager: TransactionManager,
    // TODO(Gilad): consider transitioning to a generic phantom state once the infra is stabilized
    // and we see how well it handles consuming the L1Provider when moving between states.
    state: ProviderState,
}

impl L1Provider {
    pub async fn new(_config: L1ProviderConfig) -> L1ProviderResult<Self> {
        todo!(
            "init crawler to start next crawl from ~1 hour ago, this can have l1 errors when \
             finding the latest block on L1 to 'subtract' 1 hour from."
        );
    }

    /// Retrieves up to `n_txs` transactions that have yet to be proposed or accepted on L2.
    pub fn get_txs(&mut self, n_txs: usize) -> L1ProviderResult<Vec<L1HandlerTransaction>> {
        match self.state {
            ProviderState::Propose => Ok(self.tx_manager.get_txs(n_txs)),
            ProviderState::Pending => Err(L1ProviderError::GetTransactionsInPendingState),
            ProviderState::Validate => Err(L1ProviderError::GetTransactionConsensusBug),
        }
    }

    /// Returns true if and only if the given transaction is both not included in an L2 block, and
    /// unconsumed on L1.
    pub fn validate(&self, tx_hash: TransactionHash) -> L1ProviderResult<ValidationStatus> {
        match self.state {
            ProviderState::Validate => Ok(self.tx_manager.tx_status(tx_hash)),
            ProviderState::Propose => Err(L1ProviderError::ValidateTransactionConsensusBug),
            ProviderState::Pending => Err(L1ProviderError::ValidateInPendingState),
        }
    }

    // TODO: when deciding on consensus, if possible, have commit_block also tell the node if it's
    // about to [optimistically-]propose or validate the next block.
    pub fn commit_block(&mut self, _commited_txs: &[TransactionHash]) {
        todo!(
            "Purges txs from internal buffers, if was proposer clear staging buffer, 
            reset state to Pending until we get proposing/validating notice from consensus."
        )
    }

    // TODO: pending formal consensus API, guessing the API here to keep things moving.
    // TODO: consider adding block number, it isn't strictly necessary, but will help debugging.
    pub fn validation_start(&mut self) -> L1ProviderResult<()> {
        self.state = self.state.transition_to_validate()?;
        Ok(())
    }

    pub fn proposal_start(&mut self) -> L1ProviderResult<()> {
        self.state = self.state.transition_to_propose()?;
        Ok(())
    }

    /// Simple recovery from L1 and L2 reorgs by reseting the service, which rewinds L1 and L2
    /// information.
    pub fn handle_reorg(&mut self) -> L1ProviderResult<()> {
        self.reset()
    }

    // TODO: this will likely change during integration with infra team.
    pub async fn start(&self) {
        todo!(
            "Create a process that wakes up every config.poll_interval seconds and updates
        internal L1 and L2 buffers according to collected L1 events and recent blocks created on
        L2."
        )
    }

    fn reset(&mut self) -> L1ProviderResult<()> {
        todo!(
            "resets internal buffers and rewinds the internal crawler _pointer_ back for ~1 \
             hour,so that the main loop will start collecting from that time gracefully. May hit \
             base layer errors."
        );
    }
}

#[derive(Debug, Default)]
struct TransactionManager {
    txs: IndexMap<TransactionHash, L1HandlerTransaction>,
    proposed_txs: IndexSet<TransactionHash>,
    on_l2_awaiting_l1_consumption: IndexSet<TransactionHash>,
}

impl TransactionManager {
    pub fn get_txs(&mut self, n_txs: usize) -> Vec<L1HandlerTransaction> {
        let (tx_hashes, txs): (Vec<_>, Vec<_>) = self
            .txs
            .iter()
            .skip(self.proposed_txs.len()) // Transactions are proposed FIFO.
            .take(n_txs)
            .map(|(&hash, tx)| (hash, tx.clone()))
            .unzip();

        self.proposed_txs.extend(tx_hashes);
        txs
    }

    pub fn tx_status(&self, tx_hash: TransactionHash) -> ValidationStatus {
        if self.txs.contains_key(&tx_hash) {
            ValidationStatus::Validated
        } else if self.on_l2_awaiting_l1_consumption.contains(&tx_hash) {
            ValidationStatus::AlreadyIncludedOnL2
        } else {
            ValidationStatus::ConsumedOnL1OrUnknown
        }
    }

    pub fn _add_unconsumed_l1_not_in_l2_block_tx(&mut self, _tx: L1HandlerTransaction) {
        todo!(
            "Check if tx is in L2, if it isn't on L2 add it to the txs buffer, otherwise print
             debug and do nothing."
        )
    }

    pub fn _mark_tx_included_on_l2(&mut self, _tx_hash: &TransactionHash) {
        todo!("Adds the tx hash to l2 buffer; remove tx from the txs storage if it's there.")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationStatus {
    Validated,
    AlreadyIncludedOnL2,
    ConsumedOnL1OrUnknown,
}

/// Current state of the provider, where pending means: idle, between proposal/validation cycles.
#[derive(Clone, Copy, Debug, Default)]
pub enum ProviderState {
    #[default]
    Pending,
    Propose,
    Validate,
}

impl ProviderState {
    fn transition_to_propose(self) -> L1ProviderResult<Self> {
        match self {
            ProviderState::Pending => Ok(ProviderState::Propose),
            _ => Err(L1ProviderError::UnexpectedProviderStateTransition {
                from: self,
                to: ProviderState::Propose,
            }),
        }
    }

    fn transition_to_validate(self) -> L1ProviderResult<Self> {
        match self {
            ProviderState::Pending => Ok(ProviderState::Validate),
            _ => Err(L1ProviderError::UnexpectedProviderStateTransition {
                from: self,
                to: ProviderState::Validate,
            }),
        }
    }

    fn _transition_to_pending(self) -> L1ProviderResult<Self> {
        todo!()
    }

    pub fn as_str(&self) -> &str {
        match self {
            ProviderState::Pending => "Pending",
            ProviderState::Propose => "Propose",
            ProviderState::Validate => "Validate",
        }
    }
}

impl std::fmt::Display for ProviderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub struct L1ProviderConfig;
