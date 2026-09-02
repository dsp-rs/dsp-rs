use std::{fmt::Display, time::Duration};

use axum::{
    Router,
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
};
use futures::{StreamExt, stream};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::{mpsc::Receiver, oneshot},
    time::interval,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use crate::{
    AppError,
    auth::extractor::AuthClaims,
    connector::{
        Pending, PendingStateData, StateData, TerminatedData, TerminatedDataPayload,
        app_state::{AppState, AppStateTransfer},
        validator::ValidatedJson,
    },
    model::{
        self,
        policy::Agreement,
        transfer::{
            AbstractTransferCode, DataAddress, TransferCompletion, TransferError, TransferStart,
        },
    },
    negotiation::Connector,
    store::Store,
    transfer::{
        consumer::ConsumerView,
        provider::{ProviderView, get_transfer, transfer_request},
    },
};

mod consumer;
mod provider;

pub(crate) trait TransferStateData {
    type RequestedData: StateData + Pending;
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum TransferState<V: TransferStateData> {
    Requested(V::RequestedData),
    Started(StartedData),
    Suspended(SuspendedData),
    Completed(CompletedData),
    Terminated(TerminatedData),
}

impl<V: TransferStateData> Pending for TransferState<V> {
    fn is_pending(&self) -> bool {
        match self {
            TransferState::Requested(s) => s.is_pending(),
            TransferState::Started(s) => s.is_pending(),
            TransferState::Suspended(s) => s.is_pending(),
            TransferState::Completed(s) => s.is_pending(),
            TransferState::Terminated(s) => s.is_pending(),
        }
    }
}

impl<V: TransferStateData> Display for TransferState<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferState::Requested(_) => f.write_str("REQUESTED"),
            TransferState::Started(_) => f.write_str("STARTED"),
            TransferState::Suspended(_) => f.write_str("SUSPENDED"),
            TransferState::Completed(_) => f.write_str("COMPLETED"),
            TransferState::Terminated(_) => f.write_str("TERMINATED"),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct StartedDataPayload {
    #[cfg(feature = "tck")]
    pub(crate) count: usize,
}

pub(crate) type StartedData = PendingStateData<StartedDataPayload>;

#[derive(Serialize, Deserialize)]
pub(crate) struct SuspendedDataPayload {
    #[cfg(feature = "tck")]
    pub(crate) count: usize,

    pub(crate) code: Option<String>,
    pub(crate) reason: Option<Vec<String>>,
}

pub(crate) type SuspendedData = PendingStateData<SuspendedDataPayload>;

pub(crate) type CompletedData = PendingStateData<()>;

#[derive(Serialize, Deserialize)]
pub(crate) struct TransferProcess<V: TransferStateData> {
    pub(crate) provider_pid: String,
    pub(crate) consumer_pid: String,
    pub(crate) state: TransferState<V>,
}

impl<V: TransferStateData> TransferProcess<V> {
    pub(crate) fn invalid_state_error(self) -> AppError {
        AppError::Transfer(TransferError::invalid_state(
            self.provider_pid,
            self.consumer_pid,
        ))
    }

    pub(crate) async fn handle_event(
        self,
        event: TransferEventPayload,
    ) -> Result<(Self, Option<Option<DataAddress>>), AppError> {
        match event {
            TransferEventPayload::Start(start) => {
                let payload = match self.state {
                    TransferState::Requested(_) => StartedDataPayload {
                        #[cfg(feature = "tck")]
                        count: 1,
                    },
                    #[cfg_attr(not(feature = "tck"), allow(unused_variables))]
                    TransferState::Suspended(s) => StartedDataPayload {
                        #[cfg(feature = "tck")]
                        count: s.payload.count + 1,
                    },
                    _ => return Err(self.invalid_state_error()),
                };

                Ok((
                    Self {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: TransferState::Started(StartedData {
                            sent: true,
                            payload,
                        }),
                    },
                    Some(start.data_address),
                ))
            }
            TransferEventPayload::Suspend(code) => match self.state {
                #[cfg_attr(not(feature = "tck"), allow(unused_variables))]
                TransferState::Started(s) => Ok((
                    Self {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: TransferState::Suspended(SuspendedData {
                            sent: true,
                            payload: SuspendedDataPayload {
                                #[cfg(feature = "tck")]
                                count: s.payload.count,
                                code: code.code,
                                reason: code.reason,
                            },
                        }),
                    },
                    None,
                )),
                _ => Err(self.invalid_state_error()),
            },
            TransferEventPayload::Complete(_) => match self.state {
                TransferState::Started(_) => Ok((
                    Self {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: TransferState::Completed(CompletedData {
                            sent: true,
                            payload: (),
                        }),
                    },
                    None,
                )),
                _ => Err(self.invalid_state_error()),
            },
            TransferEventPayload::Terminate(code) => match self.state {
                TransferState::Completed(_) => Err(self.invalid_state_error()),
                _ => Ok((
                    Self {
                        provider_pid: self.provider_pid,
                        consumer_pid: self.consumer_pid,
                        state: TransferState::Terminated(TerminatedData {
                            sent: true,
                            payload: TerminatedDataPayload {
                                code: code.code,
                                reason: code.reason,
                            },
                        }),
                    },
                    None,
                )),
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum Transfer {
    Provider {
        process: TransferProcess<ProviderView>,
        agreement: Agreement,
        format: String,
        callback_address: String,
        data_address: Option<DataAddress>,
    },
    Consumer {
        process: TransferProcess<ConsumerView>,
        agreement: Agreement,
        format: String,
        connector: Connector,
        data_address: Option<DataAddress>,
    },
}

impl Transfer {
    pub(crate) fn pid(&self) -> &str {
        match self {
            Transfer::Provider { process, .. } => &process.provider_pid,
            Transfer::Consumer { process, .. } => &process.consumer_pid,
        }
    }

    pub(crate) fn state(&self) -> String {
        let (state, pending) = match self {
            Transfer::Provider { process, .. } => {
                (process.state.to_string(), process.state.is_pending())
            }
            Transfer::Consumer { process, .. } => {
                (process.state.to_string(), process.state.is_pending())
            }
        };
        let state = state.to_lowercase();
        if pending {
            format!("{state}-pending")
        } else {
            state
        }
    }
}

impl Transfer {
    pub(crate) async fn handle_event(
        self,
        event: TransferEventPayload,
    ) -> Result<Transfer, AppError> {
        Ok(match self {
            Transfer::Provider {
                process,
                agreement,
                format,
                callback_address,
                data_address,
            } => {
                let (process, _) = process.handle_event(event).await?;
                Transfer::Provider {
                    process,
                    agreement,
                    format,
                    callback_address,
                    data_address,
                }
            }
            Transfer::Consumer {
                process,
                agreement,
                format,
                connector,
                data_address,
            } => {
                let (process, new_data_address) = process.handle_event(event).await?;
                let data_address = new_data_address.unwrap_or(data_address);
                Transfer::Consumer {
                    process,
                    agreement,
                    format,
                    connector,
                    data_address,
                }
            }
        })
    }

    pub(crate) async fn tick<T: Store>(
        self,
        store: &AppStateTransfer<T>,
    ) -> anyhow::Result<Option<Transfer>> {
        Ok(match self {
            Transfer::Provider {
                process,
                agreement,
                format,
                callback_address,
                data_address,
            } => process
                .tick(store, &agreement, &callback_address)
                .await?
                .map(|p| Transfer::Provider {
                    process: p,
                    agreement,
                    format,
                    callback_address,
                    data_address,
                }),
            Transfer::Consumer {
                process,
                agreement,
                format,
                connector,
                data_address,
            } => process
                .tick(store, &agreement, &format, &connector, &data_address)
                .await?
                .map(|p| Transfer::Consumer {
                    process: p,
                    agreement,
                    format,
                    connector,
                    data_address,
                }),
        })
    }
}

impl From<Transfer> for model::transfer::TransferProcess {
    fn from(value: Transfer) -> Self {
        match value {
            Transfer::Provider { process, .. } => process.into(),
            Transfer::Consumer { process, .. } => process.into(),
        }
    }
}

pub(crate) struct TransferEvent {
    pid: String,
    payload: TransferEventPayload,
    result: oneshot::Sender<Result<Transfer, AppError>>,
}

pub(crate) enum TransferEventPayload {
    Start(TransferStart),
    Suspend(AbstractTransferCode),
    Complete(TransferCompletion),
    Terminate(AbstractTransferCode),
}

impl From<TransferEventPayload> for (String, String) {
    fn from(value: TransferEventPayload) -> Self {
        match value {
            TransferEventPayload::Start(start) => start.into(),
            TransferEventPayload::Suspend(code) => code.into(),
            TransferEventPayload::Complete(completion) => completion.into(),
            TransferEventPayload::Terminate(code) => code.into(),
        }
    }
}

pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
    Router::new()
        // provider
        .route("/{provider_pid}", get(get_transfer))
        .route("/request", post(transfer_request))
        // both
        .route("/{pid}/start", post(transfer_start))
        .route("/{pid}/suspension", post(transfer_suspension))
        .route("/{pid}/completion", post(transfer_completion))
        .route("/{pid}/termination", post(transfer_termination))
}

async fn transfer_start<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateTransfer<T>>,
    Path(pid): Path<String>,
    ValidatedJson(request): ValidatedJson<TransferStart>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Transfer start with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(TransferEvent {
            pid,
            payload: TransferEventPayload::Start(request),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    match rx.await.map_err(anyhow::Error::new)? {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => Err(err),
    }
}

async fn transfer_suspension<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateTransfer<T>>,
    Path(pid): Path<String>,
    ValidatedJson(request): ValidatedJson<AbstractTransferCode>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Transfer suspension with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(TransferEvent {
            pid,
            payload: TransferEventPayload::Suspend(request),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    match rx.await.map_err(anyhow::Error::new)? {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => Err(err),
    }
}

async fn transfer_completion<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateTransfer<T>>,
    Path(pid): Path<String>,
    ValidatedJson(request): ValidatedJson<TransferCompletion>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Transfer completion with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(TransferEvent {
            pid,
            payload: TransferEventPayload::Complete(request),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    match rx.await.map_err(anyhow::Error::new)? {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => Err(err),
    }
}

async fn transfer_termination<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateTransfer<T>>,
    Path(pid): Path<String>,
    ValidatedJson(request): ValidatedJson<AbstractTransferCode>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Transfer termination with claims: {:?}", claims);

    let (tx, rx) = oneshot::channel();
    state
        .tx
        .send(TransferEvent {
            pid,
            payload: TransferEventPayload::Terminate(request),
            result: tx,
        })
        .await
        .map_err(anyhow::Error::new)?;

    match rx.await.map_err(anyhow::Error::new)? {
        Ok(_) => Ok(StatusCode::OK),
        Err(err) => Err(err),
    }
}

pub(crate) async fn handle_transfers<T: Store>(
    token: CancellationToken,
    state: AppStateTransfer<T>,
    mut rx: Receiver<TransferEvent>,
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
                process_transfers(&state).await;
            }
        }
    }
}

async fn handle_event<T: Store>(state: &AppStateTransfer<T>, event: TransferEvent) {
    let transfer = match state.store.get_transfer(&event.pid).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(
                "Ignoring event for an unknown transfer {pid}",
                pid = event.pid
            );
            let (provider_pid, consumer_pid) = event.payload.into();
            if let Err(_) =
                event
                    .result
                    .send(Err(AppError::Transfer(TransferError::transfer_not_found(
                        provider_pid,
                        consumer_pid,
                    ))))
            {
                error!("Failed to send result");
            }
            return;
        }
        Err(err) => {
            error!(
                "Failed to fetch transfer {pid}, error: {err}",
                pid = event.pid
            );
            if let Err(_) = event.result.send(Err(AppError::Generic(err))) {
                error!("Failed to send result");
            }
            return;
        }
    };
    let mut result = transfer.handle_event(event.payload).await;
    if let Ok(transfer) = result {
        let pid = transfer.pid().to_owned();
        result = state
            .store
            .save_transfer(&transfer)
            .await
            .map(|_| transfer)
            .map_err(|err| {
                error!("Failed to store transfer {pid}, error: {err}");
                AppError::Generic(err)
            })
    }
    if let Err(_) = event.result.send(result) {
        error!("Failed to send result");
    }
}

async fn process_transfers<T: Store>(state: &AppStateTransfer<T>) {
    let transfers = match state.store.get_pending_transfers(100).await {
        Ok(n) => n,
        Err(err) => {
            error!("Failed to fetch pending transfers, error: {err}");
            return;
        }
    };

    let updated = stream::iter(transfers)
        .map(|t| async move { t.tick(state).await })
        .buffer_unordered(32)
        .collect::<Vec<_>>()
        .await;

    for t in updated {
        match t {
            Ok(Some(t)) => {
                if let Err(err) = state.store.save_transfer(&t).await {
                    error!("Failed to save transfer, error: {err}");
                }
            }
            Ok(_) => {}
            Err(err) => {
                error!("Failed to process transfer, error: {err}");
            }
        }
    }
}

pub(crate) async fn transfer<T: Store>(
    store: &T,
    agreement: Agreement,
    format: String,
    connector: Connector,
) -> anyhow::Result<String> {
    let consumer_pid = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
    let process = TransferProcess {
        provider_pid: "".into(),
        consumer_pid: consumer_pid.clone(),
        state: TransferState::Requested(consumer::RequestedData {
            sent: false,
            payload: (),
        }),
    };
    let n = Transfer::Consumer {
        process,
        agreement,
        format: format,
        connector: connector,
        data_address: None, // FIXME: depends on format
    };
    store.save_transfer(&n).await?;

    Ok(consumer_pid)
}

#[cfg(feature = "tck")]
pub(crate) mod tck {
    use axum::{Router, routing::post};

    use crate::{
        connector::app_state::AppState, store::Store, transfer::consumer::tck::transfer_request,
    };

    pub(crate) fn router<T: Store>() -> Router<AppState<T>> {
        Router::new().route("/request", post(transfer_request))
    }
}
