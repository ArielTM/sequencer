use std::future::pending;
use std::pin::Pin;

use futures::stream::FuturesUnordered;
use futures::{Future, FutureExt, StreamExt};
use starknet_batcher::communication::{LocalBatcherServer, RemoteBatcherServer};
use starknet_consensus_manager::communication::ConsensusManagerServer;
use starknet_gateway::communication::{LocalGatewayServer, RemoteGatewayServer};
use starknet_http_server::communication::HttpServer;
use starknet_mempool::communication::{LocalMempoolServer, RemoteMempoolServer};
use starknet_mempool_p2p::propagator::{
    LocalMempoolP2pPropagatorServer,
    RemoteMempoolP2pPropagatorServer,
};
use starknet_mempool_p2p::runner::MempoolP2pRunnerServer;
use starknet_monitoring_endpoint::communication::MonitoringEndpointServer;
use starknet_sequencer_infra::component_server::{
    ComponentServerStarter,
    LocalComponentServer,
    RemoteComponentServer,
    WrapperServer,
};
use starknet_sequencer_infra::errors::ComponentServerError;
use tracing::error;

use crate::clients::SequencerNodeClients;
use crate::communication::SequencerNodeCommunication;
use crate::components::SequencerNodeComponents;
use crate::config::component_execution_config::ComponentExecutionMode;
use crate::config::node_config::SequencerNodeConfig;

type ServerFuture = Pin<Box<dyn Future<Output = Result<(), ComponentServerError>> + Send>>;

// Component servers that can run locally.
struct LocalServers {
    pub(crate) batcher: Option<Box<LocalBatcherServer>>,
    pub(crate) gateway: Option<Box<LocalGatewayServer>>,
    pub(crate) mempool: Option<Box<LocalMempoolServer>>,
    pub(crate) mempool_p2p_propagator: Option<Box<LocalMempoolP2pPropagatorServer>>,
}

// TODO(Nadin): Add a procedural attribute macro for get_server_futures and get_spawn_server_tasks.
impl LocalServers {
    pub fn get_server_futures(self) -> Vec<(&'static str, ServerFuture)> {
        let batcher_future = get_server_future(self.batcher);
        let gateway_future = get_server_future(self.gateway);
        let mempool_future = get_server_future(self.mempool);
        let mempool_p2p_propagator_future = get_server_future(self.mempool_p2p_propagator);

        vec![
            ("Batcher", batcher_future),
            ("Gateway", gateway_future),
            ("Mempool", mempool_future),
            ("Mempool P2P Propagator", mempool_p2p_propagator_future),
        ]
    }

    pub fn get_spawn_server_tasks(
        self,
    ) -> Vec<(String, tokio::task::JoinHandle<Result<(), ComponentServerError>>)> {
        self.get_server_futures()
            .into_iter()
            .map(|(name, future)| {
                let local_name = format!("Local {}", name);
                let handle = tokio::spawn(future);
                (local_name, handle)
            })
            .collect()
    }
}

// Component servers that wrap a component without a server.
struct WrapperServers {
    pub(crate) consensus_manager: Option<Box<ConsensusManagerServer>>,
    pub(crate) http_server: Option<Box<HttpServer>>,
    pub(crate) monitoring_endpoint: Option<Box<MonitoringEndpointServer>>,
    pub(crate) mempool_p2p_runner: Option<Box<MempoolP2pRunnerServer>>,
}

impl WrapperServers {
    pub fn get_server_futures(self) -> Vec<(&'static str, ServerFuture)> {
        let consensus_manager_future = get_server_future(self.consensus_manager);
        let http_server_future = get_server_future(self.http_server);
        let monitoring_endpoint_future = get_server_future(self.monitoring_endpoint);
        let mempool_p2p_runner_future = get_server_future(self.mempool_p2p_runner);

        vec![
            ("Consensus Manager", consensus_manager_future),
            ("Http", http_server_future),
            ("Monitoring Endpoint", monitoring_endpoint_future),
            ("Mempool P2P Runner", mempool_p2p_runner_future),
        ]
    }

    pub fn get_spawn_server_tasks(
        self,
    ) -> Vec<(String, tokio::task::JoinHandle<Result<(), ComponentServerError>>)> {
        self.get_server_futures()
            .into_iter()
            .map(|(name, future)| {
                let wrapper_name = format!("Wrapper {}", name);
                let handle = tokio::spawn(future);
                (wrapper_name, handle)
            })
            .collect()
    }
}

// Component servers that can run remotely.
struct RemoteServers {
    pub(crate) batcher: Option<Box<RemoteBatcherServer>>,
    pub(crate) gateway: Option<Box<RemoteGatewayServer>>,
    pub(crate) mempool: Option<Box<RemoteMempoolServer>>,
    pub(crate) mempool_p2p_propagator: Option<Box<RemoteMempoolP2pPropagatorServer>>,
}

impl RemoteServers {
    pub fn get_server_futures(self) -> Vec<(&'static str, ServerFuture)> {
        let batcher_future = get_server_future(self.batcher);
        let gateway_future = get_server_future(self.gateway);
        let mempool_future = get_server_future(self.mempool);
        let mempool_p2p_propagator_future = get_server_future(self.mempool_p2p_propagator);

        vec![
            ("Batcher", batcher_future),
            ("Gateway", gateway_future),
            ("Mempool", mempool_future),
            ("Mempool P2P Propagator", mempool_p2p_propagator_future),
        ]
    }

    pub fn get_spawn_server_tasks(
        self,
    ) -> Vec<(String, tokio::task::JoinHandle<Result<(), ComponentServerError>>)> {
        self.get_server_futures()
            .into_iter()
            .map(|(name, future)| {
                let remote_name = format!("Remote {}", name);
                let handle = tokio::spawn(future);
                (remote_name, handle)
            })
            .collect()
    }
}

// TODO(Nadin): Add get_spawn_server_tasks function.
pub struct SequencerNodeServers {
    local_servers: LocalServers,
    remote_servers: RemoteServers,
    wrapper_servers: WrapperServers,
}

/// A macro for creating a remote component server based on the component's execution mode.
/// Returns a remote server if the component is configured with Remote execution mode; otherwise,
/// returns None.
///
/// # Arguments
///
/// * `$execution_mode` - A reference to the component's execution mode, of type
///   `&ComponentExecutionMode`.
/// * `$local_client` - The local client to be used for the remote server initialization if the
///   execution mode is `Remote`.
/// * `$config` - The configuration for the remote server.
///
/// # Returns
///
/// An `Option<Box<RemoteComponentServer<LocalClientType, RequestType, ResponseType>>>` containing
/// the remote server if the execution mode is Remote, or None if the execution mode is Disabled,
/// LocalExecutionWithRemoteEnabled or LocalExecutionWithRemoteDisabled.
///
/// # Example
///
/// ```rust,ignore
/// let batcher_remote_server = create_remote_server!(
///     &config.components.batcher.execution_mode,
///     clients.get_gateway_local_client(),
///     config.remote_server_config
/// );
/// match batcher_remote_server {
///     Some(server) => println!("Remote server created: {:?}", server),
///     None => println!("Remote server not created because the execution mode is not remote."),
/// }
/// ```
#[macro_export]
macro_rules! create_remote_server {
    ($execution_mode:expr, $local_client:expr, $config:expr) => {
        match *$execution_mode {
            ComponentExecutionMode::Remote => {
                let local_client = $local_client
                    .expect("Error: local client must be initialized in Remote execution mode.");
                let config = $config
                    .as_ref()
                    .expect("Error: config must be initialized in Remote execution mode.");

                Some(Box::new(RemoteComponentServer::new(local_client, config.clone())))
            }
            ComponentExecutionMode::LocalExecutionWithRemoteDisabled
            | ComponentExecutionMode::LocalExecutionWithRemoteEnabled
            | ComponentExecutionMode::Disabled => None,
        }
    };
}

/// A macro for creating a component server, determined by the component's execution mode. Returns a
/// local server if the component is run locally, otherwise None.
///
/// # Arguments
///
/// * $execution_mode - A reference to the component's execution mode, i.e., type
///   &ComponentExecutionMode.
/// * $component - The component that will be taken to initialize the server if the execution mode
///   is enabled(LocalExecutionWithRemoteDisabled / LocalExecutionWithRemoteEnabled).
/// * $Receiver - receiver side for the server.
///
/// # Returns
///
/// An Option<Box<LocalComponentServer<ComponentType, RequestType, ResponseType>>> containing the
/// server if the execution mode is enabled(LocalExecutionWithRemoteDisabled /
/// LocalExecutionWithRemoteEnabled), or None if the execution mode is Disabled.
///
/// # Example
///
/// ```rust,ignore
/// let batcher_server = create_local_server!(
///     &config.components.batcher.execution_mode,
///     components.batcher,
///     communication.take_batcher_rx()
/// );
/// match batcher_server {
///     Some(server) => println!("Server created: {:?}", server),
///     None => println!("Server not created because the execution mode is disabled."),
/// }
/// ```
macro_rules! create_local_server {
    ($execution_mode:expr, $component:expr, $receiver:expr) => {
        match *$execution_mode {
            ComponentExecutionMode::LocalExecutionWithRemoteDisabled
            | ComponentExecutionMode::LocalExecutionWithRemoteEnabled => {
                Some(Box::new(LocalComponentServer::new(
                    $component
                        .take()
                        .expect(concat!(stringify!($component), " is not initialized.")),
                    $receiver,
                )))
            }
            ComponentExecutionMode::Disabled | ComponentExecutionMode::Remote => None,
        }
    };
}

/// A macro for creating a WrapperServer, determined by the component's execution mode. Returns a
/// wrapper server if the component is run locally, otherwise None.
///
/// # Arguments
///
/// * $execution_mode - A reference to the component's execution mode, i.e., type
///   &ComponentExecutionMode.
/// * $component - The component that will be taken to initialize the server if the execution mode
///   is enabled(LocalExecutionWithRemoteDisabled / LocalExecutionWithRemoteEnabled).
///
/// # Returns
///
/// An `Option<Box<WrapperServer<ComponentType>>>` containing the server if the execution mode is
/// enabled(LocalExecutionWithRemoteDisabled / LocalExecutionWithRemoteEnabled), or `None` if the
/// execution mode is `Disabled`.
///
/// # Example
///
/// ```rust, ignore
/// // Assuming ComponentExecutionMode and components are defined, and WrapperServer
/// // has a new method that accepts a component.
/// let consensus_manager_server = create_wrapper_server!(
///     &config.components.consensus_manager.execution_mode,
///     components.consensus_manager
/// );
///
/// match consensus_manager_server {
///     Some(server) => println!("Server created: {:?}", server),
///     None => println!("Server not created because the execution mode is disabled."),
/// }
/// ```
macro_rules! create_wrapper_server {
    ($execution_mode:expr, $component:expr) => {
        match *$execution_mode {
            ComponentExecutionMode::LocalExecutionWithRemoteDisabled
            | ComponentExecutionMode::LocalExecutionWithRemoteEnabled => {
                Some(Box::new(WrapperServer::new(
                    $component
                        .take()
                        .expect(concat!(stringify!($component), " is not initialized.")),
                )))
            }
            ComponentExecutionMode::Disabled | ComponentExecutionMode::Remote => None,
        }
    };
}

fn create_local_servers(
    config: &SequencerNodeConfig,
    communication: &mut SequencerNodeCommunication,
    components: &mut SequencerNodeComponents,
) -> LocalServers {
    let batcher_server = create_local_server!(
        &config.components.batcher.execution_mode,
        components.batcher,
        communication.take_batcher_rx()
    );
    let gateway_server = create_local_server!(
        &config.components.gateway.execution_mode,
        components.gateway,
        communication.take_gateway_rx()
    );
    let mempool_server = create_local_server!(
        &config.components.mempool.execution_mode,
        components.mempool,
        communication.take_mempool_rx()
    );
    let mempool_p2p_propagator_server = create_local_server!(
        &config.components.mempool_p2p.execution_mode,
        components.mempool_p2p_propagator,
        communication.take_mempool_p2p_propagator_rx()
    );
    LocalServers {
        batcher: batcher_server,
        gateway: gateway_server,
        mempool: mempool_server,
        mempool_p2p_propagator: mempool_p2p_propagator_server,
    }
}

fn create_remote_servers(
    config: &SequencerNodeConfig,
    clients: &SequencerNodeClients,
) -> RemoteServers {
    let batcher_client = clients.get_batcher_local_client();
    let batcher_server = create_remote_server!(
        &config.components.batcher.execution_mode,
        batcher_client,
        config.components.batcher.remote_server_config
    );

    let gateway_client = clients.get_gateway_local_client();
    let gateway_server = create_remote_server!(
        &config.components.gateway.execution_mode,
        gateway_client,
        config.components.gateway.remote_server_config
    );

    let mempool_client = clients.get_mempool_local_client();
    let mempool_server = create_remote_server!(
        &config.components.mempool.execution_mode,
        mempool_client,
        config.components.mempool.remote_server_config
    );

    let mempool_p2p_propagator_client = clients.get_mempool_p2p_propagator_local_client();
    let mempool_p2p_propagator_server = create_remote_server!(
        &config.components.mempool_p2p.execution_mode,
        mempool_p2p_propagator_client,
        config.components.mempool_p2p.remote_server_config
    );
    RemoteServers {
        batcher: batcher_server,
        gateway: gateway_server,
        mempool: mempool_server,
        mempool_p2p_propagator: mempool_p2p_propagator_server,
    }
}

fn create_wrapper_servers(
    config: &SequencerNodeConfig,
    components: &mut SequencerNodeComponents,
) -> WrapperServers {
    let consensus_manager_server = create_wrapper_server!(
        &config.components.consensus_manager.execution_mode,
        components.consensus_manager
    );
    let http_server = create_wrapper_server!(
        &config.components.http_server.execution_mode,
        components.http_server
    );

    let monitoring_endpoint_server = create_wrapper_server!(
        &config.components.monitoring_endpoint.execution_mode,
        components.monitoring_endpoint
    );

    let mempool_p2p_runner_server = create_wrapper_server!(
        &config.components.mempool_p2p.execution_mode,
        components.mempool_p2p_runner
    );
    WrapperServers {
        consensus_manager: consensus_manager_server,
        http_server,
        monitoring_endpoint: monitoring_endpoint_server,
        mempool_p2p_runner: mempool_p2p_runner_server,
    }
}

pub fn create_node_servers(
    config: &SequencerNodeConfig,
    communication: &mut SequencerNodeCommunication,
    components: SequencerNodeComponents,
    clients: &SequencerNodeClients,
) -> SequencerNodeServers {
    let mut components = components;
    let local_servers = create_local_servers(config, communication, &mut components);
    let remote_servers = create_remote_servers(config, clients);
    let wrapper_servers = create_wrapper_servers(config, &mut components);

    SequencerNodeServers { local_servers, remote_servers, wrapper_servers }
}

// TODO(Nadin): refactor this function to reduce code duplication.
pub async fn run_component_servers(servers: SequencerNodeServers) -> anyhow::Result<()> {
    // local server tasks.
    // TODO(Nadin): Modify get_spawn_server_tasks to return a stream of server tasks.
    let local_server_tasks = servers.local_servers.get_spawn_server_tasks();

    let mut local_servers_stream: FuturesUnordered<_> = local_server_tasks
        .into_iter()
        .map(|(name, handle)| async move { (name, handle.await) })
        .collect();

    // remote server tasks.
    let remote_server_tasks = servers.remote_servers.get_spawn_server_tasks();

    let mut remote_servers_stream: FuturesUnordered<_> = remote_server_tasks
        .into_iter()
        .map(|(name, handle)| async move { (name, handle.await) })
        .collect();

    // wrapper server tasks.
    let wrapper_server_tasks = servers.wrapper_servers.get_spawn_server_tasks();

    let mut wrapper_servers_stream: FuturesUnordered<_> = wrapper_server_tasks
        .into_iter()
        .map(|(name, handle)| async move { (name, handle.await) })
        .collect();

    // TODO(Nadin): Add macro for the result (continue / break).
    let result = loop {
        tokio::select! {
            Some((name, res)) = local_servers_stream.next() => {
                error!("{} Server stopped.", name);
                match res {
                    Ok(_) => continue,
                    Err(e) => break Err(e),
                }
            }
            Some((name, res)) = remote_servers_stream.next() => {
                error!("{} Server stopped.", name);
                match res {
                    Ok(_) => continue,
                    Err(e) => break Err(e),
                }
            }
            Some((name, res)) = wrapper_servers_stream.next() => {
                error!("{} Server stopped.", name);
                match res {
                    Ok(_) => continue,
                    Err(e) => break Err(e),
                }
            }
        }
    };
    error!("Servers ended with unexpected Ok.");

    Ok(result?)
}

pub fn get_server_future(
    server: Option<Box<impl ComponentServerStarter + Send + 'static>>,
) -> Pin<Box<dyn Future<Output = Result<(), ComponentServerError>> + Send>> {
    match server {
        Some(mut server) => async move { server.start().await }.boxed(),
        None => pending().boxed(),
    }
}
