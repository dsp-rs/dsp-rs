use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::{mpsc::Receiver, oneshot},
    time::interval,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use std::{fmt::Display, time::Duration};

use crate::{
    AppError,
    auth::extractor::AuthClaims,
    connector::{
        Pending, StateData, TerminatedData,
        app_state::{AppState, AppStateNegotiation},
    },
    model::{
        self,
        contract::{
            ContractAgreement, ContractAgreementVerification, ContractNegotiationError,
            ContractNegotiationEvent, ContractNegotiationTermination, ContractOffer,
        },
        policy::{Agreement, MessageOffer},
    },
    negotiation::{
        consumer::{ConsumerView, contract_agreement, contract_offer},
        provider::{
            ProviderView, contract_request, contract_request_counter, contract_verification,
        },
    },
    store::Store,
};

mod consumer;
mod provider;

pub(crate) trait NegotiationStateData {
    type RequestedData: StateData + Pending;
    type OfferedData: StateData + Pending;
    type AcceptedData: StateData + Pending;
    type AgreedData: StateData + Pending;
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum NegotiationState<V: NegotiationStateData> {
    Requested(V::RequestedData),
    Offered(V::OfferedData),
    Accepted(V::AcceptedData),
    Agreed(V::AgreedData),

    Verified(VerifiedData),
    Finalized(FinalizedData),
    Terminated(TerminatedData),
}

impl<V: NegotiationStateData> Pending for NegotiationState<V> {
    fn is_pending(&self) -> bool {
        match self {
            NegotiationState::Requested(s) => s.is_pending(),
            NegotiationState::Offered(s) => s.is_pending(),
            NegotiationState::Accepted(s) => s.is_pending(),
            NegotiationState::Agreed(s) => s.is_pending(),
            NegotiationState::Verified(s) => s.is_pending(),
            NegotiationState::Finalized(s) => s.is_pending(),
            NegotiationState::Terminated(s) => s.is_pending(),
        }
    }
}

impl<V: NegotiationStateData> Display for NegotiationState<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NegotiationState::Requested(_) => f.write_str("REQUESTED"),
            NegotiationState::Offered(_) => f.write_str("OFFERED"),
            NegotiationState::Accepted(_) => f.write_str("ACCEPTED"),
            NegotiationState::Agreed(_) => f.write_str("AGREED"),
            NegotiationState::Verified(_) => f.write_str("VERIFIED"),
            NegotiationState::Finalized(_) => f.write_str("FINALIZED"),
            NegotiationState::Terminated(_) => f.write_str("TERMINATED"),
        }
    }
}

impl<V: NegotiationStateData> NegotiationState<V> {
    pub(crate) fn description(&self) -> String {
        let mut state = self.to_string().to_lowercase();

        // special handling for finalized
        match self {
            NegotiationState::Finalized(data) => {
                state = format!("{state}-{}", data.agreement.policy_class.resource.id);
            }
            _ => {}
        }

        if self.is_pending() {
            state = format!("{state}-pending");
        }

        state
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VerifiedData {
    agreement: Agreement,
}
impl Pending for VerifiedData {}

#[derive(Serialize, Deserialize)]
pub(crate) struct FinalizedData {
    pub(crate) agreement: Agreement,
}
impl Pending for FinalizedData {}

#[derive(Serialize, Deserialize)]
pub(crate) struct ContractNegotiation<V: NegotiationStateData> {
    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
    pub(crate) state: NegotiationState<V>,
}

impl<T: NegotiationStateData> From<ContractNegotiation<T>> for (String, String) {
    fn from(value: ContractNegotiation<T>) -> Self {
        (value.provider_pid, value.consumer_pid)
    }
}

impl<V: NegotiationStateData> ContractNegotiation<V> {
    pub(crate) fn invalid_state_error(self) -> AppError {
        AppError::Contract(ContractNegotiationError::invalid_state(
            self.provider_pid,
            self.consumer_pid,
        ))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Negotiation {
    Provider {
        #[serde(flatten)]
        contract: ContractNegotiation<ProviderView>,
        callback_address: String,
    },
    Consumer {
        #[serde(flatten)]
        contract: ContractNegotiation<ConsumerView>,
        connector: Connector,
    },
}

impl Negotiation {
    pub(crate) fn pid(&self) -> &str {
        match self {
            Negotiation::Provider { contract, .. } => &contract.provider_pid,
            Negotiation::Consumer { contract, .. } => &contract.consumer_pid,
        }
    }

    pub(crate) fn state_description(&self) -> String {
        match self {
            Negotiation::Provider { contract, .. } => contract.state.description(),
            Negotiation::Consumer { contract, .. } => contract.state.description(),
        }
    }
}

impl Negotiation {
    pub(crate) async fn handle_event(
        self,
        claims: AuthClaims,
        event: NegotiationEventPayload,
    ) -> Result<Negotiation, AppError> {
        Ok(match self {
            Negotiation::Provider {
                contract,
                callback_address,
            } => Negotiation::Provider {
                contract: contract.handle_event(claims, event).await?,
                callback_address,
            },
            Negotiation::Consumer {
                contract,
                connector,
            } => Negotiation::Consumer {
                contract: contract.handle_event(claims, event).await?,
                connector,
            },
        })
    }

    pub(crate) async fn tick<T: Store>(
        self,
        state: &AppStateNegotiation<T>,
    ) -> anyhow::Result<Option<Negotiation>> {
        Ok(match self {
            Negotiation::Provider {
                contract,
                callback_address,
            } => contract
                .tick(state, &callback_address)
                .await?
                .map(|c| Negotiation::Provider {
                    contract: c,
                    callback_address,
                }),
            Negotiation::Consumer {
                contract,
                connector,
            } => contract
                .tick(state, &connector)
                .await?
                .map(|c| Negotiation::Consumer {
                    contract: c,
                    connector,
                }),
        })
    }
}

impl From<Negotiation> for (String, String) {
    fn from(value: Negotiation) -> Self {
        match value {
            Negotiation::Provider { contract, .. } => contract.into(),
            Negotiation::Consumer { contract, .. } => contract.into(),
        }
    }
}

impl From<Negotiation> for model::contract::ContractNegotiation {
    fn from(value: Negotiation) -> Self {
        match value {
            Negotiation::Provider { contract, .. } => contract.into(),
            Negotiation::Consumer { contract, .. } => contract.into(),
        }
    }
}

pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
    Router::new()
        // provider
        .route("/request", post(contract_request))
        .route("/{provider_pid}/request", post(contract_request_counter))
        .route(
            "/{provider_pid}/agreement/verification",
            post(contract_verification),
        )
        // consumer
        .route("/{consumer_pid}/offers", post(contract_offer))
        .route("/{consumer_pid}/agreement", post(contract_agreement))
        // both
        .route("/{pid}", get(get_contract))
        .route("/{pid}/events", post(contract_events))
        .route("/{pid}/termination", post(contract_termination))
}

async fn get_contract<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(pid): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Retrieving contract with claims: {:?}", claims);

    if let Some(n) = state
        .store
        .get_negotiation(&pid)
        .await
        .map_err(anyhow::Error::msg)?
    {
        return Ok(Json(model::contract::ContractNegotiation::from(n)));
    }

    Err(AppError::NotFound)
}

async fn contract_events<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(pid): Path<String>,
    Json(event): Json<ContractNegotiationEvent>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract event with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(NegotiationEvent {
            pid,
            claims,
            payload: NegotiationEventPayload::Event(event),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    match rx.await.map_err(anyhow::Error::new)? {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => Err(err),
    }
}

async fn contract_termination<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateNegotiation<T>>,
    Path(pid): Path<String>,
    Json(request): Json<ContractNegotiationTermination>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Contract termination with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(NegotiationEvent {
            pid,
            claims,
            payload: NegotiationEventPayload::Terminate(request),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    match rx.await.map_err(anyhow::Error::new)? {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => Err(err),
    }
}

pub(crate) struct NegotiationEvent {
    pid: String,
    claims: AuthClaims,
    payload: NegotiationEventPayload,
    result: oneshot::Sender<Result<Negotiation, AppError>>,
}

pub(crate) enum NegotiationEventPayload {
    Update(Negotiation),
    Offer(ContractOffer),
    Agree(ContractAgreement),
    Event(ContractNegotiationEvent),
    Verify(ContractAgreementVerification),
    Terminate(ContractNegotiationTermination),
}

impl From<NegotiationEventPayload> for (String, String) {
    fn from(value: NegotiationEventPayload) -> Self {
        match value {
            NegotiationEventPayload::Update(negotiation) => negotiation.into(),
            NegotiationEventPayload::Offer(offer) => offer.into(),
            NegotiationEventPayload::Agree(agreement) => agreement.into(),
            NegotiationEventPayload::Event(event) => event.into(),
            NegotiationEventPayload::Verify(verification) => verification.into(),
            NegotiationEventPayload::Terminate(termination) => termination.into(),
        }
    }
}

pub(crate) async fn handle_negotiations<T: Store>(
    token: CancellationToken,
    state: AppStateNegotiation<T>,
    mut rx: Receiver<NegotiationEvent>,
) {
    let mut tick = interval(Duration::from_secs(1));
    tick.tick().await;

    loop {
        select! {
            _ = token.cancelled() => {
                return;
            }
            event = rx.recv() => {
                match event {
                    Some(e) => handle_event(&state, e).await,
                    None => break,
                }
            }
            _ = tick.tick() => {
                process_negotiations(&state).await;
            }
        }
    }
}

async fn handle_event<T: Store>(state: &AppStateNegotiation<T>, event: NegotiationEvent) {
    let n = match state.store.get_negotiation(&event.pid).await {
        Ok(Some(n)) => n,
        Ok(None) => {
            warn!(
                "Ignoring event for an unknown negotiation {pid}",
                pid = event.pid
            );
            let (provider_pid, consumer_pid) = event.payload.into();
            if let Err(_) = event.result.send(Err(AppError::Contract(
                ContractNegotiationError::contract_not_found(provider_pid, consumer_pid),
            ))) {
                error!("Failed to send result");
            }
            return;
        }
        Err(err) => {
            error!(
                "Failed to fetch negotiation {pid}, error: {err}",
                pid = event.pid
            );
            if let Err(_) = event.result.send(Err(AppError::Generic(err))) {
                error!("Failed to send result");
            }
            return;
        }
    };
    let mut result = n.handle_event(event.claims, event.payload).await;
    if let Ok(negotiation) = result {
        let pid = negotiation.pid().to_owned();
        result = state
            .store
            .save_negotiation(&negotiation)
            .await
            .map(|_| negotiation)
            .map_err(|err| {
                error!("Failed to store negotiation {pid}, error: {err}");
                AppError::Generic(err)
            })
    }
    if let Err(_) = event.result.send(result) {
        error!("Failed to send result");
    }
}

async fn process_negotiations<T: Store>(state: &AppStateNegotiation<T>) {
    let negotiations = match state.store.get_pending_negotiations(100).await {
        Ok(n) => n,
        Err(err) => {
            error!("Failed to fetch pending negotiations, error: {err}");
            return;
        }
    };

    let updated = stream::iter(negotiations)
        .map(|n| async move { n.tick(state).await })
        .buffer_unordered(32)
        .collect::<Vec<_>>()
        .await;

    for n in updated {
        match n {
            Ok(Some(n)) => {
                if let Err(err) = state.store.save_negotiation(&n).await {
                    error!("Failed to save negotiation, error: {err}");
                }
            }
            Ok(_) => {}
            Err(err) => {
                error!("Failed to process negotiation, error: {err}");
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connector {
    #[serde(rename = "connectorAddress")]
    pub(crate) address: String,
    pub(crate) provider_id: String,
}

impl Connector {
    pub(crate) fn api_address(&self) -> String {
        #[cfg(feature = "tck")]
        return format!("{}", self.address);

        // NOTE: currently only 2025-1 supported
        #[cfg(not(feature = "tck"))]
        format!("{}{}", self.address, crate::connector::DSP_API_PATH_2025_1)
    }
}

pub(crate) async fn negotiate<T: Store>(
    store: &T,
    offer: MessageOffer,
    connector: Connector,
) -> anyhow::Result<String> {
    let consumer_pid = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
    let contract = ContractNegotiation {
        provider_pid: "".into(),
        consumer_pid: consumer_pid.clone(),
        state: NegotiationState::Requested(consumer::RequestedData {
            sent: false,
            payload: consumer::RequestedDataPayload {
                offer,
                provider_pid: None,
            },
        }),
    };
    let n = Negotiation::Consumer {
        contract,
        connector,
    };
    store.save_negotiation(&n).await?;

    Ok(consumer_pid)
}

#[cfg(feature = "tck")]
pub(crate) mod tck {
    use axum::{Router, routing::post};

    use crate::{
        connector::app_state::AppState, negotiation::consumer::tck::negotiation_request,
        store::Store,
    };

    pub(crate) fn tck_router<T: Store>() -> Router<AppState<T>> {
        Router::new().route("/request", post(negotiation_request))
    }
}
