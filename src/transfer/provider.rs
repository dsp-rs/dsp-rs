use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::{
    AppError,
    auth::extractor::AuthClaims,
    connector::{
        Pending, TerminatedDataPayload, app_state::AppStateTransfer, post_request,
        validator::ValidatedJson,
    },
    model::{
        self,
        policy::Agreement,
        transfer::{DataAddress, TransferError, TransferRequest},
    },
    negotiation::{ContractNegotiation, Negotiation, NegotiationState},
    policy_engine::transfer::{TransferOutcome, check_transfer_policy},
    store::Store,
    transfer::{
        CompletedData, StartedData, StartedDataPayload, SuspendedData, SuspendedDataPayload,
        TerminatedData, Transfer, TransferProcess, TransferState, TransferStateData,
    },
};

#[derive(Serialize, Deserialize)]
pub(crate) struct ProviderView;

impl TransferStateData for ProviderView {
    type RequestedData = ();
}

impl TransferProcess<ProviderView> {
    pub(crate) async fn tick<T: Store>(
        #[cfg_attr(not(feature = "tck"), allow(unused_mut))] mut self,
        state: &AppStateTransfer<T>,
        #[cfg_attr(not(feature = "tck"), allow(unused_variables))] agreement: &Agreement,
        callback_address: &str,
    ) -> anyhow::Result<Option<Self>> {
        #[cfg(feature = "tck")]
        {
            self = match tck::tick(self, agreement).await {
                Ok(tck_result) => return tck_result,
                Err(this) => this,
            };
        }

        let my_did_web = state.participant_info.did_web()?;
        let did = my_did_web.clone();
        let get_access_token = || async {
            state
                .authenticator
                .get_token(&state.client, callback_address, did)
                .await
        };

        // NOTE: currently only 2025-1 supported
        // TODO: we should retrieve the metadata and determine the path for version 2025-1
        #[cfg(not(feature = "tck"))]
        let callback_address = format!(
            "{}{}",
            callback_address,
            crate::connector::DSP_API_PATH_2025_1
        );

        let pull_endpoint = format!("{}/pull", state.participant_info.external_address);

        match self.state {
            TransferState::Requested(_) => {
                // TODO: should we auto-start?
                Ok(Some(TransferProcess {
                    provider_pid: self.provider_pid,
                    consumer_pid: self.consumer_pid,
                    state: TransferState::Started(Default::default()),
                }))
            }
            TransferState::Started(s) if s.is_pending() => {
                let token = state
                    .authenticator
                    .new_transfer_token(self.provider_pid.clone(), my_did_web)?;
                let start = model::transfer::TransferProcess::start(
                    self.provider_pid,
                    self.consumer_pid,
                    Some(DataAddress::new_http_with_token(pull_endpoint, token)), // FIXME: data address only if PULL
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/start",
                        callback_address, &start.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    start,
                    |start, _: Option<()>| TransferProcess {
                        provider_pid: start.provider_pid,
                        consumer_pid: start.consumer_pid,
                        state: TransferState::Started(StartedData {
                            sent: true,
                            payload: StartedDataPayload {
                                #[cfg(feature = "tck")]
                                count: s.payload.count,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Suspended(s) if s.is_pending() => {
                let suspend = model::transfer::TransferProcess::suspend(
                    self.provider_pid,
                    self.consumer_pid,
                    s.payload.code,
                    s.payload.reason,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/suspension",
                        callback_address, &suspend.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    suspend,
                    |suspend, _: Option<()>| TransferProcess {
                        provider_pid: suspend.provider_pid,
                        consumer_pid: suspend.consumer_pid,
                        state: TransferState::Suspended(SuspendedData {
                            sent: true,
                            payload: SuspendedDataPayload {
                                #[cfg(feature = "tck")]
                                count: s.payload.count,
                                code: suspend.code,
                                reason: suspend.reason,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Completed(s) if s.is_pending() => {
                let complete = model::transfer::TransferProcess::complete(
                    self.provider_pid,
                    self.consumer_pid,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/completion",
                        callback_address, &complete.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    complete,
                    |complete, _: Option<()>| TransferProcess {
                        provider_pid: complete.provider_pid,
                        consumer_pid: complete.consumer_pid,
                        state: TransferState::Completed(CompletedData {
                            sent: true,
                            payload: (),
                        }),
                    },
                )
                .await
                .map(Some)
            }
            TransferState::Terminated(s) if s.is_pending() => {
                let terminate = model::transfer::TransferProcess::terminate(
                    self.provider_pid,
                    self.consumer_pid,
                    s.payload.code,
                    s.payload.reason,
                );
                post_request(
                    &state.client,
                    &state.validator,
                    &format!(
                        "{}/transfers/{}/termination",
                        callback_address, &terminate.consumer_pid
                    ),
                    Some(get_access_token().await?),
                    terminate,
                    |terminate, _: Option<()>| TransferProcess {
                        provider_pid: terminate.provider_pid,
                        consumer_pid: terminate.consumer_pid,
                        state: TransferState::Terminated(TerminatedData {
                            sent: true,
                            payload: TerminatedDataPayload {
                                code: terminate.code,
                                reason: terminate.reason,
                            },
                        }),
                    },
                )
                .await
                .map(Some)
            }
            _ => Ok(None),
        }
    }
}

pub(crate) async fn get_transfer<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateTransfer<T>>,
    Path(provider_pid): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Retrieving transfer with claims: {:?}", claims);

    if let Some(n) = state
        .store
        .get_transfer(&provider_pid)
        .await
        .map_err(anyhow::Error::msg)?
    {
        return Ok(Json(model::transfer::TransferProcess::from(n)));
    }

    Err(AppError::NotFound)
}

pub(crate) async fn transfer_request<T: Store>(
    claims: AuthClaims,
    State(state): State<AppStateTransfer<T>>,
    ValidatedJson(request): ValidatedJson<TransferRequest>,
) -> Result<impl IntoResponse, AppError> {
    debug!("Transfer request with claims: {:?}", claims);

    let Some(Negotiation::Provider {
        contract:
            ContractNegotiation {
                state: NegotiationState::Finalized(s),
                ..
            },
        ..
    }) = state
        .store
        .get_finalized_negotiation(&request.agreement_id)
        .await?
    else {
        return Err(AppError::Transfer(TransferError::agreement_not_found(
            "".into(),
            request.consumer_pid,
        )));
    };

    match check_transfer_policy(&s.agreement.policy_class.policy, &claims) {
        TransferOutcome::Success => {}
        TransferOutcome::Forbidden(reason) => {
            warn!(
                "Access denied for aggreement '{}', reason: {}",
                s.agreement.policy_class.resource.id, reason
            );
            return Err(AppError::Transfer(TransferError::forbidden(
                "".into(),
                request.consumer_pid,
            )));
        }
    };

    let provider_pid = format!("urn:uuid:{}", uuid::Uuid::new_v4().to_string());
    let process = TransferProcess {
        provider_pid,
        consumer_pid: request.consumer_pid,
        state: TransferState::Requested(()),
    };
    let t = Transfer::Provider {
        process,
        agreement: s.agreement,
        format: request.format,
        callback_address: request.callback_address,
        data_address: request.data_address,
    };
    state.store.save_transfer(&t).await?;

    Ok((
        StatusCode::CREATED,
        Json(model::transfer::TransferProcess::from(t)),
    ))
}

#[cfg(feature = "tck")]
mod tck {
    use crate::{
        connector::{Pending, TerminatedDataPayload},
        model::policy::Agreement,
        transfer::{
            StartedData, StartedDataPayload, SuspendedData, SuspendedDataPayload, TerminatedData,
            TransferProcess, TransferState, provider::ProviderView,
        },
    };

    type Process = TransferProcess<ProviderView>;

    pub(crate) async fn tick(
        process: Process,
        agreement: &Agreement,
    ) -> Result<anyhow::Result<Option<Process>>, Process> {
        let state = match process.state {
            TransferState::Requested(s) => match agreement.policy_class.resource.id.as_str() {
                "ATP0105" => {
                    return Ok(Ok(Some(TransferProcess {
                        provider_pid: process.provider_pid,
                        consumer_pid: process.consumer_pid,
                        state: TransferState::Terminated(TerminatedData {
                            sent: false,
                            payload: TerminatedDataPayload {
                                code: Some("999".into()),
                                reason: Some(vec!["TCK ATP0105".into()]),
                            },
                        }),
                    })));
                }
                "ATP0301" | "ATP0302" => return Ok(Ok(None)),
                _ => TransferState::Requested(s),
            },
            TransferState::Started(s) if !s.is_pending() => {
                match agreement.policy_class.resource.id.as_str() {
                    "ATP0101" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATP0101".into()]),
                                },
                            }),
                        })));
                    }
                    "ATP0102" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Completed(Default::default()),
                        })));
                    }
                    "ATP0103" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Suspended(SuspendedData {
                                sent: false,
                                payload: SuspendedDataPayload {
                                    count: s.payload.count,
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATP0103".into()]),
                                },
                            }),
                        })));
                    }
                    "ATP0104" if s.payload.count == 0 => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Suspended(SuspendedData {
                                sent: false,
                                payload: SuspendedDataPayload {
                                    count: s.payload.count,
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATP0104".into()]),
                                },
                            }),
                        })));
                    }
                    "ATP0104" if s.payload.count > 0 => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Completed(Default::default()),
                        })));
                    }
                    _ => TransferState::Started(s),
                }
            }
            TransferState::Suspended(s) if !s.is_pending() => {
                match agreement.policy_class.resource.id.as_str() {
                    "ATP0103" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Terminated(TerminatedData {
                                sent: false,
                                payload: TerminatedDataPayload {
                                    code: Some("999".into()),
                                    reason: Some(vec!["TCK ATP0103".into()]),
                                },
                            }),
                        })));
                    }
                    "ATP0104" => {
                        return Ok(Ok(Some(TransferProcess {
                            provider_pid: process.provider_pid,
                            consumer_pid: process.consumer_pid,
                            state: TransferState::Started(StartedData {
                                sent: false,
                                payload: StartedDataPayload {
                                    count: s.payload.count + 1,
                                },
                            }),
                        })));
                    }
                    _ => TransferState::Suspended(s),
                }
            }
            s => s,
        };

        Err(Process {
            provider_pid: process.provider_pid,
            consumer_pid: process.consumer_pid,
            state,
        })
    }
}
