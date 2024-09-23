use std::any::type_name;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc::Receiver;
use tracing::{error, info};

use crate::component_definitions::{ComponentRequestAndResponseSender, ComponentRequestHandler};
use crate::component_runner::ComponentError;

#[derive(Error, Debug, PartialEq, Clone)]
pub enum ComponentServerError {
    #[error("Server has already been started.")]
    ServerAlreadyStarted,
    #[error("Http server has failed: {0}.")]
    HttpServerStartError(String),
    #[error(transparent)]
    ComponentError(#[from] ComponentError),
    #[error("Server unexpectedly stopped.")]
    ServerUnexpectedlyStopped,
}

#[async_trait]
pub trait ComponentServerStarter: Send + Sync {
    async fn start(&mut self) -> Result<(), ComponentServerError>;
}

pub async fn request_response_loop<Request, Response, Component>(
    rx: &mut Receiver<ComponentRequestAndResponseSender<Request, Response>>,
    component: &mut Component,
) where
    Component: ComponentRequestHandler<Request, Response> + Send + Sync,
    Request: Send + Sync,
    Response: Send + Sync,
{
    // TODO(Tsabary): Make requests and responses implement `std::fmt::Display`, and add the request
    // to the log.
    while let Some(request_and_res_tx) = rx.recv().await {
        info!("Component {} received request", type_name::<Component>());

        let request = request_and_res_tx.request;
        let tx = request_and_res_tx.tx;

        let res = component.handle_request(request).await;

        tx.send(res).await.expect("Response connection should be open.");
    }
}
