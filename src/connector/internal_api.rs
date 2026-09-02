use axum::{
    Json, Router,
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::{
    AppError,
    connector::app_state::{AppState, AppStateAPI},
    model::{dataset::AccessService, policy::MessageOffer},
    negotiation::{Connector, Negotiation, NegotiationState, negotiate},
    store::Store,
    transfer::transfer,
};

pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
    Router::new()
        .route("/negotiate", post(api_negotiate))
        .route("/negotiate/{consumer_pid}", get(api_negotiation_state))
        .route("/transfer", post(api_transfer))
        .route("/transfer/{consumer_pid}", get(api_transfer_state))
}

#[derive(Debug, Deserialize)]
struct NegotiateRequest {
    name: String,
    dataset_id: String,
    offer_id: String,
}

async fn api_negotiate<T: Store>(
    State(state): State<AppStateAPI<T>>,
    Json(request): Json<NegotiateRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Some(dataset) = state
        .store
        .get_federated_dataset(&request.name, &request.dataset_id)
        .await?
    else {
        return Err(AppError::NotFound);
    };

    let Some(offer) = dataset
        .dataset
        .has_policy
        .into_iter()
        .find(|p| p.policy_class.resource.id == request.offer_id)
    else {
        return Err(AppError::BadRequest("invalid offer for dataset".into()));
    };

    let Some(remote_address) = dataset
        .dataset
        .distribution
        .into_iter()
        .filter_map(|d| {
            if let AccessService::Concrete(access_service) = d.access_service {
                Some(access_service.endpoint_url)
            } else {
                None
            }
        })
        .next()
    else {
        return Err(AppError::BadRequest(
            "dataset has no valid distribution".into(),
        ));
    };

    // TODO: retrieve metadata and pass path for version 2025-1 inside connector

    let consumer_pid = negotiate(
        state.store.as_ref(),
        MessageOffer {
            policy_class: offer.policy_class,
            target: request.dataset_id,
        },
        Connector {
            address: remote_address,
            provider_id: request.name,
        },
    )
    .await?;

    Ok(Json(json!({"consumer_pid": consumer_pid})))
}

// TODO: should return an agreement enum with states pending, failed, suceeded.
async fn api_negotiation_state<T: Store>(
    State(state): State<AppStateAPI<T>>,
    Path(consumer_pid): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let Some(negotiation) = state.store.get_negotiation(&consumer_pid).await? else {
        warn!("Unkown negotiation with cunsumer pid {consumer_pid}");
        return Err(AppError::NotFound);
    };

    Ok(Json(negotiation))
}

#[derive(Debug, Deserialize)]
struct TransferRequest {
    agreement_id: String,
    format: String,
}

async fn api_transfer<T: Store>(
    State(state): State<AppStateAPI<T>>,
    Json(request): Json<TransferRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Some(Negotiation::Consumer {
        contract,
        connector,
    }) = state
        .store
        .get_finalized_negotiation(&request.agreement_id)
        .await?
    else {
        return Err(AppError::NotFound);
    };

    let NegotiationState::Finalized(finalized_state) = contract.state else {
        // should not happen, hene generic error (status 500)
        return Err(AppError::Generic(anyhow::anyhow!(
            "negotiation not finalized"
        )));
    };

    let consumer_pid = transfer(
        state.store.as_ref(),
        finalized_state.agreement,
        request.format,
        connector,
    )
    .await?;

    Ok(Json(json!({"consumer_pid": consumer_pid})))
}

// TODO: should return a transfer enum with states pending, failed, paused, started.
async fn api_transfer_state<T: Store>(
    State(state): State<AppStateAPI<T>>,
    Path(consumer_pid): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let Some(transfer) = state.store.get_transfer(&consumer_pid).await? else {
        warn!("Unkown transfer with cunsumer pid {consumer_pid}");
        return Err(AppError::NotFound);
    };

    Ok(Json(transfer))
}
