use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;
use tokio::signal;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::error;

use crate::{
    connector::start,
    model::{catalog::CatalogError, contract::ContractNegotiationError, transfer::TransferError},
};

mod model;

mod auth;
mod catalog;
mod connector;
mod negotiation;
mod policy_engine;
mod reverse_proxy;
mod store;
mod transfer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let token = CancellationToken::new();
    let tracker = TaskTracker::new();

    start(token.clone(), &tracker).await?;

    match signal::ctrl_c().await {
        Ok(_) => {}
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }
    token.cancel();

    tracker.close();
    Ok(())
}

#[derive(Error, Debug)]
pub(crate) enum AppError {
    #[error("not found")]
    NotFound,

    #[error("{0}")]
    BadRequest(String),

    #[error(transparent)]
    Catalog(#[from] CatalogError),

    #[error(transparent)]
    Contract(#[from] ContractNegotiationError),

    #[error(transparent)]
    Transfer(#[from] TransferError),

    #[error(transparent)]
    Generic(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            Self::NotFound => (StatusCode::NOT_FOUND, json!("not found")),
            Self::BadRequest(err) => (StatusCode::NOT_FOUND, json!(err)),
            Self::Catalog(err) => (StatusCode::BAD_REQUEST, json!(err)),
            Self::Contract(err) => (StatusCode::BAD_REQUEST, json!(err)),
            Self::Transfer(err) => (StatusCode::BAD_REQUEST, json!(err)),
            Self::Generic(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "error": err.to_string()
                }),
            ),
        };

        (status, Json(body)).into_response()
    }
}
